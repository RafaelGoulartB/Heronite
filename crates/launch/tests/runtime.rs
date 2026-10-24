//! Launcher runtime tests. Nothing here starts a game: plans point at a fake
//! spawner, or at harmless `sleep` / `sh` processes.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::SessionsFile;
use heronite_data::{json_store, DataPaths};
use heronite_launch::net::FakeNet;
use heronite_launch::plan::{CloudSaves, CompanionPlan, Handoff, PrepSpec};
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::prepare::CloudSync;
use heronite_launch::probe::SystemProbe;
use heronite_launch::procs::{FakeProcs, Procs, SystemProcs};
use heronite_launch::runtime::{FakeSpawner, PrepServices, ProcessHandle, Spawner};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    Command, HandoffConfig, LaunchError, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, ProcessPlan, StopOptions,
    StopSpec, SystemSpawner,
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
        handoff: HandoffConfig { soon: Duration::from_millis(500), wait: Duration::from_millis(400), gone: Duration::from_millis(60) },
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    launcher: Launcher,
    rx: Receiver<LaunchEvent>,
    spawner: Arc<FakeSpawner>,
    procs: Arc<FakeProcs>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    let home = dir.path().join("home");
    let steam = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    let spawner = Arc::new(FakeSpawner::new());
    let procs = Arc::new(FakeProcs::new());
    let launcher = Launcher::new(paths.clone(), steam, spawner.clone(), procs.clone(), fast_cfg(), tx);
    Rig { _dir: dir, paths, launcher, rx, spawner, procs }
}

fn process_plan(program: &str, args: &[&str]) -> ProcessPlan {
    ProcessPlan {
        command: Command { program: program.into(), args: args.iter().map(|s| s.to_string()).collect(), env: BTreeMap::new(), cwd: None },
        ..Default::default()
    }
}

fn request(app: &str, plan: LaunchPlan) -> LaunchRequest {
    LaunchRequest { app_name: app.into(), runner: "sideload".into(), title: format!("{app} title"), plan }
}

/// The next event, past the preparation steps (tested on their own).
fn next(rx: &Receiver<LaunchEvent>) -> LaunchEvent {
    loop {
        match rx.recv_timeout(Duration::from_secs(5)).expect("event") {
            LaunchEvent::Preparing { .. } => continue,
            ev => return ev,
        }
    }
}

/// The next event, whatever it is.
fn next_any(rx: &Receiver<LaunchEvent>) -> LaunchEvent {
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
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "g1".into() });
    let LaunchEvent::Running { pid, .. } = next(&r.rx) else { panic!() };
    assert_eq!(r.launcher.pid_of("g1"), Some(pid));
    assert!(r.launcher.is_running("g1"));
    assert_eq!(r.spawner.spawned()[0].program, "/games/run.sh");

    // One game per app name.
    assert_eq!(r.launcher.launch(request("g1", plan)).unwrap_err(), LaunchError::AlreadyRunning);

    // firstPlayed is written as soon as the process exists.
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert!(ts["g1"]["firstPlayed"].as_str().unwrap().ends_with('Z'));

    std::thread::sleep(Duration::from_millis(120));
    r.spawner.finish(0, 0);
    let LaunchEvent::Exited { app_name, elapsed, exit, stopped, record } = next(&r.rx) else { panic!() };
    assert_eq!(app_name, "g1");
    assert!(elapsed >= Duration::from_millis(100));
    assert_eq!(exit.unwrap().code, Some(0));
    assert!(!stopped);
    let record = record.unwrap();
    assert!(record.session_recorded);
    assert!(!r.launcher.is_running("g1"));

    let sessions: SessionsFile = json_store::read(&r.paths.sessions()).unwrap();
    let s = &sessions.sessions["g1"][0];
    assert_eq!(s.source, "heroic");
    assert!(s.started_at.ends_with('Z') && s.started_at.len() == 24, "{}", s.started_at);
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert_eq!(ts["g1"]["totalPlayed"], 0);
    assert!(ts["g1"]["lastPlayed"].is_string());
    let cfg: serde_json::Value = json_store::read_value(&r.paths.store_config()).unwrap();
    assert_eq!(cfg["games"]["recent"][0], serde_json::json!({"appName": "g1", "title": "g1 title"}));

    // The slot is free again.
    r.launcher.launch(request("g1", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
}

#[test]
fn a_game_another_launcher_hands_to_its_open_window_is_followed() {
    let r = rig();
    let mut plan = process_plan("lutris", &["lutris:rungameid/12"]);
    plan.handoff = Some(Handoff { prefix: Some("/pfx".into()), folder: Some("/games/celeste".into()) });
    r.launcher.launch(request("lutris_12", LaunchPlan::Native(plan.clone()))).unwrap();
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "lutris_12".into() });
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    // Lutris passes the request on and exits; the game shows up a bit later.
    r.spawner.finish(0, 0);
    std::thread::sleep(Duration::from_millis(100));
    assert!(r.launcher.is_running("lutris_12"));
    r.procs.start(900, "Celeste.exe", None);
    r.procs.set_game_pids(&[900]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(r.launcher.is_running("lutris_12"), "the game is followed past the wait for it");
    r.procs.exit(900);
    let LaunchEvent::Exited { elapsed, .. } = next(&r.rx) else { panic!() };
    assert!(elapsed >= Duration::from_millis(600));

    // A game that never shows up: the session ends after the wait.
    r.launcher.launch(request("lutris_12", LaunchPlan::Native(plan.clone()))).unwrap();
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "lutris_12".into() });
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    r.spawner.finish(1, 0);
    let LaunchEvent::Exited { elapsed, .. } = next(&r.rx) else { panic!() };
    assert!(elapsed >= Duration::from_millis(400), "{elapsed:?}");

    // The launcher failed: nothing is waited for (no made-up play time).
    r.launcher.launch(request("lutris_12", LaunchPlan::Native(plan))).unwrap();
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "lutris_12".into() });
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    r.spawner.finish(2, 1);
    let LaunchEvent::Exited { elapsed, .. } = next(&r.rx) else { panic!() };
    assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
}

