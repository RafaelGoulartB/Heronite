//! Normal processes: prepare, spawn, poll until the game is gone.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_i18n::tr;

use super::companions::Companions;
use super::sandboxed::Sandboxed;
use super::session::{fail, finish};
use super::spawner::ProcessHandle;
use super::state::{Control, Shared};
use super::{ExitInfo, LaunchEvent, LaunchRequest};
use crate::log_writer::{self, GameLog};
use crate::plan::{Command, ProcessPlan};
use crate::prepare::{Flow, Notice, Prep};
use crate::probe::Probe;
use crate::record;

/// Runs a command to completion (prefix creation, scripts, wineserver
/// --wait). A stop that arrives meanwhile ends it; one requested before it
/// started does not (the after-exit script still runs for a stopped game).
/// Returns false if tracking was aborted meanwhile.
pub(super) fn run_to_completion(shared: &Shared, control: &Control, cmd: &Command, log: Option<&Arc<GameLog>>) -> bool {
    if let Some(log) = log {
        log.info(&format!("Running: {}", log_writer::command_line_for_log(cmd)));
    }
    let mut handle = match shared.spawner.spawn_logged(cmd, log.cloned()) {
        Ok(h) => confined(shared, cmd, h),
        Err(e) => {
            log::warn!("{}: {e}", cmd.display());
            if let Some(log) = log {
                log.warning(&format!("could not start {}: {e}", cmd.program));
            }
            return true;
        }
    };
    let stopped_before = control.kill.load(Ordering::SeqCst);
    let mut killed_at: Option<Instant> = None;
    loop {
        match handle.poll() {
            Ok(None) => {}
            _ => return true,
        }
        if control.abort.load(Ordering::SeqCst) {
            return false;
        }
        if !stopped_before && control.kill.load(Ordering::SeqCst) && killed_at.is_none() {
            handle.terminate();
            killed_at = Some(Instant::now());
        }
        if killed_at.map(|t| t.elapsed() > shared.cfg.kill_grace).unwrap_or(false) {
            handle.kill();
            return true;
        }
        std::thread::sleep(shared.cfg.poll_interval.min(Duration::from_millis(50)));
    }
}

