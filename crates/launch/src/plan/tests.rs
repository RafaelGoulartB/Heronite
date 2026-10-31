use super::*;
use crate::emulators::{UserEmulator, UserEmulatorProfile};
use crate::probe::FakeProbe;
use crate::settings::{GameSettings, GlobalSettings};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalGameRom};
use heronite_data::DataPaths;
use serde_json::{json, Value};

#[cfg(unix)]
const UMU: &str = "/hr/tools/runtimes/umu/umu_run.py";

fn ctx(global: Value) -> PlanContext {
    let mut c = PlanContext::load(&DataPaths::at("/hr"));
    c.global = GlobalSettings::from_defaults(global.as_object().cloned().unwrap_or_default());
    c
}

fn settings(v: Value) -> GameSettings {
    GameSettings::from_map(v.as_object().unwrap(), "/home/u")
}

fn sideload(name: &str, exe: &str, platform: &str) -> GameInfo {
    GameInfo {
        runner: "sideload".into(),
        app_name: name.into(),
        title: format!("Title of {name}"),
        install: InstallInfo { executable: Some(exe.into()), platform: Some(platform.into()), ..Default::default() },
        ..Default::default()
    }
}

fn meta(kind: &str) -> LocalGameMeta {
    LocalGameMeta { app_name: "g".into(), runner: "sideload".into(), launch_kind: kind.into(), title: "Game".into(), ..Default::default() }
}

#[cfg(unix)]
fn proton_probe() -> FakeProbe {
    FakeProbe::new("/home/u").with_path("/p/GE/proton").with_path(UMU).with_path("/games/win/game.exe").with_path("/pfx")
}

fn proton_settings() -> GameSettings {
    settings(json!({
        "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
        "winePrefix": "/pfx",
        "enviromentOptions": [{"key": "PROTON_PREFER_SDL", "value": "1"}],
        "launcherArgs": "-windowed -w 1920"
    }))
}

// Linux games run only on Linux.
#[cfg(unix)]
#[test]
fn linux_script_runs_native_with_wrappers_and_env() {
    let probe = FakeProbe::new("/home/u").with_file("/g/run.sh", b"#!/bin/sh\n").with_bin("gamemoderun", "/usr/bin/gamemoderun");
    let s = settings(json!({"useGameMode": true, "launcherArgs": "--fast \"a b\"", "nvidiaPrime": true}));
    let game = sideload("g", "/g/run.sh", "linux");
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &probe,
        &game,
        Some(&meta("executable")),
        &s,
        &LaunchOptions { extra_args: vec!["--extra".into()], ..Default::default() },
    );
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!(p.command.program, "/usr/bin/gamemoderun");
    assert_eq!(p.command.args, ["/g/run.sh", "--fast", "a b", "--extra"]);
    assert_eq!(p.command.cwd.as_deref(), Some("/g"));
    assert_eq!(p.command.env["HEROIC_APP_NAME"], "g");
    assert_eq!(p.command.env["DRI_PRIME"], "1");
    assert_eq!(p.stop.kill_patterns, ["run.sh"]);
    assert_eq!(p.make_executable, None);
    let locked = FakeProbe::new("/home/u").with_non_executable("/g/run.sh").with_file("/g/run.sh", b"#!/bin/sh\n");
    let m = LocalGameMeta { run_as: Some("linux".into()), ..meta("executable") };
    let LaunchPlan::Native(p) =
        build_plan_with_settings(&ctx(json!({})), &locked, &game, Some(&m), &settings(json!({})), &Default::default())
    else {
        panic!()
    };
    assert_eq!(p.make_executable.as_deref(), Some("/g/run.sh"));
}

// Linux games run only on Linux.
#[cfg(unix)]
#[test]
fn native_missing_executable_and_missing_tool_are_unavailable() {
    let game = sideload("g", "/g/run.sh", "linux");
    let m = LocalGameMeta { run_as: Some("linux".into()), ..meta("executable") };
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Executable not found: /g/run.sh".into()));

    let probe = FakeProbe::new("/h").with_file("/g/run.sh", b"#!/bin/sh");
    let plan =
        build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&m), &settings(json!({"useGameMode": true})), &Default::default());
    assert!(plan.unavailable_reason().unwrap().contains("gamemoderun"));

    let none = sideload("g", "", "Windows");
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &FakeProbe::new("/h"),
        &none,
        Some(&meta("unavailable")),
        &settings(json!({})),
        &Default::default(),
    );
    assert_eq!(plan, LaunchPlan::Unavailable("No executable is configured for this game".into()));
}

