//! Running the `legendary` CLI the way Heroic's `callRunner` does:
//! `LEGENDARY_CONFIG_PATH` points at Heroic's own config folder, arguments are
//! passed as-is and output (stdout and stderr) is captured, or streamed line by
//! line for long operations that can be cancelled.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use heronite_data::HeroicPaths;
use heronite_launch::{legendary::find_legendary, PlanContext, SystemProbe};

/// The legendary binary and the config folder it works on.
#[derive(Debug, Clone)]
pub struct Legendary {
    pub bin: PathBuf,
    /// `<heroic>/legendaryConfig/legendary`.
    pub config_dir: PathBuf,
}

/// Result of a finished run.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    /// The run was stopped through its [`Cancel`].
    pub aborted: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.aborted
    }

    /// `ERROR:` / `CRITICAL:` lines of legendary's log, for messages.
    pub fn error_lines(&self) -> String {
        let all = format!("{}\n{}", self.stderr, self.stdout);
        let lines: Vec<&str> =
            all.lines().filter(|l| l.contains("ERROR") || l.contains("CRITICAL") || l.contains("Traceback")).collect();
        lines.join("\n")
    }
}

/// Lets another thread stop a running command: SIGTERM first (legendary then
/// saves its resume data), SIGKILL of the whole process group if it lingers.
#[derive(Debug, Default)]
pub struct Cancel {
    requested: AtomicBool,
    pid: AtomicI32,
}

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        {
            let pid = self.pid.load(Ordering::SeqCst);
            if pid > 0 {
                // SAFETY: plain signal to a process this struct spawned.
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

impl Legendary {
    /// Finds the binary like Heronite does (`altLegendaryBin`, the bundled one,
    /// `$PATH`; `HERONITE_LEGENDARY_BIN` overrides all).
    pub fn locate(heroic: &HeroicPaths) -> Result<Self> {
        let ctx = PlanContext::load(heroic);
        let bin = find_legendary(&ctx, &SystemProbe).ok_or_else(|| anyhow!("The legendary binary was not found"))?;
        Ok(Self { bin: PathBuf::from(bin), config_dir: heroic.legendary_config_dir() })
    }

    pub fn with_bin(bin: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into(), config_dir: config_dir.into() }
    }

    fn command(&self, args: &[String], env: &[(String, String)]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        if let Some(dir) = self.bin.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.env("LEGENDARY_CONFIG_PATH", &self.config_dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Own process group (workers die with it) and SIGTERM if this app dies.
            cmd.process_group(0);
            // SAFETY: only async-signal-safe prctl between fork and exec.
            unsafe {
                cmd.pre_exec(|| {
                    #[cfg(target_os = "linux")]
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        cmd
    }

    /// Runs to completion and returns the output. Blocking.
    pub fn run<S: AsRef<str>>(&self, args: &[S], env: &[(String, String)]) -> Result<Output> {
        self.run_streaming(args, env, &Cancel::new(), |_| {})
    }

    /// Like [`run`](Self::run) but calls `on_line` for every output line (both
    /// streams, split on `\n` and `\r`) while the command runs, and stops it
    /// when `cancel` is requested.
    pub fn run_streaming<S: AsRef<str>>(
        &self,
        args: &[S],
        env: &[(String, String)],
        cancel: &Cancel,
        mut on_line: impl FnMut(&str),
    ) -> Result<Output> {
        let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        log::info!("legendary {}", redact(&args).join(" "));
        let mut child = self
            .command(&args, env)
            .spawn()
            .with_context(|| format!("starting {}", self.bin.display()))?;
        let pid = child.id() as i32;
        cancel.pid.store(pid, Ordering::SeqCst);
        // A cancel requested before the pid was known.
        if cancel.is_requested() {
            cancel.request();
        }

        let (tx, rx) = mpsc::channel::<(bool, String)>();
        let mut readers = Vec::new();
        if let Some(pipe) = child.stdout.take() {
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || read_lines(pipe, |line| drop(tx.send((false, line))))));
        }
        if let Some(pipe) = child.stderr.take() {
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || read_lines(pipe, |line| drop(tx.send((true, line))))));
        }
        drop(tx);

        let mut out = Output::default();
        let mut term_sent_at: Option<Instant> = None;
        let mut gone_at: Option<Instant> = None;
        let mut status = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok((is_err, line)) => {
                    on_line(&line);
                    let buf = if is_err { &mut out.stderr } else { &mut out.stdout };
                    if buf.len() < 4 * 1024 * 1024 {
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(50)),
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if cancel.is_requested() {
                match term_sent_at {
                    None if status.is_none() => {
                        term_sent_at = Some(Instant::now());
                        signal_group(pid, libc::SIGTERM, false);
                    }
                    Some(at) if status.is_none() && at.elapsed() > Duration::from_secs(15) => {
                        signal_group(pid, libc::SIGKILL, true)
                    }
                    _ => {}
                }
            }
            if status.is_some() {
                if readers.iter().all(|r| r.is_finished()) {
                    // Drain what the readers sent just before finishing.
                    while let Ok((is_err, line)) = rx.try_recv() {
                        on_line(&line);
                        let buf = if is_err { &mut out.stderr } else { &mut out.stdout };
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    break;
                }
                // A worker that inherited the pipes can keep them open after
                // the main process died: stop waiting for it after a moment.
                let at = *gone_at.get_or_insert_with(Instant::now);
                if at.elapsed() > Duration::from_secs(2) {
                    signal_group(pid, libc::SIGKILL, true);
                    while let Ok((is_err, line)) = rx.try_recv() {
                        on_line(&line);
                        let buf = if is_err { &mut out.stderr } else { &mut out.stdout };
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    break;
                }
            }
        }
        // Whatever of the process group is left (download workers).
        signal_group(pid, libc::SIGKILL, true);
        cancel.pid.store(0, Ordering::SeqCst);
        let status = match status {
            Some(s) => s,
            None => child.wait()?,
        };
        out.code = status.code();
        out.aborted = cancel.is_requested();
        Ok(out)
    }
}

/// Signals the child (`whole_group = false`: legendary only, so it can save
/// its resume data) or its whole process group (download workers included).
#[cfg(unix)]
fn signal_group(pid: i32, sig: i32, whole_group: bool) {
    if pid > 0 {
        // SAFETY: signalling the child / process group created for this run.
        unsafe { libc::kill(if whole_group { -pid } else { pid }, sig) };
    }
}

#[cfg(not(unix))]
fn signal_group(_pid: i32, _sig: i32, _whole_group: bool) {}

/// Splits a stream on `\n` and `\r` (legendary redraws progress with `\r`).
fn read_lines(mut pipe: impl Read, mut emit: impl FnMut(String)) {
    let mut buf = [0u8; 8192];
    let mut line: Vec<u8> = Vec::new();
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !line.is_empty() {
                            emit(String::from_utf8_lossy(&line).into_owned());
                            line.clear();
                        }
                    } else {
                        line.push(b);
                    }
                }
            }
        }
    }
    if !line.is_empty() {
        emit(String::from_utf8_lossy(&line).into_owned());
    }
}

