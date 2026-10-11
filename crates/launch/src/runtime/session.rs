//! The session thread: opens the log, runs the plan and ends the session.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use super::process::run_process;
use super::state::{Control, Shared};
use super::steam::run_steam;
use super::{ExitInfo, LaunchEvent, LaunchRequest};
use crate::log_writer::{self, GameLog, GameLogType, HeaderInfo, LogPaths};
use crate::plan::LaunchPlan;
use crate::record::PlayRecord;

pub(super) fn finish(
    shared: &Shared,
    log: &GameLog,
    req: &LaunchRequest,
    control: &Control,
    started_at: chrono::DateTime<Utc>,
    elapsed: Duration,
    exit: Option<ExitInfo>,
) {
    log.close();
    // Free the slot first so listeners can relaunch right away.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    let mut outcome = None;
    // The last write goes through the session's ledger: it replaces what the
    // checkpoints stored instead of adding to it.
    let mut ledger = control.ledger.lock().unwrap();
    ledger.finished = true;
    if shared.cfg.record && control.record.load(Ordering::SeqCst) {
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
                    message: format!("could not record playtime: {e:#}"),
                });
            }
        }
    }
    drop(ledger);
    shared.send(LaunchEvent::Exited {
        app_name: req.app_name.clone(),
        elapsed,
        exit,
        record: outcome,
    });
}

pub(super) fn fail(shared: &Shared, log: &GameLog, req: &LaunchRequest, message: String) {
    log.error(&format!("Launch aborted: {message}"));
    log.close();
    shared.sessions.lock().unwrap().remove(&req.app_name);
    log::error!("{}: {message}", req.app_name);
    shared.send(LaunchEvent::Error {
        app_name: req.app_name.clone(),
        message,
    });
}

/// Marks the session's ledger finished however the session thread ends (also
/// on a panic), which stops its checkpointer.
struct SessionEnd(Arc<Control>);

impl Drop for SessionEnd {
    fn drop(&mut self) {
        self.0.ledger.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
        self.0.wake.notify();
    }
}

/// Saves the playtime of the running game every checkpoint interval, so a
/// crash, a kill or a power cut loses at most that much.
fn spawn_checkpointer(shared: Arc<Shared>, app_name: String, control: Arc<Control>) {
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
    let (started_at, t0) = (control.started_at, control.t0);
    let _end = SessionEnd(control.clone());
    spawn_checkpointer(shared.clone(), req.app_name.clone(), control.clone());
    let log = open_game_log(&shared, &req);
    match &req.plan {
        LaunchPlan::SteamUri { app_id, uri } => {
            run_steam(&shared, &log, &req, &control, app_id, uri, started_at, t0)
        }
        LaunchPlan::Unavailable(r) => fail(&shared, &log, &req, r.clone()),
        other => {
            let process = other.process().expect("process plan").clone();
            run_process(&shared, &log, &req, &control, &process, started_at, t0);
        }
    }
    // Every path above closed it already; this covers a panic-free early return.
    log.close();
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
