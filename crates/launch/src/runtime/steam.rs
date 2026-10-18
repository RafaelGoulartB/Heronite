//! Steam games: open the `steam://` URI and follow the game through Steam's logs.

use heronite_i18n::tr;
use std::sync::atomic::Ordering;

use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{LaunchEvent, LaunchRequest};
use crate::log_writer::GameLog;
use crate::playtime_watch::{wait_for_steam_session, SteamWaitOutcome};
use crate::prepare::Notice;
use crate::procs::processes_matching;
use crate::record;
use crate::settings::GlobalSettings;
use crate::steam::KEY_CLOSE_AFTER_GAME;

pub(super) fn run_steam(shared: &Shared, log: &GameLog, req: &LaunchRequest, control: &Control, app_id: &str, uri: &str) {
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
            super::adopt::remember(shared, control, req, super::adopt::steam_entry(shared, control, app_id, opened_steam));
            shared.send(LaunchEvent::Running { app_name: req.app_name.clone(), pid });
        },
    );
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
            finish(shared, log, req, control, control.played(), None);
            // Stopping only ends the tracking: the game may still run.
            if opened_steam && matches!(outcome, SteamWaitOutcome::Finished) {
                close_steam(shared, &req.title);
            }
        }
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
