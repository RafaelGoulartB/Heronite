//! A program alongside a real game through real Proton: Wine's Notepad plays
//! the game and Wine's Task Manager the trainer. Ignored by default (opens
//! windows, needs a Proton build and umu). Run with a Proton folder:
//!   HERONITE_REAL_PROTON=~/.local/share/heronite/tools/proton/GE-Proton-latest \
//!   cargo test -p heronite-launch --test real_companions -- --ignored --nocapture --test-threads=1

// Real Proton: Linux only.
#![cfg(unix)]

use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::GameInfo;
use heronite_data::DataPaths;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::GameSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    build_plan_with_settings, LaunchEvent, LaunchOptions, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, PlanContext, SystemSpawner,
};
use serde_json::json;

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").expect("HOME"))
}

fn proton() -> Option<String> {
    std::env::var("HERONITE_REAL_PROTON").ok().filter(|p| std::path::Path::new(p).join("proton").exists())
}

/// Pid of the Windows program `name` running in `prefix`.
fn windows_pid(prefix: &str, name: &str) -> Option<u32> {
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let argv0 = String::from_utf8_lossy(cmd.split(|b| *b == 0).next()?).to_lowercase();
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let in_prefix = env.split(|b| *b == 0).any(|kv| kv.starts_with(format!("WINEPREFIX={prefix}").as_bytes()));
        (argv0.ends_with(&format!("\\{name}")) && in_prefix).then_some(pid)
    })
}

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("timed out waiting for {what}");
}

fn run(proton: &str, umu: bool) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("data"));
    // The umu this app downloads, without downloading it again.
    let umu_dir = paths.tools_dir().join("runtimes");
    std::fs::create_dir_all(&umu_dir).unwrap();
    let real_umu = home().join(".local/share/heronite/tools/runtimes/umu");
    std::os::unix::fs::symlink(&real_umu, umu_dir.join("umu")).unwrap();

    let prefix = dir.path().join("pfx").to_string_lossy().into_owned();
    let trainer = format!("{proton}/files/lib/wine/x86_64-windows/taskmgr.exe");
    let settings = json!({
        "wineVersion": {"bin": format!("{proton}/proton"), "name": "Proton", "type": "proton"},
        "winePrefix": prefix, "disableUMU": !umu,
        "heroniteCompanions": [{"path": trainer, "delay": 3}]
    });
    let settings = GameSettings::from_map(settings.as_object().unwrap(), &home().to_string_lossy());
    let game: GameInfo = serde_json::from_value(json!({
        "runner": "sideload", "app_name": "notepad", "title": "Notepad", "is_installed": true,
        "install": {"executable": "notepad", "platform": "Windows"}
    }))
    .unwrap();
    let ctx = PlanContext::load(&paths);
    let probe = heronite_launch::SystemProbe;
    let plan = build_plan_with_settings(&ctx, &probe, &game, None, &settings, &LaunchOptions::default());
    assert!(matches!(plan, LaunchPlan::Wine { .. }), "{plan:?}");
    assert_eq!(plan.process().unwrap().companions.len(), 1, "{:?}", plan.process().unwrap().notes);

    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let steam = SteamLocator { home: dir.path().into(), steam_path: dir.path().join("steam"), system_roots: vec![] };
    let launcher = Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, LauncherConfig::default(), tx);
    let started = Instant::now();
    launcher.launch(LaunchRequest { app_name: "notepad".into(), runner: "sideload".into(), title: "Notepad".into(), plan }).unwrap();

    wait_for("the game", 240, || windows_pid(&prefix, "notepad.exe").is_some());
    eprintln!("[umu={umu}] game up after {:?}", started.elapsed());
    wait_for("the trainer", 60, || windows_pid(&prefix, "taskmgr.exe").is_some());
    eprintln!("[umu={umu}] trainer up after {:?}", started.elapsed());
    std::thread::sleep(Duration::from_secs(2));
    assert!(launcher.is_running("notepad"), "the game runs");

    // The game exits: the session ends and takes the trainer with it.
    let game_pid = windows_pid(&prefix, "notepad.exe").unwrap();
    unsafe { libc::kill(game_pid as i32, libc::SIGTERM) };
    let closed = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_secs(60)).expect("an event") {
            LaunchEvent::Exited { .. } => break,
            LaunchEvent::Error { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
    eprintln!("[umu={umu}] session ended {:?} after the game", closed.elapsed());
    wait_for("the trainer to close", 20, || windows_pid(&prefix, "taskmgr.exe").is_none());
    let log = std::fs::read_to_string(paths.root.join("logs/games/notepad_sideload/launch.log")).unwrap_or_default();
    assert!(log.contains("alongside the game"), "{log}");
}

#[test]
#[ignore]
fn a_trainer_runs_beside_the_game_and_closes_with_it() {
    let Some(proton) = proton() else { panic!("set HERONITE_REAL_PROTON to a Proton folder") };
    run(&proton, true);
    run(&proton, false);
}
