//! Starting and waiting for the programs of a client: its setup runs and
//! its own window. Their output goes to files in the logs folder.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::{Cancel, Run};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::log_writer::LogPaths;
use heronite_launch::procs::{Procs, Sig};
use heronite_launch::Command;

/// `<logs>/clients/<file>`.
pub(crate) fn log_path(paths: &DataPaths, file: &str) -> PathBuf {
    LogPaths::for_data(paths).base.join("clients").join(file)
}

/// A new, empty log file (the folder made).
pub(crate) fn create_log(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    File::create(path)
}

/// `command` as a process to spawn: its environment and folder, and the
/// host's certificate variables left out when it runs in umu's container.
pub(crate) fn std_command(command: &Command) -> std::process::Command {
    let mut cmd = std::process::Command::new(&command.program);
    cmd.args(&command.args).envs(&command.env);
    if let Some(cwd) = command.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(cwd);
    }
    heronite_launch::wine::drop_host_certs(command, &mut cmd);
    cmd
}

/// Runs `command` until it ends (it ends with the app, as a helper does),
/// its output going to `log`. `false` when it was stopped for taking longer
/// than `timeout`. Its exit code does not matter: the caller looks at what
/// it was meant to leave behind.
pub(crate) fn run_to_completion(command: &Command, cancel: &Cancel, timeout: Duration, log: &mut File) -> Result<bool> {
    let _ = writeln!(log, "$ {}", command.display());
    let mut sink = |line: &str| {
        let _ = writeln!(log, "{line}");
    };
    let out = Run::new()
        .cancel(cancel)
        .timeout(timeout)
        .no_capture()
        .on_line(&mut sink)
        .run(&mut std_command(command))
        .map_err(|e| anyhow!(tr!("Could not start {program}: {error}", program = command.program, error = e)))?;
    if out.aborted {
        bail!(tr!("Cancelled"));
    }
    Ok(!out.timed_out)
}

/// How a setup waits for the installer it started.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Waits {
    pub poll: Duration,
    /// How long the installer's programs must be gone for the client to be
    /// taken as ready while the command still runs.
    pub quiet: Duration,
    pub timeout: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Self { poll: Duration::from_secs(1), quiet: Duration::from_secs(5), timeout: Duration::from_secs(45 * 60) }
    }
}

/// What the wait looks at.
pub(crate) struct Installer<'a> {
    /// Some program of the installer still runs in the prefix.
    pub running: &'a dyn Fn() -> bool,
    /// The client's program is in the prefix.
    pub client_installed: &'a dyn Fn() -> bool,
}

