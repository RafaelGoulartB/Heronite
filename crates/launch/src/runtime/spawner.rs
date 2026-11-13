//! Process spawning behind the [`Spawner`] and [`ProcessHandle`] traits.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use heronite_platform::process::{self as os, Signal};

use crate::log_writer::GameLog;
use crate::plan::Command;
use crate::procs::{signal_tree, Procs, Sig, SystemProcs};
use crate::steam::desktop_protocol_env;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// A spawned process (and its output pipe, if the spawner owns one).
pub trait ProcessHandle: Send {
    fn pid(&self) -> u32;
    /// `None` while running. `Some` once the process has exited *and* every
    /// process of its group is gone (what Node's `close` event means).
    fn poll(&mut self) -> io::Result<Option<ExitInfo>>;
    /// SIGTERM to the process tree.
    fn terminate(&mut self);
    /// SIGKILL to the process tree.
    fn kill(&mut self);
    /// Ctrl+C to the process alone (SIGINT on Unix): a helper that finishes
    /// its work on Ctrl+C. Terminates it where that cannot be sent.
    fn interrupt(&mut self) {
        self.terminate();
    }
}

pub trait Spawner: Send + Sync {
    /// Starts `cmd`; its stdout/stderr go to `log_path` when given.
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>>;
    /// Starts `cmd`; its stdout/stderr are appended to `log` (the game's
    /// Heroic-style log) as they arrive.
    fn spawn_logged(&self, cmd: &Command, _log: Option<Arc<GameLog>>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn(cmd, None)
    }
    /// Starts a command-line helper the app runs beside games (Comet), its
    /// output appended to `log_file` as it is (nothing of the command is
    /// written: its arguments may be secret). On Windows it gets a console
    /// of its own without a window, where [`ProcessHandle::interrupt`] sends
    /// Ctrl+C; on Linux it gets SIGINT when the calling thread dies, so keep
    /// that thread alive while the helper runs.
    fn spawn_helper(&self, cmd: &Command, _log_file: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn(cmd, None)
    }
    /// Opens a URI with the desktop handler (`xdg-open`, the shell on
    /// Windows), without waiting. A `steam://` URI the desktop fails to open
    /// goes to the Steam client directly.
    fn open_uri(&self, uri: &str) -> io::Result<()>;
    /// Hands a `steam://` URI to the Steam client itself, not through the
    /// desktop (whose handler can be broken or another program), unless it
    /// already was since `asked_at` (when it was asked for through
    /// [`Spawner::open_uri`], which falls back to the client by itself).
    fn open_with_steam(&self, uri: &str, _asked_at: std::time::Instant) -> io::Result<()> {
        self.open_uri(uri)
    }
    /// Terminates the processes started at or after `since` (ticks since
    /// boot, [`crate::procs::boot_ticks_now`]) whose program is `pattern`
    /// ([`crate::procs::program_matches`]).
    fn kill_pattern(&self, pattern: &str, since: u64);
    fn make_executable(&self, path: &str) -> io::Result<()> {
        heronite_platform::fs::add_mode(Path::new(path), 0o775)
    }
    fn create_dir_all(&self, path: &str) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }
}

/// Spawns real processes. Every game gets its own process group so the whole
/// tree can be stopped, and writes its output straight to the log file.
#[derive(Clone)]
pub struct SystemSpawner {
    procs: Arc<dyn Procs>,
}

