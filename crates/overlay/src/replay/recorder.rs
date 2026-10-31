//! The recorder process while a game runs. One thread runs it and lives as
//! long as it (its `PDEATHSIG` is that thread's, see `heronite_data::proc`),
//! starts it again if it dies (a suspend, a monitor that changed mode) and
//! ends it when asked: SIGTERM, which in replay mode exits without saving
//! (SIGKILL when it lingers). Vulkan video that cannot start falls back to
//! the card's usual encoder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use heronite_data::proc::{Cancel, Run};
use heronite_i18n::tr;

use super::info::Info;
#[cfg(target_os = "linux")]
use super::ipc::save_replay;
use super::plan::Plan;

const FLATPAK_ID: &str = "com.dec05eba.gpu_screen_recorder";
const BINARY: &str = "gpu-screen-recorder";
/// Running this long, it is taken as recording (it only exits early on errors).
const READY_AFTER: Duration = Duration::from_secs(2);
/// Ending sooner than this, it could not start: trying again would not help.
const MIN_RUN: Duration = Duration::from_secs(5);
/// Starts again after it died, with these pauses; then it gives up.
const RESTART_AFTER: [Duration; 3] = [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)];
/// Running this long before it died, the pauses start over: a long session
/// survives any number of suspends.
const HEALTHY_AFTER: Duration = Duration::from_secs(120);

/// Numbers the sockets: each recorder has its own, so one that ends late
/// never takes the next one's away.
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

/// How gpu-screen-recorder is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    Native(PathBuf),
    /// Flathub's `com.dec05eba.gpu_screen_recorder`.
    Flatpak,
}

impl Program {
    /// The installed one: the system's first, else the Flatpak. Blocking.
    pub fn find() -> Option<Program> {
        if let Some(path) = in_path(BINARY) {
            return Some(Program::Native(path));
        }
        in_path("flatpak")?;
        let mut cmd = Command::new("flatpak");
        cmd.args(["info", FLATPAK_ID]);
        let found = Run::new().timeout(Duration::from_secs(5)).run(&mut cmd).is_ok_and(|o| o.success());
        found.then_some(Program::Flatpak)
    }

    fn command(&self) -> Command {
        match self {
            Program::Native(path) => Command::new(path),
            Program::Flatpak => {
                let mut cmd = Command::new("flatpak");
                cmd.args(["run", &format!("--command={BINARY}"), FLATPAK_ID]);
                cmd
            }
        }
    }

    /// What it can do here (`--info`). Blocking.
    pub fn info(&self) -> Result<Info, String> {
        let mut cmd = self.command();
        cmd.arg("--info");
        clean_env(&mut cmd);
        let out = Run::new().timeout(Duration::from_secs(15)).run(&mut cmd).map_err(|e| e.to_string())?;
        if !out.success() {
            let err = out.stderr_text();
            return Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("--info failed").trim().to_string());
        }
        Ok(Info::parse(&out.stdout_text()))
    }

    /// A new socket path both sides see (a Flatpak shares only its own
    /// folder of the runtime directory with the host).
    pub fn socket_path(&self) -> PathBuf {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = match self {
            Program::Native(_) => runtime.join("heronite"),
            Program::Flatpak => runtime.join("app").join(FLATPAK_ID),
        };
        let _ = std::fs::create_dir_all(&dir);
        let n = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        dir.join(format!("heronite-replay-{}-{n}.sock", std::process::id()))
    }
}

