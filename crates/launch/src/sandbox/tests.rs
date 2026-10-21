use super::*;
use crate::plan::{build_plan_with_settings, LaunchOptions, LaunchPlan, WineRuntime};
use crate::probe::FakeProbe;
use crate::wine_tools::{build_tool_command, kill_prefix_command, wait_prefix_command, WineTool};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta};
use serde_json::json;

const UMU: &str = "/hr/tools/runtimes/umu/umu_run.py";
const PRIVATE: &str = "/hr/sandbox/prefix-pfx-";

fn ctx() -> PlanContext {
    PlanContext::load(&DataPaths::at("/hr"))
}

fn settings(v: Value) -> GameSettings {
    GameSettings::from_map(v.as_object().unwrap(), "/home/u")
}

fn windows_settings(sandbox: Value) -> GameSettings {
    settings(json!({
        "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
        "winePrefix": "/home/u/Games/pfx",
        "heroniteSharedFolders": ["/data"],
        KEY: sandbox,
    }))
}

fn game(exe: &str, platform: &str) -> GameInfo {
    GameInfo {
        runner: "sideload".into(),
        app_name: "g1".into(),
        title: "Game One".into(),
        install: InstallInfo { executable: Some(exe.into()), platform: Some(platform.into()), ..Default::default() },
        ..Default::default()
    }
}

fn meta() -> LocalGameMeta {
    LocalGameMeta {
        app_name: "g1".into(),
        runner: "sideload".into(),
        launch_kind: "executable".into(),
        title: "Game".into(),
        ..Default::default()
    }
}

/// umu with its Steam runtime downloaded (used outside the sandbox only),
/// Winetricks, bubblewrap 0.11.
fn probe() -> FakeProbe {
    FakeProbe::new("/home/u")
        .with_path("/p/GE/proton")
        .with_path(UMU)
        .with_path("/hr/tools/winetricks")
        .with_path("/home/u/Games/win/game.exe")
        .with_path("/home/u/Games/pfx")
        .with_path("/home/u/.local/share/umu")
        .with_path("/home/u/.local/share/umu/steamrt3")
        .with_bwrap((0, 11, 0))
}

fn plan(probe: &FakeProbe, s: &GameSettings, g: &GameInfo) -> LaunchPlan {
    build_plan_with_settings(&ctx(), probe, g, Some(&meta()), s, &LaunchOptions::default())
}

/// The program of a wrapped command (`sh -c <program>`).
fn program(c: &Command) -> &str {
    assert!(is_wrapped(c), "{c:?}");
    &c.args[1]
}

#[test]
fn the_setting_is_read_and_written() {
    let s = Sandbox::from_value(Some(&json!({"enabled": true, "network": true, "devices": "yes"})));
    assert_eq!(s, Sandbox { enabled: true, network: true, ..Default::default() });
    assert_eq!(Sandbox::from_value(Some(&s.to_value())), s);
    assert_eq!(Sandbox::from_value(None), Sandbox::default());
    let all = Exception::ALL.iter().fold(Sandbox::default(), |s, e| s.with(*e, true));
    assert!(Exception::ALL.iter().all(|e| all.allows(*e)));
}

#[test]
fn private_homes_are_per_prefix_or_per_game() {
    let paths = DataPaths::at("/hr");
    let a = private_home(&paths, Owner::Prefix("/home/u/Games/My Prefix/"));
    assert!(a.starts_with("/hr/sandbox/prefix-My_Prefix-"), "{a}");
    assert_eq!(a, private_home(&paths, Owner::Prefix("/home/u/Games/My Prefix")));
    assert_ne!(a, private_home(&paths, Owner::Prefix("/other/My Prefix")));
    let evil = private_home(&paths, Owner::Game("../evil"));
    assert!(evil.starts_with("/hr/sandbox/game-.._evil-"), "{evil}");
    // Names that give the same safe name keep homes of their own.
    assert_ne!(private_home(&paths, Owner::Game("My Game")), private_home(&paths, Owner::Game("My_Game")));
}

