//! Running plans: spawning, tracking until the game is really gone, stopping,
//! and recording the session. All side effects on processes go through the
//! [`Spawner`] and [`crate::procs::Procs`] traits so tests use fakes.
//!
//! Like Heroic's backend, several games may run at once (one per app name);
//! launching an app that is already tracked is refused.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use heronite_data::HeroicPaths;

use crate::net::{Net, SystemNet};
use crate::plan::{Command, LaunchPlan, ProcessPlan, StopSpec};
use crate::playtime_watch::{wait_for_steam_session, SteamWaitConfig, SteamWaitOutcome};
use crate::first_launch::EpicSetup;
use crate::prepare::{CloudSync, Flow, Notice, Prep};
use crate::probe::{Probe, SystemProbe};
use crate::procs::{signal_tree, Procs, Sig, SystemProcs};
use crate::record::{self, PlayRecord, RecordOutcome, MIN_SESSION_SECONDS};
use crate::steam::{desktop_protocol_env, SteamLocator};

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// A spawned process (and its output pipe, if the spawner owns one).
pub trait ProcessHandle: Send {
    fn pid(&self) -> u32;
    /// `None` while running. `Some` once the process has exited *and* every
    /// holder of its output pipe is gone (what Node's `close` event means).
    fn poll(&mut self) -> io::Result<Option<ExitInfo>>;
    /// SIGTERM to the process tree.
    fn terminate(&mut self);
    /// SIGKILL to the process tree.
    fn kill(&mut self);
}

pub trait Spawner: Send + Sync {
    /// Starts `cmd`; its stdout/stderr go to `log_path` when given.
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>>;
    /// Opens a URI with the desktop handler (`xdg-open`), without waiting.
    fn open_uri(&self, uri: &str) -> io::Result<()>;
    /// `pkill -f <pattern>`.
    fn kill_pattern(&self, pattern: &str);
    fn make_executable(&self, path: &str) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o775);
        std::fs::set_permissions(path, perms)
    }
    fn create_dir_all(&self, path: &str) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }
}

// ---------------------------------------------------------------------------
// Real spawner
// ---------------------------------------------------------------------------

/// Spawns real processes. Every game gets its own process group so the whole
/// tree can be stopped, and a pipe pumped into the log file.
#[derive(Clone)]
pub struct SystemSpawner {
    procs: Arc<dyn Procs>,
}

impl Default for SystemSpawner {
    fn default() -> Self {
        Self {
            procs: Arc::new(SystemProcs),
        }
    }
}

impl SystemSpawner {
    pub fn new(procs: Arc<dyn Procs>) -> Self {
        Self { procs }
    }
}

struct SystemHandle {
    child: Option<std::process::Child>,
    pid: u32,
    exit: Option<ExitInfo>,
    pump_done: Arc<AtomicBool>,
    procs: Arc<dyn Procs>,
}

impl SystemHandle {
    fn signal_group(&self, sig: libc::c_int) {
        if self.pid > 1 {
            // SAFETY: signalling the process group we created for this child.
            unsafe {
                libc::kill(-(self.pid as libc::pid_t), sig);
            }
        }
    }
}