#[test]
fn refuses_unavailable_plans_and_reports_spawn_errors() {
    let r = rig();
    let err = r.launcher.launch(request("g", LaunchPlan::Unavailable("nope".into()))).unwrap_err();
    assert_eq!(err, LaunchError::Unavailable("nope".into()));
    assert!(!r.launcher.is_running("g"));

    r.spawner.fail_next_spawn("boom");
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/x", &[])))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let LaunchEvent::Error { message, .. } = next(&r.rx) else { panic!() };
    assert!(message.contains("boom"));
    assert!(!r.launcher.is_running("g"));
    assert!(!r.paths.timestamps().exists(), "a game that never started leaves no trace");
}

#[test]
fn stop_terminates_the_tree_and_runs_stop_commands() {
    let r = rig();
    let mut p = process_plan("/emu/pcsx2", &[]);
    p.stop = StopSpec {
        kill_patterns: vec!["pcsx2".into(), "ab".into()],
        commands: vec![Command { program: "/w/wineserver".into(), args: vec!["-k".into()], ..Default::default() }],
    };
    r.launcher.launch(request("g", LaunchPlan::Native(p))).unwrap();
    next(&r.rx);
    next(&r.rx);
    assert!(r.launcher.stop("g"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.spawner.handle(0).unwrap().lock().unwrap().terminated);
    wait_until("stop commands", || r.spawner.spawned().len() == 2);
    assert_eq!(r.spawner.spawned()[1].program, "/w/wineserver");
    assert_eq!(r.spawner.patterns(), ["pcsx2"], "patterns shorter than 3 chars are never used");
    assert!(!r.launcher.stop("g"), "nothing left to stop");
}

#[test]
fn force_clear_forgets_the_game_without_killing_it() {
    let r = rig();
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/x", &[])))).unwrap();
    next(&r.rx);
    next(&r.rx);
    assert!(r.launcher.force_clear("g"));
    let LaunchEvent::Exited { exit, record, .. } = next(&r.rx) else { panic!() };
    assert_eq!(exit, None);
    assert!(record.is_some(), "Heroic still records the time played so far");
    let h = r.spawner.handle(0).unwrap();
    let h = h.lock().unwrap();
    assert!(!h.terminated && !h.killed);
    assert!(!r.launcher.is_running("g"));

    // And without recording.
    drop(h);
    let r2 = rig();
    r2.launcher.launch(request("g", LaunchPlan::Native(process_plan("/x", &[])))).unwrap();
    next(&r2.rx);
    next(&r2.rx);
    r2.launcher.force_clear_with("g", false);
    let LaunchEvent::Exited { record, .. } = next(&r2.rx) else { panic!() };
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
    let plan = LaunchPlan::SteamUri { app_id: "400".into(), uri: "steam://rungameid/400".into() };
    r.launcher.launch(request("steam_400", plan)).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    wait_until("uri opened", || r.spawner.uris() == ["steam://rungameid/400"]);
    assert!(r.spawner.spawned().is_empty(), "Steam games are not child processes");

    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 \"x\" ProcID 7777\n").unwrap();
    let LaunchEvent::Running { pid, .. } = next(&r.rx) else { panic!() };
    assert_eq!(pid, 7777);
    assert!(r.launcher.is_running("steam_400"));
    std::thread::sleep(Duration::from_millis(100));
    r.procs.exit(7777);
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else { panic!() };
    assert!(record.unwrap().session_recorded);
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert!(ts["steam_400"]["firstPlayed"].is_string());
}

