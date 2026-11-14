//! A whole Epic launch with real processes, but fake binaries and a fake
//! Epic: `wine` is a shell script that logs what it was asked and creates the
//! files the real one would, and Epic's answers come from
//! [`heronite_launch::epic_launch::FakeEpic`]. Nothing real is started and
//! everything lives in a temporary directory.

// The fake Wine is a shell script.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use heronite_data::models::{GameInfo, InstallInfo};
use heronite_data::DataPaths;
use heronite_epic_api::launch::LaunchParams;
use heronite_launch::epic_launch::FakeEpic;
use heronite_launch::first_launch::{EpicSetup, Prerequisite};
use heronite_launch::net::FakeNet;
use heronite_launch::plan::{LaunchOptions, PlanContext};
use heronite_launch::prepare::CloudSync;
use heronite_launch::procs::SystemProcs;
use heronite_launch::runtime::PrepServices;
use heronite_launch::settings::{GameSettings, GlobalSettings};
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    build_plan_with_settings, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, LauncherConfig, SystemProbe, SystemSpawner,
};
use serde_json::json;

/// A fake binary. Scripts log with `printf '%s\n'`, not `echo`: where `/bin/sh` is
/// dash (Ubuntu), `echo` reads escapes, and `\c` in a registry key ends the line.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct LoggingCloud(PathBuf);
impl CloudSync for LoggingCloud {
    fn sync(&self, _runner: &str, app: &str, path: &str, flag: &str) -> Result<String, String> {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(&self.0).unwrap();
        writeln!(f, "cloud {app} {flag} {path}").unwrap();
        Ok(String::new())
    }
}

struct Rig {
    root: PathBuf,
    _dir: tempfile::TempDir,
    paths: DataPaths,
    plan: LaunchPlan,
    log: PathBuf,
}

/// Wine game "Calluna" with a missing prefix, DXVK on and cloud saves on.
fn rig(skip_version_check: bool) -> Rig {
    rig_with(skip_version_check, None)
}