fn in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Variables of the app's own packaging (an AppImage's libraries) or of a
/// game's tools that must not reach the recorder.
fn clean_env(cmd: &mut Command) {
    const EXACT: [&str; 6] = ["LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT", "APPIMAGE", "APPDIR", "OWD"];
    const PREFIX: [&str; 6] = ["MANGOHUD", "ENABLE_VKBASALT", "OBS_VKCAPTURE", "DXVK_", "VKD3D_", "PROTON_"];
    for (key, _) in std::env::vars_os() {
        let Some(k) = key.to_str() else { continue };
        if EXACT.contains(&k) || PREFIX.iter().any(|p| k.starts_with(p)) {
            cmd.env_remove(&key);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Starting,
    /// Through Vulkan video, else the card's usual encoder.
    Recording {
        vulkan: bool,
    },
    /// It could not start, or died too often; why, for the user.
    Failed(String),
    Stopped,
}

struct Shared {
    socket: PathBuf,
    cancel: Arc<Cancel>,
    state: Mutex<State>,
    on_state: Box<dyn Fn(&State) + Send + Sync>,
}

impl Shared {
    fn set(&self, state: State) {
        let mut current = self.state.lock().unwrap();
        if *current != state {
            *current = state.clone();
            drop(current);
            (self.on_state)(&state);
        }
    }

    /// Waits `d`, or less when asked to stop; `true` when asked.
    fn pause(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if self.cancel.is_requested() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.cancel.is_requested()
    }
}

/// A running recorder. Dropping it stops it (blocking for a few seconds at
/// most: drop it off the UI thread).
pub struct Recorder {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Starts recording; `on_state` hears every change (from the recorder's
    /// thread).
    pub fn start(program: Program, plan: Plan, on_state: impl Fn(&State) + Send + Sync + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            socket: plan.socket.clone(),
            cancel: Cancel::new(),
            state: Mutex::new(State::Starting),
            on_state: Box::new(on_state),
        });
        let s = shared.clone();
        let thread = std::thread::Builder::new().name("replay-recorder".into()).spawn(move || supervise(&s, &program, plan))?;
        Ok(Self { shared, thread: Some(thread) })
    }

    pub fn state(&self) -> State {
        self.shared.state.lock().unwrap().clone()
    }

    /// Saves the last `seconds` into the plan's folder and returns the file.
    /// Blocking.
    pub fn save(&self, seconds: u32) -> Result<PathBuf, String> {
        if let State::Failed(why) = self.state() {
            return Err(why);
        }
        save_replay(&self.shared.socket, seconds)
    }

    /// Ends it and waits for it.
    pub fn stop(mut self) {
        self.end();
    }

    fn end(&mut self) {
        self.shared.cancel.request();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.end();
    }
}

fn supervise(shared: &Shared, program: &Program, mut plan: Plan) {
    let _ = std::fs::create_dir_all(&plan.out_dir);
    let mut restarts = 0;
    loop {
        let _ = std::fs::remove_file(&plan.socket);
        shared.set(State::Starting);
        let started = Instant::now();
        let ready = AtomicBool::new(false);
        let vulkan = plan.encoding.vulkan;
        let poll = || {
            if !ready.load(Ordering::Relaxed) && started.elapsed() >= READY_AFTER {
                ready.store(true, Ordering::Relaxed);
                shared.set(State::Recording { vulkan });
            }
            false
        };
        let mut last_error: Option<String> = None;
        let mut on_line = |line: &str| log_line(line, &mut last_error);
        let mut cmd = program.command();
        cmd.args(plan.args());
        clean_env(&mut cmd);
        log::info!(
            "replay: starting the recorder on {} ({}, {} s in {}, {} kbit/s)",
            plan.screen.name,
            plan.codec_arg(),
            plan.buffer_seconds,
            plan.storage.as_str(),
            plan.video_kbps
        );
        let out =
            Run::new().cancel(&shared.cancel).poll(&poll).on_line(&mut on_line).grace(Duration::from_secs(5)).no_capture().run(&mut cmd);
        if shared.cancel.is_requested() {
            break;
        }
        let why = match out {
            Ok(o) => describe(o.code, last_error),
            Err(e) => {
                shared.set(State::Failed(tr!("gpu-screen-recorder could not start: {err}", err = e.to_string())));
                return;
            }
        };
        log::warn!("replay: the recorder ended: {why}");
        let ran = started.elapsed();
        if ran < MIN_RUN && plan.encoding.vulkan {
            // Vulkan video is experimental in gpu-screen-recorder.
            log::warn!("replay: Vulkan video could not record: the card's usual encoder now");
            plan.encoding.vulkan = false;
            continue;
        }
        if ran >= HEALTHY_AFTER {
            restarts = 0;
        }
        if ran < MIN_RUN || restarts >= RESTART_AFTER.len() {
            shared.set(State::Failed(why));
            remove_socket(&plan.socket);
            return;
        }
        if shared.pause(RESTART_AFTER[restarts]) {
            break;
        }
        restarts += 1;
    }
    remove_socket(&plan.socket);
    shared.set(State::Stopped);
}

/// Without Unix sockets there is no recorder to ask (nor one to find).
#[cfg(not(target_os = "linux"))]
fn save_replay(_socket: &Path, _seconds: u32) -> Result<PathBuf, String> {
    Err(tr!("gpu-screen-recorder is not installed").into())
}

fn remove_socket(socket: &Path) {
    let _ = std::fs::remove_file(socket);
}

/// Logs a line of the recorder and keeps the last error.
fn log_line(line: &str, last_error: &mut Option<String>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if let Some(e) = line.strip_prefix("gsr error:") {
        log::warn!("replay: {line}");
        *last_error = Some(e.trim().to_string());
    } else if line.starts_with("gsr warning:") {
        log::warn!("replay: {line}");
    } else {
        log::info!("replay: {line}");
    }
}

/// Why it ended, from its exit code (`gpu-screen-recorder`'s
/// `include/recorder/error.h`) or its last error line.
fn describe(code: Option<i32>, last_error: Option<String>) -> String {
    let known = match code {
        Some(2) => Some(tr!("No graphics card it can record with was found")),
        Some(3) => Some(tr!("The screen could not be captured")),
        Some(22) => Some(tr!("The graphics libraries could not be loaded")),
        Some(50) => Some(tr!("The audio source was not found")),
        Some(51) => Some(tr!("The monitor was not found")),
        Some(52) | Some(54) => Some(tr!("The graphics card cannot encode this video format")),
        Some(53) => Some(tr!("The graphics card cannot encode video this large")),
        Some(126) | Some(127) => Some(tr!("No permission to capture the screen (gsr-kms-server)")),
        _ => None,
    };
    match (known, last_error) {
        (Some(k), _) => k.to_string(),
        (None, Some(e)) => e,
        (None, None) => match code {
            Some(c) => tr!("gpu-screen-recorder exited with code {code}", code = c),
            None => tr!("gpu-screen-recorder was stopped by a signal").into(),
        },
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use heronite_data::replay_prefs::Codec;

    use super::super::plan::Encoding;
    use super::*;

    #[test]
    fn exit_codes_say_why() {
        assert_eq!(describe(Some(51), Some("x".into())), "The monitor was not found");
        assert_eq!(describe(Some(1), Some("Failed to open window".into())), "Failed to open window");
        assert_eq!(describe(Some(9), None), "gpu-screen-recorder exited with code 9");
        let mut last = None;
        log_line("gsr error: no codec", &mut last);
        log_line("gsr info: fine", &mut last);
        assert_eq!(last.as_deref(), Some("no codec"));
    }

    // `/bin/false` and a shell script stand in for the recorder.
    #[cfg(unix)]
    #[test]
    fn a_dead_recorder_fails_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        // `false` exits at once with code 1: it could not start.
        let program = Program::Native(PathBuf::from("/bin/false"));
        let plan = Plan::new(
            &Default::default(),
            super::super::info::Screen { name: "DP-1".into(), width: 1920, height: 1080 },
            Encoding { codec: Codec::H264, vulkan: false },
            None,
            dir.path().join("out"),
            dir.path().join("s.sock"),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let rec = Recorder::start(program, plan, move |s| {
            let _ = tx.send(s.clone());
        })
        .unwrap();
        let mut seen = Vec::new();
        while let Ok(s) = rx.recv_timeout(Duration::from_secs(10)) {
            let failed = matches!(s, State::Failed(_));
            seen.push(s);
            if failed {
                break;
            }
        }
        assert!(matches!(seen.last(), Some(State::Failed(_))), "{seen:?}");
        assert!(rec.save(30).is_err());
        rec.stop();
    }

    #[cfg(unix)]
    #[test]
    fn vulkan_that_cannot_start_falls_back_to_the_usual_encoder() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let bin = dir.path().join("fake-gsr");
        std::fs::write(&bin, format!("#!/bin/sh\necho \"$@\" >> {}\nexit 1\n", calls.display())).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plan = Plan::new(
            &Default::default(),
            super::super::info::Screen { name: "DP-1".into(), width: 1920, height: 1080 },
            Encoding { codec: Codec::Hevc, vulkan: true },
            None,
            dir.path().join("out"),
            dir.path().join("s.sock"),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let rec = Recorder::start(Program::Native(bin), plan, move |s| {
            let _ = tx.send(s.clone());
        })
        .unwrap();
        while let Ok(s) = rx.recv_timeout(Duration::from_secs(10)) {
            if matches!(s, State::Failed(_)) {
                break;
            }
        }
        rec.stop();
        let calls = std::fs::read_to_string(calls).unwrap();
        let ks: Vec<&str> = calls.lines().map(|l| l.split(" -k ").nth(1).unwrap().split(' ').next().unwrap()).collect();
        assert_eq!(ks, ["hevc_vulkan", "hevc"], "once with Vulkan, once without; then it gives up");
    }

    #[test]
    fn every_recorder_has_its_own_socket() {
        let p = Program::Native(PathBuf::from("/bin/false"));
        assert_ne!(p.socket_path(), p.socket_path());
    }

    /// Records the real screen for a few seconds and saves 3 of them:
    /// `cargo test -p heronite-overlay live_recording -- --ignored`.
    #[test]
    #[ignore = "records the screen with the installed gpu-screen-recorder"]
    fn live_recording() {
        let program = Program::find().expect("gpu-screen-recorder is installed");
        let info = program.info().unwrap();
        assert!(info.recent_enough(), "{info:?}");
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = heronite_data::replay_prefs::ReplayPrefs::default();
        prefs.options.audio = heronite_data::replay_prefs::Audio::Off;
        let codec = info.codec_for(prefs.options.codec).expect("a hardware encoder");
        let encoding = Encoding { codec, vulkan: info.uses_vulkan(prefs.options.encoder, codec) };
        let plan = Plan::new(&prefs.options, info.screens[0].clone(), encoding, None, dir.path().join("out"), program.socket_path());
        let rec = Recorder::start(program, plan, |s| eprintln!("state: {s:?}")).unwrap();
        std::thread::sleep(Duration::from_secs(6));
        assert!(matches!(rec.state(), State::Recording { .. }), "{:?}", rec.state());
        let saved = rec.save(3).unwrap();
        assert!(saved.starts_with(dir.path().join("out")), "{}", saved.display());
        assert!(std::fs::metadata(&saved).unwrap().len() > 0);
        let started = Instant::now();
        rec.stop();
        assert!(started.elapsed() < Duration::from_secs(5), "it ends quickly");
    }
}
