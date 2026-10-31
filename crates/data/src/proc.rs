//! One way to run helper processes (rsync, tar, ludusavi, legendary, the Epic
//! login window...).
//!
//! Helpers must not outlive the app: each one gets its own process group and,
//! on Linux, `PR_SET_PDEATHSIG(SIGTERM)` so it is told to stop when the
//! thread that spawned it dies (the app quitting or being killed). The
//! blocking runner here also supports cancellation: the helper is asked to
//! stop first (SIGTERM; Ctrl+C on Windows, see
//! `heronite_platform::process::ask_to_stop`), the whole group is killed
//! after a grace period. On Windows the group is a job object and a helper
//! has a console of its own without a window.
//!
//! Do NOT use this for games / emulators (they must survive the launcher) nor
//! for detached openers (`xdg-open`, file managers, editors).
//!
//! Note: `PDEATHSIG` is tied to the spawning *thread*, so spawn from a thread
//! that stays alive while it waits for the child (all callers block on it).

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use heronite_platform::process::{self as os, Signal};

/// Makes `cmd` a guarded helper: own process group, SIGTERM when the spawning
/// thread dies. The `PDEATHSIG` part is Linux-only.
pub fn guard(cmd: &mut Command) -> &mut Command {
    os::new_helper_group(cmd);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: only an async-signal-safe prctl between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    cmd
}

/// Idle CPU scheduling and idle I/O class for the calling thread (and the
/// processes it spawns from now on): it only gets time nobody else wants, so
/// a game or the UI never waits for it. Call it only on a thread of its own.
pub fn idle_priority() {
    #[cfg(target_os = "linux")]
    {
        const IOPRIO_WHO_PROCESS: libc::c_int = 1;
        const IOPRIO_CLASS_IDLE: libc::c_int = 3;
        const IOPRIO_CLASS_SHIFT: libc::c_int = 13;
        let param = libc::sched_param { sched_priority: 0 };
        // SAFETY: pid 0 is the calling thread; both calls only change its
        // own scheduling and fail harmlessly.
        unsafe {
            if libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) != 0 {
                log::debug!("SCHED_IDLE: {}", std::io::Error::last_os_error());
            }
            libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT);
        }
    }
}

/// Lets another thread stop a running command: asked first, once (SIGTERM,
/// Ctrl+C on Windows: legendary then saves its resume data), the whole
/// process group killed if it lingers.
#[derive(Debug, Default)]
pub struct Cancel {
    requested: AtomicBool,
    pid: AtomicI32,
    group: AtomicBool,
    /// The running command was asked to stop (a second Ctrl+C would cut a
    /// Python tool's own clean-up short).
    asked: AtomicBool,
}

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid > 0 {
            self.ask(pid);
        }
    }

    /// Asks the command `pid` to stop, unless it already was.
    fn ask(&self, pid: i32) {
        if !self.asked.swap(true, Ordering::SeqCst) {
            os::ask_to_stop(pid as u32, self.group.load(Ordering::SeqCst));
        }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Whether a run is in flight (the pid is published while it lives).
    pub fn is_running(&self) -> bool {
        self.pid.load(Ordering::SeqCst) > 0
    }
}

/// `cmd.spawn()`, tried again for a moment when the program is "busy"
/// (`ETXTBSY`). That happens right after the file was written (a download,
/// a generated script): a process forked meanwhile by another thread holds the
/// written file open until its own `exec`, and executing a file open for
/// writing fails.
///
/// The child's process group starts here too (a job object on Windows,
/// see `heronite_platform::process::track_group`).
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let mut tries = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 40 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(child) => {
                os::track_group(&child);
                return Ok(child);
            }
            other => return other,
        }
    }
}

/// Signals the child (`whole_group = false`) or its whole process group.
fn signal(pid: i32, sig: Signal, whole_group: bool) {
    if pid > 0 {
        if whole_group {
            os::signal_group(pid as u32, sig);
        } else {
            os::signal(pid as u32, sig);
        }
    }
}

/// Result of a finished run.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    /// Stopped through the [`Cancel`] / poll function.
    pub aborted: bool,
    pub timed_out: bool,
}

impl RunOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.aborted && !self.timed_out
    }
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

const CAPTURE_LIMIT: usize = 4 * 1024 * 1024;

