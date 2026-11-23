//! An Epic game started the Windows way (`ProcessPlan::ask_legendary`), with
//! the real spawner: legendary (a fake tool) only names the game's program
//! and exits; the session is the game's (a fake tool that runs a while), so
//! it ends when the game ends, not when legendary does.

use std::fs;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::testing::{FakeTool, Reply};
use heronite_data::DataPaths;
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{Command, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, ProcessPlan, SystemSpawner};
use serde_json::json;

struct Rig {
    dir: tempfile::TempDir,
    paths: DataPaths,
    launcher: Launcher,
    rx: Receiver<LaunchEvent>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heronite"));
    fs::create_dir_all(paths.games_config_dir()).unwrap();
    let steam = SteamLocator {
        home: dir.path().join("home"),
        steam_path: dir.path().join("home/.steam/steam"),
        system_roots: vec![],
        platform_roots: vec![],
    };
    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let cfg = LauncherConfig {
        poll_interval: Duration::from_millis(20),
        kill_grace: Duration::from_millis(400),
        min_session_seconds: 0,
        steam: SteamWaitConfig::default(),
        record: true,
        handoff: Default::default(),
    };
    let launcher = Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, cfg, tx);
    Rig { dir, paths, launcher, rx }
}

/// legendary answering `launch --json` with `answer`.
fn legendary(dir: &Path, answer: Reply) -> FakeTool {
    FakeTool::new(&dir.join("lg"), "legendary").record_env("LEGENDARY_CONFIG_PATH").otherwise(answer)
}

fn plan(paths: &DataPaths, lg: &FakeTool) -> LaunchPlan {
    let mut env = std::collections::BTreeMap::new();
    env.insert("LEGENDARY_CONFIG_PATH".to_string(), paths.legendary_config_dir().to_string_lossy().into_owned());
    LaunchPlan::Legendary {
        app_name: "Calluna".into(),
        process: ProcessPlan {
            command: Command {
                program: lg.bin().to_string_lossy().into_owned(),
                args: ["launch", "Calluna", "--json"].map(String::from).to_vec(),
                env,
                cwd: None,
            },
            ask_legendary: true,
            ..Default::default()
        },
    }
}

/// Launches and waits for the end: the `Exited` (elapsed) or `Error` event.
fn run(r: &Rig, plan: LaunchPlan) -> Result<Duration, String> {
    let req = LaunchRequest { app_name: "Calluna".into(), runner: "legendary".into(), title: "Control".into(), plan };
    r.launcher.launch(req).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match r.rx.recv_timeout(Duration::from_secs(60)).expect("event") {
            LaunchEvent::Exited { elapsed, .. } => return Ok(elapsed),
            LaunchEvent::Error { message, .. } => return Err(message),
            _ => {}
        }
    }
    panic!("the session did not end")
}

fn game_log(paths: &DataPaths) -> String {
    let dir = paths.root.join("logs").join("games");
    let mut text = String::new();
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        if let Ok(t) = fs::read_to_string(e.path().join("launch.log")) {
            text.push_str(&t);
        }
    }
    text
}

#[test]
fn the_session_follows_the_game_legendary_names_not_legendary() {
    let r = rig();
    let game_dir = r.dir.path().join("Control");
    let game = FakeTool::new(&game_dir, "Control").otherwise(Reply::new().out("the game runs").sleep(Duration::from_millis(1500)));
    let answer = json!({
        "game_executable": game.bin().file_name().unwrap().to_string_lossy(),
        "game_directory": game_dir.to_string_lossy(),
        "game_parameters": ["-nolauncher"],
        "egl_parameters": ["-AUTH_LOGIN=unused", "-AUTH_PASSWORD=s3cretcode", "-AUTH_TYPE=exchangecode"],
        "launch_command": [], "working_directory": "", "user_parameters": [], "environment": {}
    });
    let lg = legendary(r.dir.path(), Reply::new().err("[cli] INFO: Logging in...").out(&answer.to_string()));
    let elapsed = run(&r, plan(&r.paths, &lg)).unwrap();
    assert!(elapsed >= Duration::from_millis(1400), "the session ended with legendary: {elapsed:?}");
    assert_eq!(lg.calls(), ["launch Calluna --json"]);
    assert_eq!(lg.env_seen("LEGENDARY_CONFIG_PATH"), [r.paths.legendary_config_dir().to_string_lossy()]);
    assert_eq!(game.calls(), ["-nolauncher -AUTH_LOGIN=unused -AUTH_PASSWORD=s3cretcode -AUTH_TYPE=exchangecode"]);
    let log = game_log(&r.paths);
    assert!(log.contains("-AUTH_PASSWORD=<hidden>"), "{log}");
    assert!(!log.contains("s3cretcode"), "the sign-in reached the log:\n{log}");
}

