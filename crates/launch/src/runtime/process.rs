//! Normal processes: prepare, spawn, poll until the game is gone.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;

use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{ExitInfo, LaunchEvent, LaunchRequest};
use crate::log_writer::{self, GameLog};
use crate::plan::{Command, ProcessPlan};
use crate::prepare::{Flow, Notice, Prep};
use crate::record;

/// Runs a command to completion (prefix creation, wineserver --wait).
/// Returns false if tracking was aborted meanwhile.
fn run_to_completion(shared: &Shared, control: &Control, cmd: &Command, log: Option<&Arc<GameLog>>) -> bool {
    if let Some(log) = log {
        log.info(&format!("Running: {}", log_writer::command_line_for_log(cmd)));
    }
    let mut handle = match shared.spawner.spawn_logged(cmd, log.cloned()) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("{}: {e}", cmd.display());
            return true;
        }
    };
    let mut killed_at: Option<Instant> = None;
    loop {
        match handle.poll() {
            Ok(None) => {}
            _ => return true,
        }
        if control.abort.load(Ordering::SeqCst) {
            return false;
        }
        if control.kill.load(Ordering::SeqCst) && killed_at.is_none() {
            handle.terminate();
            killed_at = Some(Instant::now());
        }
        if killed_at
            .map(|t| t.elapsed() > shared.cfg.kill_grace)
            .unwrap_or(false)
        {
            handle.kill();
            return true;
        }
        std::thread::sleep(shared.cfg.poll_interval.min(Duration::from_millis(50)));
    }
}

pub(super) fn run_process(
    shared: &Shared,
    log: &Arc<GameLog>,
    req: &LaunchRequest,
    control: &Control,
    plan: &ProcessPlan,
    started_at: chrono::DateTime<Utc>,
    t0: Instant,
) {
    for dir in &plan.create_dirs {
        if let Err(e) = shared.spawner.create_dir_all(dir) {
            log::warn!("creating {dir}: {e}");
        }
    }
    let services = shared.services.lock().unwrap().clone();
    let status = |n: Notice| {
        if n.warning {
            log.warning(&n.message);
        } else {
            log.info(&n.message);
        }
        shared.send(LaunchEvent::Status {
            app_name: req.app_name.clone(),
            message: n.message,
            warning: n.warning,
        })
    };
    let cancelled = || control.abort.load(Ordering::SeqCst) || control.kill.load(Ordering::SeqCst);
    let prep = Prep {
        paths: &shared.paths,
        probe: &*services.probe,
        net: &*services.net,
        spawner: &*shared.spawner,
        cloud: services.cloud.as_deref(),
        cancel: &cancelled,
        notify: &status,
    };
    // Cloud saves down, umu runtime.
    match prep.before_prefix(&req.app_name, &plan.prep) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req, t0),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    if !plan.pre_commands.is_empty() {
        status(Notice::info("Preparing prefix..."));
    }
    for cmd in &plan.pre_commands {
        if control.kill.load(Ordering::SeqCst) || !run_to_completion(shared, control, cmd, Some(log)) {
            return finish_aborted(shared, log, req, t0);
        }
    }
    // Redistributables, EOS overlay and known fixes the first time the game
    // runs in this prefix.
    let epic = shared.epic.lock().unwrap().clone();
    match prep.first_launch(&plan.prep, epic.as_deref()) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req, t0),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    // DXVK / VKD3D into the prefix, anti-cheat runtimes.
    match prep.after_prefix(&plan.prep) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req, t0),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    if cancelled() {
        // Stopped while preparing: the game must not start afterwards.
        return finish_aborted(shared, log, req, t0);
    }
    if let Some(path) = &plan.make_executable {
        if let Err(e) = shared.spawner.make_executable(path) {
            log::warn!("chmod {path}: {e}");
        }
    }
    log.info(&format!(
        "Launching {}: {}\n",
        req.title,
        log_writer::command_line_for_log(&plan.command)
    ));
    log.info("Game Output:");
    let mut handle = match shared.spawner.spawn_logged(&plan.command, Some(log.clone())) {
        Ok(h) => h,
        Err(e) => {
            return fail(
                shared,
                log,
                req,
                format!("could not start {}: {e}", plan.command.program),
            )
        }
    };
    control.pid.store(handle.pid(), Ordering::SeqCst);
    let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
    control.playing.store(true, Ordering::SeqCst);
    shared.send(LaunchEvent::Running {
        app_name: req.app_name.clone(),
        pid: handle.pid(),
    });

    let mut terminated_at: Option<Instant> = None;
    let mut exit: Option<ExitInfo> = None;
    let mut aborted = false;
    loop {
        match handle.poll() {
            Ok(Some(info)) => {
                exit = Some(info);
                break;
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("polling {}: {e}", req.app_name);
                break;
            }
        }
        if control.abort.load(Ordering::SeqCst) {
            aborted = true;
            break;
        }
        if control.kill.load(Ordering::SeqCst) {
            match terminated_at {
                None => {
                    handle.terminate();
                    terminated_at = Some(Instant::now());
                }
                Some(t) if t.elapsed() > shared.cfg.kill_grace => {
                    handle.kill();
                    // Give the kernel a moment, then stop waiting for stragglers.
                    if t.elapsed() > shared.cfg.kill_grace + Duration::from_secs(2) {
                        break;
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(shared.cfg.poll_interval);
    }

    match exit {
        Some(info) => log.info(&describe_exit(info)),
        None if aborted => log.info("Stopped tracking the game"),
        None => {}
    }
    if !aborted && !control.kill.load(Ordering::SeqCst) {
        if let Some(cmd) = &plan.wait_after_exit {
            run_to_completion(shared, control, cmd, Some(log));
        }
    }
    // The session ends here; saving to the cloud is not playtime.
    let elapsed = t0.elapsed();
    if !aborted {
        prep.after_exit(&req.app_name, &plan.prep);
    }
    finish(shared, log, req, control, started_at, elapsed, exit);
}

fn finish_aborted(shared: &Shared, log: &GameLog, req: &LaunchRequest, t0: Instant) {
    log.info("Launch stopped before the game started");
    log.close();
    // Stopped before the game even started: nothing was played.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    shared.send(LaunchEvent::Exited {
        app_name: req.app_name.clone(),
        elapsed: t0.elapsed(),
        exit: None,
        record: None,
    });
}

fn describe_exit(info: ExitInfo) -> String {
    match (info.code, info.signal) {
        (Some(c), _) => format!("Game process exited with code {c}"),
        (None, Some(s)) => format!("Game process was terminated by signal {s}"),
        _ => "Game process ended".to_string(),
    }
}