pub(super) fn run_process(shared: &Shared, log: &Arc<GameLog>, req: &LaunchRequest, control: &Control, plan: &ProcessPlan) {
    for dir in &plan.create_dirs {
        if let Err(e) = shared.spawner.create_dir_all(dir) {
            log::warn!("creating {dir}: {e}");
        }
    }
    let services = shared.services.lock().unwrap().clone();
    let status = |n: Notice| {
        if n.warning {
            log.warning(&n.log);
        } else {
            log.info(&n.log);
        }
        // Until the game runs, progress is the step the launch is at.
        let app_name = req.app_name.clone();
        shared.send(if !n.warning && !control.playing.load(Ordering::SeqCst) {
            LaunchEvent::Preparing { app_name, message: n.message, cancellable: !n.uncancellable }
        } else {
            LaunchEvent::Status { app_name, message: n.message, warning: n.warning }
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
    // How long each step took, for the log: a slow launch shows where.
    let mut steps = Steps::new();
    // Cloud saves down, umu runtime.
    match prep.before_prefix(&req.app_name, &plan.prep) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    steps.done("cloud saves and umu");
    // A prefix whose creation was cut short is created again next time,
    // even if wineboot had already written the files that make it look done.
    let creating = plan
        .prep
        .wine
        .as_ref()
        .filter(|_| !plan.pre_commands.is_empty())
        .map(|s| std::path::Path::new(&s.wine_prefix).join(crate::wine::CREATING_MARKER));
    if let Some(marker) = &creating {
        // Written in place, a link put there from a sandbox would be followed.
        let _ = std::fs::remove_file(marker);
        if let Err(e) = std::fs::OpenOptions::new().write(true).create_new(true).open(marker) {
            log::debug!("{}: {e}", marker.display());
        }
    }
    if !plan.pre_commands.is_empty() {
        status(Notice::info("Preparing prefix...", tr!("Preparing prefix...")));
    }
    for cmd in &plan.pre_commands {
        if control.kill.load(Ordering::SeqCst) || !run_to_completion(shared, control, cmd, Some(log)) {
            return finish_aborted(shared, log, req);
        }
    }
    if control.kill.load(Ordering::SeqCst) {
        return finish_aborted(shared, log, req);
    }
    if let Some(marker) = &creating {
        let _ = std::fs::remove_file(marker);
    }
    steps.done("prefix creation");
    // Redistributables, EOS overlay and known fixes the first time the game
    // runs in this prefix.
    let epic = shared.epic.lock().unwrap().clone();
    let store = shared.store_setup.lock().unwrap().clone();
    match prep.first_launch(&plan.prep, epic.as_deref(), store.as_deref()) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    steps.done("first launch setup");
    // DXVK / VKD3D into the prefix, anti-cheat runtimes.
    match prep.after_prefix(&plan.prep) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, log, req),
        Flow::Fail(message) => return fail(shared, log, req, message),
    }
    steps.done("Wine tools and anti-cheat runtimes");
    if cancelled() {
        // Stopped while preparing: the game must not start afterwards.
        return finish_aborted(shared, log, req);
    }
    if let Some(path) = &plan.make_executable {
        if let Err(e) = shared.spawner.make_executable(path) {
            log::warn!("chmod {path}: {e}");
        }
    }
    // After the prefix is ready, so the script can use it.
    if let Some(script) = &plan.before_script {
        status(Notice::info("Running the script before launch...", tr!("Running the script before launch...")));
        if !run_script(shared, control, &*services.probe, script, log, &status) || cancelled() {
            return finish_aborted(shared, log, req);
        }
    }
    steps.done("script before launch");
    log.info(&steps.summary());
    // MangoHud's limit moves into a file of ours so the overlay can change it.
    let mut command = plan.command.clone();
    let live = crate::live_fps::prepare(&mut command, &req.app_name, &*services.probe);
    *control.live_fps.lock().unwrap() = live;
    log.info(&format!("Launching {}: {}\n", req.title, log_writer::command_line_for_log(&command)));
    log.info("Game Output:");
    let mut handle = match shared.spawner.spawn_logged(&command, Some(log.clone())) {
        Ok(h) => match plan.sandboxed {
            true => Box::new(Sandboxed::new(h, shared.procs.clone())),
            false => h,
        },
        Err(e) => {
            let program = &plan.command.program;
            let notice =
                Notice::warn(format!("could not start {program}: {e}"), tr!("could not start {program}: {error}", program, error = e));
            return fail(shared, log, req, notice);
        }
    };
    control.pid.store(handle.pid(), Ordering::SeqCst);
    // The session clock starts here, not at the launch request.
    let started_at = control.set_playing();
    let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
    // Written down so the next run of the app can pick the game up if this
    // one quits or crashes first.
    if let Some(tracked) = super::adopt::process_entry(shared, handle.pid(), plan) {
        super::adopt::remember(shared, control, req, tracked);
    }
    shared.send(LaunchEvent::Running { app_name: req.app_name.clone(), pid: handle.pid() });
    let mut awake = plan.keep_awake.as_ref().and_then(|cmd| match shared.spawner.spawn(cmd, None) {
        Ok(h) => Some(h),
        Err(e) => {
            log.warning(&format!("Could not keep the computer awake ({}): {e}", cmd.program));
            None
        }
    });
    let mut companions = Companions::new(&plan.companions, Instant::now(), plan.sandboxed.then(|| handle.pid()));
    let prefix = plan.prep.wine.as_ref().map(|s| s.wine_prefix.clone());

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
        if terminated_at.is_none() {
            companions.start_due(shared, &*services.probe, log, &status);
            // The game is over but what started it waits for these programs.
            if let Some(prefix) = prefix.as_deref().filter(|_| companions.active()) {
                if companions.game_left(&*shared.procs, prefix) {
                    companions.stop(shared.cfg.kill_grace, log);
                }
            }
        }
        // Sleeps until the next poll unless a stop arrives first.
        let kill_pending = terminated_at.is_none();
        let wait = companions.next_due(Instant::now()).map_or(shared.cfg.poll_interval, |d| d.min(shared.cfg.poll_interval));
        control.wake.wait_while(wait, || !control.abort.load(Ordering::SeqCst) && !(kill_pending && control.kill.load(Ordering::SeqCst)));
    }

    match exit {
        Some(info) => log.info(&describe_exit(info)),
        None if aborted => log.info("Stopped tracking the game"),
        None => {}
    }
    if !aborted && !control.kill.load(Ordering::SeqCst) {
        if let Some(prefix) = prefix.filter(|_| companions.active()) {
            aborted = follow_the_prefix(shared, control, &mut companions, &prefix, &*services.probe, log, &status);
        }
    }
    if aborted {
        companions.leave_running();
    } else {
        companions.stop(shared.cfg.kill_grace, log);
    }
    if let Some(h) = awake.as_mut() {
        h.terminate();
    }
    if !aborted && !control.kill.load(Ordering::SeqCst) {
        if let Some(cmd) = &plan.wait_after_exit {
            run_to_completion(shared, control, cmd, Some(log));
        }
    }
    // The session ends here; saving to the cloud is not playtime.
    let elapsed = control.played();
    if !aborted {
        if let Some(script) = &plan.after_script {
            run_script(shared, control, &*services.probe, script, log, &status);
        }
        prep.after_exit(&req.app_name, &plan.prep);
    }
    finish(shared, log, req, control, elapsed, exit);
}