/// Arguments for the log with the authorization code hidden.
pub fn redact(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for (i, a) in args.iter().enumerate() {
        if (a == "--code" || a == "--token" || a == "--sid") && i + 1 < out.len() {
            out[i + 1] = "<redacted>".into();
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A fake `legendary`: a shell script that records its arguments and the
    /// config path, then runs `body`.
    pub fn fake(dir: &Path, body: &str) -> Legendary {
        let bin = dir.join("legendary");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\necho \"CFG=$LEGENDARY_CONFIG_PATH\" >> \"{log}\"\n{body}\n",
            log = dir.join("calls.log").display()
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Legendary::with_bin(bin, dir.join("cfg"))
    }

    pub fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with("CFG="))
            .map(str::to_string)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn captures_both_streams_and_passes_the_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let lg = fake(dir.path(), "echo out; echo err >&2; exit 3");
        let out = lg.run(&["list", "--third-party"], &[]).unwrap();
        assert_eq!(out.code, Some(3));
        assert!(!out.success());
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(calls(dir.path()), ["list --third-party"]);
        let log = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
        assert!(log.contains(&format!("CFG={}", dir.path().join("cfg").display())), "{log}");
    }

    #[test]
    fn streams_lines_split_on_carriage_returns() {
        let dir = tempfile::tempdir().unwrap();
        let lg = fake(dir.path(), "printf 'a\\rb\\nc' >&2");
        let mut lines = Vec::new();
        let out = lg.run_streaming(&["x"], &[], &Cancel::new(), |l| lines.push(l.to_string())).unwrap();
        assert!(out.success());
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn cancel_stops_the_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let lg = fake(dir.path(), "echo started; sleep 60 & wait");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.request();
        });
        let started = Instant::now();
        let out = lg.run_streaming(&["install"], &[], &cancel, |_| {}).unwrap();
        t.join().unwrap();
        assert!(out.aborted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[test]
    fn missing_binary_is_an_error() {
        let lg = Legendary::with_bin("/nonexistent/legendary", "/tmp/x");
        assert!(lg.run(&["list"], &[]).is_err());
    }

    #[test]
    fn codes_are_redacted_in_logs() {
        let args: Vec<String> = ["auth", "--code", "secret"].iter().map(|s| s.to_string()).collect();
        assert_eq!(redact(&args), ["auth", "--code", "<redacted>"]);
    }
}