#[test]
fn shared_folders_are_hidden_unless_allowed() {
    assert!(windows_settings(json!({"enabled": true})).shared_folders.is_empty());
    assert_eq!(windows_settings(json!({"enabled": true, "sharedFolders": true})).shared_folders, ["/data"]);
    assert_eq!(windows_settings(json!({})).shared_folders, ["/data"]);
}

#[test]
fn a_windows_game_runs_proton_directly_in_the_sandbox() {
    let s = windows_settings(json!({"enabled": true}));
    let p = plan(&probe(), &s, &game("/home/u/Games/win/game.exe", "Windows"));
    let LaunchPlan::Wine { runtime: WineRuntime::Proton, process, .. } = &p else { panic!("{p:?}") };
    let c = &process.command;
    assert_eq!(c.program, "/bin/sh");
    assert_eq!(c.args[2..], [NAME_FOR_TESTS, "/p/GE/proton", "waitforexitandrun", "/home/u/Games/win/game.exe"]);
    assert!(process.notes.contains(&without_umu()), "{:?}", process.notes);
    let script = program(c);
    assert!(script.contains("--bind /home/u/Games/pfx /home/u/Games/pfx"), "{script}");
    assert!(script.contains("--ro-bind-try /home/u/Games/win /home/u/Games/win"));
    assert!(script.contains(&format!("--bind {PRIVATE}")));
    assert!(!script.contains(".local/share/umu"), "umu's runtime stays out");
    assert!(script.contains("--unshare-user --unshare-pid"));
    assert!(script.contains("--unshare-net"));
    assert!(!script.contains("/data /data"), "shared folders stay out");
    assert!(process.sandboxed);
    assert!(process.stop.commands.is_empty());
    assert!(process.wait_after_exit.is_none());
    // A new prefix is created in the sandbox too.
    let fresh = probe().with_path("/home/u/Games/pfx");
    let mut empty = fresh.clone();
    empty.paths.remove("/home/u/Games/pfx");
    let LaunchPlan::Wine { process, .. } = plan(&empty, &s, &game("/home/u/Games/win/game.exe", "Windows")) else { panic!() };
    assert!(!process.pre_commands.is_empty() && process.pre_commands.iter().all(is_wrapped));
}

#[test]
fn exceptions_reach_the_sandbox() {
    let s = windows_settings(json!({"enabled": true, "network": true, "gameFolder": true, "sharedFolders": true}));
    let LaunchPlan::Wine { process, .. } = plan(&probe(), &s, &game("/home/u/Games/win/game.exe", "Windows")) else { panic!() };
    let script = program(&process.command);
    assert!(!script.contains("--unshare-net"));
    assert!(script.contains("--bind-try /home/u/Games/win /home/u/Games/win"));
    assert!(script.contains("--bind-try /data /data"));
}

#[test]
fn without_bubblewrap_nothing_runs() {
    let mut no_bwrap = probe();
    no_bwrap.bwrap = None;
    let s = windows_settings(json!({"enabled": true}));
    let p = plan(&no_bwrap, &s, &game("/home/u/Games/win/game.exe", "Windows"));
    assert!(p.unavailable_reason().is_some_and(|r| r.contains("bubblewrap")), "{p:?}");
    let broken = probe();
    let mut broken = broken;
    broken.bwrap.as_mut().unwrap().error = Some("setting up uid map: Permission denied".into());
    let p = plan(&broken, &s, &game("/home/u/Games/win/game.exe", "Windows"));
    assert!(p.unavailable_reason().is_some_and(|r| r.contains("uid map")));
    assert!(build_tool_command(&ctx(), &no_bwrap, &s, &WineTool::Winecfg).is_err());
    // Off, bubblewrap does not matter.
    let off = windows_settings(json!({"enabled": false}));
    assert!(plan(&no_bwrap, &off, &game("/home/u/Games/win/game.exe", "Windows")).unavailable_reason().is_none());
}