// Linux games run only on Linux.
#[cfg(unix)]
#[test]
fn a_linux_game_started_by_a_command_runs_it_in_its_folder() {
    let mut game = sideload("g", "", "linux");
    game.install.install_path = Some("/games/gow3".into());
    game.rest.insert(heronite_data::sideload::COMMAND_KEY.into(), json!("env DXVK_HUD=1 ./gow3 --fullscreen"));
    let m = meta("executable");
    let none = FakeProbe::new("/h");
    let plan = build_plan_with_settings(&ctx(json!({})), &none, &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Game folder not found: /games/gow3".into()));

    let probe = FakeProbe::new("/h").with_file("/games/gow3", b"").with_file("/games/gow3/gow3", b"\x7fELF");
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &probe,
        &game,
        Some(&m),
        &settings(json!({"launcherArgs": "-windowed"})),
        &Default::default(),
    );
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!(p.command.program, "/bin/sh");
    assert_eq!(p.command.args, ["-c", "env DXVK_HUD=1 ./gow3 --fullscreen \"$@\"", "sh", "-windowed"]);
    assert_eq!(p.command.cwd.as_deref(), Some("/games/gow3"));
    assert_eq!(p.stop.kill_patterns, ["gow3"]);

    // An alternative executable in the settings still wins.
    let probe = probe.with_file("/games/gow3/other", b"#!/bin/sh");
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &probe,
        &game,
        Some(&LocalGameMeta { run_as: Some("linux".into()), ..meta("executable") }),
        &settings(json!({"targetExe": "/games/gow3/other"})),
        &Default::default(),
    );
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!(p.command.args, Vec::<String>::new());
    assert!(p.command.program.ends_with("other"));
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn windows_exe_uses_umu_with_proton_path() {
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &proton_probe(),
        &game,
        Some(&meta("executable")),
        &proton_settings(),
        &Default::default(),
    );
    let LaunchPlan::Wine { process, runtime, prefix } = plan else { panic!("{plan:?}") };
    assert_eq!(runtime, WineRuntime::Umu);
    assert_eq!(prefix, "/pfx");
    let c = &process.command;
    assert_eq!(c.program, UMU);
    assert_eq!(c.args, ["/games/win/game.exe", "-windowed", "-w", "1920"]);
    assert_eq!(c.cwd.as_deref(), Some("/games/win"));
    assert_eq!(c.env["PROTONPATH"], "/p/GE");
    assert_eq!(c.env["WINEPREFIX"], "/pfx");
    assert_eq!(c.env["GAMEID"], "umu-0");
    assert_eq!(c.env["PROTON_VERB"], "waitforexitandrun");
    assert_eq!(c.env["PROTON_PREFER_SDL"], "1");
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn heronite_switches_reach_a_umu_launch() {
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let mut c = ctx(json!({}));
    c.umu_database = vec![crate::umu_ids::Entry { title: "Game".into(), umu_id: "umu-55".into(), ..Default::default() }];
    let probe = proton_probe().with_bin("systemd-inhibit", "/usr/bin/systemd-inhibit");
    let s = settings(json!({
        "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
        "winePrefix": "/pfx", "heronitePreferSdl": true, "heroniteKeepAwake": true
    }));
    let m = LocalGameMeta { steam_app_id: Some("55".into()), ..meta("executable") };
    let plan = build_plan_with_settings(&c, &probe, &game, Some(&m), &s, &Default::default());
    let LaunchPlan::Wine { process, .. } = plan else { panic!("{plan:?}") };
    assert_eq!(process.command.env["GAMEID"], "umu-55", "the metadata's Steam id is in the database");
    assert_eq!(process.command.env["PROTON_PREFER_SDL"], "1");
    assert_eq!(process.keep_awake.as_ref().unwrap().program, "/usr/bin/systemd-inhibit");
    // The game's own id wins; without the switches nothing is added.
    let mine = settings(json!({
        "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
        "winePrefix": "/pfx", "heroniteUmuId": "1091500", "heroniteKeepAwake": false
    }));
    let plan = build_plan_with_settings(&c, &probe, &game, Some(&m), &mine, &Default::default());
    let LaunchPlan::Wine { process, .. } = plan else { panic!("{plan:?}") };
    assert_eq!(process.command.env["GAMEID"], "umu-1091500");
    assert!(!process.command.env.contains_key("PROTON_PREFER_SDL"));
    assert!(process.keep_awake.is_none(), "turned off");
    // Never set: on by default.
    let unset = settings(json!({"wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"}, "winePrefix": "/pfx"}));
    let LaunchPlan::Wine { process, .. } = build_plan_with_settings(&c, &probe, &game, Some(&m), &unset, &Default::default()) else {
        panic!()
    };
    assert!(process.keep_awake.is_some());
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn missing_prefix_files_trigger_prefix_creation() {
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let probe = proton_probe();
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&meta("executable")), &proton_settings(), &Default::default());
    let LaunchPlan::Wine { process, .. } = plan else { panic!() };
    // proton_probe has /pfx but none of the files proton needs.
    assert_eq!(process.create_dirs, ["/pfx"]);
    assert_eq!(process.pre_commands.len(), 1);
    assert_eq!(process.pre_commands[0].args, ["createprefix"]);
    assert_eq!(process.pre_commands[0].env["PROTON_VERB"], "run");
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn disabled_umu_runs_proton_directly() {
    let probe = FakeProbe::new("/home/u").with_path("/p/GE/proton").with_path("/games/win/game.exe");
    let game = sideload("g", "/games/win/game.exe", "Windows");
    // umu is wanted but not installed: the launch downloads it (Heroic
    // does too) instead of silently running Proton without the runtime.
    let LaunchPlan::Wine { process, runtime, .. } =
        build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&meta("executable")), &proton_settings(), &Default::default())
    else {
        panic!()
    };
    assert_eq!(runtime, WineRuntime::Umu);
    assert_eq!(process.command.program, UMU);
    assert!(process.prep.umu);
    assert!(process.notes.iter().any(|n| n.contains("umu-run")));
    assert_eq!(process.prep.wine.as_ref().unwrap().wine_prefix, "/pfx");
    // With umu in place there is nothing to download.
    let LaunchPlan::Wine { process, .. } = build_plan_with_settings(
        &ctx(json!({})),
        &proton_probe(),
        &game,
        Some(&meta("executable")),
        &proton_settings(),
        &Default::default(),
    ) else {
        panic!()
    };
    assert!(!process.prep.umu);
    // disableUMU runs Proton directly.
    let probe2 = proton_probe();
    let mut s = proton_settings();
    s.disable_umu = Some(true);
    let LaunchPlan::Wine { runtime, .. } =
        build_plan_with_settings(&ctx(json!({})), &probe2, &game, Some(&meta("executable")), &s, &Default::default())
    else {
        panic!()
    };
    assert_eq!(runtime, WineRuntime::Proton);
    let LaunchPlan::Wine { process, .. } =
        build_plan_with_settings(&ctx(json!({})), &probe2, &game, Some(&meta("executable")), &s, &Default::default())
    else {
        panic!()
    };
    assert_eq!(process.command.program, "/p/GE/proton");
    assert_eq!(process.command.args[..2], ["waitforexitandrun", "/games/win/game.exe"]);
    assert!(!process.prep.umu);
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn plain_wine_waits_for_wineserver() {
    let probe = FakeProbe::new("/home/u").with_path("/w/bin/wine").with_path("/w/bin/wineserver").with_path("/games/win/game.exe");
    let s = settings(json!({"wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"}, "winePrefix": "/pfx"}));
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let LaunchPlan::Wine { process, runtime, .. } = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &s, &Default::default())
    else {
        panic!()
    };
    assert_eq!(runtime, WineRuntime::Wine);
    assert_eq!(process.command.program, "/w/bin/wine");
    assert_eq!(process.command.args, ["/games/win/game.exe"]);
    assert_eq!(process.wait_after_exit.as_ref().unwrap().program, "/w/bin/wineserver");
    assert_eq!(process.wait_after_exit.as_ref().unwrap().args, ["--wait"]);
    assert_eq!(process.stop.commands[0].program, "/w/bin/wineserver");
    assert_eq!(process.stop.commands[0].args, ["-k"]);
    assert_eq!(process.pre_commands[0].args, ["wineboot", "--init"]);
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn invalid_game_wine_falls_back_to_global_default() {
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let s = settings(json!({"wineVersion": {"bin": "", "name": "Default Wine - Not Found", "type": "wine"}, "winePrefix": "/pfx"}));
    let global = json!({"wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"}});
    let LaunchPlan::Wine { process, runtime, .. } =
        build_plan_with_settings(&ctx(global), &proton_probe(), &game, None, &s, &Default::default())
    else {
        panic!()
    };
    assert_eq!(runtime, WineRuntime::Umu);
    assert!(process.notes[0].contains("Default Wine - Not Found"));
    let none = build_plan_with_settings(&ctx(json!({})), &proton_probe(), &game, None, &s, &Default::default());
    assert!(none.unavailable_reason().unwrap().contains("not valid"));
}