fn rig_with(skip_version_check: bool, known_fixes: Option<serde_json::Value>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = DataPaths::at(root.join("heroic"));
    let log = root.join("calls.log");
    let prefix = root.join("pfx");
    let install = root.join("games/Control");
    fs::create_dir_all(&install).unwrap();

    // Fake wine: logs, and `wineboot --init` makes a Wine prefix.
    let wine = root.join("w/bin/wine");
    script(
        &wine,
        &format!(
            r#"printf '%s\n' "wine $* prefix=$WINEPREFIX" >> "{log}"
if [ "$1" = wineboot ] && [ "$2" = --init ]; then
  mkdir -p "$WINEPREFIX/drive_c/windows/system32" "$WINEPREFIX/drive_c/windows/syswow64" "$WINEPREFIX/dosdevices"
  touch "$WINEPREFIX/system.reg" "$WINEPREFIX/user.reg" "$WINEPREFIX/userdef.reg" "$WINEPREFIX/.update-timestamp"
fi"#,
            log = log.display()
        ),
    );
    script(&root.join("w/bin/wineserver"), &format!("printf '%s\\n' \"wineserver $*\" >> \"{}\"", log.display()));
    fs::write(install.join("Control.exe"), "").unwrap();

    // An installed DXVK release.
    let release = paths.tools_dir().join("dxvk/dxvk-9");
    for d in ["x32", "x64"] {
        fs::create_dir_all(release.join(d)).unwrap();
        fs::write(release.join(d).join("d3d11.dll"), d).unwrap();
    }
    fs::write(paths.tools_dir().join("dxvk/latest_dxvk"), "dxvk-9").unwrap();

    let mut ctx = PlanContext::load(&paths);
    ctx.global = GlobalSettings::from_defaults(serde_json::Map::new());
    ctx.ui_language = Some("en".into());
    ctx.known_fixes = known_fixes;

    let settings = GameSettings::from_map(
        json!({
            "wineVersion": {"bin": wine.to_str().unwrap(), "name": "Wine", "type": "wine"},
            "winePrefix": prefix.to_str().unwrap(),
            "autoInstallDxvk": true,
            "autoSyncSaves": true,
            "savesPath": "/saves/Control",
        })
        .as_object()
        .unwrap(),
        root.to_str().unwrap(),
    );
    let game = GameInfo {
        runner: "legendary".into(),
        app_name: "Calluna".into(),
        title: "Control".into(),
        is_installed: true,
        install: InstallInfo {
            install_path: Some(install.to_string_lossy().into_owned()),
            platform: Some("Windows".into()),
            executable: Some("Control.exe".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let opts = LaunchOptions { skip_version_check, ..Default::default() };
    let plan = build_plan_with_settings(&ctx, &SystemProbe, &game, None, &settings, &opts);
    assert!(matches!(&plan, LaunchPlan::Wine { process, .. } if process.epic.is_some()), "{plan:?}");
    Rig { root, _dir: dir, paths, plan, log }
}

/// What the fake Epic answers: the game's program and this session's sign-in.
fn epic(r: &Rig, update: bool) -> FakeEpic {
    let dir = r.root.join("games/Control").to_string_lossy().into_owned();
    let params = LaunchParams {
        game_executable: "Control.exe".into(),
        game_directory: dir.clone(),
        working_directory: dir,
        game_parameters: vec!["-nolauncher".into()],
        egl_parameters: vec!["-AUTH_PASSWORD=code".into(), "-epicapp=Calluna".into()],
        user_parameters: vec![],
    };
    FakeEpic { params, update, ..Default::default() }
}

fn run(r: &Rig) -> (Vec<LaunchEvent>, Vec<String>) {
    run_with(r, None, epic(r, false))
}

fn run_with(r: &Rig, setup: Option<Arc<dyn EpicSetup>>, service: FakeEpic) -> (Vec<LaunchEvent>, Vec<String>) {
    let (tx, rx) = channel();
    let home = r.root.join("home");
    let steam = SteamLocator {
        home: home.clone(),
        steam_path: home.join(".steam/steam"),
        system_roots: vec![],
        platform_roots: vec![home.join(".steam/steam")],
    };
    let cfg = LauncherConfig { poll_interval: Duration::from_millis(10), min_session_seconds: 0, ..Default::default() };
    let procs = Arc::new(SystemProcs);
    let launcher = Launcher::new(r.paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs, cfg, tx);
    launcher.set_prep_services(PrepServices {
        probe: Arc::new(SystemProbe),
        net: Arc::new(FakeNet::online()),
        cloud: Some(Arc::new(LoggingCloud(r.log.clone()))),
        gpu_registry: Arc::new(heronite_launch::gpu::preference::FakeRegistry::default()),
    });
    if let Some(setup) = setup {
        launcher.set_epic_setup(setup);
    }
    launcher.set_epic_service(Arc::new(service));
    launcher
        .launch(LaunchRequest { app_name: "Calluna".into(), runner: "legendary".into(), title: "Control".into(), plan: r.plan.clone() })
        .unwrap();
    let mut events = Vec::new();
    loop {
        let ev = rx.recv_timeout(Duration::from_secs(20)).expect("launch events");
        let done = matches!(ev, LaunchEvent::Exited { .. } | LaunchEvent::Error { .. });
        events.push(ev);
        if done {
            break;
        }
    }
    let log = fs::read_to_string(&r.log).unwrap_or_default().lines().map(str::to_string).collect();
    (events, log)
}

#[test]
fn an_epic_launch_prepares_everything_in_heroics_order() {
    let r = rig(false);
    let (events, log) = run(&r);
    assert!(matches!(events.last(), Some(LaunchEvent::Exited { .. })), "{events:?}");

    // Order of the side effects: saves down, prefix, the first-launch Epic
    // setup, DXVK overrides, game, saves up.
    let kinds: Vec<&str> = log
        .iter()
        .map(|l| {
            if l.starts_with("cloud") {
                if l.contains("--skip-upload") {
                    "saves-down"
                } else {
                    "saves-up"
                }
            } else if l.starts_with("wine ") && l.contains("Control.exe") {
                "game"
            } else if l.starts_with("wine wineboot") {
                "prefix"
            } else if l.starts_with("wine reg add HKEY_CLASSES_ROOT") {
                "epic-setup"
            } else if l.starts_with("wine reg add") {
                "override"
            } else if l.starts_with("wineserver") {
                "wineserver"
            } else {
                "?"
            }
        })
        .collect();
    assert_eq!(kinds, ["saves-down", "prefix", "wineserver", "epic-setup", "override", "game", "wineserver", "saves-up"], "{log:#?}");
    // The prefix remembers the game, so the setup does not run again.
    assert_eq!(fs::read_to_string(r.root.join("pfx/installed_games")).unwrap(), r#"["Calluna"]"#);
    assert_eq!(log[0], "cloud Calluna --skip-upload /saves/Control".to_string());
    assert!(log.last().unwrap().contains("--skip-download"));

    // DXVK ended up in the new prefix.
    let pfx = r.root.join("pfx");
    assert_eq!(fs::read_to_string(pfx.join("current_dxvk")).unwrap(), "dxvk-9");
    assert!(pfx.join("drive_c/windows/system32/d3d11.dll").exists());
    assert!(pfx.join("drive_c/windows/syswow64/d3d11.dll").exists());

    // The game's program through the Wine of the settings, with the build's
    // arguments and this session's sign-in from Epic.
    let game = log.iter().find(|l| l.starts_with("wine ") && l.contains("Control.exe")).unwrap();
    let exe = r.root.join("games/Control/Control.exe");
    assert!(game.starts_with(&format!("wine {} -nolauncher -AUTH_PASSWORD=code -epicapp=Calluna", exe.display())), "{game}");

    // The user was told what happens.
    let messages: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            LaunchEvent::Status { message, .. } | LaunchEvent::Preparing { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        messages,
        [
            "Downloading cloud saves...",
            "Preparing prefix...",
            "Installing Redistributables (EPIC)",
            "Installing DXVK in the prefix...",
            "Logging in to Epic Games...",
            "Uploading cloud saves..."
        ]
    );
    // Before the game runs they are the launch's steps, and the cloud saves
    // download cannot be cut short.
    let steps: Vec<(&str, bool)> = events
        .iter()
        .filter_map(|e| match e {
            LaunchEvent::Preparing { message, cancellable, .. } => Some((message.as_str(), *cancellable)),
            _ => None,
        })
        .collect();
    assert_eq!(steps[0], ("Downloading cloud saves...", false));
    assert!(steps[1..].iter().all(|(_, c)| *c), "{steps:?}");
    assert!(events.iter().any(|e| matches!(e, LaunchEvent::Status { message, .. } if message == "Uploading cloud saves...")));
}

#[test]
fn an_update_stops_the_game_unless_play_anyway() {
    let r = rig(false);
    let (events, log) = run_with(&r, None, epic(&r, true));
    assert!(matches!(events.last(), Some(LaunchEvent::Error { message, .. }) if message.contains("Play anyway")), "{events:?}");
    assert!(!log.iter().any(|l| l.contains("Control.exe")), "{log:#?}");
    let r = rig(true);
    let (events, log) = run_with(&r, None, epic(&r, true));
    assert!(matches!(events.last(), Some(LaunchEvent::Exited { .. })), "{events:?}");
    assert!(log.iter().any(|l| l.contains("Control.exe")));
}

struct FakeSetup;
impl EpicSetup for FakeSetup {
    fn prerequisites(&self, _: &str, _: &str) -> Result<Option<Prerequisite>, String> {
        Ok(Some(Prerequisite { path: "Redist\\setup.exe".into(), args: "/q /norestart".into() }))
    }
    fn eos_overlay_installed(&self) -> bool {
        false
    }
    fn eos_overlay_enabled(&self, _: &str) -> bool {
        false
    }
    fn eos_overlay_enable(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// The first launch in a prefix installs the game's prerequisites and its
/// known fixes before DXVK and the game; the second one skips all of it.
#[test]
fn the_first_launch_in_a_prefix_runs_prerequisites_and_fixes_once() {
    let fixes = json!({"epic": {"Calluna": {"runInPrefix": ["Fix/vc.exe"]}}});
    let r = rig_with(false, Some(fixes));
    let setup: Arc<dyn EpicSetup> = Arc::new(FakeSetup);
    let (events, log) = run_with(&r, Some(setup.clone()), epic(&r, false));
    assert!(matches!(events.last(), Some(LaunchEvent::Exited { .. })), "{events:?}");

    let game_dir = r.root.join("games/Control");
    let wine: Vec<&String> = log.iter().filter(|l| l.starts_with("wine ")).collect();
    let at = |needle: &str| wine.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {wine:#?}"));
    assert!(at("wineboot --init") < at("com.epicgames.launcher"));
    assert!(at("com.epicgames.launcher") < at(&format!("{}/Redist/setup.exe /q /norestart", game_dir.display())));
    assert!(at("Redist/setup.exe") < at(&format!("{}/Fix/vc.exe", game_dir.display())));
    assert!(at("Fix/vc.exe") < at("DllOverrides"), "fixes come before DXVK: {wine:#?}");
    assert_eq!(fs::read_to_string(r.root.join("pfx/installed_games")).unwrap(), r#"["Calluna"]"#);
    let messages: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            LaunchEvent::Status { message, .. } | LaunchEvent::Preparing { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(messages.contains(&"Installing Redistributables (EPIC)".to_string()), "{messages:?}");
    assert!(messages.contains(&"Installing Redistributables (FIXES)".to_string()), "{messages:?}");

    // Second launch: the prefix remembers the game.
    fs::remove_file(&r.log).unwrap();
    let (_, log) = run_with(&r, Some(setup), epic(&r, false));
    assert!(!log.iter().any(|l| l.contains("setup.exe") || l.contains("vc.exe") || l.contains("com.epicgames")), "{log:#?}");
    assert!(log.iter().any(|l| l.starts_with("wine ") && l.contains("Control.exe")));
}
