//! Launcher runtime tests. Nothing here starts a game: plans point at a fake
//! spawner, or at harmless `sleep` / `sh` processes.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::SessionsFile;
use heronite_data::{json_store, HeroicPaths};
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::procs::{FakeProcs, Procs, SystemProcs};
use heronite_launch::runtime::FakeSpawner;
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    Command, LaunchError, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig,
    ProcessPlan, StopOptions, StopSpec, SystemSpawner,
};

fn fast_cfg() -> LauncherConfig {
    LauncherConfig {
        poll_interval: Duration::from_millis(10),
        kill_grace: Duration::from_millis(400),
        min_session_seconds: 0,
        steam: SteamWaitConfig {
            start_timeout: Duration::from_millis(600),
            poll_interval: Duration::from_millis(10),
            resolve_interval: Duration::from_millis(20),
        },
        record: true,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    paths: HeroicPaths,
    launcher: Launcher,
    rx: Receiver<LaunchEvent>,
    spawner: Arc<FakeSpawner>,
    procs: Arc<FakeProcs>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = HeroicPaths::at(dir.path().join("heroic"));
    let home = dir.path().join("home");
    let steam = SteamLocator {
        home: home.clone(),
        steam_path: home.join(".steam/steam"),
        system_roots: vec![],
    };
    let (tx, rx) = channel();
    let spawner = Arc::new(FakeSpawner::new());
    let procs = Arc::new(FakeProcs::new());
    let launcher = Launcher::new(
        paths.clone(),
        steam,
        spawner.clone(),
        procs.clone(),
        fast_cfg(),
        tx,
    );
    Rig {
        _dir: dir,
        paths,
        launcher,
        rx,
        spawner,
        procs,
    }
}

fn process_plan(program: &str, args: &[&str]) -> ProcessPlan {
    ProcessPlan {
        command: Command {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: BTreeMap::new(),
            cwd: None,
        },
        ..Default::default()
    }
}

fn request(app: &str, plan: LaunchPlan) -> LaunchRequest {
    LaunchRequest {
        app_name: app.into(),
        runner: "sideload".into(),
        title: format!("{app} title"),
        plan,
    }
}

fn next(rx: &Receiver<LaunchEvent>) -> LaunchEvent {
    rx.recv_timeout(Duration::from_secs(5)).expect("event")
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn full_lifecycle_records_everything_like_heronite() {
    let r = rig();
    let plan = LaunchPlan::Native(process_plan("/games/run.sh", &["--fast"]));
    r.launcher.launch(request("g1", plan.clone())).unwrap();
    assert_eq!(
        next(&r.rx),
        LaunchEvent::Launching {
            app_name: "g1".into()
        }
    );
    let LaunchEvent::Running { pid, .. } = next(&r.rx) else {
        panic!()
    };
    assert_eq!(r.launcher.pid_of("g1"), Some(pid));
    assert!(r.launcher.is_running("g1"));
    assert_eq!(r.spawner.spawned()[0].program, "/games/run.sh");

    // One game per app name.
    assert_eq!(
        r.launcher.launch(request("g1", plan)).unwrap_err(),
        LaunchError::AlreadyRunning
    );

    // firstPlayed is written as soon as the process exists.
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert!(ts["g1"]["firstPlayed"].as_str().unwrap().ends_with('Z'));

    std::thread::sleep(Duration::from_millis(120));
    r.spawner.finish(0, 0);
    let LaunchEvent::Exited {
        app_name,
        elapsed,
        exit,
        record,
    } = next(&r.rx)
    else {
        panic!()
    };
    assert_eq!(app_name, "g1");
    assert!(elapsed >= Duration::from_millis(100));
    assert_eq!(exit.unwrap().code, Some(0));
    let record = record.unwrap();
    assert!(record.session_recorded);
    assert!(!r.launcher.is_running("g1"));

    let sessions: SessionsFile = json_store::read(&r.paths.sessions()).unwrap();
    let s = &sessions.sessions["g1"][0];
    assert_eq!(s.source, "heroic");
    assert!(
        s.started_at.ends_with('Z') && s.started_at.len() == 24,
        "{}",
        s.started_at
    );
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert_eq!(ts["g1"]["totalPlayed"], 0);
    assert!(ts["g1"]["lastPlayed"].is_string());
    let cfg: serde_json::Value = json_store::read_value(&r.paths.config()).unwrap();
    assert_eq!(
        cfg["games"]["recent"][0],
        serde_json::json!({"appName": "g1", "title": "g1 title"})
    );

    // The slot is free again.
    r.launcher
        .launch(request(
            "g1",
            LaunchPlan::Native(process_plan("/games/run.sh", &[])),
        ))
        .unwrap();
}

#[test]
fn refuses_unavailable_plans_and_reports_spawn_errors() {
    let r = rig();
    let err = r
        .launcher
        .launch(request("g", LaunchPlan::Unavailable("nope".into())))
        .unwrap_err();
    assert_eq!(err, LaunchError::Unavailable("nope".into()));
    assert!(!r.launcher.is_running("g"));

    r.spawner.fail_next_spawn("boom");
    r.launcher
        .launch(request("g", LaunchPlan::Native(process_plan("/x", &[]))))
        .unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let LaunchEvent::Error { message, .. } = next(&r.rx) else {
        panic!()
    };
    assert!(message.contains("boom"));
    assert!(!r.launcher.is_running("g"));
    assert!(
        !r.paths.timestamps().exists(),
        "a game that never started leaves no trace"
    );
}

#[test]
fn stop_terminates_the_tree_and_runs_stop_commands() {
    let r = rig();
    let mut p = process_plan("/emu/pcsx2", &[]);
    p.stop = StopSpec {
        kill_patterns: vec!["pcsx2".into(), "ab".into()],
        commands: vec![Command {
            program: "/w/wineserver".into(),
            args: vec!["-k".into()],
            ..Default::default()
        }],
    };
    r.launcher
        .launch(request("g", LaunchPlan::Native(p)))
        .unwrap();
    next(&r.rx);
    next(&r.rx);
    assert!(r.launcher.stop("g"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.spawner.handle(0).unwrap().lock().unwrap().terminated);
    wait_until("stop commands", || r.spawner.spawned().len() == 2);
    assert_eq!(r.spawner.spawned()[1].program, "/w/wineserver");
    assert_eq!(
        r.spawner.patterns(),
        ["pcsx2"],
        "patterns shorter than 3 chars are never pkill'ed"
    );
    assert!(!r.launcher.stop("g"), "nothing left to stop");
}

#[test]
fn force_clear_forgets_the_game_without_killing_it() {
    let r = rig();
    r.launcher
        .launch(request("g", LaunchPlan::Native(process_plan("/x", &[]))))
        .unwrap();
    next(&r.rx);
    next(&r.rx);
    assert!(r.launcher.force_clear("g"));
    let LaunchEvent::Exited { exit, record, .. } = next(&r.rx) else {
        panic!()
    };
    assert_eq!(exit, None);
    assert!(
        record.is_some(),
        "Heroic still records the time played so far"
    );
    let h = r.spawner.handle(0).unwrap();
    let h = h.lock().unwrap();
    assert!(!h.terminated && !h.killed);
    assert!(!r.launcher.is_running("g"));

    // And without recording.
    drop(h);
    let r2 = rig();
    r2.launcher
        .launch(request("g", LaunchPlan::Native(process_plan("/x", &[]))))
        .unwrap();
    next(&r2.rx);
    next(&r2.rx);
    r2.launcher.force_clear_with("g", false);
    let LaunchEvent::Exited { record, .. } = next(&r2.rx) else {
        panic!()
    };
    assert!(record.is_none());
    assert!(!r2.paths.sessions().exists());
}

fn write_steam_log(r: &Rig, text: &str) -> std::path::PathBuf {
    let log = r._dir.path().join("home/.steam/steam/logs/console_log.txt");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, text).unwrap();
    log
}

#[test]
fn steam_games_are_found_through_the_log_and_followed_until_exit() {
    let r = rig();
    let log = write_steam_log(&r, "");
    let plan = LaunchPlan::SteamUri {
        app_id: "400".into(),
        uri: "steam://rungameid/400".into(),
    };
    r.launcher.launch(request("steam_400", plan)).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    wait_until("uri opened", || {
        r.spawner.uris() == ["steam://rungameid/400"]
    });
    assert!(
        r.spawner.spawned().is_empty(),
        "Steam games are not child processes"
    );

    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"Game process added : AppID 400 \"x\" ProcID 7777\n")
        .unwrap();
    let LaunchEvent::Running { pid, .. } = next(&r.rx) else {
        panic!()
    };
    assert_eq!(pid, 7777);
    assert!(r.launcher.is_running("steam_400"));
    std::thread::sleep(Duration::from_millis(100));
    r.procs.exit(7777);
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else {
        panic!()
    };
    assert!(record.unwrap().session_recorded);
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert!(ts["steam_400"]["firstPlayed"].is_string());
}

#[test]
fn steam_stop_only_ends_tracking_unless_asked_to_kill() {
    let r = rig();
    let log = write_steam_log(&r, "");
    let plan = LaunchPlan::SteamUri {
        app_id: "400".into(),
        uri: "steam://rungameid/400".into(),
    };
    r.launcher.launch(request("steam_400", plan)).unwrap();
    next(&r.rx);
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"Game process added : AppID 400 x ProcID 7777\n")
        .unwrap();
    next(&r.rx);
    assert!(r.launcher.stop("steam_400"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.procs.signals().is_empty(), "the game keeps running");

    let r = rig();
    let log = write_steam_log(&r, "");
    r.launcher
        .launch(request(
            "steam_400",
            LaunchPlan::SteamUri {
                app_id: "400".into(),
                uri: "u".into(),
            },
        ))
        .unwrap();
    next(&r.rx);
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"Game process added : AppID 400 x ProcID 7777\n")
        .unwrap();
    next(&r.rx);
    r.launcher.stop_with(
        "steam_400",
        StopOptions {
            kill_steam_processes: true,
        },
    );
    next(&r.rx);
    assert!(r.procs.signals().iter().any(|(pid, _)| *pid == 7777));
}