// Wine and Proton: Linux only (Windows runs a Windows game as it is).
#[cfg(unix)]
#[test]
fn run_as_windows_sends_an_elf_through_wine() {
    let probe = proton_probe().with_file("/games/win/game.exe", b"\x7fELF");
    let game = sideload("g", "/games/win/game.exe", "linux");
    let m = LocalGameMeta { run_as: Some("windows".into()), ..meta("executable") };
    assert_eq!(build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&m), &proton_settings(), &Default::default()).kind(), "wine");
    let linux = LocalGameMeta { run_as: Some("linux".into()), ..meta("executable") };
    assert_eq!(
        build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&linux), &proton_settings(), &Default::default()).kind(),
        "native"
    );
}

#[test]
fn steam_games_become_uris_when_steam_exists() {
    let m = LocalGameMeta { steam_app_id: Some("400".into()), ..meta("steam-uri") };
    let game = sideload("steam_400", "/usr/bin/steam", "linux");
    // Linux finds `steam` on the PATH, Windows `steam.exe` in Steam's folder.
    let with_steam = if cfg!(windows) {
        let exe = crate::steam::platform_roots(std::path::Path::new("/h"))[0].join("steam.exe");
        FakeProbe::new("/h").with_path(&exe.to_string_lossy())
    } else {
        FakeProbe::new("/h").with_bin("steam", "/usr/bin/steam")
    };
    assert_eq!(
        build_plan_with_settings(&ctx(json!({})), &with_steam, &game, Some(&m), &settings(json!({})), &Default::default()),
        LaunchPlan::SteamUri { app_id: "400".into(), uri: "steam://rungameid/400".into() }
    );
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Steam client was not found on PATH".into()));
}