#[test]
fn umu_never_runs_in_the_sandbox() {
    // Its container would hold a copy of its Steam runtime in memory.
    let s = windows_settings(json!({"enabled": true}));
    let LaunchPlan::Wine { runtime, .. } = plan(&probe(), &s, &game("/home/u/Games/win/game.exe", "Windows")) else { panic!() };
    assert_eq!(runtime, WineRuntime::Proton);
    let off = windows_settings(json!({}));
    let LaunchPlan::Wine { runtime, .. } = plan(&probe(), &off, &game("/home/u/Games/win/game.exe", "Windows")) else { panic!() };
    assert_eq!(runtime, WineRuntime::Umu, "outside, as before");
    // A Linux game in the Steam runtime runs without it.
    let s = settings(json!({KEY: {"enabled": true}, "steamRuntime": "steamrt3"}));
    let p = probe().with_file("/home/u/Games/lin/run.sh", b"#!/bin/sh\n");
    let LaunchPlan::Native(process) = plan(&p, &s, &game("/home/u/Games/lin/run.sh", "linux")) else { panic!() };
    assert_eq!(process.command.args[2..], [NAME_FOR_TESTS, "/home/u/Games/lin/run.sh"]);
    assert!(process.notes.iter().any(|n| n.contains("Steam runtime")), "{:?}", process.notes);
}

/// Every command of `plan` (and its companions') runs in the sandbox: its
/// command has the prelude (after the wrappers that stay outside), and what
/// runs before it is wrapped.
fn all_inside(plan: &LaunchPlan) {
    let process = plan.process().unwrap_or_else(|| panic!("{plan:?}"));
    assert!(process.sandboxed, "{plan:?}");
    let c = &process.command;
    let words: Vec<&str> = std::iter::once(c.program.as_str()).chain(c.args.iter().map(String::as_str)).collect();
    let wrapper = c.args.iter().position(|a| a == "--wrapper").map(|i| shlex::split(&c.args[i + 1]).unwrap());
    let words: Vec<String> = match wrapper {
        Some(w) => w,
        None => words.iter().map(|w| w.to_string()).collect(),
    };
    assert!(words.windows(4).any(|w| w[0] == "/bin/sh" && w[1] == "-c" && w[3] == NAME_FOR_TESTS), "{words:?}");
    assert!(process.pre_commands.iter().all(is_wrapped), "{:?}", process.pre_commands);
    assert!(process.stop.commands.is_empty() && process.wait_after_exit.is_none());
}

#[test]
fn every_way_to_start_a_game_or_a_tool_runs_inside() {
    let s = windows_settings(json!({"enabled": true}));
    let mut fresh = probe().with_path("/home/u/Downloads/setup.exe");
    fresh.paths.remove("/home/u/Games/pfx");
    let win = game("/home/u/Games/win/game.exe", "Windows");
    all_inside(&plan(&probe(), &s, &win));
    all_inside(&plan(&fresh, &s, &win));
    let linux = settings(json!({KEY: {"enabled": true}}));
    let p = probe().with_file("/home/u/Games/lin/run.sh", b"#!/bin/sh\n");
    all_inside(&plan(&p, &linux, &game("/home/u/Games/lin/run.sh", "linux")));
    for tool in [WineTool::Winecfg, WineTool::Winetricks, WineTool::RunExe("/home/u/Downloads/setup.exe".into())] {
        assert!(is_wrapped(&build_tool_command(&ctx(), &fresh, &s, &tool).unwrap().command), "{tool:?}");
    }
    assert!(crate::wine_tools::create_prefix_commands(&ctx(), &fresh, &s).unwrap().iter().all(is_wrapped));
    assert!(is_wrapped(&kill_prefix_command(&ctx(), &fresh, &s).unwrap()));
    assert!(is_wrapped(&crate::wine_tools::winetricks_verbs_command(&ctx(), &fresh, &s, &["corefonts".into()]).unwrap().command));
    let parts = vec!["msiexec".to_string(), "/i".into(), "/home/u/Downloads/setup.exe".into()];
    assert!(is_wrapped(&crate::wine_tools::wine_command_for(&ctx(), &fresh, &s, parts, None).unwrap()));
}