/// Builder for a blocking, guarded, cancellable run.
pub struct Run<'a> {
    cancel: Option<&'a Cancel>,
    poll: Option<&'a dyn Fn() -> bool>,
    on_line: Option<&'a mut dyn FnMut(&str)>,
    timeout: Option<Duration>,
    grace: Duration,
    term_group: bool,
    lines: bool,
    /// Whether the output is kept for [`RunOutput`].
    keep: bool,
}

impl Default for Run<'_> {
    fn default() -> Self {
        Self {
            cancel: None,
            poll: None,
            on_line: None,
            timeout: None,
            grace: Duration::from_secs(15),
            term_group: true,
            lines: false,
            keep: true,
        }
    }
}

impl<'a> Run<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Stop when another thread calls [`Cancel::request`].
    pub fn cancel(mut self, cancel: &'a Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }
    /// Stop when the function returns true (polled every ~100 ms).
    pub fn poll(mut self, poll: &'a dyn Fn() -> bool) -> Self {
        self.poll = Some(poll);
        self
    }
    /// Called for every output line of both streams (split on `\n` and `\r`).
    pub fn on_line(mut self, f: &'a mut dyn FnMut(&str)) -> Self {
        self.on_line = Some(f);
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    /// Time between asking (SIGTERM, Ctrl+C on Windows) and the kill of the
    /// group (default 15 s).
    pub fn grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }
    /// SIGTERM only the main process, so it can save its state (legendary);
    /// the whole group is still SIGKILLed after the grace period. (On Windows
    /// the Ctrl+C reaches every process on the helper's console either way.)
    pub fn term_main_only(mut self) -> Self {
        self.term_group = false;
        self
    }
    /// Captured text is the lines joined with `\n` (progress redraws with `\r`
    /// become separate lines) instead of the raw bytes.
    pub fn line_capture(mut self) -> Self {
        self.lines = true;
        self
    }
    /// Keeps none of the output (it only reaches [`Run::on_line`]): for a
    /// program that runs for hours.
    pub fn no_capture(mut self) -> Self {
        self.keep = false;
        self
    }

    /// Spawns `cmd` (guarded, stdin closed, stdout / stderr captured) and
    /// waits for it. `Err` means it could not be started.
    pub fn run(mut self, cmd: &mut Command) -> io::Result<RunOutput> {
        guard(cmd);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = spawn(cmd)?;
        let pid = child.id() as i32;
        let local = Cancel::default();
        let cancel = self.cancel.unwrap_or(&local);
        cancel.group.store(self.term_group, Ordering::SeqCst);
        cancel.asked.store(false, Ordering::SeqCst);
        cancel.pid.store(pid, Ordering::SeqCst);
        // A cancel requested before the pid was known.
        if cancel.is_requested() {
            cancel.request();
        }
        let result = self.wait(&mut child, pid, cancel);
        // Whatever of the process group is left (download workers).
        signal(pid, Signal::Kill, true);
        cancel.pid.store(0, Ordering::SeqCst);
        result
    }

    fn wait(&mut self, child: &mut Child, pid: i32, cancel: &Cancel) -> io::Result<RunOutput> {
        let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
        let mut readers = Vec::new();
        if let Some(pipe) = child.stdout.take() {
            readers.push(spawn_reader(pipe, false, tx.clone()));
        }
        if let Some(pipe) = child.stderr.take() {
            readers.push(spawn_reader(pipe, true, tx.clone()));
        }
        drop(tx);

        let mut out = RunOutput::default();
        let mut split = [LineSplitter::default(), LineSplitter::default()];
        let started = Instant::now();
        let mut term_sent_at: Option<Instant> = None;
        let mut gone_at: Option<Instant> = None;
        let mut status: Option<ExitStatus> = None;
        let (lines, keep) = (self.lines, self.keep);
        let feed =
            |out: &mut RunOutput, split: &mut [LineSplitter; 2], on_line: &mut Option<&mut dyn FnMut(&str)>, is_err: bool, chunk: &[u8]| {
                let buf = if is_err { &mut out.stderr } else { &mut out.stdout };
                if keep && !lines && buf.len() < CAPTURE_LIMIT {
                    buf.extend_from_slice(chunk);
                }
                split[is_err as usize].feed(chunk, |line| {
                    if let Some(f) = on_line.as_mut() {
                        f(line);
                    }
                    if keep && lines && buf.len() < CAPTURE_LIMIT {
                        buf.extend_from_slice(line.as_bytes());
                        buf.push(b'\n');
                    }
                });
            };
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok((is_err, chunk)) => feed(&mut out, &mut split, &mut self.on_line, is_err, &chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(30)),
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if status.is_none() && !out.timed_out && self.timeout.is_some_and(|t| started.elapsed() > t) {
                out.timed_out = true;
            }
            if cancel.is_requested() || self.poll.is_some_and(|p| p()) {
                out.aborted = true;
            }
            if (out.aborted || out.timed_out) && status.is_none() {
                match term_sent_at {
                    None => {
                        term_sent_at = Some(Instant::now());
                        cancel.ask(pid);
                    }
                    Some(at) if at.elapsed() > self.grace => signal(pid, Signal::Kill, true),
                    _ => {}
                }
            }
            if status.is_some() {
                if readers.iter().all(|r| r.is_finished()) {
                    // Drain what the readers sent just before finishing.
                    while let Ok((is_err, chunk)) = rx.try_recv() {
                        feed(&mut out, &mut split, &mut self.on_line, is_err, &chunk);
                    }
                    break;
                }
                // A worker that inherited the pipes can keep them open after
                // the main process died: stop waiting for it after a moment.
                let at = *gone_at.get_or_insert_with(Instant::now);
                if at.elapsed() > Duration::from_secs(2) {
                    signal(pid, Signal::Kill, true);
                    while let Ok((is_err, chunk)) = rx.try_recv() {
                        feed(&mut out, &mut split, &mut self.on_line, is_err, &chunk);
                    }
                    break;
                }
            }
        }
        for (i, s) in split.iter_mut().enumerate() {
            let is_err = i == 1;
            let buf = if is_err { &mut out.stderr } else { &mut out.stdout };
            s.finish(|line| {
                if let Some(f) = self.on_line.as_mut() {
                    f(line);
                }
                if keep && lines && buf.len() < CAPTURE_LIMIT {
                    buf.extend_from_slice(line.as_bytes());
                    buf.push(b'\n');
                }
            });
        }
        let status = match status {
            Some(s) => s,
            None => child.wait()?,
        };
        out.code = status.code();
        Ok(out)
    }
}