#[test]
fn another_launcher_starts_its_games_and_the_session_can_follow_them() {
    let mut m = LocalGameMeta {
        source: "lutris".into(),
        launcher_args: Some("lutris lutris:rungameid/12".into()),
        ..meta(heronite_data::launchers::LAUNCH_KIND)
    };
    m.extra.insert(heronite_data::launchers::PREFIX_KEY.into(), json!("/pfx/celeste"));
    let mut game = sideload("lutris_12", "", "windows");
    game.install.install_path = Some("/games/celeste".into());
    let probe = FakeProbe::new("/h").with_bin("lutris", "/usr/bin/lutris").with_path("/games/celeste");
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&m), &settings(json!({})), &Default::default());
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!((p.command.program.as_str(), p.command.args.as_slice()), ("lutris", &["lutris:rungameid/12".to_string()][..]));
    assert_eq!(p.command.cwd.as_deref(), Some("/games/celeste"));
    let handoff = p.handoff.unwrap();
    assert_eq!((handoff.prefix.as_deref(), handoff.folder.as_deref()), (Some("/pfx/celeste"), Some("/games/celeste")));
    assert!(handoff.client.is_none(), "a launcher's game is followed by its processes, not by a client's");
    // Without Lutris on this PC.
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Lutris was not found on this PC".into()));
}

/// Bottles names the bottle and the program, spaces and quotes included.
#[test]
fn bottles_gets_the_bottle_and_program_whole() {
    let m = LocalGameMeta {
        source: "bottles".into(),
        launcher_args: Some(r"flatpak run --command=bottles-cli com.usebottles.bottles run -b 'My Games' -p 'Diablo'\''s Hellfire'".into()),
        ..meta(heronite_data::launchers::LAUNCH_KIND)
    };
    let game = sideload("bottles_d1", "", "windows");
    let probe = FakeProbe::new("/h").with_bin("flatpak", "/usr/bin/flatpak");
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, Some(&m), &settings(json!({})), &Default::default());
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!(p.command.program, "flatpak");
    assert_eq!(
        p.command.args,
        ["run", "--command=bottles-cli", "com.usebottles.bottles", "run", "-b", "My Games", "-p", "Diablo's Hellfire"]
    );
    assert!(p.handoff.is_none(), "nothing to find the game by");
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Bottles was not found on this PC".into()));
}

fn emu_ctx(emu: UserEmulator) -> PlanContext {
    let mut c = ctx(json!({}));
    c.emulators.emulators.push(emu);
    c
}

fn profile(id: &str, args: &str) -> UserEmulatorProfile {
    UserEmulatorProfile { id: id.into(), definition_profile_id: Some(id.into()), arguments: args.into(), ..Default::default() }
}

fn emu_meta(roms: &[&str]) -> LocalGameMeta {
    LocalGameMeta {
        emulator_id: Some("e1".into()),
        emulator_profile_id: Some("custom".into()),
        roms: Some(roms.iter().map(|r| LocalGameRom { name: None, path: r.to_string(), extra: Default::default() }).collect()),
        ..meta("emulator")
    }
}

#[test]
fn native_emulator_gets_its_rom_and_stop_pattern() {
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "custom".into(),
        name: "PCSX2".into(),
        executable: "/emu/pcsx2.AppImage".into(),
        launch_kind: "native".into(),
        profiles: vec![profile("custom", "{ImagePath}")],
        ..Default::default()
    };
    let probe = FakeProbe::new("/h").with_path("/emu/pcsx2.AppImage").with_path("/roms/a.iso").with_path("/roms/b.iso");
    let m = LocalGameMeta { selected_rom_path: Some("/roms/b.iso".into()), ..emu_meta(&["/roms/a.iso", "/roms/b.iso"]) };
    let game = sideload("g", "/emu/pcsx2.AppImage", "linux");
    let plan = build_plan_with_settings(&emu_ctx(emu), &probe, &game, Some(&m), &settings(json!({})), &Default::default());
    let LaunchPlan::Emulator { process, emulator, rom } = plan else { panic!("{plan:?}") };
    assert_eq!(emulator, "PCSX2");
    assert_eq!(rom.as_deref(), Some("/roms/b.iso"));
    assert_eq!(process.command.program, "/emu/pcsx2.AppImage");
    assert_eq!(process.command.args, ["/roms/b.iso"]);
    assert_eq!(process.command.cwd.as_deref(), Some("/emu"));
    assert!(process.command.env.is_empty());
    assert_eq!(process.stop.kill_patterns, ["pcsx2.AppImage"]);
}