/// `msiexec /i ~/setup.msi`: the file is shown, not the whole home.
#[test]
fn a_file_in_the_home_folder_is_shown_alone() {
    let s = windows_settings(json!({"enabled": true}));
    let p = probe().with_path("/home/u/setup.msi");
    let parts = vec!["msiexec".to_string(), "/i".into(), "/home/u/setup.msi".into()];
    let cmd = crate::wine_tools::wine_command_for(&ctx(), &p, &s, parts, None).unwrap();
    assert!(cmd.args[1].contains("--ro-bind-try /home/u/setup.msi /home/u/setup.msi"), "{cmd:?}");
}

#[test]
fn a_game_that_cannot_be_put_in_the_sandbox_does_not_start() {
    let linux = settings(json!({KEY: {"enabled": true}}));
    let m = LocalGameMeta { launch_kind: "steam-uri".into(), steam_app_id: Some("70".into()), ..meta() };
    let p = probe().with_bin("steam", "/usr/bin/steam");
    let plan = build_plan_with_settings(&ctx(), &p, &game("/x", "linux"), Some(&m), &linux, &LaunchOptions::default());
    assert!(plan.unavailable_reason().is_some_and(|r| r.contains("cannot run in the sandbox")), "{plan:?}");
    // Off, as before.
    let off = settings(json!({}));
    let plan = build_plan_with_settings(&ctx(), &p, &game("/x", "linux"), Some(&m), &off, &LaunchOptions::default());
    assert!(matches!(plan, LaunchPlan::SteamUri { .. }), "{plan:?}");
}

#[test]
fn a_game_that_would_show_the_whole_home_is_refused() {
    let s = windows_settings(json!({"enabled": true}));
    let p = probe().with_path("/home/u/game.exe");
    let plan = plan(&p, &s, &game("/home/u/game.exe", "Windows"));
    assert!(plan.unavailable_reason().is_some_and(|r| r.contains("whole home folder")), "{plan:?}");
}

#[test]
fn a_linux_game_gets_a_private_home_of_its_own() {
    let s = settings(json!({KEY: {"enabled": true}}));
    let p = probe().with_file("/home/u/Games/lin/run.sh", b"#!/bin/sh\n");
    let LaunchPlan::Native(process) = plan(&p, &s, &game("/home/u/Games/lin/run.sh", "linux")) else { panic!() };
    assert_eq!(process.command.args[2..], [NAME_FOR_TESTS, "/home/u/Games/lin/run.sh"]);
    let script = program(&process.command);
    assert!(script.contains(&format!("--bind {} /home/u", private_home(&DataPaths::at("/hr"), Owner::Game("g1")))), "{script}");
    assert!(script.contains("--ro-bind-try /home/u/Games/lin /home/u/Games/lin"));
    assert!(process.sandboxed);
}

#[test]
fn emulators_are_not_run_in_the_sandbox() {
    let s = settings(json!({KEY: {"enabled": true}}));
    let m = LocalGameMeta { launch_kind: "emulator".into(), ..meta() };
    let p = build_plan_with_settings(&ctx(), &probe(), &game("/x/rom.iso", "linux"), Some(&m), &s, &LaunchOptions::default());
    assert!(p.unavailable_reason().is_some_and(|r| r.contains("emulators")));
}