#[test]
fn steam_stop_only_ends_tracking_unless_asked_to_kill() {
    let r = rig();
    let log = write_steam_log(&r, "");
    let plan = LaunchPlan::SteamUri { app_id: "400".into(), uri: "steam://rungameid/400".into() };
    r.launcher.launch(request("steam_400", plan)).unwrap();
    next(&r.rx);
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 7777\n").unwrap();
    next(&r.rx);
    assert!(r.launcher.stop("steam_400"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.procs.signals().is_empty(), "the game keeps running");

    let r = rig();
    let log = write_steam_log(&r, "");
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 7777\n").unwrap();
    next(&r.rx);
    r.launcher.stop_with("steam_400", StopOptions { kill_steam_processes: true });
    next(&r.rx);
    assert!(r.procs.signals().iter().any(|(pid, _)| *pid == 7777));
}

/// Plays `steam_400` to its end. `steam_open_before`: Steam's pid file names
/// a live process before the launch (else Steam "starts" with the game).
/// Returns the URIs opened.
fn play_steam_game(close_after: bool, steam_open_before: bool) -> Vec<String> {
    let r = rig();
    let log = write_steam_log(&r, "");
    json_store::write(&r.paths.global_config(), &serde_json::json!({"defaultSettings": {"heroniteCloseSteamAfterGame": close_after}}))
        .unwrap();
    let steam_running = || {
        fs::write(r._dir.path().join("home/.steam/steam.pid"), "5000\n").unwrap();
        r.procs.start(5000, "steam", None);
    };
    if steam_open_before {
        steam_running();
    }
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    wait_until("uri opened", || r.spawner.uris() == ["u"]);
    steam_running();
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 7777\n").unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    r.procs.exit(7777);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    // Steam is closed right after the session ends, on the same thread.
    std::thread::sleep(Duration::from_millis(100));
    r.spawner.uris()
}

#[test]
fn steam_opened_for_a_game_is_closed_after_it_only_when_asked() {
    assert_eq!(play_steam_game(true, false), ["u", "steam://exit"]);
    assert_eq!(play_steam_game(false, false), ["u"], "the setting is off");
    assert_eq!(play_steam_game(true, true), ["u"], "Steam was already open");
}

#[test]
fn steam_that_never_starts_the_game_is_an_error_without_recording() {
    let r = rig();
    write_steam_log(&r, "");
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    let LaunchEvent::Error { message, .. } = next(&r.rx) else { panic!() };
    assert!(message.contains("did not start"));
    assert!(!r.paths.sessions().exists());
    assert!(!r.launcher.is_running("steam_400"));

    r.spawner.fail_next_uri("xdg-open missing");
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    let LaunchEvent::Error { message, .. } = next(&r.rx) else { panic!() };
    assert!(message.contains("xdg-open missing"));
}

// ---------------------------------------------------------------------------
// Real (harmless) processes
// ---------------------------------------------------------------------------

fn real_rig() -> (tempfile::TempDir, DataPaths, Launcher, Receiver<LaunchEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    let steam = SteamLocator { home: dir.path().join("home"), steam_path: dir.path().join("home/.steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let launcher = Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, fast_cfg(), tx);
    (dir, paths, launcher, rx)
}

#[test]
fn real_process_exit_code_log_and_playtime() {
    let (_d, paths, launcher, rx) = real_rig();
    let plan = LaunchPlan::Native(process_plan("sh", &["-c", "echo hello from the game; echo oops >&2; exit 3"]));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    next(&rx);
    let LaunchEvent::Exited { exit, record, .. } = next(&rx) else { panic!() };
    assert_eq!(exit.unwrap().code, Some(3));
    assert!(record.is_none(), "dying at once with an error is a failed launch, not playtime");
    // Heroic's layout: <logs>/games/<app>_<runner>/launch.log.
    let log = fs::read_to_string(paths.root.join("logs/games/g_sideload/launch.log")).unwrap();
    assert!(log.contains("hello from the game") && log.contains("oops"), "{log}");
    assert!(log.contains("Launching \"g title\" (sideload)"), "{log}");
    assert!(log.contains("Game Output:") && log.contains("exited with code 3"), "{log}");
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
    assert!(t.elapsed() >= Duration::from_millis(900), "{:?}", t.elapsed());
}

/// A script (not executable yet, as a picked file often is) that appends its
/// name and `HEROIC_GAME_SCRIPT_TYPE` to `out`.
fn write_script(dir: &std::path::Path, name: &str, out: &std::path::Path) -> String {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\necho \"{name} $HEROIC_GAME_SCRIPT_TYPE\" >> '{}'\n", out.display())).unwrap();
    path.to_string_lossy().into_owned()
}

fn script(program: &str, kind: &str) -> Command {
    let env = BTreeMap::from([("HEROIC_GAME_SCRIPT_TYPE".to_string(), kind.to_string())]);
    Command { program: program.into(), env, ..Default::default() }
}

#[test]
fn real_scripts_run_before_and_after_the_game() {
    let (d, paths, launcher, rx) = real_rig();
    let out = d.path().join("order.txt");
    let mut plan = process_plan("sh", &["-c", &format!("echo game >> '{}'", out.display())]);
    plan.before_script = Some(script(&write_script(d.path(), "before.sh", &out), "before"));
    plan.after_script = Some(script(&write_script(d.path(), "after.sh", &out), "after"));
    launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    assert!(matches!(next(&rx), LaunchEvent::Launching { .. }));
    assert_eq!(status_of(next_any(&rx)), ("Running the script before launch...".to_string(), false));
    assert!(matches!(next(&rx), LaunchEvent::Running { .. }));
    let LaunchEvent::Exited { record, .. } = next(&rx) else { panic!() };
    assert!(record.is_some());
    assert_eq!(fs::read_to_string(&out).unwrap(), "before.sh before\ngame\nafter.sh after\n");
    let log = fs::read_to_string(paths.root.join("logs/games/g_sideload/launch.log")).unwrap();
    assert!(log.contains("before.sh"), "{log}");
}

#[test]
fn real_a_missing_script_is_reported_and_the_game_still_runs() {
    let (d, _paths, launcher, rx) = real_rig();
    let mut plan = process_plan("true", &[]);
    plan.before_script = Some(script(&d.path().join("nope.sh").to_string_lossy(), "before"));
    launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&rx);
    let _ = next_any(&rx);
    let (message, warning) = status_of(next_any(&rx));
    assert!(warning && message.contains("nope.sh"), "{message}");
    assert!(matches!(next(&rx), LaunchEvent::Running { .. }));
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
}

#[test]
fn real_the_after_script_runs_for_a_stopped_game() {
    let (d, _paths, launcher, rx) = real_rig();
    let out = d.path().join("order.txt");
    let mut plan = process_plan("sleep", &["60"]);
    plan.after_script = Some(script(&write_script(d.path(), "after.sh", &out), "after"));
    launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&rx);
    assert!(matches!(next(&rx), LaunchEvent::Running { .. }));
    assert!(launcher.stop("g"));
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert_eq!(fs::read_to_string(&out).unwrap(), "after.sh after\n");
}

#[test]
fn real_stop_kills_the_whole_tree() {
    let (_d, _paths, launcher, rx) = real_rig();
    // sh -> sleep (grandchild) in the same group; `wait` keeps sh alive.
    let plan = LaunchPlan::Native(process_plan("sh", &["-c", "sleep 60 & wait"]));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else { panic!() };
    let procs = SystemProcs;
    wait_until("grandchild", || !procs.children(pid).is_empty());
    let grandchild = procs.children(pid)[0];
    assert!(procs.is_alive(pid) && procs.is_alive(grandchild));

    let t = Instant::now();
    assert!(launcher.stop("g"));
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    wait_until("both dead", || !procs.is_alive(pid) && !procs.is_alive(grandchild));
}

#[test]
fn real_sigterm_ignoring_process_gets_sigkill_after_the_grace_period() {
    let (_d, _paths, launcher, rx) = real_rig();
    let plan = LaunchPlan::Native(process_plan("sh", &["-c", "trap '' TERM; while true; do sleep 0.05; done"]));
    launcher.launch(request("g", plan)).unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else { panic!() };
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    launcher.stop("g");
    let LaunchEvent::Exited { exit, .. } = next(&rx) else { panic!() };
    assert!(t.elapsed() >= Duration::from_millis(350), "grace period respected: {:?}", t.elapsed());
    assert_eq!(exit.map(|e| e.signal), Some(Some(9)));
    wait_until("dead", || !SystemProcs.is_alive(pid));
}

#[test]
fn real_force_clear_leaves_the_process_running() {
    let (_d, _paths, launcher, rx) = real_rig();
    launcher.launch(request("g", LaunchPlan::Native(process_plan("sleep", &["30"])))).unwrap();
    next(&rx);
    let LaunchEvent::Running { pid, .. } = next(&rx) else { panic!() };
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
    launcher.launch(request("g", LaunchPlan::Native(process_plan("/definitely/not/here", &[])))).unwrap();
    next(&rx);
    let LaunchEvent::Error { message, .. } = next(&rx) else { panic!() };
    assert!(message.contains("could not start"), "{message}");
}