#[test]
fn open_emulator_runs_it_without_a_game() {
    let native = UserEmulator { executable: "/emu/pcsx2.AppImage".into(), launch_kind: "native".into(), ..Default::default() };
    let probe = FakeProbe::new("/h").with_path("/emu/pcsx2.AppImage");
    let c = open_emulator_command(&ctx(json!({})), &probe, &settings(json!({})), &native).unwrap();
    assert_eq!(c.program, "/emu/pcsx2.AppImage");
    assert!(c.args.is_empty());
    assert_eq!(c.cwd.as_deref(), Some("/emu"));

    let flatpak = UserEmulator { launch_kind: "flatpak".into(), flatpak_id: Some("net.rpcs3.RPCS3".into()), ..Default::default() };
    let c = open_emulator_command(&ctx(json!({})), &probe, &settings(json!({})), &flatpak);
    if cfg!(windows) {
        assert_eq!(c.unwrap_err(), "Flatpak emulators do not run on Windows");
    } else {
        let c = c.unwrap();
        assert_eq!(c.program, "flatpak");
        assert_eq!(c.args, ["run", "net.rpcs3.RPCS3"]);
    }

    let gone = UserEmulator { executable: "/emu/gone".into(), launch_kind: "native".into(), ..Default::default() };
    assert!(open_emulator_command(&ctx(json!({})), &probe, &settings(json!({})), &gone).is_err());
}

#[test]
fn save_state_args_go_first_and_can_replace_the_image() {
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "custom".into(),
        executable: "/emu/x".into(),
        launch_kind: "native".into(),
        profiles: vec![profile("custom", "-batch {ImagePath}")],
        ..Default::default()
    };
    let probe = FakeProbe::new("/h").with_path("/emu/x").with_path("/roms/a.iso");
    let m = emu_meta(&["/roms/a.iso"]);
    let game = sideload("g", "/emu/x", "linux");
    let args_for = |opts: LaunchOptions| {
        let LaunchPlan::Emulator { process, .. } =
            build_plan_with_settings(&emu_ctx(emu.clone()), &probe, &game, Some(&m), &settings(json!({})), &opts)
        else {
            panic!()
        };
        process.command.args
    };
    let with_state = args_for(LaunchOptions { state_args: vec!["-statefile".into(), "/s/1.state".into()], ..Default::default() });
    assert_eq!(with_state, ["-statefile", "/s/1.state", "-batch", "/roms/a.iso"]);
    let replaced = args_for(LaunchOptions {
        state_args: vec!["--load".into(), "/s/1.state".into()],
        state_replaces_image: true,
        ..Default::default()
    });
    assert_eq!(replaced, ["--load", "/s/1.state", "-batch"]);
    assert_eq!(args_for(Default::default()), ["-batch", "/roms/a.iso"]);
}

#[test]
fn requested_rom_wins_and_missing_roms_are_unavailable() {
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "custom".into(),
        executable: "/emu/x".into(),
        launch_kind: "native".into(),
        profiles: vec![profile("custom", "{ImagePath}")],
        ..Default::default()
    };
    let probe = FakeProbe::new("/h").with_path("/emu/x").with_path("/roms/a.iso").with_path("/roms/b.iso");
    let m = emu_meta(&["/roms/a.iso", "/roms/b.iso"]);
    let game = sideload("g", "/emu/x", "linux");
    let opts = LaunchOptions { extra_args: vec!["/roms/b.iso".into()], ..Default::default() };
    let LaunchPlan::Emulator { process, .. } =
        build_plan_with_settings(&emu_ctx(emu.clone()), &probe, &game, Some(&m), &settings(json!({})), &opts)
    else {
        panic!()
    };
    assert_eq!(process.command.args, ["/roms/b.iso"]);

    let gone = emu_meta(&["/roms/zzz.iso"]);
    let plan = build_plan_with_settings(&emu_ctx(emu.clone()), &probe, &game, Some(&gone), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("ROM file was not found".into()));

    let plan = build_plan_with_settings(&emu_ctx(emu), &FakeProbe::new("/h"), &game, Some(&m), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Emulator executable was not found".into()));
}

// Flatpak is Linux's.
#[cfg(unix)]
#[test]
fn retroarch_core_in_flatpak() {
    let mut p = profile("snes", "");
    p.core_path = Some("/cores/snes9x_libretro.so".into());
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "retroarch".into(),
        name: "RetroArch".into(),
        launch_kind: "flatpak".into(),
        flatpak_id: Some("org.libretro.RetroArch".into()),
        profiles: vec![p],
        ..Default::default()
    };
    let probe = FakeProbe::new("/h").with_path("/roms/zelda.sfc");
    let m = LocalGameMeta { emulator_profile_id: Some("snes".into()), ..emu_meta(&["/roms/zelda.sfc"]) };
    let game = sideload("g", "flatpak", "linux");
    let LaunchPlan::Emulator { process, .. } =
        build_plan_with_settings(&emu_ctx(emu), &probe, &game, Some(&m), &settings(json!({})), &Default::default())
    else {
        panic!()
    };
    assert_eq!(process.command.program, "flatpak");
    assert_eq!(process.command.args, ["run", "org.libretro.RetroArch", "-L", "/cores/snes9x_libretro.so", "/roms/zelda.sfc"]);
    assert_eq!(process.command.cwd, None);
    assert_eq!(process.stop.kill_patterns, ["retroarch", "RetroArch"]);
}

