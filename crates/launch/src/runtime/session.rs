//! The session thread: opens the log, runs the plan and ends the session.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use heronite_i18n::tr;

use super::process::run_process;
use super::state::{Control, Shared};
use super::steam::run_steam;
use super::{failed_launch, ExitInfo, LaunchEvent, LaunchRequest};
use crate::log_writer::{self, GameLog, GameLogType, HeaderInfo, LogPaths};
use crate::plan::LaunchPlan;
use crate::prepare::Notice;
use crate::record::PlayRecord;

pub(super) fn finish(shared: &Shared, log: &GameLog, req: &LaunchRequest, control: &Control, elapsed: Duration, exit: Option<ExitInfo>) {
    log.close();
    // Free the slot first so listeners can relaunch right away.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    super::adopt::forget(shared, control, &req.app_name);
    let mut outcome = None;
    // The last write goes through the session's ledger: it replaces what the
    // checkpoints stored instead of adding to it.
    let mut ledger = control.ledger.lock().unwrap();
    ledger.finished = true;
    let stopped = control.stopped();
    // A game that dies right after starting was not played: leave its history
    // alone, unless a checkpoint already stored part of the session.
    let failed = failed_launch(exit, stopped, elapsed).is_some() && !ledger.cp.has_written();
    let record = shared.cfg.record && control.record.load(Ordering::SeqCst) && !failed;
    // No start time: stopped while Steam was still starting the game, so
    // nothing was played.
    if let Some(started_at) = control.started_at().filter(|_| record) {
        let rec = PlayRecord {
            app_name: req.app_name.clone(),
            title: req.title.clone(),
            started_at,
            ended_at: started_at + chrono::Duration::milliseconds(elapsed.as_millis() as i64),
        };
        match ledger.cp.write(&shared.paths, &rec, shared.cfg.min_session_seconds) {
            Ok(o) => outcome = Some(o),
            Err(e) => {
                log::error!("recording session of {}: {e:#}", req.app_name);
                shared.send(LaunchEvent::Error {
                    app_name: req.app_name.clone(),
                    message: tr!("could not record playtime: {error}", error = format!("{e:#}")),
                });
            }
        }
    }
    drop(ledger);
    shared.send(LaunchEvent::Exited { app_name: req.app_name.clone(), elapsed, exit, stopped, record: outcome });
}

pub(super) fn fail(shared: &Shared, log: &GameLog, req: &LaunchRequest, notice: Notice) {
    log.error(&format!("Launch aborted: {}", notice.log));
    log.close();
    shared.sessions.lock().unwrap().remove(&req.app_name);
    log::error!("{}: {}", req.app_name, notice.log);
    shared.send(LaunchEvent::Error { app_name: req.app_name.clone(), message: notice.message });
}

/// Ends the session however its thread ends. The ledger is marked finished
/// (which stops the checkpointer); after a panic the slot is also freed and
/// the listeners told, or the game would stay "playing" and refuse every
/// later launch.
pub(super) struct SessionEnd {
    shared: Arc<Shared>,
    app_name: String,
    title: String,
    control: Arc<Control>,
}

impl SessionEnd {
    pub(super) fn new(shared: Arc<Shared>, app_name: &str, title: &str, control: Arc<Control>) -> Self {
        Self { shared, app_name: app_name.to_string(), title: title.to_string(), control }
    }
}

impl Drop for SessionEnd {
    fn drop(&mut self) {
        self.control.ledger.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
        self.control.wake.notify();
        if !std::thread::panicking() {
            return;
        }
        log::error!("the session thread of {} panicked", self.app_name);
        {
            // Only our own entry: a relaunch may already hold the slot.
            let mut sessions = self.shared.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if sessions.get(&self.app_name).is_some_and(|c| Arc::ptr_eq(c, &self.control)) {
                sessions.remove(&self.app_name);
            }
        }
        super::adopt::forget(&self.shared, &self.control, &self.app_name);
        let message = tr!("Lost track of {title}: the launcher hit an internal error", title = self.title);
        self.shared.send(LaunchEvent::Error { app_name: self.app_name.clone(), message });
    }
}

/// Saves the playtime of the running game every checkpoint interval, so a
/// crash, a kill or a power cut loses at most that much.
pub(super) fn spawn_checkpointer(shared: Arc<Shared>, app_name: String, control: Arc<Control>) {
    let _ = std::thread::Builder::new().name(format!("checkpoint-{app_name}")).spawn(move || {
        let finished = || control.ledger.lock().map(|l| l.finished).unwrap_or(true);
        // Until the game runs (preparing a prefix can take minutes).
        control.wake.wait_while(Duration::MAX, || !finished() && !control.playing.load(Ordering::SeqCst));
        loop {
            let interval = Duration::from_millis(shared.checkpoint_ms.load(Ordering::SeqCst));
            control.wake.wait_while(interval, || !finished());
            if finished() {
                return;
            }
            shared.checkpoint(&control, &app_name);
        }
    });
}