/// Waits for an installer whose command may never return: the client it
/// starts keeps the container (umu) or the Wine server running. Done when
/// the command ended and no installer program runs, or when the client is
/// installed and no installer program has run for `waits.quiet`. A command
/// still running then is left alone (it may be the client) and only reaped
/// when it ends. Cancelling or the timeout stops the command's process
/// group.
pub(crate) fn wait_for_installer(
    mut child: Child,
    procs: &dyn Procs,
    installer: &Installer,
    cancel: &Cancel,
    waits: &Waits,
    name: &str,
) -> Result<()> {
    let started = Instant::now();
    let mut last_seen = Instant::now();
    loop {
        if cancel.is_requested() {
            stop_group(&mut child, procs);
            bail!(tr!("Cancelled"));
        }
        let exited = child.try_wait()?.is_some();
        let running = (installer.running)();
        if running {
            last_seen = Instant::now();
        }
        if !running && (exited || ((installer.client_installed)() && last_seen.elapsed() >= waits.quiet)) {
            if !exited {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            return Ok(());
        }
        if started.elapsed() > waits.timeout {
            stop_group(&mut child, procs);
            bail!(tr!("{name} took too long to install", name));
        }
        std::thread::sleep(waits.poll);
    }
}

/// Ends the command and what it started (its process group): asks first,
/// then insists.
fn stop_group(child: &mut Child, procs: &dyn Procs) {
    let group = child.id();
    procs.signal_group(group, Sig::Term);
    for _ in 0..30 {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    procs.signal_group(group, Sig::Kill);
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use heronite_launch::procs::{FakeProcs, SystemProcs};
    use std::cell::Cell;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    fn sleeper(seconds: u32) -> Child {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", &format!("sleep {seconds}")]).stdin(Stdio::null()).process_group(0);
        cmd.spawn().unwrap()
    }

    fn quick() -> Waits {
        Waits { poll: Duration::from_millis(10), quiet: Duration::from_millis(60), timeout: Duration::from_secs(20) }
    }

    fn alive(pid: u32) -> bool {
        SystemProcs.is_alive(pid)
    }

    #[test]
    fn a_command_that_ended_is_done_once_no_installer_program_runs() {
        let mut quick_exit = std::process::Command::new("true").spawn().unwrap();
        quick_exit.wait().unwrap();
        let polls = Cell::new(0);
        // The installer's own program is seen twice more, then it is gone.
        let running = || {
            polls.set(polls.get() + 1);
            polls.get() <= 2
        };
        let installer = Installer { running: &running, client_installed: &|| false };
        wait_for_installer(quick_exit, &FakeProcs::new(), &installer, &Cancel::new(), &quick(), "Client").unwrap();
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn a_client_that_keeps_the_command_alive_is_left_running() {
        let child = sleeper(30);
        let pid = child.id();
        let installer = Installer { running: &|| false, client_installed: &|| true };
        let started = Instant::now();
        wait_for_installer(child, &SystemProcs, &installer, &Cancel::new(), &quick(), "Client").unwrap();
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(alive(pid), "the client the installer started is not killed");
        SystemProcs.signal_group(pid, Sig::Kill);
    }

    #[test]
    fn nothing_is_ready_while_the_client_is_missing_or_the_installer_runs() {
        let waits = Waits { timeout: Duration::from_millis(300), ..quick() };
        for (running, installed) in [(false, false), (true, true)] {
            let child = sleeper(30);
            let pid = child.id();
            let installer = Installer { running: &move || running, client_installed: &move || installed };
            let err = wait_for_installer(child, &SystemProcs, &installer, &Cancel::new(), &waits, "Battle.net").unwrap_err();
            assert_eq!(err.to_string(), "Battle.net took too long to install");
            std::thread::sleep(Duration::from_millis(50));
            assert!(!alive(pid), "the installer's group was stopped");
        }
    }

    #[test]
    fn cancelling_stops_the_installers_group() {
        let child = sleeper(30);
        let pid = child.id();
        let cancel = Cancel::new();
        cancel.request();
        let installer = Installer { running: &|| true, client_installed: &|| false };
        let err = wait_for_installer(child, &SystemProcs, &installer, &cancel, &quick(), "Client").unwrap_err();
        assert_eq!(err.to_string(), "Cancelled");
        assert!(!alive(pid));
    }

    #[test]
    fn a_command_runs_to_its_end_with_its_output_in_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = create_log(&dir.path().join("logs/setup.log")).unwrap();
        let mut command = Command {
            program: "sh".into(),
            args: vec!["-c".into(), "echo out; echo err >&2; echo $CLIENT_VAR".into()],
            ..Default::default()
        };
        command.env.insert("CLIENT_VAR".into(), "set".into());
        assert!(run_to_completion(&command, &Cancel::new(), Duration::from_secs(20), &mut log).unwrap());
        let text = std::fs::read_to_string(dir.path().join("logs/setup.log")).unwrap();
        assert!(text.starts_with("$ sh -c "), "{text}");
        for line in ["out", "err", "set"] {
            assert!(text.lines().any(|l| l == line), "{line} in {text}");
        }

        let slow = Command { program: "sleep".into(), args: vec!["30".into()], ..Default::default() };
        assert!(!run_to_completion(&slow, &Cancel::new(), Duration::from_millis(200), &mut log).unwrap());
        let cancel = Cancel::new();
        cancel.request();
        assert_eq!(run_to_completion(&slow, &cancel, Duration::from_secs(20), &mut log).unwrap_err().to_string(), "Cancelled");
        let missing = Command { program: "/nope/never".into(), ..Default::default() };
        assert!(run_to_completion(&missing, &Cancel::new(), Duration::from_secs(5), &mut log)
            .unwrap_err()
            .to_string()
            .starts_with("Could not start /nope/never"));
    }
}