#[test]
fn mame_runs_from_the_rom_folder() {
    let game = sideload("g", "/usr/bin/mame", "linux");
    let mut mame_profile = profile("arcade", "");
    mame_profile.definition_profile_id = Some("arcade".into());
    let mame = UserEmulator {
        id: "e1".into(),
        definition_id: "mame".into(),
        executable: "/usr/bin/mame".into(),
        launch_kind: "native".into(),
        profiles: vec![mame_profile],
        ..Default::default()
    };
    let probe = FakeProbe::new("/h").with_path("/usr/bin/mame").with_path("/roms/arcade/pacman.zip");
    let m = LocalGameMeta { emulator_profile_id: Some("arcade".into()), ..emu_meta(&["/roms/arcade/pacman.zip"]) };
    let LaunchPlan::Emulator { process, .. } =
        build_plan_with_settings(&emu_ctx(mame), &probe, &game, Some(&m), &settings(json!({})), &Default::default())
    else {
        panic!()
    };
    assert_eq!(process.command.args, ["pacman"]);
    assert_eq!(process.command.cwd.as_deref(), Some("/roms/arcade"));
}

// Wine runs Windows emulators on Linux only.
#[cfg(unix)]
#[test]
fn exe_emulators_run_through_wine() {
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "custom".into(),
        executable: "/games/win/game.exe".into(),
        launch_kind: "wine".into(),
        profiles: vec![profile("custom", "-fullscreen {ImagePath}")],
        ..Default::default()
    };
    let probe = proton_probe().with_path("/roms/a.iso");
    let m = emu_meta(&["/roms/a.iso"]);
    let game = sideload("g", "/games/win/game.exe", "Windows");
    let LaunchPlan::Wine { process, .. } =
        build_plan_with_settings(&emu_ctx(emu), &probe, &game, Some(&m), &proton_settings(), &Default::default())
    else {
        panic!()
    };
    assert_eq!(process.command.args, ["/games/win/game.exe", "-fullscreen", "/roms/a.iso"]);
}

#[test]
fn browser_and_unknown_runners_are_unavailable() {
    let mut browser = sideload("b", "", "Browser");
    browser.install.platform = Some("Browser".into());
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &browser, None, &settings(json!({})), &Default::default());
    assert!(plan.unavailable_reason().unwrap().contains("Browser"));
    let zoom = GameInfo { runner: "zoom".into(), app_name: "x".into(), ..Default::default() };
    let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &zoom, None, &settings(json!({})), &Default::default());
    assert!(plan.unavailable_reason().unwrap().contains("zoom"));
    // Store games that are not installed say so.
    for runner in ["gog", "nile"] {
        let game = GameInfo { runner: runner.into(), app_name: "x".into(), ..Default::default() };
        let plan = build_plan_with_settings(&ctx(json!({})), &FakeProbe::new("/h"), &game, None, &settings(json!({})), &Default::default());
        assert!(plan.unavailable_reason().unwrap().contains("not installed"), "{runner}");
    }
}

fn store_game(runner: &str, app: &str, path: &str, platform: &str) -> GameInfo {
    GameInfo {
        runner: runner.into(),
        app_name: app.into(),
        title: "Store Game".into(),
        is_installed: true,
        install: InstallInfo { install_path: Some(path.into()), platform: Some(platform.into()), ..Default::default() },
        ..Default::default()
    }
}

// Wine and Linux builds: Linux (GOG on Windows is in `crate::gog`).
#[cfg(unix)]
#[test]
fn gog_windows_games_run_their_primary_task_in_wine() {
    let info = br#"{"playTasks": [{"category": "game", "isPrimary": true, "type": "FileTask", "path": "bin\\game.exe", "arguments": "-x", "workingDir": "bin"}]}"#;
    let probe = proton_probe().with_path("/games/gog").with_path("/games/gog/bin/game.exe").with_file("/games/gog/goggame-42.info", info);
    let game = store_game("gog", "42", "/games/gog", "windows");
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &proton_settings(), &Default::default());
    let LaunchPlan::Wine { process, .. } = plan else { panic!("{plan:?}") };
    let args = &process.command.args;
    assert!(args.iter().any(|a| a == "/games/gog/bin/game.exe"), "{args:?}");
    assert!(args.ends_with(&["-x".to_string(), "-windowed".into(), "-w".into(), "1920".into()]), "{args:?}");
    assert_eq!(process.command.cwd.as_deref(), Some("/games/gog/bin"));
    assert_eq!(process.command.env["HEROIC_APP_SOURCE"], "gog");
    assert_eq!(process.prep.cloud_saves, None);
    let first = process.prep.first_launch.expect("first launch setup");
    assert_eq!((first.runner.as_str(), first.app_name.as_str()), ("gog", "42"));
    let mut sync = proton_settings();
    sync.auto_sync_saves = true;
    let LaunchPlan::Wine { process, .. } = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &sync, &Default::default())
    else {
        panic!()
    };
    assert_eq!(process.prep.cloud_saves, Some(CloudSaves::new("gog", "")));

    // Without the info file there is nothing to run.
    let bare = proton_probe().with_path("/games/gog");
    let plan = build_plan_with_settings(&ctx(json!({})), &bare, &game, None, &proton_settings(), &Default::default());
    assert!(plan.unavailable_reason().unwrap().contains("goggame-42.info"), "{plan:?}");
}

