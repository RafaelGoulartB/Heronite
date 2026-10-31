//! Games a Windows store client starts (`plan::ClientHandoff`): the session
//! follows the game, not the client, which stays open. Nothing here starts a
//! game: the spawner and the process table are fakes.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::{json_store, DataPaths};
use heronite_launch::plan::{ClientHandoff, Handoff};
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::procs::{FakeProcs, Sig};
use heronite_launch::runtime::FakeSpawner;
use heronite_launch::steam::SteamLocator;
use heronite_launch::{Command, HandoffConfig, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, ProcessPlan};

const PREFIX: &str = "/pfx/bnet";

fn fast_cfg() -> LauncherConfig {
    LauncherConfig {
        poll_interval: Duration::from_millis(10),
        kill_grace: Duration::from_millis(150),
        min_session_seconds: 0,
        steam: SteamWaitConfig::default(),
        record: true,
        handoff: HandoffConfig {
            client_wait: Duration::from_millis(900),
            client_gone: Duration::from_millis(100),
            client_settle: Duration::from_millis(300),
            ..Default::default()
        },
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

fn command(program: &str, args: &[&str]) -> Command {
    Command { program: program.into(), args: args.iter().map(|s| s.to_string()).collect(), env: BTreeMap::new(), cwd: None }
}

/// Battle.net's game: the command starts the client, `join` asks the one that runs.
fn client_plan() -> ProcessPlan {
    ProcessPlan {
        command: command("/umu", &["Battle.net.exe", "--exec=launch WoW"]),
        handoff: Some(Handoff {
            prefix: Some(PREFIX.into()),
            folder: None,
            client: Some(ClientHandoff {
                name: "Battle.net".into(),
                programs: vec!["battle.net.exe".into(), "agent.exe".into()],
                main: "battle.net.exe".into(),
                join: command("/umu", &["join", "Battle.net.exe", "--exec=launch WoW"]),
            }),
        }),
        ..Default::default()
    }
}

fn launch(r: &Rig, plan: ProcessPlan) {
    let req = LaunchRequest {
        app_name: "wow".into(),
        runner: "sideload".into(),
        title: "World of Warcraft".into(),
        plan: LaunchPlan::Native(plan),
    };
    r.launcher.launch(req).unwrap();
}

fn next(rx: &Receiver<LaunchEvent>) -> LaunchEvent {
    rx.recv_timeout(Duration::from_secs(5)).expect("event")
}

/// The next event past the start of the launch and the wait for the game.
fn next_after_waiting(rx: &Receiver<LaunchEvent>) -> LaunchEvent {
    loop {
        match next(rx) {
            LaunchEvent::Launching { .. } | LaunchEvent::Preparing { .. } => continue,
            ev => return ev,
        }
    }
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

/// The client's programs (pids 800 and 801) plus what else runs.
fn running(r: &Rig, others: &[(u32, &str)]) {
    let mut all = vec![(800, "battle.net.exe"), (801, "agent.exe")];
    all.extend_from_slice(others);
    for (pid, name) in &all {
        r.procs.start(*pid, name, None);
    }
    r.procs.set_prefix_programs(&all);
}

fn nothing_played(r: &Rig) -> bool {
    !r.paths.timestamps().exists() && !r.paths.sessions().exists()
}

#[test]
fn the_session_starts_with_the_game_and_ends_after_it_while_the_client_stays() {
    let r = rig();
    let mut plan = client_plan();
    plan.keep_awake = Some(command("/bin/inhibit", &[]));
    launch(&r, plan);
    assert_eq!(next(&r.rx), LaunchEvent::Launching { app_name: "wow".into() });
    let LaunchEvent::Preparing { message, cancellable, .. } = next(&r.rx) else { panic!() };
    assert_eq!((message.as_str(), cancellable), ("Waiting for Battle.net to start the game…", true));
    wait_until("the command", || !r.spawner.spawned().is_empty());
    assert_eq!(r.spawner.spawned()[0].args, ["Battle.net.exe", "--exec=launch WoW"]);

    // The client opens, signs in, updates: the clock does not run.
    running(&r, &[]);
    std::thread::sleep(Duration::from_millis(150));
    assert!(r.rx.try_recv().is_err(), "nothing to say yet");
    assert!(nothing_played(&r));
    assert_eq!(r.spawner.spawned().len(), 1, "no keep-awake lock before the game");

    // The game shows up beside the client.
    let before_game = Instant::now();
    r.procs.start(900, "wow.exe", None);
    r.procs.set_prefix_programs(&[(800, "battle.net.exe"), (801, "agent.exe"), (900, "wow.exe")]);
    assert_eq!(next(&r.rx), LaunchEvent::Running { app_name: "wow".into(), pid: 900 });
    assert_eq!(r.launcher.pid_of("wow"), Some(900));
    wait_until("the lock", || r.spawner.spawned().len() == 2);
    assert!(r.paths.timestamps().exists(), "first played is written when the game shows up");
    let file = json_store::read_value(&r.paths.local_library().join("running.json")).unwrap();
    assert_eq!(file["sessions"]["wow"]["tracked"]["pid"], 900, "the next run of the app follows the game");

    // The command exits (a hand-off) and the game goes on.
    r.spawner.finish(0, 0);
    std::thread::sleep(Duration::from_millis(150));
    assert!(r.launcher.is_running("wow"));

    // The game closes; the client does not.
    let closed = Instant::now();
    r.procs.exit(900);
    let LaunchEvent::Exited { elapsed, exit, stopped, record, .. } = next(&r.rx) else { panic!() };
    let exited = Instant::now();
    assert!(closed.elapsed() >= Duration::from_millis(100), "a game restarting itself has a moment");
    // Played up to when the game closed, not up to when that was sure: the
    // clock starts after `before_game` and the 100 ms of grace end before `exited`.
    let longest = exited.duration_since(before_game).saturating_sub(Duration::from_millis(90));
    assert!(elapsed >= Duration::from_millis(140) && elapsed <= longest, "{elapsed:?} > {longest:?}");
    assert!(!stopped && exit.is_none() && record.is_some());
    assert!(!r.launcher.is_running("wow"));
    assert!(r.spawner.handle(1).unwrap().lock().unwrap().terminated, "the lock goes with the game");
    let client = r.spawner.handle(0).unwrap();
    assert!(!client.lock().unwrap().terminated && !client.lock().unwrap().killed);
    assert!(r.procs.signals().is_empty());
    let file = json_store::read_value(&r.paths.local_library().join("running.json")).unwrap();
    assert!(file["sessions"].get("wow").is_none());
}

#[test]
fn a_client_that_already_runs_is_asked_instead_of_started() {
    let r = rig();
    running(&r, &[]);
    launch(&r, client_plan());
    wait_until("the request", || !r.spawner.spawned().is_empty());
    assert_eq!(r.spawner.spawned()[0].args, ["join", "Battle.net.exe", "--exec=launch WoW"]);
    // The request returns at once.
    r.spawner.finish(0, 0);
    std::thread::sleep(Duration::from_millis(100));
    assert!(r.launcher.is_running("wow"), "the client is still open: the game is awaited");
    r.procs.start(900, "wow.exe", None);
    r.procs.set_prefix_programs(&[(800, "battle.net.exe"), (900, "wow.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { pid: 900, .. }));
    r.procs.exit(900);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { stopped: false, .. }));
}

#[test]
fn a_game_that_restarts_through_its_launcher_keeps_the_session() {
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[(900, "wow.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { pid: 900, .. }));
    r.procs.exit(900);
    std::thread::sleep(Duration::from_millis(40));
    r.procs.start(901, "wow.exe", None);
    r.procs.set_prefix_programs(&[(800, "battle.net.exe"), (801, "agent.exe"), (901, "wow.exe")]);
    std::thread::sleep(Duration::from_millis(250));
    assert!(r.launcher.is_running("wow"));
    r.procs.exit(901);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }));
}

#[test]
fn programs_run_alongside_the_game_are_not_the_game() {
    let r = rig();
    let mut plan = client_plan();
    plan.companions = vec![heronite_launch::plan::CompanionPlan {
        path: "/t/Trainer.exe".into(),
        command: command("/umu", &["/t/Trainer.exe"]),
        delay: Duration::from_secs(60),
    }];
    launch(&r, plan);
    running(&r, &[(900, "wow.exe"), (950, "trainer.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { pid: 900, .. }));
    r.procs.exit(900);
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { .. }), "the trainer alone does not keep the session");
}

#[test]
fn a_game_that_never_shows_up_ends_the_session_without_playtime() {
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[]);
    let t = Instant::now();
    let LaunchEvent::Error { message, .. } = next_after_waiting(&r.rx) else { panic!() };
    assert_eq!(message, "Battle.net did not start the game");
    assert!(t.elapsed() >= Duration::from_millis(800), "the client may be updating: {:?}", t.elapsed());
    assert!(!r.launcher.is_running("wow"));
    assert!(nothing_played(&r));
    let client = r.spawner.handle(0).unwrap();
    assert!(!client.lock().unwrap().terminated && !client.lock().unwrap().killed, "the client is left alone");
}

#[test]
fn a_client_the_user_closed_ends_the_wait_early() {
    let r = rig();
    launch(&r, client_plan());
    wait_until("the command", || !r.spawner.spawned().is_empty());
    let t = Instant::now();
    r.spawner.finish(0, 0);
    let LaunchEvent::Error { message, .. } = next_after_waiting(&r.rx) else { panic!() };
    assert_eq!(message, "Battle.net was closed before it started the game");
    assert!(t.elapsed() >= Duration::from_millis(200), "a starting client is not mistaken for a closed one: {:?}", t.elapsed());
    assert!(t.elapsed() < Duration::from_millis(800), "{:?}", t.elapsed());
    assert!(nothing_played(&r));
}

#[test]
fn a_command_that_fails_while_no_client_runs_is_a_failed_launch() {
    let r = rig();
    launch(&r, client_plan());
    wait_until("the command", || !r.spawner.spawned().is_empty());
    r.spawner.finish(0, 1);
    let LaunchEvent::Error { message, .. } = next_after_waiting(&r.rx) else { panic!() };
    assert_eq!(message, "Battle.net closed with an error before it started the game");
    assert!(nothing_played(&r));
    // A request that fails while the client runs is not: the game may come.
    let r = rig();
    running(&r, &[]);
    launch(&r, client_plan());
    wait_until("the request", || !r.spawner.spawned().is_empty());
    r.spawner.finish(0, 1);
    std::thread::sleep(Duration::from_millis(400));
    assert!(r.launcher.is_running("wow"));
}

#[test]
fn stopping_signals_only_the_game() {
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[(900, "wow.exe"), (901, "wowvoice.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { pid: 900, .. }));
    assert!(r.launcher.stop("wow"));
    wait_until("the game to be asked to close", || r.procs.signals().len() >= 2);
    let mut asked = r.procs.signals();
    asked.sort_by_key(|(pid, _)| *pid);
    assert_eq!(asked, [(900, Sig::Term), (901, Sig::Term)]);
    r.procs.exit(900);
    r.procs.exit(901);
    let LaunchEvent::Exited { stopped, record, .. } = next(&r.rx) else { panic!() };
    assert!(stopped && record.is_some(), "what was played is kept");
    let client = r.spawner.handle(0).unwrap();
    assert!(!client.lock().unwrap().terminated && !client.lock().unwrap().killed);

    // A game that ignores the request is killed after the grace; the client never.
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[(900, "wow.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { .. }));
    r.launcher.stop("wow");
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { stopped: true, .. }));
    let signals = r.procs.signals();
    assert!(signals.contains(&(900, Sig::Term)) && signals.contains(&(900, Sig::Kill)), "{signals:?}");
    assert!(signals.iter().all(|(pid, _)| *pid == 900), "{signals:?}");
    assert!(!r.spawner.handle(0).unwrap().lock().unwrap().killed);
}

#[test]
fn stopping_before_the_game_shows_up_plays_nothing() {
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[]);
    wait_until("the command", || !r.spawner.spawned().is_empty());
    std::thread::sleep(Duration::from_millis(50));
    assert!(r.launcher.stop("wow"));
    let LaunchEvent::Exited { elapsed, stopped, record, .. } = next_after_waiting(&r.rx) else { panic!() };
    assert!(stopped && elapsed == Duration::ZERO && record.is_none());
    assert!(!r.launcher.is_running("wow"));
    assert!(nothing_played(&r));
    assert!(r.procs.signals().is_empty(), "the client is not signalled");
    let client = r.spawner.handle(0).unwrap();
    assert!(!client.lock().unwrap().terminated && !client.lock().unwrap().killed);
}

#[test]
fn clearing_the_game_leaves_it_and_the_client_running() {
    let r = rig();
    launch(&r, client_plan());
    running(&r, &[(900, "wow.exe")]);
    assert!(matches!(next_after_waiting(&r.rx), LaunchEvent::Running { .. }));
    assert!(r.launcher.force_clear("wow"));
    assert!(matches!(next(&r.rx), LaunchEvent::Exited { stopped: true, .. }));
    assert!(r.procs.signals().is_empty());
}

/// With the game's folder known, what the client runs from elsewhere first
/// (a redistributable's installer) is not the game.
#[test]
fn only_what_runs_from_the_game_folder_is_the_game() {
    use heronite_launch::procs::WinProgram;
    let r = rig();
    let mut plan = client_plan();
    plan.handoff.as_mut().unwrap().folder = Some(format!("{PREFIX}/drive_c/Program Files (x86)/World of Warcraft"));
    launch(&r, plan);
    wait_until("the command", || !r.spawner.spawned().is_empty());
    let program = |pid: u32, name: &str, path: &str| WinProgram { pid, name: name.into(), path: path.into(), from_folder: false };
    let client = program(800, "battle.net.exe", "C:\\Program Files (x86)\\Battle.net\\Battle.net.exe");
    let redist = program(850, "vc_redist.x64.exe", "C:\\ProgramData\\Battle.net\\Setup\\vc_redist.x64.exe");
    for p in [&client, &redist] {
        r.procs.start(p.pid, &p.name, None);
    }
    r.procs.set_win_programs(vec![client.clone(), redist.clone()]);
    std::thread::sleep(Duration::from_millis(150));
    assert!(r.launcher.pid_of("wow").is_none_or(|pid| pid != 850), "the installer is not the game");

    let wow = program(900, "wow.exe", "C:\\Program Files (x86)\\World of Warcraft\\_retail_\\Wow.exe");
    r.procs.start(900, "wow.exe", None);
    r.procs.set_win_programs(vec![client, redist, wow]);
    assert_eq!(next_after_waiting(&r.rx), LaunchEvent::Running { app_name: "wow".into(), pid: 900 });
    r.launcher.stop("wow");
}
