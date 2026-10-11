//! Game logs written by the launcher: Heroic's layout, header, captured
//! output, `.old` rotation and the `disableLogs` / `verboseLogs` switches.
//! Only harmless `sh -c` processes run.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use heronite_data::DataPaths;
use heronite_launch::playtime_watch::SteamWaitConfig;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{Command, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, ProcessPlan, SystemSpawner};

struct Rig {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    launcher: Launcher,
    rx: Receiver<LaunchEvent>,
}

fn rig(config_json: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    fs::create_dir_all(paths.games_config_dir()).unwrap();
    fs::write(paths.root.join("config.json"), config_json).unwrap();
    let steam = SteamLocator { home: dir.path().join("home"), steam_path: dir.path().join("home/.steam/steam"), system_roots: vec![] };
    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let cfg = LauncherConfig {
        poll_interval: Duration::from_millis(10),
        kill_grace: Duration::from_millis(400),
        min_session_seconds: 0,
        steam: SteamWaitConfig::default(),
        record: true,
    };
    let launcher = Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, cfg, tx);
    launcher.set_system_info(Arc::new(|| "CPU: fake\nOS: test".to_string()));
    Rig { _dir: dir, paths, launcher, rx }
}

fn sh(script: &str) -> LaunchPlan {
    LaunchPlan::Native(ProcessPlan {
        command: Command {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::from([("HERONITE_TEST_VAR".to_string(), "1".to_string()), ("MY_API_TOKEN".to_string(), "s3cret".to_string())]),
            cwd: None,
        },
        notes: vec!["Using the default Wine".into()],
        ..Default::default()
    })
}

fn run(r: &Rig, app: &str, plan: LaunchPlan) {
    r.launcher.launch(LaunchRequest { app_name: app.into(), runner: "sideload".into(), title: format!("{app} title"), plan }).unwrap();
    loop {
        match r.rx.recv_timeout(Duration::from_secs(10)).expect("event") {
            LaunchEvent::Exited { .. } | LaunchEvent::Error { .. } => return,
            _ => {}
        }
    }
}

fn log_path(r: &Rig, app: &str) -> PathBuf {
    r.paths.root.join(format!("logs/games/{app}_sideload/launch.log"))
}

const VERBOSE: &str = r#"{"defaultSettings":{"verboseLogs":true},"version":"v0"}"#;

#[test]
fn launch_log_has_heroics_header_and_the_whole_output() {
    let r = rig(VERBOSE);
    run(&r, "g", sh("echo out-$((1+1)); echo err-$((2+2)) >&2; exit 2"));
    let log = fs::read_to_string(log_path(&r, "g")).unwrap();
    let pos = |needle: &str| log.find(needle).unwrap_or_else(|| panic!("{needle:?} missing in\n{log}"));
    assert!(pos("Launching \"g title\" (sideload)") < pos("Native? true"));
    assert!(pos("Native? true") < pos("System Info: \nCPU: fake"));
    assert!(pos("System Info") < pos("Game Settings:"));
    assert!(pos("Game Settings:") < pos("Using the default Wine"));
    assert!(pos("Using the default Wine") < pos("Game Output:"));
    assert!(pos("Game Output:") < pos("out-2\n") && pos("Game Output:") < pos("err-4\n"));
    assert!(pos("out-2\n") < pos("exited with code 2"));
    assert!(log.contains("HERONITE_TEST_VAR=1"));
    assert!(!log.contains("s3cret"), "secrets are hidden:\n{log}");
    assert!(log.trim_end().ends_with("============= End of log ============="), "{log}");
    assert!(log.contains("(") && log.contains("[INFO]:    Native? true"));
}

#[test]
fn a_second_launch_moves_the_first_log_to_old() {
    let r = rig(VERBOSE);
    run(&r, "g", sh("echo first-$((1+0))"));
    run(&r, "g", sh("echo second-$((1+1))"));
    let now = fs::read_to_string(log_path(&r, "g")).unwrap();
    let old = fs::read_to_string(log_path(&r, "g").with_extension("log.old")).unwrap();
    assert!(now.contains("second-2\n") && !now.contains("first-1\n"));
    assert!(old.contains("first-1\n") && old.contains("End of log"));
}