// Wine and Linux builds: Linux (GOG on Windows is in `crate::gog`).
#[cfg(unix)]
#[test]
fn gog_linux_builds_start_their_script() {
    let probe = FakeProbe::new("/home/u").with_path("/games/l").with_file("/games/l/start.sh", b"#!/bin/sh\n");
    let game = store_game("gog", "7", "/games/l", "linux");
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &settings(json!({})), &Default::default());
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!(p.command.program, "/games/l/start.sh");
    assert_eq!(p.command.env["HEROIC_APP_RUNNER"], "gog");
}

#[cfg(windows)]
#[test]
fn amazon_games_run_as_they_are_on_windows() {
    let fuel = br#"{"Main": {"Command": "Game.exe", "Args": ["-a"]}, "PostInstall": [{"Command": "vc.exe", "Args": ["/q"]}]}"#;
    let probe = FakeProbe::new(r"C:\Users\u")
        .with_path(r"D:\amz")
        .with_path(r"D:\amz\Game.exe")
        .with_path(r"D:\amz\vc.exe")
        .with_file(r"D:\amz\fuel.json", fuel);
    let game = store_game("nile", "amzn1.p", r"D:\amz", "Windows");
    // Settings brought from Linux (Proton) are left aside.
    let s = settings(json!({"wineVersion": {"bin": "/p/GE/proton", "type": "proton"}, "winePrefix": "/pfx"}));
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &s, &Default::default());
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!((p.command.program.as_str(), p.command.args.as_slice()), (r"D:\amz\Game.exe", &["-a".to_string()][..]));
    assert_eq!(p.command.cwd.as_deref(), Some(r"D:\amz"));
    assert_eq!(p.prep.first_launch.expect("first launch setup").post_install[0].parts, [r"D:\amz\vc.exe", "/q"]);
}

