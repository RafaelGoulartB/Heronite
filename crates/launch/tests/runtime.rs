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
use heronite_launch::plan::PrepSpec;
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::prepare::CloudSync;
use heronite_launch::probe::SystemProbe;
use heronite_launch::procs::{FakeProcs, Procs, SystemProcs};
use heronite_launch::runtime::{FakeSpawner, PrepServices};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    Command, LaunchError, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, ProcessPlan, StopOptions, StopSpec,
    SystemSpawner,
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
    let LaunchEvent::Exited { app_name, elapsed, exit, record } = next(&r.rx) else { panic!() };
    assert_eq!(app_name, "g1");
    assert!(elapsed >= Duration::from_millis(100));
    assert_eq!(exit.unwrap().code, Some(0));
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
    assert_eq!(r.spawner.patterns(), ["pcsx2"], "patterns shorter than 3 chars are never pkill'ed");
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
    assert!(record.is_some());
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
    fn sync(&self, app: &str, path: &str, flag: &str) -> Result<String, String> {
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
        other => panic!("expected a status, got {other:?}"),
    }
}

#[test]
fn cloud_saves_are_downloaded_before_and_uploaded_after_the_game() {
    let r = rig();
    let cloud = Arc::new(RecordingCloud { upload_delay: Duration::from_millis(500), ..Default::default() });
    r.launcher.set_prep_services(services(true, Some(cloud.clone())));
    let mut process = process_plan("/games/run.sh", &[]);
    process.prep = PrepSpec { cloud_saves: Some("/saves".into()), ..Default::default() };
    r.launcher.launch(request("Calluna", LaunchPlan::Native(process))).unwrap();

    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    assert_eq!(status_of(next(&r.rx)), ("Downloading cloud saves...".to_string(), false));
    assert!(matches!(next(&r.rx), LaunchEvent::Running { .. }));
    // The download ran before the game was spawned.
    assert_eq!(*cloud.calls.lock().unwrap(), [("--skip-upload".to_string(), "Calluna:/saves".to_string())]);

    std::thread::sleep(Duration::from_millis(60));
    r.spawner.finish(0, 0);
    assert_eq!(status_of(next(&r.rx)), ("Uploading cloud saves...".to_string(), false));
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
    process.prep = PrepSpec { cloud_saves: Some("/s".into()), ..Default::default() };
    r.launcher.launch(request("g", LaunchPlan::Native(process))).unwrap();
    assert!(matches!(next(&r.rx), LaunchEvent::Launching { .. }));
    let _download = next(&r.rx);
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
    process.prep = PrepSpec { cloud_saves: Some("/s".into()), ..Default::default() };
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
    assert_eq!(status_of(next(&r.rx)), ("Preparing prefix...".to_string(), false));
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
    let _ = next(&r.rx);
    wait_until("the prefix command", || !r.spawner.spawned().is_empty());
    assert!(r.launcher.stop("g"));
    let LaunchEvent::Exited { record, .. } = next(&r.rx) else { panic!() };
    assert!(record.is_none());
    assert_eq!(r.spawner.spawned().len(), 1, "the game never started");
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