impl Default for SystemSpawner {
    fn default() -> Self {
        Self { procs: Arc::new(SystemProcs) }
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
    procs: Arc<dyn Procs>,
}

/// Whether any process of group `pgid` is left (the group leader's pid).
/// A group lives as long as one member does, so this is "the process and
/// everything it started (that stayed in its group) is gone".
fn group_alive(pgid: u32) -> bool {
    SystemProcs.group_alive(pgid)
}

impl SystemHandle {
    fn signal_group(&self, sig: Signal) {
        os::signal_group(self.pid, sig);
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
                self.exit = Some(ExitInfo { code: status.code(), signal: os::exit_signal(&status) });
            }
        }
        // Like Node's `close` event (which waits for the output pipe to
        // close): a launcher script that backgrounds the real game keeps the
        // session open until the whole process group is gone.
        match self.exit {
            Some(info) if !group_alive(self.pid) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn terminate(&mut self) {
        signal_tree(&*self.procs, self.pid, Sig::Term);
        self.signal_group(Signal::Term);
    }

    fn kill(&mut self) {
        signal_tree(&*self.procs, self.pid, Sig::Kill);
        self.signal_group(Signal::Kill);
    }

    fn interrupt(&mut self) {
        os::interrupt(self.pid);
    }
}

/// Where the output of a spawned process goes.
enum Sink {
    None,
    /// A plain file, created fresh with a "Launching:" header.
    File(PathBuf),
    /// The game's log.
    Log(Arc<GameLog>),
    /// A file the output is appended to, with no header.
    Append(PathBuf),
}

impl SystemSpawner {
    /// The child's stdout and stderr are the file itself (no pipe, no pump
    /// thread): the game keeps working, and keeps logging, if the launcher
    /// exits before it does.
    /// `helper`: see [`Spawner::spawn_helper`].
    fn spawn_with(&self, cmd: &Command, sink: Sink, helper: bool) -> io::Result<Box<dyn ProcessHandle>> {
        use std::process::Stdio;
        let host = if cfg!(windows) { script_host(&cmd.program) } else { None };
        let mut c = match host {
            Some((program, args)) => {
                let mut c = std::process::Command::new(program);
                c.args(args).arg(&cmd.program);
                c
            }
            None => std::process::Command::new(&cmd.program),
        };
        c.args(&cmd.args).envs(&cmd.env);
        crate::wine::drop_host_certs(cmd, &mut c);
        if let Some(dir) = cmd.cwd.as_deref().filter(|d| !d.is_empty()) {
            c.current_dir(dir);
        }
        // Another Flatpak (an emulator) starts from the host when Heronite
        // runs as a Flatpak itself.
        if cmd.program == "flatpak" {
            c = heronite_platform::flatpak::host(c);
        }
        c.stdin(Stdio::null());
        // A script's console would stay empty (its output goes to the log):
        // Windows opens none for it.
        if cfg!(windows) && (helper || host.is_some() || is_batch(&cmd.program)) {
            os::new_helper_group(&mut c);
        } else {
            os::new_group(&mut c);
        }
        #[cfg(target_os = "linux")]
        if helper {
            use std::os::unix::process::CommandExt;
            // SAFETY: only an async-signal-safe prctl between fork and exec.
            unsafe {
                c.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGINT);
                    Ok(())
                });
            }
        }
        let (out, err) = match &sink {
            Sink::None => (Stdio::null(), Stdio::null()),
            Sink::File(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::File::create(p) {
                    Ok(mut f) => {
                        let _ = f.write_all(format!("Launching: {}\n\nGame Log:\n", cmd.display()).as_bytes());
                        // stdout and stderr share one open file description.
                        match f.try_clone() {
                            Ok(f2) => (Stdio::from(f), Stdio::from(f2)),
                            Err(_) => (Stdio::from(f), Stdio::null()),
                        }
                    }
                    Err(_) => (Stdio::null(), Stdio::null()),
                }
            }
            Sink::Log(log) => (log.child_stdio(), log.child_stdio()),
            Sink::Append(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::OpenOptions::new().create(true).append(true).open(p) {
                    Ok(f) => match f.try_clone() {
                        Ok(f2) => (Stdio::from(f), Stdio::from(f2)),
                        Err(_) => (Stdio::from(f), Stdio::null()),
                    },
                    Err(_) => (Stdio::null(), Stdio::null()),
                }
            }
        };
        c.stdout(out).stderr(err);
        let child = heronite_data::proc::spawn(&mut c)?;
        let pid = child.id();
        Ok(Box::new(SystemHandle { child: Some(child), pid, exit: None, procs: self.procs.clone() }))
    }
}