// Through Wine: Linux (Amazon on Windows is in `crate::nile`).
#[cfg(unix)]
#[test]
fn amazon_games_run_their_fuel_command_with_the_sdk_variables() {
    let fuel = br#"{"Main": {"Command": "Game.exe", "Args": ["-a"], "WorkingSubdirOverride": ""}, "PostInstall": [{"Command": "vc.exe", "Args": ["/q"]}]}"#;
    let probe = proton_probe()
        .with_path("/games/amz")
        .with_path("/games/amz/Game.exe")
        .with_path("/games/amz/vc.exe")
        .with_file("/games/amz/fuel.json", fuel)
        .with_file("/hr/nile_config/nile/library.json", br#"[{"id": "ent-1", "product": {"id": "amzn1.p", "sku": "SKU9"}}]"#)
        .with_file("/hr/nile_config/nile/current_user.json", br#"{"name": "Ana", "user_id": "u"}"#);
    let game = store_game("nile", "amzn1.p", "/games/amz", "Windows");
    let plan = build_plan_with_settings(&ctx(json!({})), &probe, &game, None, &proton_settings(), &Default::default());
    let LaunchPlan::Wine { process, .. } = plan else { panic!("{plan:?}") };
    assert!(process.command.args.iter().any(|a| a == "/games/amz/Game.exe"));
    assert_eq!(process.command.cwd.as_deref(), Some("/games/amz"));
    let env = &process.command.env;
    assert_eq!(env["AMAZON_GAMES_FUEL_ENTITLEMENT_ID"], "ent-1");
    assert_eq!(env["AMAZON_GAMES_FUEL_PRODUCT_SKU"], "SKU9");
    assert_eq!(env["AMAZON_GAMES_FUEL_DISPLAY_NAME"], "Ana");
    assert_eq!(env["HEROIC_APP_SOURCE"], "amazon");
    let first = process.prep.first_launch.expect("first launch setup");
    assert_eq!(first.runner, "nile");
    assert_eq!(first.post_install[0].parts, ["/games/amz/vc.exe", "/q"]);
}

#[cfg(windows)]
#[test]
fn a_windows_program_runs_as_it_is_on_windows() {
    let exe = r"C:\Games\Hades\Hades.exe";
    let probe = FakeProbe::new(r"C:\Users\u").with_path(exe);
    // Settings brought from Linux (Proton, MangoHud, the sandbox) are left aside.
    let s = settings(
        json!({"wineVersion": {"bin": "/p/GE/proton", "type": "proton"}, "showMangohud": true, "heroniteSandbox": {"enabled": true}, "launcherArgs": "-skipintro"}),
    );
    let plan = build_plan_with_settings(
        &ctx(json!({})),
        &probe,
        &sideload("g", exe, "Windows"),
        Some(&meta("executable")),
        &s,
        &Default::default(),
    );
    let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
    assert_eq!((p.command.program.as_str(), p.command.args.as_slice()), (exe, &["-skipintro".to_string()][..]));
    assert_eq!(p.command.cwd.as_deref(), Some(r"C:\Games\Hades"));
    assert_eq!(p.stop.kill_patterns, ["Hades.exe"]);
    assert_eq!(p.command.env.get("HEROIC_APP_NAME").map(String::as_str), Some("g"));

    let linux = build_plan_with_settings(
        &ctx(json!({})),
        &probe.with_file(r"C:\Games\x\run.sh", b"#!/bin/sh"),
        &sideload("g", r"C:\Games\x\run.sh", "linux"),
        Some(&meta("executable")),
        &settings(json!({})),
        &Default::default(),
    );
    assert!(matches!(linux, LaunchPlan::Unavailable(_)), "{linux:?}");
    let missing = build_plan_with_settings(
        &ctx(json!({})),
        &FakeProbe::new(r"C:\Users\u"),
        &sideload("g", exe, "Windows"),
        Some(&meta("executable")),
        &settings(json!({})),
        &Default::default(),
    );
    assert!(matches!(missing, LaunchPlan::Unavailable(_)), "{missing:?}");
}

#[cfg(windows)]
#[test]
fn a_windows_game_holds_the_computer_awake_unless_turned_off() {
    let exe = r"C:\Games\Hades\Hades.exe";
    let probe = FakeProbe::new(r"C:\Users\u").with_path(exe);
    let plan = |s: Value| {
        let plan = build_plan_with_settings(
            &ctx(json!({})),
            &probe,
            &sideload("g", exe, "Windows"),
            Some(&meta("executable")),
            &settings(s),
            &Default::default(),
        );
        let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
        p
    };
    // On unless turned off, with no tool to find: the runtime holds it itself.
    let unset = plan(json!({}));
    assert!(unset.hold_awake && unset.keep_awake.is_none() && unset.notes.is_empty());
    assert!(plan(json!({"heroniteKeepAwake": true})).hold_awake);
    assert!(!plan(json!({"heroniteKeepAwake": false})).hold_awake);
}

#[cfg(windows)]
#[test]
fn a_windows_emulator_runs_as_it_is_on_windows() {
    let exe = r"C:\Emulators\PCSX2\pcsx2-qt.exe";
    let mut p = profile("ps2", "-fullscreen -- {ImagePath}");
    p.definition_profile_id = Some("ps2".into());
    // A Windows program set up on Linux (`wine`) runs the same way.
    for kind in ["native", "wine"] {
        let emu = UserEmulator {
            id: "e1".into(),
            definition_id: "pcsx2".into(),
            name: "PCSX2".into(),
            executable: exe.into(),
            launch_kind: kind.into(),
            profiles: vec![p.clone()],
            ..Default::default()
        };
        let rom = r"D:\ROMs\ps2\Okami (USA).iso";
        let probe = FakeProbe::new(r"C:\Users\u").with_path(exe).with_path(rom);
        let m = LocalGameMeta { emulator_profile_id: Some("ps2".into()), ..emu_meta(&[rom]) };
        let opts = LaunchOptions { state_args: vec!["-statefile".into(), r"C:\s\1.p2s".into()], ..Default::default() };
        let game = sideload("g", exe, "Windows");
        let plan = build_plan_with_settings(&emu_ctx(emu.clone()), &probe, &game, Some(&m), &proton_settings(), &opts);
        let LaunchPlan::Emulator { process, emulator, rom: picked } = plan else { panic!("{plan:?}") };
        assert_eq!(emulator, "PCSX2");
        assert_eq!(picked.as_deref(), Some(rom));
        assert_eq!(process.command.program, exe);
        assert_eq!(process.command.args, ["-statefile", r"C:\s\1.p2s", "-fullscreen", "--", rom]);
        assert_eq!(process.command.cwd.as_deref(), Some(r"C:\Emulators\PCSX2"));
        assert!(process.command.env.is_empty() && !process.sandboxed);
        assert_eq!(process.stop.kill_patterns, ["pcsx2"]);
        assert!(process.hold_awake, "an emulator keeps the computer awake too");

        let c = open_emulator_command(&ctx(json!({})), &probe, &settings(json!({})), &emu).unwrap();
        assert_eq!((c.program.as_str(), c.cwd.as_deref()), (exe, Some(r"C:\Emulators\PCSX2")));
    }
}

#[cfg(windows)]
#[test]
fn flatpak_emulators_do_not_run_on_windows() {
    let emu = UserEmulator {
        id: "e1".into(),
        definition_id: "retroarch".into(),
        launch_kind: "flatpak".into(),
        flatpak_id: Some("org.libretro.RetroArch".into()),
        profiles: vec![profile("custom", "{ImagePath}")],
        ..Default::default()
    };
    let rom = r"D:\ROMs\a.sfc";
    let probe = FakeProbe::new(r"C:\Users\u").with_path(rom);
    let game = sideload("g", "flatpak", "linux");
    let plan = build_plan_with_settings(&emu_ctx(emu), &probe, &game, Some(&emu_meta(&[rom])), &settings(json!({})), &Default::default());
    assert_eq!(plan, LaunchPlan::Unavailable("Flatpak emulators do not run on Windows".into()));
}