pub(super) fn run_session(shared: Arc<Shared>, req: LaunchRequest, control: Arc<Control>) {
    let _end = SessionEnd::new(shared.clone(), &req.app_name, &req.title, control.clone());
    spawn_checkpointer(shared.clone(), req.app_name.clone(), control.clone());
    let log = open_game_log(&shared, &req);
    if let LaunchPlan::Unavailable(r) = &req.plan {
        return fail(&shared, &log, &req, Notice::warn(r.clone(), r.clone()));
    }
    // Process plans put the mods in place inside `run_process`, after the
    // launch preparation: what the prefix, the store setup and the fixes
    // write into the game folder must land there, not in the overwrite folder.
    match &req.plan {
        LaunchPlan::SteamUri { app_id, uri } => run_steam(&shared, &log, &req, &control, app_id, uri),
        other => run_process(&shared, &log, &req, &control, other.process().expect("process plan")),
    }
    // Every path above closed it already; this covers a panic-free early return.
    log.close();
}

/// Puts the game's mods in place (see [`super::GameMods`]); the result is held
/// until the session ends, dropping it takes the mods away.
pub(super) fn apply_mods(
    shared: &Shared,
    log: &GameLog,
    req: &LaunchRequest,
    control: &Control,
) -> Result<Option<super::AppliedMods>, Notice> {
    let Some(mods) = shared.mods.lock().unwrap().clone() else {
        return Ok(None);
    };
    let mut status = |message: &str| {
        if !control.playing.load(Ordering::SeqCst) {
            shared.send(LaunchEvent::Preparing { app_name: req.app_name.clone(), message: message.to_string(), cancellable: false });
        }
    };
    match mods.apply(&req.app_name, &req.runner, &mut status) {
        Ok(Some(applied)) => {
            log.info(&applied.summary);
            Ok(Some(applied))
        }
        Ok(None) => Ok(None),
        Err(message) => Err(Notice::warn(format!("mods: {message}"), message)),
    }
}

/// Takes away the mods of an adopted game that ended (see [`super::GameMods::release`]).
pub(super) fn release_mods(shared: &Shared, log: &GameLog, app_name: &str, runner: &str) {
    let Some(mods) = shared.mods.lock().unwrap().clone() else { return };
    mods.release(app_name, runner);
    log.info("Mods taken away");
}

/// Adds the mods' variables to the command's; `WINEDLLOVERRIDES` lists are
/// joined (the command's own entries stay, the mods' are added after).
pub(super) fn add_env(env: &mut crate::env::Env, add: &[(String, String)]) {
    for (key, value) in add {
        let merged = match env.get(key) {
            Some(old) if key == "WINEDLLOVERRIDES" && !old.is_empty() => format!("{old};{value}"),
            _ => value.clone(),
        };
        env.insert(key.clone(), merged);
    }
}

/// Creates the game's launch log and writes what Heroic's `prepareLaunch`
/// writes first: title, native or not, install folder, system info and the
/// settings in use. Honours `disableLogs` and `verboseLogs`.
fn open_game_log(shared: &Shared, req: &LaunchRequest) -> Arc<GameLog> {
    let (disabled_globally, verbose, merged) = log_writer::launch_log_switches(&shared.paths, &req.app_name);
    let path = LogPaths::for_data(&shared.paths).game(&req.app_name, &req.runner, GameLogType::Launch);
    let log = GameLog::new(path, disabled_globally || !verbose);
    if !verbose {
        log_writer::write_disabled_notice(&log);
    }
    if log.is_disabled() {
        return log;
    }
    let process = req.plan.process();
    let native = matches!(req.plan, LaunchPlan::Native(_) | LaunchPlan::Emulator { .. } | LaunchPlan::SteamUri { .. });
    let system_info = shared.system_info.lock().unwrap().clone().map(|f| f());
    let settings_path = shared.paths.game_config(&req.app_name);
    let mut notes = process.map(|p| p.notes.clone()).unwrap_or_default();
    if let LaunchPlan::SteamUri { uri, .. } = &req.plan {
        notes.push(format!("Steam game, opened through {uri}"));
    }
    let installed_in = process.and_then(|p| p.command.cwd.clone());
    log_writer::write_launch_header(
        &log,
        &HeaderInfo {
            title: &req.title,
            runner: &req.runner,
            app_name: &req.app_name,
            native,
            installed_in: installed_in.as_deref(),
            system_info: system_info.as_deref(),
            settings: &merged,
            settings_path: &settings_path,
            notes: &notes,
        },
    );
    log
}
