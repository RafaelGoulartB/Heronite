//! Starting and waiting for the programs of a client: its setup runs and
//! its own window. Their output goes to files in the logs folder.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use heronite_data::clients::Client;
use heronite_data::proc::{Cancel, Run};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::log_writer::LogPaths;
use heronite_launch::net::{Net, CANCELLED};
use heronite_launch::procs::{Procs, Sig};
use heronite_launch::runtimes::{self, Runtime};
use heronite_launch::settings::GameSettings;
use heronite_launch::{Command, PlanContext, Probe};

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

/// Downloads the umu runtime when `settings` run a Proton build through it
/// and neither `umu-run` on `$PATH` nor the bundled copy is present yet.
/// `Ok(true)` when umu was downloaded, `Ok(false)` when nothing was needed.
/// `can_disable_umu` includes the hint to turn on "Disable umu" in Wine
/// settings (available once the client is installed, hidden during first setup).
pub(crate) fn ensure_umu(
    ctx: &PlanContext,
    probe: &dyn Probe,
    net: &dyn Net,
    settings: &GameSettings,
    cancel: &Cancel,
    can_disable_umu: bool,
    before_download: impl FnOnce(),
) -> Result<bool, String> {
    if !heronite_launch::wine::prepare_for_launch(ctx, probe, settings)?.umu_download {
        return Ok(false);
    }
    static UMU_LOCK: Mutex<()> = Mutex::new(());
    let _guard = UMU_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !heronite_launch::wine::prepare_for_launch(ctx, probe, settings)?.umu_download {
        return Ok(false);
    }
    if cancel.is_requested() {
        return Err(tr!("Cancelled").into());
    }
    if !net.is_online() {
        return Err(umu_download_failed("no internet connection", can_disable_umu));
    }
    before_download();
    let check = || cancel.is_requested();
    match runtimes::download(&ctx.paths, net, Runtime::Umu, &check) {
        Ok(()) => Ok(true),
        Err(e) if e == CANCELLED => Err(tr!("Cancelled").into()),
        Err(e) => Err(umu_download_failed(&e, can_disable_umu)),
    }
}

fn umu_download_failed(error: &str, can_disable_umu: bool) -> String {
    if can_disable_umu {
        tr!(
            "The umu runtime is not installed and could not be downloaded ({error}). Enable \"Disable umu\" in Wine settings to run Proton directly.",
            error
        )
    } else {
        tr!("The umu runtime is not installed and could not be downloaded ({error}).", error)
    }
}

