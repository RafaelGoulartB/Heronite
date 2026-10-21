//! The recorder process while a game runs. One thread runs it and lives as
//! long as it (its `PDEATHSIG` is that thread's, see `heronite_data::proc`),
//! starts it again if it dies (a suspend, a monitor that changed mode) and
//! ends it when asked: SIGTERM, which in replay mode exits without saving.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use heronite_data::proc::{Cancel, Run};
use heronite_i18n::tr;

use super::info::Info;
use super::ipc;
use super::plan::Plan;

const FLATPAK_ID: &str = "com.dec05eba.gpu_screen_recorder";
const BINARY: &str = "gpu-screen-recorder";
/// Running this long, it is taken as recording (it only exits early on errors).
const READY_AFTER: Duration = Duration::from_secs(2);
/// Ending sooner than this, it could not start: trying again would not help.
const MIN_RUN: Duration = Duration::from_secs(5);
/// Starts again after it died, with these pauses; then it gives up.
const RESTART_AFTER: [Duration; 3] = [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)];

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

    /// A socket path both sides see (a Flatpak shares only its own folder
    /// of the runtime directory with the host).
    pub fn socket_path(&self) -> PathBuf {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = match self {
            Program::Native(_) => runtime.join("heronite"),
            Program::Flatpak => runtime.join("app").join(FLATPAK_ID),
        };
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("heronite-replay-{}.sock", std::process::id()))
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
    Recording,
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
        let thread = std::thread::Builder::new().name("replay-recorder".into()).spawn(move || supervise(&s, &program, &plan))?;
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
        ipc::save_replay(&self.shared.socket, seconds)
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

fn supervise(shared: &Shared, program: &Program, plan: &Plan) {
    let _ = std::fs::create_dir_all(&plan.out_dir);
    let mut restarts = 0;
    loop {
        let _ = std::fs::remove_file(&plan.socket);
        shared.set(State::Starting);
        let started = Instant::now();
        let ready = AtomicBool::new(false);
        let poll = || {
            if !ready.load(Ordering::Relaxed) && started.elapsed() >= READY_AFTER {
                ready.store(true, Ordering::Relaxed);
                shared.set(State::Recording);
            }
            false
        };
        let mut last_error: Option<String> = None;
        let mut on_line = |line: &str| log_line(line, &mut last_error);
        let mut cmd = program.command();
        cmd.args(plan.args());
        clean_env(&mut cmd);
        log::info!("replay: starting the recorder on {} ({} s, {} kbit/s)", plan.screen.name, plan.buffer_seconds, plan.video_kbps);
        let out =
            Run::new().cancel(&shared.cancel).poll(&poll).on_line(&mut on_line).grace(Duration::from_secs(5)).line_capture().run(&mut cmd);
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
        if started.elapsed() < MIN_RUN || restarts >= RESTART_AFTER.len() {
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

    #[test]
    fn a_dead_recorder_fails_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        // `false` exits at once with code 1: it could not start.
        let program = Program::Native(PathBuf::from("/bin/false"));
        let plan = Plan::new(
            &Default::default(),
            super::super::info::Screen { name: "DP-1".into(), width: 1920, height: 1080 },
            heronite_data::replay_prefs::Codec::H264,
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

    /// Records the real screen for a few seconds and saves 3 of them:
    /// `cargo test -p heronite-overlay live_recording -- --ignored`.
    #[test]
    #[ignore = "records the screen with the installed gpu-screen-recorder"]
    fn live_recording() {
        let program = Program::find().expect("gpu-screen-recorder is installed");
        let info = program.info().unwrap();
        assert!(info.recent_enough(), "{info:?}");
        let dir = tempfile::tempdir().unwrap();
        let prefs = heronite_data::replay_prefs::ReplayPrefs { audio: heronite_data::replay_prefs::Audio::Off, ..Default::default() };
        let codec = info.codec_for(prefs.codec).expect("a hardware encoder");
        let plan = Plan::new(&prefs, info.screens[0].clone(), codec, None, dir.path().join("out"), program.socket_path());
        let rec = Recorder::start(program, plan, |s| eprintln!("state: {s:?}")).unwrap();
        std::thread::sleep(Duration::from_secs(6));
        assert_eq!(rec.state(), State::Recording);
        let saved = rec.save(3).unwrap();
        assert!(saved.starts_with(dir.path().join("out")), "{}", saved.display());
        assert!(std::fs::metadata(&saved).unwrap().len() > 0);
        let started = Instant::now();
        rec.stop();
        assert!(started.elapsed() < Duration::from_secs(5), "it ends quickly");
    }
}