fn spawn_reader(mut pipe: impl Read + Send + 'static, is_err: bool, tx: mpsc::Sender<(bool, Vec<u8>)>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((is_err, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

/// Splits a byte stream on `\n` and `\r` (legendary redraws progress with
/// `\r`); empty lines are dropped.
#[derive(Default)]
struct LineSplitter {
    line: Vec<u8>,
}

impl LineSplitter {
    fn feed(&mut self, chunk: &[u8], mut emit: impl FnMut(&str)) {
        for &b in chunk {
            if b == b'\n' || b == b'\r' {
                if !self.line.is_empty() {
                    emit(&String::from_utf8_lossy(&self.line));
                    self.line.clear();
                }
            } else {
                self.line.push(b);
            }
        }
    }

    fn finish(&mut self, mut emit: impl FnMut(&str)) {
        if !self.line.is_empty() {
            emit(&String::from_utf8_lossy(&self.line));
            self.line.clear();
        }
    }
}

/// Runs `cmd` guarded and captures its output, not cancellable.
pub fn output(cmd: &mut Command) -> io::Result<RunOutput> {
    Run::new().run(cmd)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn script(dir: &Path, body: &str) -> Command {
        let bin = dir.join("helper.sh");
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Command::new(bin)
    }

    #[test]
    fn a_program_still_open_for_writing_is_started_once_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = script(dir.path(), "exit 0");
        let writer = std::fs::OpenOptions::new().append(true).open(dir.path().join("helper.sh")).unwrap();
        assert_eq!(cmd.spawn().unwrap_err().kind(), io::ErrorKind::ExecutableFileBusy);
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(writer);
        });
        let mut child = spawn(&mut cmd).unwrap();
        assert!(child.wait().unwrap().success());
        closer.join().unwrap();
        // Other errors are not retried.
        let started = Instant::now();
        assert_eq!(spawn(&mut Command::new(dir.path().join("missing"))).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < Duration::from_millis(20));
    }

    fn alive(pid: i32) -> bool {
        // Zombies count as dead.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => s.rsplit(") ").next().is_some_and(|rest| !rest.starts_with('Z')),
            Err(_) => false,
        }
    }

    fn wait_dead(pid: i32, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        !alive(pid)
    }

    #[test]
    fn guard_puts_the_child_in_its_own_group() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = script(dir.path(), "echo \"$$ $(cut -d' ' -f5 /proc/$$/stat)\"");
        let out = output(&mut cmd).unwrap();
        let text = out.stdout_text();
        let mut it = text.split_whitespace();
        let (pid, pgrp) = (it.next().unwrap(), it.next().unwrap());
        assert_eq!(pid, pgrp, "the helper leads its own process group");
        assert_ne!(pgrp.parse::<u32>().unwrap(), unsafe { libc::getpgrp() } as u32);
    }

    #[test]
    fn helpers_get_sigterm_when_the_spawning_thread_dies() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = script(dir.path(), "sleep 60");
        let (tx, rx) = mpsc::channel();
        let t = std::thread::spawn(move || {
            guard(&mut cmd);
            let child = cmd.stdin(Stdio::null()).spawn().unwrap();
            tx.send(child.id() as i32).unwrap();
            // The thread ends here without waiting for the child.
            std::mem::forget(child);
        });
        let pid = rx.recv().unwrap();
        t.join().unwrap();
        assert!(wait_dead(pid, 5), "helper {pid} survived its spawning thread");
        // Reap the zombie.
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
    }

    #[test]
    fn captures_raw_and_line_output() {
        let dir = tempfile::tempdir().unwrap();
        let body = "printf 'a\\rb\\nc'; echo err >&2; exit 3";
        let out = Run::new().run(&mut script(dir.path(), body)).unwrap();
        assert_eq!(out.stdout, b"a\rb\nc");
        assert_eq!(out.stderr_text().trim(), "err");
        assert_eq!(out.code, Some(3));
        assert!(!out.success());
        let mut seen = Vec::new();
        let mut on_line = |l: &str| seen.push(l.to_string());
        let out = Run::new().line_capture().on_line(&mut on_line).run(&mut script(dir.path(), body)).unwrap();
        assert_eq!(out.stdout_text(), "a\nb\nc\n");
        seen.sort();
        assert_eq!(seen, ["a", "b", "c", "err"]);
        let mut count = 0;
        let mut on_line = |_: &str| count += 1;
        let out = Run::new().no_capture().on_line(&mut on_line).run(&mut script(dir.path(), body)).unwrap();
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "nothing kept");
        assert_eq!((count, out.code), (4, Some(3)), "the lines still reach on_line");
    }

    #[test]
    fn cancel_stops_the_whole_group() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = script(dir.path(), "sleep 60 & echo $!; wait");
        let cancel = Cancel::new();
        let bg = Arc::new(AtomicI32::new(0));
        let (c2, bg2) = (cancel.clone(), bg.clone());
        let mut on_line = move |l: &str| {
            bg2.store(l.trim().parse().unwrap_or(0), Ordering::SeqCst);
            c2.request();
        };
        let started = Instant::now();
        let out = Run::new().cancel(&cancel).on_line(&mut on_line).run(&mut cmd).unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
        let pid = bg.load(Ordering::SeqCst);
        assert!(pid > 0);
        assert!(wait_dead(pid, 5), "background worker {pid} was left behind");
        assert!(!cancel.is_running());
    }

    #[test]
    fn poll_function_cancels_and_stubborn_children_get_killed_after_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        // Ignores SIGTERM: only the SIGKILL after the grace period stops it.
        let mut cmd = script(dir.path(), "trap '' TERM; while true; do sleep 0.1; done");
        let stop_at = Instant::now() + Duration::from_millis(300);
        let poll = move || Instant::now() > stop_at;
        let started = Instant::now();
        let out = Run::new().poll(&poll).grace(Duration::from_millis(500)).run(&mut cmd).unwrap();
        assert!(out.aborted);
        assert_eq!(out.code, None, "killed by a signal");
        assert!(started.elapsed() < Duration::from_secs(8), "took {:?}", started.elapsed());
    }

    #[test]
    fn timeouts_kill_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = script(dir.path(), "sleep 60");
        let started = Instant::now();
        let out = Run::new().timeout(Duration::from_millis(300)).grace(Duration::from_millis(300)).run(&mut cmd).unwrap();
        assert!(out.timed_out && !out.success());
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn missing_binary_is_an_error() {
        assert!(output(&mut Command::new("/nonexistent/helper")).is_err());
    }
}
