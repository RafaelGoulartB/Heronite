//! Process spawning behind the [`Spawner`] and [`ProcessHandle`] traits.

use std::io::{self, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// Opens a URI with the desktop handler (`xdg-open`), without waiting.
    fn open_uri(&self, uri: &str) -> io::Result<()>;
    /// Terminates the processes started at or after `since` (ticks since
    /// boot, [`crate::procs::boot_ticks_now`]) whose program is `pattern`
    /// ([`crate::procs::program_matches`]).
    fn kill_pattern(&self, pattern: &str, since: u64);
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
                self.exit = Some(ExitInfo { code: status.code(), signal: status.signal() });
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
        self.signal_group(libc::SIGTERM);
    }

    fn kill(&mut self) {
        signal_tree(&*self.procs, self.pid, Sig::Kill);
        self.signal_group(libc::SIGKILL);
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
        let mut c = std::process::Command::new(&cmd.program);
        c.args(&cmd.args).envs(&cmd.env).stdin(Stdio::null());
        crate::wine::drop_host_certs(cmd, &mut c);
        if let Some(dir) = cmd.cwd.as_deref().filter(|d| !d.is_empty()) {
            c.current_dir(dir);
        }
        c.process_group(0);
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

impl Spawner for SystemSpawner {
    fn spawn(&self, cmd: &Command, log_path: Option<&Path>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log_path.map_or(Sink::None, |p| Sink::File(p.to_path_buf())))
    }

    fn spawn_logged(&self, cmd: &Command, log: Option<Arc<GameLog>>) -> io::Result<Box<dyn ProcessHandle>> {
        self.spawn_with(cmd, log.map_or(Sink::None, Sink::Log))
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

    fn kill_pattern(&self, pattern: &str, since: u64) {
        // Not `pkill -f`: that matches anywhere in any command line, so a game
        // called `game` would take Steam or a terminal down with it.
        for pid in crate::procs::processes_matching(pattern, since) {
            self.procs.signal(pid, Sig::Term);
        }
    }
}