#[test]
fn prefix_tools_run_in_the_sandbox() {
    let s = windows_settings(json!({"enabled": true}));
    let p = probe().with_path("/home/u/Downloads/setup.exe");
    let t = build_tool_command(&ctx(), &p, &s, &WineTool::RunExe("/home/u/Downloads/setup.exe".into())).unwrap();
    assert!(program(&t.command).contains("--ro-bind-try /home/u/Downloads /home/u/Downloads"));
    assert_eq!(t.command.args.last().map(String::as_str), Some("/home/u/Downloads/setup.exe"));
    let inside = p.clone().with_path("/home/u/Games/pfx/drive_c/setup.exe");
    let t = build_tool_command(&ctx(), &inside, &s, &WineTool::RunExe("/home/u/Games/pfx/drive_c/setup.exe".into())).unwrap();
    assert!(!program(&t.command).contains("--ro-bind-try /home/u/Games/pfx/drive_c"), "the prefix stays writable");
    let t = build_tool_command(&ctx(), &p, &s, &WineTool::Winetricks).unwrap();
    assert!(is_wrapped(&t.command));
    assert!(t.notes.iter().any(|n| n.contains("Winetricks")));
    assert!(is_wrapped(&kill_prefix_command(&ctx(), &p, &s).unwrap()));
    assert!(wait_prefix_command(&ctx(), &p, &s.clone()).unwrap().is_none());
    // Off: as before.
    let off = windows_settings(json!({}));
    assert!(!is_wrapped(&build_tool_command(&ctx(), &p, &off, &WineTool::Winecfg).unwrap().command));
}

#[test]
fn epic_games_get_the_sandbox_inside_legendarys_wrapper() {
    let s = windows_settings(json!({"enabled": true}));
    let g = GameInfo {
        runner: "legendary".into(),
        app_name: "Calluna".into(),
        title: "Calluna".into(),
        is_installed: true,
        install: InstallInfo { install_path: Some("/home/u/Games/Calluna".into()), platform: Some("Windows".into()), ..Default::default() },
        ..Default::default()
    };
    let p = probe().with_path("/home/u/Games/Calluna").with_path("/hr/tools/bin/legendary").with_bin("legendary", "/usr/bin/legendary");
    let plan = build_plan_with_settings(&ctx(), &p, &g, None, &s, &LaunchOptions::default());
    let LaunchPlan::Legendary { process, .. } = &plan else { panic!("{plan:?}") };
    let args = &process.command.args;
    let wrapper = &args[args.iter().position(|a| a == "--wrapper").unwrap() + 1];
    let words = shlex::split(wrapper).unwrap();
    assert_eq!(words[..2], ["/bin/sh", "-c"]);
    assert_eq!(words[3..], [NAME_FOR_TESTS, "/p/GE/proton", "waitforexitandrun"]);
    assert!(words[2].contains("--ro-bind-try /home/u/Games/Calluna /home/u/Games/Calluna"));
    assert!(process.sandboxed && process.stop.commands.is_empty());
    // legendary splits the wrapper with Python's shlex: same words.
    let py = std::process::Command::new("python3")
        .args(["-c", "import json, shlex, sys; print(json.dumps(shlex.split(sys.argv[1])))", wrapper])
        .output();
    if let Some(out) = py.ok().filter(|o| o.status.success()) {
        let python: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(python, words);
    }
}

#[test]
fn confine_wraps_prefix_creation_and_drops_host_commands() {
    let c = |p: &str| Command { program: p.into(), ..Default::default() };
    let stop = StopSpec { kill_patterns: vec!["game.exe".into()], commands: vec![c("wineserver")] };
    let (pre, wait, stop2) = confine(None, vec![c("wineboot")], Some(c("wineserver")), stop.clone());
    assert_eq!((pre.len(), wait.is_some(), stop2.clone()), (1, true, stop.clone()));
    let prelude = vec!["/bin/sh".to_string(), "-c".into(), "x".into(), NAME_FOR_TESTS.into()];
    let (pre, wait, stop2) = confine(Some(&prelude), vec![c("wineboot")], Some(c("wineserver")), stop);
    assert!(is_wrapped(&pre[0]) && pre[0].args[3] == "wineboot");
    assert!(wait.is_none() && stop2.commands.is_empty());
    assert_eq!(stop2.kill_patterns, ["game.exe"]);
}

const NAME_FOR_TESTS: &str = "heronite-sandbox";
