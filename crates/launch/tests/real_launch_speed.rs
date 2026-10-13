//! How long a launch takes until the game's process exists, through the app
//! and through a bare `umu-run` (what lighter launchers do), in the same
//! prefix with the same Proton. Wine's Notepad plays the game. Ignored by
//! default (opens windows, needs a Proton build, umu and the network):
//!   HERONITE_REAL_PROTON=~/.local/share/heronite/tools/proton/GE-Proton-latest \
//!   cargo test -p heronite-launch --test real_launch_speed -- --ignored --nocapture

use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::GameInfo;
use heronite_data::DataPaths;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::GameSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    build_plan_with_settings, LaunchEvent, LaunchOptions, LaunchRequest, Launcher, LauncherConfig, PlanContext, SystemSpawner,
};
use serde_json::json;

fn home() -> String {
    std::env::var("HOME").expect("HOME")
}

fn notepad_pid(prefix: &str) -> Option<u32> {
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let argv0 = String::from_utf8_lossy(cmd.split(|b| *b == 0).next()?).to_lowercase();
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let ours = env.split(|b| *b == 0).any(|kv| kv.starts_with(format!("WINEPREFIX={prefix}").as_bytes()));
        (argv0.ends_with("\\notepad.exe") && ours).then_some(pid)
    })
}

fn wait_for(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Closes Notepad and waits until nothing of the prefix runs.
fn close(prefix: &str) {
    if let Some(pid) = notepad_pid(prefix) {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    assert!(wait_for(60, || !SystemProcs.windows_programs(prefix).iter().any(|n| n == "notepad.exe")));
    // Let wineserver go away like it does between two plays.
    std::thread::sleep(Duration::from_secs(4));
}

fn next_exit(rx: &Receiver<LaunchEvent>) {
    loop {
        match rx.recv_timeout(Duration::from_secs(60)).expect("an event") {
            LaunchEvent::Exited { .. } => return,
            LaunchEvent::Error { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
}

#[test]
#[ignore]
fn launch_speed_against_bare_umu() {
    let proton = std::env::var("HERONITE_REAL_PROTON").expect("set HERONITE_REAL_PROTON to a Proton folder");
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("data"));
    let runtimes = paths.tools_dir().join("runtimes");
    std::fs::create_dir_all(&runtimes).unwrap();
    for name in ["umu", "eac_runtime", "battleye_runtime"] {
        let real = format!("{}/.local/share/heronite/tools/runtimes/{name}", home());
        if std::path::Path::new(&real).exists() {
            std::os::unix::fs::symlink(&real, runtimes.join(name)).unwrap();
        }
    }
    let prefix = dir.path().join("pfx").to_string_lossy().into_owned();
    // The defaults of a new setup that matter for a launch.
    let settings = json!({
        "wineVersion": {"bin": format!("{proton}/proton"), "name": "Proton", "type": "proton"},
        "winePrefix": prefix, "autoInstallDxvk": true, "autoInstallVkd3d": true, "autoInstallDxvkNvapi": true,
        "eacRuntime": true, "battlEyeRuntime": true, "enableEsync": true, "enableFsync": true, "verboseLogs": true
    });
    let settings = GameSettings::from_map(settings.as_object().unwrap(), &home());
    let game: GameInfo = serde_json::from_value(json!({
        "runner": "sideload", "app_name": "notepad", "title": "Notepad", "is_installed": true,
        "install": {"executable": "notepad", "platform": "Windows"}
    }))
    .unwrap();
    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let steam = SteamLocator { home: dir.path().into(), steam_path: dir.path().join("steam"), system_roots: vec![] };
    let launcher = Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, LauncherConfig::default(), tx);
    let log = paths.root.join("logs/games/notepad_sideload/launch.log");

    let mut app = Vec::new();
    for run in 0..4 {
        let t = Instant::now();
        let ctx = PlanContext::load(&paths);
        let plan = build_plan_with_settings(&ctx, &heronite_launch::SystemProbe, &game, None, &settings, &LaunchOptions::default());
        launcher.launch(LaunchRequest { app_name: "notepad".into(), runner: "sideload".into(), title: "Notepad".into(), plan }).unwrap();
        assert!(wait_for(240, || notepad_pid(&prefix).is_some()), "the game did not start");
        let took = t.elapsed();
        let prepared =
            std::fs::read_to_string(&log).unwrap_or_default().lines().find(|l| l.contains("Prepared in")).unwrap_or("").to_string();
        eprintln!("app run {run}: game up after {:.2} s  [{}]", took.as_secs_f64(), prepared.trim());
        if run > 0 {
            app.push(took);
        }
        close(&prefix);
        next_exit(&rx);
    }

    let umu = format!("{}/.local/share/heronite/tools/runtimes/umu/umu-run", home());
    let mut bare = Vec::new();
    for run in 0..3 {
        let t = Instant::now();
        let mut child = std::process::Command::new("python3")
            .arg(&umu)
            .arg("notepad")
            .env("WINEPREFIX", &prefix)
            .env("GAMEID", "umu-0")
            .env("PROTONPATH", &proton)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_for(240, || notepad_pid(&prefix).is_some()));
        let took = t.elapsed();
        eprintln!("bare umu-run {run}: game up after {:.2} s", took.as_secs_f64());
        bare.push(took);
        close(&prefix);
        let _ = child.wait();
    }
    let avg = |v: &[Duration]| v.iter().map(Duration::as_secs_f64).sum::<f64>() / v.len() as f64;
    eprintln!("average: app {:.2} s, bare umu-run {:.2} s", avg(&app), avg(&bare));
}