// ---------------------------------------------------------------------------
// Launch preparation: cloud saves, umu, prefix creation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RecordingCloud {
    calls: std::sync::Mutex<Vec<(String, String)>>,
    upload_delay: Duration,
}

impl CloudSync for RecordingCloud {
    fn sync(&self, _runner: &str, app: &str, path: &str, flag: &str) -> Result<String, String> {
        self.calls.lock().unwrap().push((flag.to_string(), format!("{app}:{path}")));
        if flag == "--skip-download" {
            std::thread::sleep(self.upload_delay);
        }
        Ok(String::new())
    }
}

fn services(online: bool, cloud: Option<Arc<RecordingCloud>>) -> PrepServices {
    PrepServices {
        probe: Arc::new(SystemProbe),
        net: Arc::new(if online { FakeNet::online() } else { FakeNet::offline() }),
        cloud: cloud.map(|c| c as Arc<dyn CloudSync>),
    }
}

fn status_of(ev: LaunchEvent) -> (String, bool) {
    match ev {
        LaunchEvent::Status { message, warning, .. } => (message, warning),
        LaunchEvent::Preparing { message, .. } => (message, false),
        other => panic!("expected a status, got {other:?}"),
    }
}

#[test]
fn cloud_saves_are_downloaded_before_and_uploaded_after_the_game() {
    let r = rig();
    let cloud = Arc::new(RecordingCloud { upload_delay: Duration::from_millis(500), ..Default::default() });
    r.launcher.set_prep_services(services(true, Some(cloud.clone())));
    let mut process = process_plan("/games/run.sh", &[]);
    process.prep = PrepSpec { cloud_saves: Some(CloudSaves::new("legendary", "/saves")), ..Default::default() };
    r.launcher.launch(request("Calluna", LaunchPlan::Native(process))).unwrap();

    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert_eq!(status_of(next_any(&r.rx)), ("Downloading cloud saves...".to_string(), false));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    // The download ran before the game was spawned.
    assert_eq!(*cloud.calls.lock().unwrap(), [("--skip-upload".to_string(), "Calluna:/saves".to_string())]);

    std::thread::sleep(Duration::from_millis(60));
    r.spawner.finish(0, 0);
    assert_eq!(status_of(next_any(&r.rx)), ("Uploading cloud saves...".to_string(), false));
    let LaunchEvent::Exited { elapsed, .. } = next(&r.rx) else { panic!() };
    // The upload (500 ms) is not playtime.
    assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
    assert_eq!(cloud.calls.lock().unwrap()[1], ("--skip-download".to_string(), "Calluna:/saves".to_string()));
    assert!(!r.launcher.is_running("Calluna"));
}

