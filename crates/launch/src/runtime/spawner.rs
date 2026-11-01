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
}

pub trait Spawner: Send + Sync {
    /// Starts `cmd`; its stdout/stderr go to `log_path` when given.
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>>;
    /// Starts `cmd`; its stdout/stderr are appended to `log` (the game's
    /// Heroic-style log) as they arrive.
    fn spawn_logged(&self, cmd: &Command, _log: Option<Arc<GameLog>>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn(cmd, None)
    }
    /// Opens a URI with the desktop handler (`xdg-open`, the shell on
    /// Windows), without waiting.
    fn open_uri(&self, uri: &str) -> io::Result<()>;
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
}

/// Where the output of a spawned process goes.
enum Sink {
    None,
    /// A plain file, created fresh with a "Launching:" header.
    File(PathBuf),
    /// The game's log.
    Log(Arc<GameLog>),
}

impl SystemSpawner {
    /// The child's stdout and stderr are the file itself (no pipe, no pump
    /// thread): the game keeps working, and keeps logging, if the launcher
    /// exits before it does.
    fn spawn_with(&self, cmd: &Command, sink: Sink) -> io::Result<Box<dyn ProcessHandle>> {
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
        if cfg!(windows) && (host.is_some() || is_batch(&cmd.program)) {
            os::new_helper_group(&mut c);
        } else {
            os::new_group(&mut c);
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

fn extension(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

impl Spawner for SystemSpawner {
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log_path.map_or(Sink::None, |p| Sink::File(p.to_path_buf())))
    }

    fn spawn_logged(&self, cmd: &Command, log: Option<Arc<GameLog>>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log.map_or(Sink::None, Sink::Log))
    }

    fn open_uri(&self, uri: &str) -> io::Result<()> {
        if cfg!(windows) {
            return heronite_platform::shell::open(uri);
        }
        let mut c = std::process::Command::new("xdg-open");
        c.arg(uri)
            .env_clear()
            .envs(desktop_protocol_env(std::env::vars()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        os::new_group(&mut c);
        let mut child = c.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
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