#[test]
fn big_output_is_stored_completely_without_stalling_the_game() {
    let r = rig(VERBOSE);
    // ~6 MB of output through the pipe.
    run(&r, "big", sh("i=0; while [ $i -lt 60000 ]; do echo \"line $i xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"; i=$((i+1)); done"));
    let log = fs::read_to_string(log_path(&r, "big")).unwrap();
    assert!(log.contains("line 0 ") && log.contains("line 59999 "));
    assert!(log.len() > 5_000_000);
}

#[test]
fn without_verbose_logs_only_the_notice_is_written() {
    let r = rig(VERBOSE);
    fs::write(r.paths.game_config("quiet"), r#"{"quiet":{"verboseLogs":false},"version":"v0"}"#).unwrap();
    run(&r, "quiet", sh("echo not-in-$((1+1))"));
    let log = fs::read_to_string(log_path(&r, "quiet")).unwrap();
    assert!(log.contains("[WARNING]: IMPORTANT: Logs are disabled"), "{log}");
    assert!(log.contains("Enable verbose logs in Game's settings"));
    assert!(!log.contains("not-in-2") && !log.contains("Game Settings"));
    assert!(log.contains("End of log"));
}

#[test]
fn disable_logs_writes_nothing_at_all() {
    let r = rig(r#"{"defaultSettings":{"disableLogs":true,"verboseLogs":true},"version":"v0"}"#);
    run(&r, "g", sh("echo nothing"));
    assert!(!log_path(&r, "g").exists());
    assert!(!r.paths.root.join("logs/games").exists());
}

#[test]
fn a_launch_that_cannot_start_logs_the_error() {
    let r = rig(VERBOSE);
    run(
        &r,
        "bad",
        LaunchPlan::Native(ProcessPlan {
            command: Command { program: "/definitely/not/here".into(), ..Default::default() },
            ..Default::default()
        }),
    );
    let log = fs::read_to_string(log_path(&r, "bad")).unwrap();
    assert!(log.contains("[ERROR]:") && log.contains("could not start /definitely/not/here"), "{log}");
    assert!(log.contains("End of log"));
}

#[test]
fn the_game_writes_straight_to_the_log_file_without_a_pipe() {
    let r = rig(VERBOSE);
    // Each stream of the game is the log file itself: nothing of the launcher
    // is in between, so the game keeps logging if the launcher goes away.
    run(&r, "direct", sh("readlink /proc/$$/fd/1; readlink /proc/$$/fd/2; echo after-$((1+1))"));
    let log = fs::read_to_string(log_path(&r, "direct")).unwrap();
    let path = log_path(&r, "direct");
    let target = fs::canonicalize(&path).unwrap();
    assert_eq!(log.matches(target.to_str().unwrap()).count(), 2, "{log}");
    assert!(!log.contains("pipe:["), "{log}");
    assert!(log.contains("after-2\n") && log.trim_end().ends_with("End of log ============="), "{log}");
}

#[test]
fn launcher_and_game_lines_interleave_in_order() {
    let r = rig(VERBOSE);
    run(&r, "order", sh("echo one-$((1+0)); sleep 0.1; echo two-$((1+1))"));
    let log = fs::read_to_string(log_path(&r, "order")).unwrap();
    let pos = |needle: &str| log.find(needle).unwrap_or_else(|| panic!("{needle:?} missing in\n{log}"));
    assert!(pos("Game Output:") < pos("one-1\n") && pos("one-1\n") < pos("two-2\n"));
    assert!(pos("two-2\n") < pos("exited with code 0") && pos("exited with code 0") < pos("End of log"));
}

#[test]
fn with_logs_disabled_the_game_output_goes_nowhere() {
    let r = rig(r#"{"defaultSettings":{"disableLogs":true,"verboseLogs":true},"version":"v0"}"#);
    run(&r, "g", sh("readlink /proc/$$/fd/1 >&2; echo hi"));
    assert!(!log_path(&r, "g").exists());
}