impl Drop for SystemHandle {
    fn drop(&mut self) {
        // Tracking was dropped while the process may still run (force clear):
        // reap it whenever it ends so it never lingers as a zombie.
        if let (None, Some(mut child)) = (self.exit, self.child.take()) {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

impl ProcessHandle for SystemHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
        if self.exit.is_none() {
            let Some(child) = self.child.as_mut() else {
                return Ok(None);
            };
            if let Some(status) = child.try_wait()? {
                self.exit = Some(ExitInfo {
                    code: status.code(),
                    signal: status.signal(),
                });
            }
        }
        match self.exit {
            Some(info) if self.pump_done.load(Ordering::SeqCst) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn terminate(&mut self) {
        signal_tree(&*self.procs, self.pid, Sig::Term);
        self.signal_group(libc::SIGTERM);
    }

    fn kill(&mut self) {
        signal_tree(&*self.procs, self.pid, Sig::Kill);
        self.signal_group(libc::SIGKILL);
    }
}

impl Spawner for SystemSpawner {
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        let mut c = std::process::Command::new(&cmd.program);
        c.args(&cmd.args)
            .envs(&cmd.env)
            .stdin(std::process::Stdio::null());
        if let Some(dir) = cmd.cwd.as_deref().filter(|d| !d.is_empty()) {
            c.current_dir(dir);
        }
        c.process_group(0);
        let (mut reader, writer) = std::io::pipe()?;
        c.stdout(writer.try_clone()?).stderr(writer);
        let child = c.spawn()?;
        // Drop our copies of the write end, or the pipe never reaches EOF.
        drop(c);

        let pump_done = Arc::new(AtomicBool::new(false));
        let done = pump_done.clone();
        let header = format!("Launching: {}\n\nGame Log:\n", cmd.display());
        let log: Option<PathBuf> = log_path.map(Path::to_path_buf);
        std::thread::spawn(move || {
            let mut file = log.as_deref().and_then(|p| {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::File::create(p).ok()
            });
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(header.as_bytes());
            }
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Some(f) = file.as_mut() {
                            let _ = f.write_all(&buf[..n]);
                        }
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
        });

        let pid = child.id();
        Ok(Box::new(SystemHandle {
            child: Some(child),
            pid,
            exit: None,
            pump_done,
            procs: self.procs.clone(),
        }))
    }

    fn open_uri(&self, uri: &str) -> io::Result<()> {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(uri)
            .env_clear()
            .envs(desktop_protocol_env(std::env::vars()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        let mut child = c.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn kill_pattern(&self, pattern: &str) {
        let _ = std::process::Command::new("pkill")
            .args(["-f", pattern])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

// ---------------------------------------------------------------------------
// Events and API types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum LaunchEvent {
    /// Accepted; the process (or the Steam URI) is being started.
    Launching { app_name: String },
    /// The game process exists. For Steam this is the pid found in Steam's logs.
    Running { app_name: String, pid: u32 },
    /// The game is gone (or tracking was cleared). `record` is what was
    /// written to the data files, `None` if recording failed or was disabled.
    Exited {
        app_name: String,
        elapsed: Duration,
        exit: Option<ExitInfo>,
        record: Option<RecordOutcome>,
    },
    /// Spawning failed, Steam never started the game, or recording failed.
    Error { app_name: String, message: String },
    /// Progress or a non-fatal problem while the launch is prepared or
    /// finished ("Preparing prefix...", "Downloading DXVK...", a cloud save
    /// error). The game keeps going; `warning` says something went wrong.
    Status {
        app_name: String,
        message: String,
        warning: bool,
    },
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    pub plan: LaunchPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    Unavailable(String),
    AlreadyRunning,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Unavailable(r) => write!(f, "cannot launch: {r}"),
            LaunchError::AlreadyRunning => write!(f, "this game is already running"),
        }
    }
}

impl std::error::Error for LaunchError {}

#[derive(Debug, Clone)]
pub struct LauncherConfig {
    /// How often a running process is polled.
    pub poll_interval: Duration,
    /// After SIGTERM, how long before SIGKILL.
    pub kill_grace: Duration,
    /// Sessions shorter than this are not stored (Heroic: 5 s).
    pub min_session_seconds: i64,
    pub steam: SteamWaitConfig,
    /// Write timestamps / sessions / recent games on exit.
    pub record: bool,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            kill_grace: Duration::from_secs(5),
            min_session_seconds: MIN_SESSION_SECONDS,
            steam: SteamWaitConfig::default(),
            record: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StopOptions {
    /// Steam games: also SIGTERM the game processes found in Steam's logs.
    /// Heronite's own stop only ends tracking for Steam games.
    pub kill_steam_processes: bool,
}

// ---------------------------------------------------------------------------
// Launcher
// ---------------------------------------------------------------------------

struct Control {
    /// Stop tracking (force clear, or the end of a stop).
    abort: AtomicBool,
    /// Kill the tracked process tree.
    kill: AtomicBool,
    /// Record the session when tracking ends (default true).
    record: AtomicBool,
    /// Steam games are tracked through logs; stopping only ends tracking.
    steam: bool,
    pid: AtomicU32,
    stop: StopSpec,
    steam_pids: Arc<Mutex<BTreeSet<u32>>>,
}

/// What the pre-launch preparation talks to (replaceable for tests and the
/// app: the Epic crate provides the cloud saves).
#[derive(Clone)]
pub struct PrepServices {
    pub probe: Arc<dyn Probe>,
    pub net: Arc<dyn Net>,
    pub cloud: Option<Arc<dyn CloudSync>>,
}

impl Default for PrepServices {
    fn default() -> Self {
        Self {
            probe: Arc::new(SystemProbe),
            net: Arc::new(SystemNet),
            cloud: None,
        }
    }
}

struct Shared {
    services: Mutex<PrepServices>,
    epic: Mutex<Option<Arc<dyn EpicSetup>>>,
    paths: HeroicPaths,
    spawner: Arc<dyn Spawner>,
    procs: Arc<dyn Procs>,
    steam: SteamLocator,
    cfg: LauncherConfig,
    tx: Mutex<Sender<LaunchEvent>>,
    sessions: Mutex<HashMap<String, Arc<Control>>>,
}

impl Shared {
    fn send(&self, ev: LaunchEvent) {
        let _ = self.tx.lock().unwrap().send(ev);
    }
}

/// Tracks running games. Cheap to clone.
#[derive(Clone)]
pub struct Launcher {
    shared: Arc<Shared>,
}

impl Launcher {
    pub fn new(
        paths: HeroicPaths,
        steam: SteamLocator,
        spawner: Arc<dyn Spawner>,
        procs: Arc<dyn Procs>,
        cfg: LauncherConfig,
        events: Sender<LaunchEvent>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                services: Mutex::new(PrepServices::default()),
                epic: Mutex::new(None),
                paths,
                spawner,
                procs,
                steam,
                cfg,
                tx: Mutex::new(events),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Replaces what launch preparation uses (network, machine, cloud saves).
    pub fn set_prep_services(&self, services: PrepServices) {
        *self.shared.services.lock().unwrap() = services;
    }

    /// What the first launch of an Epic game in a prefix asks legendary
    /// (prerequisites, EOS overlay).
    pub fn set_epic_setup(&self, epic: Arc<dyn EpicSetup>) {
        *self.shared.epic.lock().unwrap() = Some(epic);
    }

    /// Real processes, default timings.
    pub fn system(paths: HeroicPaths, steam: SteamLocator, events: Sender<LaunchEvent>) -> Self {
        let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
        Self::new(
            paths,
            steam,
            Arc::new(SystemSpawner::new(procs.clone())),
            procs,
            LauncherConfig::default(),
            events,
        )
    }

    pub fn is_running(&self, app_name: &str) -> bool {
        self.shared.sessions.lock().unwrap().contains_key(app_name)
    }

    pub fn running_apps(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .shared
            .sessions
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// Pid of the tracked process, once known.
    pub fn pid_of(&self, app_name: &str) -> Option<u32> {
        let s = self.shared.sessions.lock().unwrap();
        s.get(app_name)
            .map(|c| c.pid.load(Ordering::SeqCst))
            .filter(|p| *p != 0)
    }

    /// Starts the game in the background; progress arrives as [`LaunchEvent`]s.
    pub fn launch(&self, req: LaunchRequest) -> Result<(), LaunchError> {
        if let LaunchPlan::Unavailable(reason) = &req.plan {
            return Err(LaunchError::Unavailable(reason.clone()));
        }
        let stop = req
            .plan
            .process()
            .map(|p| p.stop.clone())
            .unwrap_or_default();
        let control = Arc::new(Control {
            abort: AtomicBool::new(false),
            kill: AtomicBool::new(false),
            record: AtomicBool::new(true),
            steam: matches!(req.plan, LaunchPlan::SteamUri { .. }),
            pid: AtomicU32::new(0),
            stop,
            steam_pids: Arc::new(Mutex::new(BTreeSet::new())),
        });
        {
            let mut sessions = self.shared.sessions.lock().unwrap();
            if sessions.contains_key(&req.app_name) {
                return Err(LaunchError::AlreadyRunning);
            }
            sessions.insert(req.app_name.clone(), control.clone());
        }
        self.shared.send(LaunchEvent::Launching {
            app_name: req.app_name.clone(),
        });
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name(format!("launch-{}", req.app_name))
            .spawn(move || run_session(shared, req, control))
            .map_err(|e| LaunchError::Unavailable(e.to_string()))?;
        Ok(())
    }

    /// Stops a game: kills its process tree (not for Steam games unless
    /// asked), runs the plan's stop commands and `pkill` patterns.
    /// Returns false if the game is not being tracked.
    pub fn stop(&self, app_name: &str) -> bool {
        self.stop_with(app_name, StopOptions::default())
    }

    pub fn stop_with(&self, app_name: &str, opts: StopOptions) -> bool {
        let Some(control) = self.shared.sessions.lock().unwrap().get(app_name).cloned() else {
            return false;
        };
        let steam_pids: Vec<u32> = control.steam_pids.lock().unwrap().iter().copied().collect();
        if opts.kill_steam_processes {
            for pid in &steam_pids {
                signal_tree(&*self.shared.procs, *pid, Sig::Term);
            }
        }
        if control.steam {
            // Heronite's stop only ends tracking for Steam games.
            control.abort.store(true, Ordering::SeqCst);
        } else {
            control.kill.store(true, Ordering::SeqCst);
        }
        let spawner = self.shared.spawner.clone();
        let stop = control.stop.clone();
        // Off the caller's thread: wineserver -k / pkill can take a moment.
        std::thread::spawn(move || {
            for pattern in &stop.kill_patterns {
                if pattern.len() >= 3 {
                    spawner.kill_pattern(pattern);
                }
            }
            for cmd in &stop.commands {
                if let Ok(mut h) = spawner.spawn(cmd, None) {
                    let deadline = Instant::now() + Duration::from_secs(15);
                    while Instant::now() < deadline {
                        if !matches!(h.poll(), Ok(None)) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        });
        true
    }

    /// "Force clear playing": forget the game without killing anything. The
    /// session is still recorded with the time elapsed so far (as Heroic does).
    pub fn force_clear(&self, app_name: &str) -> bool {
        self.force_clear_with(app_name, true)
    }

    pub fn force_clear_with(&self, app_name: &str, record: bool) -> bool {
        let Some(control) = self.shared.sessions.lock().unwrap().get(app_name).cloned() else {
            return false;
        };
        control.record.store(record, Ordering::SeqCst);
        control.abort.store(true, Ordering::SeqCst);
        true
    }
}

// ---------------------------------------------------------------------------
// Session thread
// ---------------------------------------------------------------------------

fn finish(
    shared: &Shared,
    req: &LaunchRequest,
    control: &Control,
    started_at: chrono::DateTime<Utc>,
    elapsed: Duration,
    exit: Option<ExitInfo>,
) {
    // Free the slot first so listeners can relaunch right away.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    let mut outcome = None;
    if shared.cfg.record && control.record.load(Ordering::SeqCst) {
        let rec = PlayRecord {
            app_name: req.app_name.clone(),
            title: req.title.clone(),
            started_at,
            ended_at: started_at + chrono::Duration::milliseconds(elapsed.as_millis() as i64),
        };
        match record::record_play(&shared.paths, &rec, shared.cfg.min_session_seconds) {
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
    shared.send(LaunchEvent::Exited {
        app_name: req.app_name.clone(),
        elapsed,
        exit,
        record: outcome,
    });
}

fn fail(shared: &Shared, req: &LaunchRequest, message: String) {
    shared.sessions.lock().unwrap().remove(&req.app_name);
    log::error!("{}: {message}", req.app_name);
    shared.send(LaunchEvent::Error {
        app_name: req.app_name.clone(),
        message,
    });
}

fn run_session(shared: Arc<Shared>, req: LaunchRequest, control: Arc<Control>) {
    let started_at = Utc::now();
    let t0 = Instant::now();
    match &req.plan {
        LaunchPlan::SteamUri { app_id, uri } => {
            run_steam(&shared, &req, &control, app_id, uri, started_at, t0)
        }
        LaunchPlan::Unavailable(r) => fail(&shared, &req, r.clone()),
        other => {
            let process = other.process().expect("process plan").clone();
            run_process(&shared, &req, &control, &process, started_at, t0);
        }
    }
}

fn run_steam(
    shared: &Shared,
    req: &LaunchRequest,
    control: &Control,
    app_id: &str,
    uri: &str,
    started_at: chrono::DateTime<Utc>,
    t0: Instant,
) {
    if let Err(e) = shared.spawner.open_uri(uri) {
        return fail(shared, req, format!("could not open {uri}: {e}"));
    }
    let locator = shared.steam.clone();
    let mut announced: Option<u32> = None;
    let outcome = wait_for_steam_session(
        app_id,
        &*shared.procs,
        &move || locator.log_files(),
        &shared.cfg.steam,
        &control.abort,
        &control.steam_pids,
        &mut |pid| {
            announced = Some(pid);
            control.pid.store(pid, Ordering::SeqCst);
            // Only now do we know the game is really running.
            let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
            shared.send(LaunchEvent::Running {
                app_name: req.app_name.clone(),
                pid,
            });
        },
    );
    match outcome {
        SteamWaitOutcome::NotStarted => fail(
            shared,
            req,
            format!(
                "Steam did not start {} within {}s",
                req.title,
                shared.cfg.steam.start_timeout.as_secs()
            ),
        ),
        SteamWaitOutcome::Finished | SteamWaitOutcome::Aborted => {
            finish(shared, req, control, started_at, t0.elapsed(), None)
        }
    }
}

/// Runs a command to completion (prefix creation, wineserver --wait).
/// Returns false if tracking was aborted meanwhile.
fn run_to_completion(shared: &Shared, control: &Control, cmd: &Command) -> bool {
    let mut handle = match shared.spawner.spawn(cmd, None) {
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

fn run_process(
    shared: &Shared,
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
        Flow::Cancelled => return finish_aborted(shared, req, t0),
        Flow::Fail(message) => return fail(shared, req, message),
    }
    if !plan.pre_commands.is_empty() {
        status(Notice::info("Preparing prefix..."));
    }
    for cmd in &plan.pre_commands {
        if control.kill.load(Ordering::SeqCst) || !run_to_completion(shared, control, cmd) {
            return finish_aborted(shared, req, t0);
        }
    }
    // Redistributables, EOS overlay and known fixes the first time the game
    // runs in this prefix.
    let epic = shared.epic.lock().unwrap().clone();
    match prep.first_launch(&plan.prep, epic.as_deref()) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, req, t0),
        Flow::Fail(message) => return fail(shared, req, message),
    }
    // DXVK / VKD3D into the prefix, anti-cheat runtimes.
    match prep.after_prefix(&plan.prep) {
        Flow::Go => {}
        Flow::Cancelled => return finish_aborted(shared, req, t0),
        Flow::Fail(message) => return fail(shared, req, message),
    }
    if cancelled() {
        // Stopped while preparing: the game must not start afterwards.
        return finish_aborted(shared, req, t0);
    }
    if let Some(path) = &plan.make_executable {
        if let Err(e) = shared.spawner.make_executable(path) {
            log::warn!("chmod {path}: {e}");
        }
    }
    let log_path = shared
        .paths
        .games_config_dir()
        .join(format!("{}-lastPlay.log", req.app_name));
    let mut handle = match shared.spawner.spawn(&plan.command, Some(&log_path)) {
        Ok(h) => h,
        Err(e) => {
            return fail(
                shared,
                req,
                format!("could not start {}: {e}", plan.command.program),
            )
        }
    };
    control.pid.store(handle.pid(), Ordering::SeqCst);
    let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
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

    if !aborted && !control.kill.load(Ordering::SeqCst) {
        if let Some(cmd) = &plan.wait_after_exit {
            run_to_completion(shared, control, cmd);
        }
    }
    // The session ends here; saving to the cloud is not playtime.
    let elapsed = t0.elapsed();
    if !aborted {
        prep.after_exit(&req.app_name, &plan.prep);
    }
    finish(shared, req, control, started_at, elapsed, exit);
}

fn finish_aborted(shared: &Shared, req: &LaunchRequest, t0: Instant) {
    // Stopped before the game even started: nothing was played.
    shared.sessions.lock().unwrap().remove(&req.app_name);
    shared.send(LaunchEvent::Exited {
        app_name: req.app_name.clone(),
        elapsed: t0.elapsed(),
        exit: None,
        record: None,
    });
}

// ---------------------------------------------------------------------------
// Fake spawner for tests (also usable from the app's tests)
// ---------------------------------------------------------------------------

/// Records everything instead of running it. Handles finish when told to.
#[derive(Default)]
pub struct FakeSpawner {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    spawned: Vec<Command>,
    uris: Vec<String>,
    patterns: Vec<String>,
    handles: Vec<Arc<Mutex<FakeHandleState>>>,
    fail_spawn: Option<String>,
    fail_uri: Option<String>,
    next_pid: u32,
}

#[derive(Default)]
pub struct FakeHandleState {
    pub exit: Option<ExitInfo>,
    pub terminated: bool,
    pub killed: bool,
}

struct FakeHandle {
    pid: u32,
    state: Arc<Mutex<FakeHandleState>>,
    /// Exits by itself when terminated, like a well-behaved process.
    exit_on_terminate: bool,
}

impl ProcessHandle for FakeHandle {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn poll(&mut self) -> io::Result<Option<ExitInfo>> {
        Ok(self.state.lock().unwrap().exit)
    }
    fn terminate(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.terminated = true;
        if self.exit_on_terminate {
            s.exit = Some(ExitInfo {
                code: None,
                signal: Some(15),
            });
        }
    }
    fn kill(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.killed = true;
        s.exit = Some(ExitInfo {
            code: None,
            signal: Some(9),
        });
    }
}

impl FakeSpawner {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn spawned(&self) -> Vec<Command> {
        self.state.lock().unwrap().spawned.clone()
    }
    pub fn uris(&self) -> Vec<String> {
        self.state.lock().unwrap().uris.clone()
    }
    pub fn patterns(&self) -> Vec<String> {
        self.state.lock().unwrap().patterns.clone()
    }
    pub fn handle(&self, index: usize) -> Option<Arc<Mutex<FakeHandleState>>> {
        self.state.lock().unwrap().handles.get(index).cloned()
    }
    /// Makes the `index`-th spawned process exit.
    pub fn finish(&self, index: usize, code: i32) {
        if let Some(h) = self.handle(index) {
            h.lock().unwrap().exit = Some(ExitInfo {
                code: Some(code),
                signal: None,
            });
        }
    }
    pub fn fail_next_spawn(&self, message: &str) {
        self.state.lock().unwrap().fail_spawn = Some(message.to_string());
    }
    pub fn fail_next_uri(&self, message: &str) {
        self.state.lock().unwrap().fail_uri = Some(message.to_string());
    }
}

impl Spawner for FakeSpawner {
    fn spawn(&self, cmd: &Command, _log: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        let mut s = self.state.lock().unwrap();
        if let Some(msg) = s.fail_spawn.take() {
            return Err(io::Error::new(io::ErrorKind::NotFound, msg));
        }
        s.next_pid += 1;
        let pid = 40_000 + s.next_pid;
        s.spawned.push(cmd.clone());
        let st = Arc::new(Mutex::new(FakeHandleState::default()));
        s.handles.push(st.clone());
        Ok(Box::new(FakeHandle {
            pid,
            state: st,
            exit_on_terminate: true,
        }))
    }
    fn open_uri(&self, uri: &str) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        if let Some(msg) = s.fail_uri.take() {
            return Err(io::Error::new(io::ErrorKind::NotFound, msg));
        }
        s.uris.push(uri.to_string());
        Ok(())
    }
    fn kill_pattern(&self, pattern: &str) {
        self.state
            .lock()
            .unwrap()
            .patterns
            .push(pattern.to_string());
    }
    fn make_executable(&self, _path: &str) -> io::Result<()> {
        Ok(())
    }
    fn create_dir_all(&self, _path: &str) -> io::Result<()> {
        Ok(())
    }
}