#[test]
fn a_forced_clear_does_not_upload_saves() {
    let r = rig();
    let cloud = Arc::new(RecordingCloud::default());
    r.launcher.set_prep_services(services(true, Some(cloud.clone())));
    let mut process = process_plan("/games/run.sh", &[]);
    process.prep = PrepSpec { cloud_saves: Some(CloudSaves::new("legendary", "/s")), ..Default::default() };
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let _download = next_any(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert!(r.launcher.force_clear("g"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert_eq!(cloud.calls.lock().unwrap().len(), 1);
}

#[test]
fn offline_launches_skip_cloud_saves_silently() {
    let r = rig();
    let cloud = Arc::new(RecordingCloud::default());
    r.launcher.set_prep_services(services(false, Some(cloud.clone())));
    let mut process = process_plan("/games/run.sh", &[]);
    process.prep = PrepSpec { cloud_saves: Some(CloudSaves::new("legendary", "/s")), ..Default::default() };
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(cloud.calls.lock().unwrap().is_empty());
}

#[test]
fn a_missing_umu_runtime_that_cannot_be_downloaded_fails_the_launch() {
    let r = rig();
    r.launcher.set_prep_services(services(false, None));
    let mut process = process_plan("/games/run.sh", &[]);
    process.prep = PrepSpec { umu: true, ..Default::default() };
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let LaunchEvent::Error { message, .. } = next(&r.rx) else { panic!() };
    assert!(message.contains("umu"), "{message}");
    assert!(r.spawner.spawned().is_empty());
    assert!(!r.launcher.is_running("g"));
}

#[test]
fn prefix_creation_is_announced_and_runs_before_the_game() {
    let r = rig();
    let mut process = process_plan("/games/run.sh", &[]);
    process.pre_commands = vec![Command { program: "/w/wine".into(), args: vec!["wineboot".into()], ..Default::default() }];
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert_eq!(status_of(next_any(&r.rx)), ("Preparing prefix...".to_string(), false));
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    assert_eq!(r.spawner.spawned()[0].program, "/w/wine");
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert_eq!(r.spawner.spawned()[1].program, "/games/run.sh");
    r.spawner.finish(1, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
}

#[test]
fn stopping_while_preparing_ends_the_launch_without_a_session() {
    let r = rig();
    let mut process = process_plan("/games/run.sh", &[]);
    process.pre_commands = vec![Command { program: "/w/wine".into(), ..Default::default() }];
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let _ = next_any(&r.rx);
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    assert!(r.launcher.stop("g"));
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else { panic!() };
    assert!(record.is_none());
    assert_eq!(r.spawner.spawned().len(), 1, "the game never started");
}

#[test]
fn a_game_that_dies_right_after_starting_is_not_recorded() {
    let r = rig();
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    std::thread::sleep(Duration::from_millis(30));
    r.spawner.finish(0, 3);
    let LaunchEvent::Exited { exit, stopped, record, .. } = next(&r.rx) else { panic!() };
    assert_eq!(exit.unwrap().code, Some(3));
    assert!(!stopped);
    assert!(record.is_none());
    assert!(!r.paths.sessions().exists());
    assert!(!r.paths.store_config().exists(), "not moved up in recent");
    let ts: serde_json::Value = json_store::read_value(&r.paths.timestamps()).unwrap();
    assert!(ts["g"]["lastPlayed"].is_null());
}

#[test]
fn a_stopped_game_is_recorded_and_says_it_was_stopped() {
    let r = rig();
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert!(r.launcher.stop("g"));
    // The fake dies of SIGTERM, which would be a failed launch had the user not asked.
    let LaunchEvent::Exited { exit, stopped, record, .. } = next(&r.rx) else { panic!() };
    assert_eq!(exit.unwrap().signal, Some(15));
    assert!(stopped);
    assert!(record.is_some());
}

#[test]
fn preparing_the_launch_is_not_playtime() {
    let r = rig();
    let mut process = process_plan("/games/run.sh", &[]);
    process.pre_commands = vec![Command { program: "/w/wine".into(), ..Default::default() }];
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let _ = next_any(&r.rx);
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    std::thread::sleep(Duration::from_millis(500));
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    std::thread::sleep(Duration::from_millis(100));
    r.spawner.finish(1, 0);
    let LaunchEvent::Exited { elapsed, record, .. } = next(&r.rx) else { panic!() };
    assert!(elapsed >= Duration::from_millis(100) && elapsed < Duration::from_millis(400), "{elapsed:?}");
    assert!(record.unwrap().elapsed_seconds < 1);
    let s = &sessions_of(&r.paths, "g")[0];
    let (start, end) =
        (s.started_at.parse::<chrono::DateTime<chrono::Utc>>().unwrap(), s.ended_at.parse::<chrono::DateTime<chrono::Utc>>().unwrap());
    assert!(end - start < chrono::Duration::milliseconds(400), "{} -> {}", s.started_at, s.ended_at);
}

#[test]
fn stopping_a_steam_game_before_it_starts_records_nothing() {
    let r = rig();
    write_steam_log(&r, "");
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    wait_until("uri opened", || !r.spawner.uris().is_empty());
    assert!(r.launcher.stop("steam_400"));
    let LaunchEvent::Exited { stopped, record, .. } = next(&r.rx) else { panic!() };
    assert!(stopped);
    assert!(record.is_none());
    assert!(!r.paths.sessions().exists());
    assert!(!r.paths.store_config().exists());
}

/// A spawner whose game start panics, to see what the launcher does about it.
struct PanickingSpawner(FakeSpawner);

impl Spawner for PanickingSpawner {
    fn spawn(&self, _cmd: &Command, _log: Option<&std::path::Path>) -> std::io::Result<Box<dyn ProcessHandle>> {
        panic!("spawner bug");
    }
    fn open_uri(&self, uri: &str) -> std::io::Result<()> {
        self.0.open_uri(uri)
    }
    fn kill_pattern(&self, pattern: &str, since: u64) {
        self.0.kill_pattern(pattern, since)
    }
}

#[test]
fn a_panicking_session_frees_the_game_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    let home = dir.path().join("home");
    let steam = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    let launcher = Launcher::new(paths, steam, Arc::new(PanickingSpawner(FakeSpawner::new())), Arc::new(FakeProcs::new()), fast_cfg(), tx);
    launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
    assert!(matches!(next(&rx), LaunchEvent::Launching { .. }));
    let LaunchEvent::Error { app_name, message } = next(&rx) else { panic!() };
    assert_eq!(app_name, "g");
    assert!(message.contains("g title"), "{message}");
    assert!(!launcher.is_running("g"));
    // Not stuck on "already running" any more.
    launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
}

// ---------------------------------------------------------------------------
// Playtime checkpoints
// ---------------------------------------------------------------------------

fn sessions_of(paths: &DataPaths, app: &str) -> Vec<heronite_data::models::LocalGameSession> {
    let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
    doc.sessions.get(app).cloned().unwrap_or_default()
}

#[test]
fn a_running_game_is_checkpointed_and_the_end_replaces_the_checkpoints() {
    let r = rig();
    r.launcher.set_checkpoint_interval(Duration::from_millis(300));
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    // The crash case: the game "runs"; its progress is on disk already.
    wait_until("a checkpoint", || !sessions_of(&r.paths, "g").is_empty());
    let first = sessions_of(&r.paths, "g");
    assert_eq!(first.len(), 1);
    wait_until("a later checkpoint", || sessions_of(&r.paths, "g")[0].ended_at != first[0].ended_at);
    let ts: serde_json::Value = serde_json::from_str(&fs::read_to_string(r.paths.timestamps()).unwrap()).unwrap();
    assert!(ts["g"]["lastPlayed"].is_string() && ts["g"]["firstPlayed"].is_string());

    r.spawner.finish(0, 0);
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else { panic!() };
    assert!(record.is_some());
    let after = sessions_of(&r.paths, "g");
    assert_eq!(after.len(), 1, "one row, updated in place: {after:?}");
    assert_eq!(after[0].started_at, first[0].started_at);
    // No checkpoint lands after the end.
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(sessions_of(&r.paths, "g")[0].ended_at, after[0].ended_at);
}

#[test]
fn checkpoint_all_saves_every_running_game_now() {
    let r = rig();
    // Default interval: only the explicit call can have written anything.
    assert_eq!(r.launcher.checkpoint_all(), 0);
    r.launcher.launch(request("a", LaunchPlan::Native(process_plan("/games/a.sh", &[])))).unwrap();
    r.launcher.launch(request("b", LaunchPlan::Native(process_plan("/games/b.sh", &[])))).unwrap();
    let mut running = 0;
    while running < 2 {
        if matches!(next(&r.rx), LaunchEvent::Running { .. }) {
            running += 1;
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(!r.paths.sessions().exists());
    assert_eq!(r.launcher.checkpoint_all(), 2);
    assert_eq!(sessions_of(&r.paths, "a").len(), 1);
    assert_eq!(sessions_of(&r.paths, "b").len(), 1);
    // Calling it again moves the same rows.
    assert_eq!(r.launcher.checkpoint_all(), 2);
    assert_eq!(sessions_of(&r.paths, "a").len(), 1);
    r.spawner.finish(0, 0);
    r.spawner.finish(1, 0);
    let mut exited = 0;
    while exited < 2 {
        if matches!(next(&r.rx), LaunchEvent::Exited { .. }) {
            exited += 1;
        }
    }
    assert_eq!(sessions_of(&r.paths, "a").len(), 1);
}

#[test]
fn a_game_that_is_still_preparing_has_nothing_to_checkpoint() {
    let r = rig();
    let mut process = process_plan("/games/run.sh", &[]);
    process.pre_commands = vec![Command { program: "/w/wine".into(), ..Default::default() }];
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    assert_eq!(r.launcher.checkpoint_all(), 0);
    assert!(!r.paths.timestamps().exists());
    r.launcher.stop("g");
}

#[test]
fn the_keep_awake_lock_lives_as_long_as_the_game() {
    let r = rig();
    let mut plan = process_plan("/games/run.sh", &[]);
    plan.keep_awake = Some(Command { program: "/usr/bin/systemd-inhibit".into(), ..Default::default() });
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    wait_until("the lock", || r.spawner.spawned().len() == 2);
    assert_eq!(r.spawner.spawned()[1].program, "/usr/bin/systemd-inhibit");
    assert!(!r.spawner.handle(1).unwrap().lock().unwrap().terminated);
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.spawner.handle(1).unwrap().lock().unwrap().terminated, "released with the game");
}

// ---------------------------------------------------------------------------
// Programs alongside the game
// ---------------------------------------------------------------------------

/// A program alongside the game, run as `program` (Wine for an `.exe`).
fn companion(path: &std::path::Path, program: &str, delay_ms: u64) -> CompanionPlan {
    CompanionPlan {
        path: path.to_string_lossy().into_owned(),
        command: Command { program: program.into(), ..Default::default() },
        delay: Duration::from_millis(delay_ms),
    }
}

fn trainer_file(r: &Rig) -> std::path::PathBuf {
    let path = r._dir.path().join("Trainer.exe");
    fs::write(&path, "MZ").unwrap();
    path
}

#[test]
fn a_program_alongside_starts_after_its_delay_and_closes_with_the_game() {
    let r = rig();
    let mut plan = process_plan("/games/run.sh", &[]);
    plan.companions = vec![companion(&trainer_file(&r), "/umu", 300)];
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(r.spawner.spawned().len(), 1, "not before its delay");
    wait_until("the program", || r.spawner.spawned().len() == 2);
    assert_eq!(r.spawner.spawned()[1].program, "/umu");
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(r.spawner.handle(1).unwrap().lock().unwrap().terminated, "closed with the game");
}

#[test]
fn with_plain_proton_the_session_follows_the_game_in_its_prefix() {
    let r = rig();
    let mut plan = process_plan("/proton", &["waitforexitandrun", "game.exe"]);
    plan.prep.wine = Some(heronite_launch::settings::GameSettings { wine_prefix: "/pfx".into(), ..Default::default() });
    plan.companions = vec![companion(&trainer_file(&r), "/proton", 150)];
    // Proton returns right away; the game goes on in the prefix.
    r.procs.set_windows_programs("/pfx", &["services.exe", "game.exe"]);
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    r.spawner.finish(0, 0);
    // Started although the first process is gone.
    wait_until("the program", || r.spawner.spawned().len() == 2);
    r.procs.set_windows_programs("/pfx", &["services.exe", "game.exe", "trainer.exe"]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(r.launcher.is_running("g"), "the game still runs in the prefix");
    assert!(!r.spawner.handle(1).unwrap().lock().unwrap().terminated);
    // Only Wine's services and the trainer are left: the game is over.
    r.procs.set_windows_programs("/pfx", &["services.exe", "explorer.exe", "trainer.exe"]);
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else { panic!() };
    assert!(record.is_some());
    assert!(r.spawner.handle(1).unwrap().lock().unwrap().terminated);
}

#[test]
fn clearing_a_game_leaves_its_programs_running_and_missing_ones_are_reported() {
    let r = rig();
    let mut plan = process_plan("/games/run.sh", &[]);
    plan.companions = vec![companion(&r._dir.path().join("nope.exe"), "/umu", 0), companion(&trainer_file(&r), "/umu", 0)];
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    let (message, warning) = status_of(next_any(&r.rx));
    assert!(warning && message.contains("nope.exe"), "{message}");
    wait_until("the program", || r.spawner.spawned().len() == 2);
    assert!(r.launcher.force_clear("g"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(!r.spawner.handle(1).unwrap().lock().unwrap().terminated);
}

#[test]
fn programs_close_when_the_game_leaves_its_prefix_even_if_its_launcher_waits_for_them() {
    let r = rig();
    let mut plan = process_plan("/proton", &["waitforexitandrun", "game.exe"]);
    plan.prep.wine = Some(heronite_launch::settings::GameSettings { wine_prefix: "/pfx".into(), ..Default::default() });
    plan.companions = vec![companion(&trainer_file(&r), "/proton", 0)];
    r.procs.set_windows_programs("/pfx", &["steam.exe", "game.exe"]);
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    wait_until("the program", || r.spawner.spawned().len() == 2);
    // The game exits; Proton's process stays, waiting for the trainer.
    r.procs.set_windows_programs("/pfx", &["steam.exe", "services.exe", "trainer.exe"]);
    let t = Instant::now();
    wait_until("the trainer to be closed", || r.spawner.handle(1).unwrap().lock().unwrap().terminated);
    assert!(t.elapsed() >= Duration::from_secs(2), "a launcher may hand over to the game first: {:?}", t.elapsed());
    assert!(r.launcher.is_running("g"));
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
}

/// Another launcher on the same data and processes: the app started again
/// while the first one's games still run.
fn restarted(r: &Rig) -> (Launcher, Receiver<LaunchEvent>) {
    let home = r._dir.path().join("home");
    let steam = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    (Launcher::new(r.paths.clone(), steam, Arc::new(FakeSpawner::new()), r.procs.clone(), fast_cfg(), tx), rx)
}

fn running_file(paths: &DataPaths) -> serde_json::Value {
    json_store::read_value(&paths.local_library().join("running.json")).unwrap()
}

#[test]
fn a_game_left_running_is_picked_up_by_the_next_run() {
    let r = rig();
    // The fake spawner's first pid.
    r.procs.start(40_001, "game", None);
    r.procs.set_start_ticks(40_001, 777);
    let mut plan = process_plan("/games/game", &[]);
    plan.stop.kill_patterns = vec!["game".into()];
    r.launcher.launch(request("g", LaunchPlan::Native(plan))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { pid: 40_001, .. }));
    assert_eq!(running_file(&r.paths)["sessions"]["g"]["tracked"]["startTicks"], 777);
    // Quitting saves the playtime; the game keeps running.
    r.launcher.checkpoint_all();

    // Up to 30 minutes into the session, the app starts again. The clock
    // counts from boot, so a machine up for less than that (a fresh CI
    // runner) gets a shorter session.
    let mut minutes = 0;
    json_store::update_value(&r.paths.local_library().join("running.json"), |doc| {
        let mono = doc["sessions"]["g"]["monoStartMs"].as_u64().unwrap();
        minutes = (mono / 60_000).min(30);
        doc["sessions"]["g"]["monoStartMs"] = serde_json::json!(mono - minutes * 60_000);
        Ok(())
    })
    .unwrap();
    assert!(minutes > 0, "the machine has been up for less than a minute");
    let (again, rx) = restarted(&r);
    let mut adopted = Vec::new();
    assert_eq!(again.adopt(&mut |g| adopted.push(g.title.clone())), 1);
    assert_eq!(adopted, ["g title"]);
    assert_eq!(next(&rx), LaunchEvent::Running { app_name: "g".into(), pid: 40_001 });
    assert!(again.is_running("g"));

    // The game ends: recorded once, from the session's real start.
    r.procs.exit(40_001);
    let LaunchEvent::Exited { stopped, record, elapsed, .. } = next(&rx) else { panic!() };
    assert!(!stopped);
    assert!(elapsed >= Duration::from_secs(minutes * 60));
    assert_eq!(record.unwrap().total_played_minutes as u64, minutes);
    let doc: SessionsFile = json_store::read(&r.paths.sessions()).unwrap();
    assert_eq!(doc.sessions["g"].len(), 1, "the checkpointed row is updated in place");
    assert!(running_file(&r.paths)["sessions"].get("g").is_none());
}

#[test]
fn games_that_ended_meanwhile_or_before_a_reboot_are_dropped() {
    let r = rig();
    r.procs.start(40_001, "game", None);
    r.procs.set_start_ticks(40_001, 5);
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/game", &[])))).unwrap();
    next(&r.rx);
    next(&r.rx);
    // Its pid now belongs to another program that started later.
    r.procs.set_start_ticks(40_001, 6);
    let (again, rx) = restarted(&r);
    assert_eq!(again.adopt(&mut |_| {}), 0);
    assert!(rx.try_recv().is_err());
    assert!(running_file(&r.paths)["sessions"].get("g").is_none());

    // Another boot: the same pid and start ticks mean nothing.
    let r = rig();
    r.procs.start(40_001, "game", None);
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/game", &[])))).unwrap();
    next(&r.rx);
    next(&r.rx);
    r.procs.set_boot_id("another-boot");
    assert_eq!(restarted(&r).0.adopt(&mut |_| {}), 0);
}

#[test]
fn an_adopted_game_whose_leader_left_is_followed_through_its_group() {
    let r = rig();
    r.procs.start(40_001, "launcher", None);
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/launcher", &[])))).unwrap();
    next(&r.rx);
    next(&r.rx);
    // The launcher script exited; the game it started stays in its group.
    r.procs.exit(40_001);
    r.procs.start(40_050, "game", Some(1));
    r.procs.set_group(40_050, 40_001);
    let (again, rx) = restarted(&r);
    assert_eq!(again.adopt(&mut |_| {}), 1);
    next(&rx);
    // Stop reaches the whole group.
    assert!(again.stop("g"));
    wait_until("the group is signalled", || r.procs.group_signals().contains(&(40_001, heronite_launch::procs::Sig::Term)));
    r.procs.exit(40_050);
    let LaunchEvent::Exited { stopped, .. } = next(&rx) else { panic!() };
    assert!(stopped);
}

#[test]
fn a_steam_game_left_running_is_picked_up_by_its_pids() {
    let r = rig();
    let log = write_steam_log(&r, "");
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    r.procs.start(7777, "reaper", None);
    r.procs.set_start_ticks(7777, 42);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 7777\n").unwrap();
    next(&r.rx);
    assert_eq!(running_file(&r.paths)["sessions"]["steam_400"]["tracked"]["pids"], serde_json::json!([[7777, 42]]));

    let (again, rx) = restarted(&r);
    assert_eq!(again.adopt(&mut |_| {}), 1);
    assert_eq!(next(&rx), LaunchEvent::Running { app_name: "steam_400".into(), pid: 7777 });
    // Stop closes the game's processes, never Steam.
    again.stop_with("steam_400", StopOptions { kill_steam_processes: true });
    let LaunchEvent::Exited { stopped, .. } = next(&rx) else { panic!() };
    assert!(stopped);
    assert!(r.procs.signals().iter().any(|(pid, _)| *pid == 7777));
}

#[test]
fn steam_opened_for_a_game_followed_again_is_closed_after_it() {
    let r = rig();
    let log = write_steam_log(&r, "");
    json_store::write(&r.paths.global_config(), &serde_json::json!({"defaultSettings": {"heroniteCloseSteamAfterGame": true}})).unwrap();
    r.launcher.launch(request("steam_400", LaunchPlan::SteamUri { app_id: "400".into(), uri: "u".into() })).unwrap();
    next(&r.rx);
    wait_until("uri opened", || r.spawner.uris() == ["u"]);
    fs::write(r._dir.path().join("home/.steam/steam.pid"), "5000").unwrap();
    r.procs.start(5000, "steam", None);
    r.procs.start(7777, "reaper", None);
    fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"Game process added : AppID 400 x ProcID 7777\n").unwrap();
    next(&r.rx);
    assert_eq!(running_file(&r.paths)["sessions"]["steam_400"]["tracked"]["openedSteam"], true);

    let home = r._dir.path().join("home");
    let steam = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    let spawner = Arc::new(FakeSpawner::new());
    let again = Launcher::new(r.paths.clone(), steam, spawner.clone(), r.procs.clone(), fast_cfg(), tx);
    assert_eq!(again.adopt(&mut |_| {}), 1);
    next(&rx);
    r.procs.exit(7777);
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    wait_until("Steam closed", || spawner.uris() == ["steam://exit"]);
}

#[test]
fn a_prefix_whose_creation_was_stopped_is_created_again() {
    let r = rig();
    let prefix = r._dir.path().join("prefix");
    fs::create_dir_all(&prefix).unwrap();
    let marker = prefix.join(heronite_launch::wine::CREATING_MARKER);
    let plan = |prefix: &std::path::Path| {
        let mut process = process_plan("/games/run.sh", &[]);
        process.pre_commands = vec![Command { program: "/w/wineboot".into(), ..Default::default() }];
        let settings = heronite_launch::settings::GameSettings { wine_prefix: prefix.to_string_lossy().into(), ..Default::default() };
        process.prep = PrepSpec { wine: Some(settings), ..Default::default() };
        LaunchPlan::Native(process)
    };
    r.launcher.launch(request("g", plan(&prefix))).unwrap();
    wait_until("wineboot", || !r.spawner.spawned().is_empty());
    assert!(marker.exists(), "set while the prefix is being created");
    assert!(r.launcher.stop("g"));
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    assert!(marker.exists(), "kept: the creation was cut short");

    r.launcher.launch(request("g", plan(&prefix))).unwrap();
    wait_until("wineboot again", || r.spawner.spawned().len() == 2);
    r.spawner.finish(1, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert!(!marker.exists(), "removed once the prefix was created");
}

/// Mods for the tests: records when the guard is dropped.
struct FakeMods {
    fail: bool,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl heronite_launch::GameMods for FakeMods {
    fn apply(&self, app_name: &str, _runner: &str, status: &mut dyn FnMut(&str)) -> Result<Option<heronite_launch::AppliedMods>, String> {
        status("Applying mods...");
        if self.fail {
            return Err("no FUSE".into());
        }
        if app_name != "modded" {
            return Ok(None);
        }
        Ok(Some(heronite_launch::AppliedMods {
            env: vec![("WINEDLLOVERRIDES".into(), "winhttp=n,b".into())],
            summary: "Mods: 1 of 1 enabled".into(),
            guard: Box::new(DropFlag(self.dropped.clone())),
        }))
    }
}

#[test]
fn mods_are_applied_for_the_session_and_taken_away_after() {
    let r = rig();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    r.launcher.set_game_mods(Arc::new(FakeMods { fail: false, dropped: dropped.clone() }));
    let mut plan = process_plan("/games/run.sh", &[]);
    plan.command.env.insert("WINEDLLOVERRIDES".into(), "dxgi=n".into());
    r.launcher.launch(request("modded", LaunchPlan::Native(plan))).unwrap();
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "modded".into() });
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert_eq!(r.spawner.spawned()[0].env["WINEDLLOVERRIDES"], "dxgi=n;winhttp=n,b");
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst), "mods stay while the game runs");
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
    wait_until("mods taken away", || dropped.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn mods_that_cannot_be_applied_stop_the_launch() {
    let r = rig();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    r.launcher.set_game_mods(Arc::new(FakeMods { fail: true, dropped }));
    r.launcher.launch(request("modded", LaunchPlan::Native(process_plan("/games/run.sh", &[])))).unwrap();
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "modded".into() });
    let LaunchEvent::Error { message, .. } = next(&r.rx) else { panic!() };
    assert_eq!(message, "no FUSE");
    assert!(r.spawner.spawned().is_empty());
    assert!(!r.launcher.is_running("modded"));
}

/// Mods that note how many commands had run when they were applied.
struct OrderedMods {
    spawner: Arc<FakeSpawner>,
    spawned_when_applied: Arc<std::sync::Mutex<Option<usize>>>,
}

impl heronite_launch::GameMods for OrderedMods {
    fn apply(&self, _app: &str, _runner: &str, _status: &mut dyn FnMut(&str)) -> Result<Option<heronite_launch::AppliedMods>, String> {
        *self.spawned_when_applied.lock().unwrap() = Some(self.spawner.spawned().len());
        Ok(None)
    }
}

#[test]
fn mods_are_applied_after_the_launch_preparation() {
    let r = rig();
    let seen = Arc::new(std::sync::Mutex::new(None));
    r.launcher.set_game_mods(Arc::new(OrderedMods { spawner: r.spawner.clone(), spawned_when_applied: seen.clone() }));
    let mut plan = process_plan("/games/run.sh", &[]);
    plan.pre_commands = vec![Command { program: "/w/wineboot".into(), ..Default::default() }];
    r.launcher.launch(request("modded", LaunchPlan::Native(plan))).unwrap();
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(*seen.lock().unwrap(), None, "not applied while the prefix is being made");
    r.spawner.finish(0, 0);
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    assert_eq!(*seen.lock().unwrap(), Some(1), "applied once the prefix command ran, before the game spawned");
    assert_eq!(r.spawner.spawned()[1].program, "/games/run.sh");
}

/// Mods that note which games were released.
struct ReleaseMods(Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl heronite_launch::GameMods for ReleaseMods {
    fn apply(&self, _app: &str, _runner: &str, _status: &mut dyn FnMut(&str)) -> Result<Option<heronite_launch::AppliedMods>, String> {
        Ok(None)
    }

    fn release(&self, app_name: &str, runner: &str) {
        self.0.lock().unwrap().push((app_name.into(), runner.into()));
    }
}

#[test]
fn the_mods_of_an_adopted_game_are_taken_away_when_it_ends() {
    let r = rig();
    r.procs.start(40_001, "game", None);
    r.procs.set_start_ticks(40_001, 777);
    r.launcher.launch(request("g", LaunchPlan::Native(process_plan("/games/game", &[])))).unwrap();
    next(&r.rx);
    assert!(matches!(next(&r.rx), LaunchEvent::Running { pid: 40_001, .. }));
    // The app quits (the guard of the mount is gone with it) and starts again.
    let (again, rx) = restarted(&r);
    let released = Arc::new(std::sync::Mutex::new(Vec::new()));
    again.set_game_mods(Arc::new(ReleaseMods(released.clone())));
    assert_eq!(again.adopt(&mut |_| {}), 1);
    next(&rx);
    assert!(released.lock().unwrap().is_empty(), "the game still runs");
    r.procs.exit(40_001);
    assert!(matches!(next(&rx), LaunchEvent::Exited { .. }));
    assert_eq!(*released.lock().unwrap(), [("g".to_string(), "sideload".to_string())]);
}