/// The program that runs the script `path` on Windows, with its arguments
/// before the script: PowerShell for `.ps1`, Windows Script Host for `.vbs`,
/// `.js` and `.wsf`. `None` for what starts on its own (programs, and batch
/// files, which the standard library hands to `cmd.exe`).
fn script_host(path: &str) -> Option<(&'static str, &'static [&'static str])> {
    const POWERSHELL: &[&str] = &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"];
    const CSCRIPT: &[&str] = &["//NoLogo"];
    match extension(path).as_str() {
        "ps1" => Some(("powershell.exe", POWERSHELL)),
        "vbs" | "js" | "wsf" => Some(("cscript.exe", CSCRIPT)),
        _ => None,
    }
}

fn is_batch(path: &str) -> bool {
    matches!(extension(path).as_str(), "bat" | "cmd")
}

/// How soon xdg-open must fail for its failure to mean the desktop could not
/// open the URI (later, it ran the handler itself and that one ended).
const XDG_OPEN_QUICK_FAILURE: std::time::Duration = std::time::Duration::from_secs(15);

fn is_steam_uri(uri: &str) -> bool {
    uri.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("steam://"))
}

/// Runs `command` (a program and its arguments) with `uri` last, on its own,
/// with the desktop's environment (none of this app's runtime variables).
/// Its stderr is piped when asked.
fn spawn_detached(command: &[String], uri: &str, stderr: bool) -> io::Result<std::process::Child> {
    let (program, args) = command.split_first().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no program"))?;
    let mut c = std::process::Command::new(program);
    c.args(args)
        .arg(uri)
        .env_clear()
        .envs(desktop_protocol_env(std::env::vars()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(if stderr { std::process::Stdio::piped() } else { std::process::Stdio::null() });
    os::new_group(&mut c);
    c.spawn()
}

/// What a program writes to `pipe`, its first few KiB kept (the rest read
/// and dropped: a handler xdg-open runs in the foreground inherits the pipe
/// and must never block on it).
fn drain(pipe: Option<std::process::ChildStderr>) -> (Arc<std::sync::Mutex<Vec<u8>>>, Option<std::thread::JoinHandle<()>>) {
    use std::io::Read;
    let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
    let thread = pipe.map(|mut pipe| {
        let kept = kept.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut k = kept.lock().unwrap();
                let room = 4096usize.saturating_sub(k.len());
                k.extend_from_slice(&buf[..n.min(room)]);
            }
        })
    });
    (kept, thread)
}

/// When each `steam://` URI was last handed to the Steam client directly.
static HANDED_TO_STEAM: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// Hands `uri` to the Steam client itself (`steam <uri>`), which passes it to
/// the running client or starts Steam with it: what Steam's own desktop entry
/// does, without the desktop's URI handling. Not when it was already handed
/// to it since `asked_at` (when this request was made): the fallback after
/// xdg-open and the launch's own check could otherwise both send it.
fn open_with_steam_client(uri: &str, asked_at: std::time::Instant) -> io::Result<()> {
    {
        let mut handed = HANDED_TO_STEAM.lock().unwrap();
        if let Some(at) = handed.iter_mut().find(|(u, _)| u == uri).map(|(_, at)| at) {
            if *at >= asked_at {
                log::info!("{uri} was already handed to the Steam client");
                return Ok(());
            }
            *at = std::time::Instant::now();
        } else {
            handed.push((uri.to_string(), std::time::Instant::now()));
        }
    }
    // The running client's own launcher (a Flatpak or Snap Steam beside a
    // native `steam` on PATH) first, then the usual `steam`.
    let commands = crate::steam_launcher::client_commands(&crate::probe::SystemProbe, &crate::steam_launcher::steam_processes());
    let mut error = io::Error::new(io::ErrorKind::NotFound, "Steam client not found");
    for steam in commands {
        log::info!("handing {uri} to the Steam client ({})", steam.join(" "));
        match spawn_detached(&steam, uri, false) {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(e) => {
                log::warn!("could not run {}: {e}", steam.join(" "));
                error = e;
            }
        }
    }
    Err(error)
}