/// `handle` followed until its sandbox ends, when `cmd` starts one.
fn confined(shared: &Shared, cmd: &Command, handle: Box<dyn ProcessHandle>) -> Box<dyn ProcessHandle> {
    match crate::sandbox::is_wrapped(cmd) {
        true => Box::new(Sandboxed::new(handle, shared.procs.clone())),
        false => handle,
    }
}

/// The duration of each preparation step.
struct Steps {
    start: Instant,
    last: Instant,
    done: Vec<(&'static str, Duration)>,
}

impl Steps {
    fn new() -> Self {
        let now = Instant::now();
        Self { start: now, last: now, done: Vec::new() }
    }

    fn done(&mut self, step: &'static str) {
        let now = Instant::now();
        self.done.push((step, now - self.last));
        self.last = now;
    }

    /// "Prepared in 1.24 s (Wine tools and anti-cheat runtimes 1.20 s)":
    /// the steps that took a noticeable time.
    fn summary(&self) -> String {
        let total = self.last - self.start;
        let slow: Vec<String> = self
            .done
            .iter()
            .filter(|(_, d)| *d >= Duration::from_millis(10))
            .map(|(s, d)| format!("{s} {:.2} s", d.as_secs_f64()))
            .collect();
        if slow.is_empty() {
            format!("Prepared in {:.2} s", total.as_secs_f64())
        } else {
            format!("Prepared in {:.2} s ({})", total.as_secs_f64(), slow.join(", "))
        }
    }
}

/// Keeps the session (and the programs alongside the game) going while the
/// game still runs in its prefix after the process that started it exited.
/// Returns true when tracking was aborted meanwhile.
fn follow_the_prefix(
    shared: &Shared,
    control: &Control,
    companions: &mut Companions,
    prefix: &str,
    probe: &dyn Probe,
    log: &Arc<GameLog>,
    status: &dyn Fn(Notice),
) -> bool {
    while companions.game_may_run(&*shared.procs, prefix) {
        if control.abort.load(Ordering::SeqCst) {
            return true;
        }
        if control.kill.load(Ordering::SeqCst) {
            return false;
        }
        companions.start_due(shared, probe, log, status);
        let wait = companions.next_due(Instant::now()).map_or(shared.cfg.poll_interval, |d| d.min(shared.cfg.poll_interval));
        control.wake.wait_while(wait, || !control.abort.load(Ordering::SeqCst) && !control.kill.load(Ordering::SeqCst));
    }
    false
}

/// Runs one of the game's scripts to completion. Its exit code is ignored and
/// a missing script is only reported, as in Heroic: the game still runs.
/// Returns false if tracking was aborted meanwhile.
pub(super) fn run_script(
    shared: &Shared,
    control: &Control,
    probe: &dyn Probe,
    script: &Command,
    log: &Arc<GameLog>,
    status: &dyn Fn(Notice),
) -> bool {
    let program = &script.program;
    if !probe.exists(program) {
        status(Notice::warn(format!("Script not found: {program}"), tr!("Script not found: {program}", program)));
        return true;
    }
    if !probe.is_executable(program) {
        if let Err(e) = shared.spawner.make_executable(program) {
            log::warn!("chmod {program}: {e}");
        }
    }
    run_to_completion(shared, control, script, Some(log))
}

fn finish_aborted(shared: &Shared, log: &GameLog, req: &LaunchRequest) {
    log.info("Launch stopped before the game started");
    log.close();
    // Stopped before the game even started: nothing was played.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    shared.send(LaunchEvent::Exited { app_name: req.app_name.clone(), elapsed: Duration::ZERO, exit: None, stopped: true, record: None });
}

fn describe_exit(info: ExitInfo) -> String {
    match (info.code, info.signal) {
        (Some(c), _) => format!("Game process exited with code {c}"),
        (None, Some(s)) => format!("Game process was terminated by signal {s}"),
        _ => "Game process ended".to_string(),
    }
}