#[test]
fn legendarys_refusal_ends_the_launch_with_its_reason() {
    let r = rig();
    let lg = legendary(r.dir.path(), Reply::new().err("[cli] ERROR: Login failed, cannot continue!").exit(1));
    let why = run(&r, plan(&r.paths, &lg)).unwrap_err();
    assert!(why.contains("Login failed, cannot continue!"), "{why}");
}

#[test]
fn a_store_app_that_is_not_installed_is_named() {
    let r = rig();
    let lg = legendary(r.dir.path(), Reply::new().out(r#"{"uri": "heronite-test-no-such-app://launchgame/x?AUTH_PASSWORD=1"}"#));
    let why = run(&r, plan(&r.paths, &lg)).unwrap_err();
    assert!(why.contains("EA app") && why.contains("not installed"), "{why}");
}

#[test]
fn legendary_wrapper_exe_runs_as_the_game_process_and_keeps_gpu_pref_on_the_game() {
    let r = rig();
    let reg = Arc::new(heronite_launch::gpu::preference::FakeRegistry::default());
    r.launcher.set_prep_services(heronite_launch::runtime::PrepServices { gpu_registry: reg.clone(), ..Default::default() });
    let wrapper = FakeTool::new(&r.dir.path().join("wrap"), "EpicGamesLauncher")
        .record_env(heronite_launch::fake_epic::WRAPPER_ENV_KEY)
        .otherwise(Reply::new().out("wrapper ran").sleep(Duration::from_millis(1500)));
    let answer = json!({
        "game_executable": "PlayGTAV.exe",
        "game_directory": r"C:\Games\GTAV",
        "game_parameters": ["-nolauncher"],
        "egl_parameters": ["-AUTH_LOGIN=unused", "-AUTH_PASSWORD=s3cretcode", "-AUTH_TYPE=exchangecode"],
        "launch_command": [], "working_directory": r.dir.path().to_string_lossy(), "user_parameters": [], "environment": {}
    });
    let lg = legendary(r.dir.path(), Reply::new().out(&answer.to_string()));
    let mut p = plan(&r.paths, &lg);
    if let LaunchPlan::Legendary { process, .. } = &mut p {
        process.command.env.insert(heronite_launch::fake_epic::WRAPPER_ENV_KEY.into(), wrapper.bin().to_string_lossy().into_owned());
        process.gpu_preference = Some(heronite_launch::gpu::preference::Preference::HighPerformance);
    }
    let req = LaunchRequest { app_name: "Calluna".into(), runner: "legendary".into(), title: "Control".into(), plan: p };
    r.launcher.launch(req).unwrap();
    loop {
        match r.rx.recv_timeout(Duration::from_secs(60)).expect("event") {
            LaunchEvent::Running { .. } => break,
            LaunchEvent::Error { message, .. } => panic!("unexpected error: {message}"),
            _ => {}
        }
    }
    // While running, GPU preference is set on the real game exe (not the wrapper).
    assert_eq!(reg.get(r"C:\Games\GTAV\PlayGTAV.exe").as_deref(), Some("GpuPreference=2;"));
    assert_eq!(reg.get(&wrapper.bin().to_string_lossy()), None);
    loop {
        match r.rx.recv_timeout(Duration::from_secs(60)).expect("event") {
            LaunchEvent::Exited { .. } => break,
            LaunchEvent::Error { message, .. } => panic!("unexpected error: {message}"),
            _ => {}
        }
    }
    // Restored after exit.
    assert_eq!(reg.get(r"C:\Games\GTAV\PlayGTAV.exe"), None);
    let calls = wrapper.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("PlayGTAV.exe") && calls[0].contains("-AUTH_PASSWORD=s3cretcode"), "{calls:?}");
    assert_eq!(wrapper.env_seen(heronite_launch::fake_epic::WRAPPER_ENV_KEY), [""]);
    let log = game_log(&r.paths);
    assert!(log.contains("-AUTH_PASSWORD=<hidden>") && !log.contains("s3cretcode"), "{log}");
}