fn extension(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

impl Spawner for SystemSpawner {
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log_path.map_or(Sink::None, |p| Sink::File(p.to_path_buf())), false)
    }

    fn spawn_logged(&self, cmd: &Command, log: Option<Arc<GameLog>>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log.map_or(Sink::None, Sink::Log), false)
    }

    fn spawn_helper(&self, cmd: &Command, log_file: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log_file.map_or(Sink::None, |p| Sink::Append(p.to_path_buf())), true)
    }

    fn open_uri(&self, uri: &str) -> io::Result<()> {
        if cfg!(windows) {
            return heronite_platform::shell::open(uri);
        }
        let steam = is_steam_uri(uri);
        let mut child = match spawn_detached(&["xdg-open".to_string()], uri, steam) {
            Ok(child) => child,
            Err(e) if steam => {
                log::warn!("could not run xdg-open for {uri} ({e}); asking the Steam client directly");
                return open_with_steam_client(uri, std::time::Instant::now())
                    .map_err(|e2| io::Error::new(e2.kind(), format!("xdg-open: {e}; Steam client: {e2}")));
            }
            Err(e) => return Err(e),
        };
        // The desktop can fail to hand `steam://` to Steam even with Steam as
        // its handler (`gio: … not supported`, a stale `steam.desktop`): the
        // Steam client is asked directly then. Only a quick failure counts:
        // xdg-open can run the handler in the foreground, and Steam started
        // that way ends with the whole Steam session.
        let (stderr, drained) = drain(child.stderr.take());
        let uri = uri.to_string();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let status = child.wait();
            let failed = !status.as_ref().is_ok_and(|s| s.success());
            if steam && failed && started.elapsed() < XDG_OPEN_QUICK_FAILURE {
                // What xdg-open said (`gio: …: Operation not supported`).
                let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
                while drained.as_ref().is_some_and(|t| !t.is_finished()) && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let said = String::from_utf8_lossy(&stderr.lock().unwrap()).trim().to_string();
                log::warn!("xdg-open could not open {uri} ({status:?}: {said}); asking the Steam client directly");
                if let Err(e) = open_with_steam_client(&uri, started) {
                    log::warn!("could not ask the Steam client to open {uri}: {e}");
                }
            }
        });
        Ok(())
    }

    fn open_with_steam(&self, uri: &str, asked_at: std::time::Instant) -> io::Result<()> {
        if cfg!(windows) {
            return self.open_uri(uri);
        }
        open_with_steam_client(uri, asked_at)
    }

    fn kill_pattern(&self, pattern: &str, since: u64) {
        // Not `pkill -f`: that matches anywhere in any command line, so a game
        // called `game` would take Steam or a terminal down with it.
        for pid in crate::procs::processes_matching(pattern, since) {
            self.procs.signal(pid, Sig::Term);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_get_their_host_on_windows() {
        let (program, args) = script_host(r"C:\Scripts\Before Game.PS1").unwrap();
        assert_eq!(program, "powershell.exe");
        assert_eq!(args.last(), Some(&"-File"), "the script comes right after");
        assert_eq!(script_host("/s/trainer.vbs").unwrap().0, "cscript.exe");
        assert_eq!(script_host("tool.wsf").unwrap().0, "cscript.exe");
        for starts_alone in [r"C:\Scripts\before.bat", "after.CMD", "trainer.exe", "/s/before.sh", "noext"] {
            assert!(script_host(starts_alone).is_none(), "{starts_alone}");
        }
        assert!(is_batch("a.Bat") && is_batch("b.cmd") && !is_batch("c.ps1") && !is_batch("bat"));
    }

    /// Runs the script `name` (its text `body`) like a game's script and gives
    /// what it wrote to `out.txt`.
    #[cfg(windows)]
    fn run_script(name: &str, body: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join(name);
        std::fs::write(&script, body).unwrap();
        let cmd = Command {
            program: script.to_string_lossy().into_owned(),
            env: [("HERONITE_GAME_SCRIPT_TYPE".to_string(), "before".to_string())].into_iter().collect(),
            cwd: Some(dir.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        let mut handle = SystemSpawner::default().spawn(&cmd, None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let exit = loop {
            if let Some(exit) = handle.poll().unwrap() {
                break exit;
            }
            assert!(std::time::Instant::now() < deadline, "{name} did not finish");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(exit.code, Some(0), "{name}");
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap().trim().to_string()
    }

    #[cfg(windows)]
    #[test]
    fn windows_runs_batch_files_and_powershell_scripts() {
        assert_eq!(run_script("before.bat", "@echo %HERONITE_GAME_SCRIPT_TYPE%> out.txt\r\n"), "before");
        assert_eq!(run_script("Before Game.ps1", "Set-Content -Path out.txt -Value $env:HERONITE_GAME_SCRIPT_TYPE"), "before");
    }
}
