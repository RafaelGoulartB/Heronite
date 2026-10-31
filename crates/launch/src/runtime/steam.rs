//! Steam games: open the `steam://` URI and follow the game through Steam's logs.
//! On Windows the game's scripts run before and after, and its programs
//! alongside while it runs (see [`crate::steam_extras`]); Steam still starts
//! the game itself.

use heronite_i18n::tr;
use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::companions::Companions;
use super::process::{finish_aborted, run_script};
use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{LaunchEvent, LaunchRequest};
use crate::log_writer::GameLog;
use crate::plan::SteamExtras;
use crate::playtime_watch::{wait_for_steam_session, SteamWaitOutcome};
use crate::prepare::Notice;
use crate::procs::processes_matching;
use crate::record;
use crate::settings::GlobalSettings;
use crate::steam::KEY_CLOSE_AFTER_GAME;

pub(super) fn run_steam(
    shared: &Shared,
    log: &Arc<GameLog>,
    req: &LaunchRequest,
    control: &Control,
    app_id: &str,
    uri: &str,
    extras: &SteamExtras,
) {
    let probe = shared.services.lock().unwrap().probe.clone();
    let status = |n: Notice| {
        if n.warning {
            log.warning(&n.log);
        } else {
            log.info(&n.log);
        }
        let app_name = req.app_name.clone();
        shared.send(if !n.warning && !control.playing.load(Ordering::SeqCst) {
            LaunchEvent::Preparing { app_name, message: n.message, cancellable: !n.uncancellable }
        } else {
            LaunchEvent::Status { app_name, message: n.message, warning: n.warning }
        })
    };
    if let Some(script) = &extras.before_script {
        status(Notice::info("Running the script before launch...", tr!("Running the script before launch...")));
        let cancelled = || control.abort.load(Ordering::SeqCst) || control.kill.load(Ordering::SeqCst);
        if !run_script(shared, control, &*probe, script, log, &status) || cancelled() {
            return finish_aborted(shared, log, req);
        }
    }
    log.info(&format!("Opening {uri}"));
    shared.send(LaunchEvent::Preparing {
        app_name: req.app_name.clone(),
        message: tr!("Waiting for Steam to start the game...").into(),
        cancellable: true,
    });
    // Steam closes after the game only when this launch is what opened it.
    let opened_steam = !shared.steam.client_running(&*shared.procs);
    if let Err(e) = shared.spawner.open_uri(uri) {
        return fail(
            shared,
            log,
            req,
            Notice::warn(format!("could not open {uri}: {e}"), tr!("could not open {uri}: {error}", uri, error = e)),
        );
    }
    let locator = shared.steam.clone();
    let mut announced: Option<u32> = None;
    // Started once Steam's logs show the game; their delays count from then.
    let companions: RefCell<Option<Companions>> = RefCell::new(None);
    let outcome = wait_for_steam_session(
        app_id,
        &*shared.procs,
        &move || locator.log_files(),
        &shared.cfg.steam,
        &control.abort,
        &control.wake,
        &control.steam_pids,
        &mut |pid| {
            announced = Some(pid);
            log.info(&format!("Steam started the game (pid {pid})"));
            control.pid.store(pid, Ordering::SeqCst);
            // Only now do we know the game is really running: the session
            // clock starts here, not when Steam was asked to open it.
            let started_at = control.set_playing();
            let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
            super::adopt::remember(
                shared,
                control,
                req,
                super::adopt::steam_entry(shared, control, app_id, opened_steam, extras.after_script.clone()),
            );
            shared.send(LaunchEvent::Running { app_name: req.app_name.clone(), pid });
            if !extras.companions.is_empty() {
                *companions.borrow_mut() = Some(Companions::new(&extras.companions, Instant::now(), None));
            }
        },
        &mut || {
            if let Some(c) = companions.borrow_mut().as_mut() {
                c.start_due(shared, &*probe, log, &status);
            }
        },
    );
    let mut companions = companions.into_inner();
    match outcome {
        SteamWaitOutcome::NotStarted => {
            let seconds = shared.cfg.steam.start_timeout.as_secs();
            let title = &req.title;
            let notice = Notice::warn(
                format!("Steam did not start {title} within {seconds}s"),
                tr!("Steam did not start {title} within {seconds}s", title, seconds),
            );
            fail(shared, log, req, notice)
        }
        SteamWaitOutcome::Finished | SteamWaitOutcome::Aborted => {
            log.info("Steam game finished");
            let finished = matches!(outcome, SteamWaitOutcome::Finished);
            if let Some(c) = companions.as_mut() {
                if finished || game_gone(shared, control) {
                    c.stop(shared.cfg.kill_grace, log);
                }
            }
            // The session ends here; the script after it is not playtime.
            let elapsed = control.played();
            if finished {
                if let Some(script) = &extras.after_script {
                    run_script(shared, control, &*probe, script, log, &status);
                }
            }
            finish(shared, log, req, control, elapsed, None);
            // Stopping only ends the tracking: the game may still run.
            if opened_steam && matches!(outcome, SteamWaitOutcome::Finished) {
                close_steam(shared, &req.title);
            }
        }
    }
}

/// Whether the game is gone after its tracking stopped: Stop in the app ends
/// its processes too (they get the grace period), so its programs alongside
/// close with it; tracking dropped while the game runs leaves them running.
fn game_gone(shared: &Shared, control: &Control) -> bool {
    let pids: Vec<u32> = control.steam_pids.lock().unwrap().iter().copied().collect();
    let deadline = Instant::now() + shared.cfg.kill_grace;
    loop {
        if pids.iter().all(|p| !shared.procs.is_alive(*p)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Closes Steam (opened for `title`, which just ended) when the setting asks
/// for it, unless it is still needed: another Steam game this app follows, or
/// any game Steam runs (its `reaper`), started from Steam itself meanwhile.
pub(super) fn close_steam(shared: &Shared, title: &str) {
    if !GlobalSettings::load(&shared.paths).flag(KEY_CLOSE_AFTER_GAME) || !shared.steam.client_running(&*shared.procs) {
        return;
    }
    if shared.sessions.lock().unwrap().values().any(|c| c.steam) || !processes_matching("reaper", 0).is_empty() {
        log::info!("Steam stays open after {title}: a game still uses it");
        return;
    }
    log::info!("closing Steam, opened for {title}");
    if let Err(e) = shared.spawner.open_uri("steam://exit") {
        log::warn!("could not close Steam: {e}");
    }
}