#[test]
fn steam_that_never_starts_the_game_is_an_error_without_recording() {
    let r = rig();
    write_steam_log(&r, "");
    r.launcher
        .launch(request(
            "steam_400",
            LaunchPlan::SteamUri {
                app_id: "400".into(),
                uri: "u".into(),
            },
        ))
        .unwrap();
    next(&r.rx);
    let LaunchEvent::Error { message, .. } = next(&r.rx) else {
        panic!()
    };
    assert!(message.contains("did not start"));
    assert!(!r.paths.sessions().exists());
    assert!(!r.launcher.is_running("steam_400"));

    r.spawner.fail_next_uri("xdg-open missing");
    r.launcher
        .launch(request(
            "steam_400",
            LaunchPlan::SteamUri {
                app_id: "400".into(),
                uri: "u".into(),
            },
        ))
        .unwrap();
    next(&r.rx);
    let LaunchEvent::Error { message, .. } = next(&r.rx) else {
        panic!()
    };
    assert!(message.contains("xdg-open missing"));
}

// ---------------------------------------------------------------------------
// Real (harmless) processes
// ---------------------------------------------------------------------------

fn real_rig() -> (
    tempfile::TempDir,
    HeroicPaths,
    Launcher,
    Receiver<LaunchEvent>,
) {
    let dir = tempfile::tempdir().unwrap();
    let paths = HeroicPaths::at(dir.path().join("heroic"));
    let steam = SteamLocator {
        home: dir.path().join("home"),
        steam_path: dir.path().join("home/.steam/steam"),
        system_roots: vec![],
    };
    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let launcher = Launcher::new(
        paths.clone(),
        steam,
        Arc::new(SystemSpawner::new(procs.clone())),
        procs,
        fast_cfg(),
        tx,
    );
    (dir, paths, launcher, rx)
}