/// Logs the runner, Proton build (if any) and log file of a client command to
/// the main app log, so launcher logs show what ran.
pub(crate) fn log_launch(client: Client, action: &str, command: &Command, log_file: &Path) {
    match command.env.get("PROTONPATH") {
        Some(proton) => log::info!("{}: {action} via {} (proton: {}, log: {})", client.name(), command.program, proton, log_file.display()),
        None => log::info!("{}: {action} via {} (log: {})", client.name(), command.program, log_file.display()),
    }
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
pub(crate) mod tests {
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

    /// Real filesystem probe that hides any host `umu-run` on `$PATH`, so tests
    /// behave the same on machines with and without a system umu install.
    pub(crate) struct DiskProbeNoUmu(pub String);

    impl Probe for DiskProbeNoUmu {
        fn exists(&self, path: &str) -> bool {
            !path.is_empty() && Path::new(path).exists()
        }
        fn read_head(&self, _path: &str, _n: usize) -> Option<Vec<u8>> {
            None
        }
        fn which(&self, _name: &str) -> Option<String> {
            None
        }
        fn env_var(&self, _key: &str) -> Option<String> {
            None
        }
        fn home(&self) -> String {
            self.0.clone()
        }
        fn steam_roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn steam_system_roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    /// A `FakeNet` serving a `umu` archive whose `umu_run.py` runs `script`.
    pub(crate) fn umu_net(dir: &Path, script: &str) -> heronite_launch::net::FakeNet {
        let src = dir.join("umu-src/umu");
        std::fs::create_dir_all(&src).unwrap();
        let py = src.join("umu_run.py");
        std::fs::write(&py, script).unwrap();
        heronite_platform::fs::make_executable(&py).unwrap();
        let archive = dir.join("umu.tar.gz");
        let ok = std::process::Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(dir.join("umu-src"))
            .arg("umu")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let bytes = std::fs::read(&archive).unwrap();
        heronite_launch::net::FakeNet::online()
            .with_json(runtimes::RUNTIMES_URL, serde_json::json!([{"name": "umu", "url": "http://x/umu.tar.gz", "created_at": "T1"}]))
            .with_file("http://x/umu.tar.gz", bytes)
    }

    #[test]
    fn ensure_umu_downloads_for_proton_when_missing_and_skips_otherwise() {
        use heronite_launch::net::FakeNet;
        use heronite_launch::probe::FakeProbe;
        use heronite_launch::settings::WineVersion;

        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let ctx = PlanContext::load(&paths);
        let mut proton = GameSettings {
            wine_version: WineVersion { bin: "/p/GE/proton".into(), name: "GE".into(), kind: "proton".into(), ..Default::default() },
            wine_prefix: "/pfx".into(),
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_path("/p/GE/proton");

        // Plain Wine, disableUMU, sandbox, or an existing umu: no network call.
        let quiet = FakeNet::online();
        let wine = GameSettings {
            wine_version: WineVersion { bin: "/w/wine".into(), name: "W".into(), kind: "wine".into(), ..Default::default() },
            ..proton.clone()
        };
        let wine_probe = FakeProbe::new("/h").with_path("/w/wine").with_path("/w/wineserver");
        assert_eq!(ensure_umu(&ctx, &wine_probe, &quiet, &wine, &Cancel::new(), true, || panic!("no download")), Ok(false));

        let mut no_umu = proton.clone();
        no_umu.disable_umu = Some(true);
        assert_eq!(ensure_umu(&ctx, &probe, &quiet, &no_umu, &Cancel::new(), true, || panic!("no download")), Ok(false));

        let with_sys_umu = probe.clone().with_bin("umu-run", "/usr/bin/umu-run");
        assert_eq!(ensure_umu(&ctx, &with_sys_umu, &quiet, &proton, &Cancel::new(), true, || panic!("no download")), Ok(false));
        assert!(quiet.requests().is_empty());

        // Offline: installed client points to "Disable umu", first-time setup does not.
        let err = ensure_umu(&ctx, &probe, &FakeNet::offline(), &proton, &Cancel::new(), true, || panic!("offline")).unwrap_err();
        assert!(err.contains("umu") && err.contains("Disable umu"), "{err}");
        let setup_err = ensure_umu(&ctx, &probe, &FakeNet::offline(), &proton, &Cancel::new(), false, || panic!("offline")).unwrap_err();
        assert!(setup_err.contains("umu") && !setup_err.contains("Wine settings"), "{setup_err}");

        // Cancelled before download.
        let cancel = Cancel::new();
        cancel.request();
        let err = ensure_umu(&ctx, &probe, &FakeNet::online(), &proton, &cancel, true, || panic!("cancelled")).unwrap_err();
        assert_eq!(err, "Cancelled");

        // Missing and online: downloads and unpacks `umu_run.py`.
        let net = umu_net(dir.path(), "#!/bin/sh\n");
        let notified = Cell::new(false);
        assert_eq!(ensure_umu(&ctx, &probe, &net, &proton, &Cancel::new(), true, || notified.set(true)), Ok(true));
        assert!(notified.get());
        assert!(runtimes::umu_script(&paths).is_file());

        // Sandbox skips umu too.
        proton.sandbox.enabled = true;
        assert_eq!(ensure_umu(&ctx, &probe, &quiet, &proton, &Cancel::new(), true, || panic!("sandbox")), Ok(false));
    }

    #[test]
    fn concurrent_ensure_umu_downloads_once() {
        use heronite_launch::settings::WineVersion;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let proton_bin = dir.path().join("proton");
        std::fs::write(&proton_bin, "#!/bin/sh\n").unwrap();
        let settings = GameSettings {
            wine_version: WineVersion {
                bin: proton_bin.to_string_lossy().into_owned(),
                name: "GE".into(),
                kind: "proton".into(),
                ..Default::default()
            },
            wine_prefix: dir.path().join("pfx").to_string_lossy().into_owned(),
            ..Default::default()
        };
        let net = Arc::new(umu_net(dir.path(), "#!/bin/sh\n"));
        let barrier = Arc::new(Barrier::new(2));
        let downloads = Arc::new(AtomicUsize::new(0));
        let results: Vec<bool> = std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..2 {
                let (paths, settings, net, barrier, downloads) =
                    (paths.clone(), settings.clone(), net.clone(), barrier.clone(), downloads.clone());
                let home = dir.path().to_string_lossy().into_owned();
                handles.push(s.spawn(move || {
                    let ctx = PlanContext::load(&paths);
                    let probe = DiskProbeNoUmu(home);
                    barrier.wait();
                    ensure_umu(&ctx, &probe, net.as_ref(), &settings, &Cancel::new(), true, || {
                        downloads.fetch_add(1, Ordering::SeqCst);
                    })
                    .unwrap()
                }));
            }
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        assert_eq!(results.iter().filter(|&&d| d).count(), 1);
        assert!(runtimes::umu_script(&paths).is_file());
    }
}