#[test]
fn real_process_exit_code_log_and_playtime() {
    let (_d, paths, launcher, rx) = real_rig();
    let plan = LaunchPlan::Native(process_plan(
        "sh",
        &["-c", "echo hello from the game; echo oops >&2; exit 3"],
    ));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    next(&rx);
    let LaunchEvent::Exited { exit, record, .. } = next(&rx) else {
        panic!()
    };
    assert_eq!(exit.unwrap().code, Some(3));
    assert!(record.is_some());
    let log = fs::read_to_string(paths.games_config_dir().join("g-lastPlay.log")).unwrap();
    assert!(
        log.contains("hello from the game") && log.contains("oops"),
        "{log}"
    );
    assert!(log.starts_with("Launching: sh -c"));
}

#[test]
fn real_process_is_not_done_while_a_child_holds_its_output() {
    // Heroic waits for the output pipe to close, so a launcher script that
    // backgrounds the real game keeps the session open.
    let (_d, _paths, launcher, rx) = real_rig();
    let plan = LaunchPlan::Native(process_plan("sh", &["-c", "sleep 1 & echo started"]));
    let t = Instant::now();
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    next(&rx);
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn real_stop_kills_the_whole_tree() {
    let (_d, _paths, launcher, rx) = real_rig();
    // sh -> sleep (grandchild) in the same group; `wait` keeps sh alive.
    let plan = LaunchPlan::Native(process_plan("sh", &["-c", "sleep 60 & wait"]));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else {
        panic!()
    };
    let procs = SystemProcs;
    wait_until("grandchild", || !procs.children(pid).is_empty());
    let grandchild = procs.children(pid)[0];
    assert!(procs.is_alive(pid) && procs.is_alive(grandchild));

    let t = Instant::now();
    assert!(launcher.stop("g"));
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    wait_until("both dead", || {
        !procs.is_alive(pid) && !procs.is_alive(grandchild)
    });
}

#[test]
fn real_sigterm_ignoring_process_gets_sigkill_after_the_grace_period() {
    let (_d, _paths, launcher, rx) = real_rig();
    let plan = LaunchPlan::Native(process_plan(
        "sh",
        &["-c", "trap '' TERM; while true; do sleep 0.05; done"],
    ));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else {
        panic!()
    };
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    launcher.stop("g");
    let LaunchEvent::Exited { exit, .. } = next(&rx) else {
        panic!()
    };
    assert!(
        t.elapsed() >= Duration::from_millis(350),
        "grace period respected: {:?}",
        t.elapsed()
    );
    assert_eq!(exit.map(|e| e.signal), Some(Some(9)));
    wait_until("dead", || !SystemProcs.is_alive(pid));
}

#[test]
fn real_force_clear_leaves_the_process_running() {
    let (_d, _paths, launcher, rx) = real_rig();
    launcher
        .launch(request(
            "g",
            LaunchPlan::Native(process_plan("sleep", &["30"])),
        ))
        .unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else {
        panic!()
    };
    launcher.force_clear("g");
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert!(SystemProcs.is_alive(pid), "force clear must not kill");
    // Clean up our own test process.
    SystemProcs.signal(pid, heronite_launch::procs::Sig::Kill);
    wait_until("cleanup", || !SystemProcs.is_alive(pid));
}

#[test]
fn real_spawn_failure_is_reported() {
    let (_d, _paths, launcher, rx) = real_rig();
    launcher
        .launch(request(
            "g",
            LaunchPlan::Native(process_plan("/definitely/not/here", &[])),
        ))
        .unwrap();
    next(&rx);
    let LaunchEvent::Error { message, .. } = next(&rx) else {
        panic!()
    };
    assert!(message.contains("could not start"), "{message}");
}
