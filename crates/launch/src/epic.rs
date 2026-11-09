//! Epic games (port of Heroic's `LegendaryGame.launch`): the game's own
//! program, through the same Wine / Proton / umu plan as GOG's games on
//! Linux and as it is on Windows. What only Epic can give (the sign-in, the
//! build's arguments, the update check, the link of an EA app or Ubisoft
//! Connect game) is asked for when the game starts ([`crate::epic_launch`]),
//! so building the plan uses no network.

use std::path::Path;

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::env::{join, setup_wrapper_env_vars, Env};
use crate::epic_launch::{EpicLaunch, StoreLink};
use crate::host::{run_mode, unavailable_reason, Os, RunMode};
use crate::plan::{CloudSaves, Command, FirstLaunch, Handoff, LaunchOptions, LaunchPlan, PlanContext, ProcessPlan, StopSpec, WineRuntime};
use crate::probe::Probe;
use crate::settings::GameSettings;
use crate::wine::{self, StoreGame};

/// Heroic's `known_fixes.json` env variables for a game of `store`
/// (`epic`, `gog`, `amazon`).
pub(crate) fn known_fixes_env(ctx: &PlanContext, store: &str, app_name: &str) -> Env {
    let mut env = Env::new();
    let fixes = ctx
        .known_fixes
        .as_ref()
        .and_then(|k| k.get(store))
        .and_then(|e| e.get(app_name))
        .and_then(|f| f.get("envVariables"))
        .and_then(Value::as_object);
    for (k, v) in fixes.into_iter().flatten() {
        let value = match v {
            Value::String(s) => s.clone(),
            Value::Null => continue,
            other => other.to_string(),
        };
        env.insert(k.clone(), value);
    }
    env
}

/// `known_fixes.json` entries that change the prefix: the winetricks verbs
/// and the programs to run in it (`installFixes`), for a game of `store`.
pub fn known_fixes_steps(ctx: &PlanContext, store: &str, app_name: &str) -> (Vec<String>, Vec<String>) {
    let entry = ctx.known_fixes.as_ref().and_then(|k| k.get(store)).and_then(|e| e.get(app_name));
    let list = |key: &str| -> Vec<String> {
        entry
            .and_then(|f| f.get(key))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect())
            .unwrap_or_default()
    };
    (list("winetricks"), list("runInPrefix"))
}

/// The umu id cached for `<runner>_<appName>`.
pub(crate) fn cached_umu_id(ctx: &PlanContext, runner: &str, app_name: &str) -> Option<String> {
    ctx.umu_ids.as_ref()?.get(format!("{runner}_{app_name}"))?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn flag(rest: &heronite_data::models::Extra, key: &str) -> bool {
    rest.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// What starts and the user's arguments: a playable DLC starts itself; a
/// basic launch option's arguments come before the user's own (the order
/// legendary put them in).
fn launched_and_args(app: &str, s: &GameSettings, opts: &LaunchOptions) -> (String, Vec<String>) {
    use crate::launch_option::LaunchOption;
    let (launched, option_args) = match &s.launch_option {
        Some(LaunchOption::Dlc { app_name, .. }) => (app_name.as_str(), String::new()),
        Some(LaunchOption::Basic { parameters, .. }) => (app, parameters.clone()),
        None => (app, String::new()),
    };
    let extra = Some(option_args)
        .into_iter()
        .chain(opts.extra_args.iter().cloned())
        .chain(Some(s.launcher_args.clone()))
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let args = if extra.is_empty() { Vec::new() } else { crate::args::split_windows_args(&extra) };
    (launched.to_string(), args)
}

/// The Epic part of the launch, done when the game starts.
fn epic_launch(ctx: &PlanContext, game: &GameInfo, s: &GameSettings, opts: &LaunchOptions, launched: String) -> EpicLaunch {
    let language = if !s.language.is_empty() { Some(s.language.clone()) } else { ctx.ui_language.clone() };
    let store_link = if flag(&game.rest, "isEAManaged") {
        Some(StoreLink::EaApp)
    } else if flag(&game.rest, "isUbisoftManaged") {
        Some(StoreLink::Ubisoft)
    } else {
        None
    };
    EpicLaunch {
        app_name: launched,
        game: game.app_name.clone(),
        exe_index: None,
        offline: s.offline_mode,
        check_update: !opts.skip_version_check,
        language,
        override_exe: override_exe(s, opts),
        store_link,
    }
}

/// Another program of the install: "Play with" of the launch options, else
/// the game's `targetExe`.
fn override_exe(s: &GameSettings, opts: &LaunchOptions) -> Option<String> {
    opts.alt_exe.clone().filter(|e| !e.is_empty()).or_else(|| Some(s.target_exe.clone()).filter(|e| !e.is_empty()))
}

/// The program that starts, as far as the plan knows it (Epic may name
/// another one for a launchable DLC): the override, else the installed
/// program, below the install folder unless it is a full path.
fn program_of(install_path: &str, game: &GameInfo, override_exe: Option<&str>) -> Option<String> {
    let rel = override_exe.or(game.install.executable.as_deref()).filter(|e| !e.is_empty())?;
    if Path::new(rel).is_absolute() || (cfg!(windows) && rel.contains(':')) {
        return Some(rel.to_string());
    }
    let rel = rel.replace('\\', "/");
    let rel = rel.trim_start_matches('/');
    Some(if cfg!(windows) {
        let mut path = std::path::PathBuf::from(install_path);
        path.extend(rel.split('/').filter(|p| !p.is_empty()));
        path.to_string_lossy().into_owned()
    } else {
        join(install_path, rel)
    })
}

/// A Windows build on Windows: no Wine, prefix, wrappers, winetricks or
/// known fixes. The game's program starts as it is; the first launch runs
/// its prerequisites ([`crate::epic_native_setup`]).
fn build_native(
    ctx: &PlanContext,
    game: &GameInfo,
    s: &GameSettings,
    opts: &LaunchOptions,
    install_path: &str,
    third_party: bool,
) -> LaunchPlan {
    let app = game.app_name.as_str();
    let mut env = setup_wrapper_env_vars(app, "legendary");
    env.extend(s.env_options.iter().cloned());
    let (launched, args) = launched_and_args(app, s, opts);
    let launch = epic_launch(ctx, game, s, opts, launched);
    // A store app game opens through its link: there is no program here.
    let program = if third_party { None } else { program_of(install_path, game, launch.override_exe.as_deref()) };
    let program = match program {
        Some(p) => p,
        None if third_party => String::new(),
        None => return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable = install_path)),
    };
    let mut prep = crate::plan::PrepSpec::default();
    if s.auto_sync_saves && !s.saves_path.trim().is_empty() {
        prep.cloud_saves = Some(CloudSaves::new("legendary", &s.saves_path));
    }
    prep.first_launch = Some(FirstLaunch {
        runner: "legendary".into(),
        app_name: app.to_string(),
        install_path: install_path.to_string(),
        platform: game.install.platform.clone().unwrap_or_default(),
        ea_managed: flag(&game.rest, "isEAManaged"),
        ubisoft_managed: flag(&game.rest, "isUbisoftManaged"),
        ..Default::default()
    });
    // A stop ends the game's job; a game that left it (started again through
    // another program) is found by its program's name.
    let exe = Some(crate::emulators::node_basename(&program)).filter(|n| !n.is_empty());
    let stop = StopSpec { kill_patterns: exe.into_iter().collect(), commands: Vec::new() };
    // A game that hands itself to another process at once is followed by the
    // programs running from its folder.
    let handoff =
        (!third_party && !install_path.is_empty()).then(|| Handoff { folder: Some(install_path.to_string()), ..Default::default() });
    let cwd = (!program.is_empty()).then(|| crate::emulators::node_dirname(&program));
    LaunchPlan::Native(ProcessPlan {
        command: Command { program, args, env, cwd },
        stop,
        prep,
        handoff,
        epic: Some(launch),
        ..Default::default()
    })
}

pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, opts: &LaunchOptions) -> LaunchPlan {
    let app = game.app_name.as_str();
    if !game.is_installed {
        return LaunchPlan::Unavailable(tr!("This Epic game is not installed").into());
    }
    let install_path = game.install.install_path.as_deref().unwrap_or("");
    // EA app / Ubisoft Connect games live inside the prefix, with their store
    // app, which opens them through their link.
    let third_party = game.rest.get("thirdPartyManagedApp").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if !third_party && (install_path.is_empty() || !probe.exists(install_path)) {
        return LaunchPlan::Unavailable(tr!("Install folder not found: {install_path}", install_path));
    }

    // A Windows build on Windows runs as it is; on Linux every Epic game runs
    // through Wine/Proton.
    let os = Os::from_name(game.install.platform.as_deref().unwrap_or("")).unwrap_or(Os::Windows);
    match run_mode(os) {
        RunMode::Native => return build_native(ctx, game, settings, opts, install_path, third_party),
        RunMode::Unavailable if Os::HOST == Os::Windows => return LaunchPlan::Unavailable(unavailable_reason(os)),
        _ => {}
    }

    let (launched, args) = launched_and_args(app, settings, opts);
    let mut launch = epic_launch(ctx, game, settings, opts, launched);
    // A store app game: Wine's `start` opens its link in the prefix (the
    // link and the sign-in in it come when the game starts).
    let (exe, args) = if third_party {
        ("start".to_string(), Vec::new())
    } else {
        match program_of(install_path, game, launch.override_exe.as_deref()) {
            Some(exe) => (exe, args),
            None => return LaunchPlan::Unavailable(tr!("Executable not found: {executable}", executable = install_path)),
        }
    };
    let store = StoreGame {
        install_path: Some(install_path).filter(|p| !third_party && !p.is_empty()),
        game_id: game.folder_name.as_deref().unwrap_or("0"),
    };
    let mut plan = wine::build_wine_plan_for(ctx, probe, game, settings, &exe, &args, &store);
    if let LaunchPlan::Wine { process, runtime, .. } = &mut plan {
        process.command.env.extend(known_fixes_env(ctx, "epic", app));
        if *runtime == WineRuntime::Umu {
            if let Some(id) = cached_umu_id(ctx, "legendary", app) {
                process.command.env.insert("GAMEID".into(), id);
            }
        }
        // The program is right before the user's arguments, at the end.
        launch.exe_index = process.command.args.len().checked_sub(args.len() + 1);
        if third_party {
            // `start` returns at once; the store app is its own program.
            process.stop.kill_patterns.clear();
        }
        // Cloud saves are synced around the launch when `autoSyncSaves` is on
        // and there is a folder to sync.
        let s = process.prep.wine.clone().unwrap_or_else(|| settings.clone());
        if s.auto_sync_saves && !s.saves_path.trim().is_empty() {
            process.prep.cloud_saves = Some(CloudSaves::new("legendary", &s.saves_path));
        }
        // Redistributables, EOS overlay and known fixes: once per game and prefix.
        let (winetricks, run_in_prefix) = known_fixes_steps(ctx, "epic", app);
        process.prep.first_launch = Some(FirstLaunch {
            runner: "legendary".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: game.install.platform.clone().unwrap_or_default(),
            ea_managed: flag(&game.rest, "isEAManaged"),
            ubisoft_managed: flag(&game.rest, "isUbisoftManaged"),
            winetricks,
            run_in_prefix,
            post_install: Vec::new(),
        });
        process.epic = Some(launch);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan_with_settings, LaunchPlan};
    use crate::probe::FakeProbe;
    use heronite_data::models::InstallInfo;
    use heronite_data::DataPaths;
    use serde_json::json;

    const UMU: &str = "/hr/tools/runtimes/umu/umu_run.py";
    const EXE: &str = "/games/Control/Binaries/Win64/Control.exe";

    fn ctx() -> PlanContext {
        let mut c = PlanContext::load(&DataPaths::at("/hr"));
        c.ui_language = Some("en".into());
        c.known_fixes = Some(json!({"epic": {"Calluna": {"envVariables": {"FOO": "bar", "FLAG": true}}}}));
        c.umu_ids = Some(json!({"legendary_Calluna": "umu-870780", "legendary_Other": null}));
        c
    }

    fn game() -> GameInfo {
        let mut g = GameInfo {
            runner: "legendary".into(),
            app_name: "Calluna".into(),
            title: "Control".into(),
            is_installed: true,
            folder_name: Some("Control".into()),
            install: InstallInfo {
                install_path: Some("/games/Control".into()),
                platform: Some("Windows".into()),
                executable: Some(r"Binaries\Win64\Control.exe".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        g.rest.insert("isEAManaged".into(), json!(false));
        g
    }

    fn probe() -> FakeProbe {
        FakeProbe::new("/home/u")
            .with_path("/games/Control")
            .with_path(EXE)
            .with_path("/games/Control/Binaries/Alt.exe")
            .with_path("/p/GE/proton")
            .with_path(UMU)
            .with_bin("gamemoderun", "/usr/bin/gamemoderun")
    }

    fn settings(extra: serde_json::Value) -> GameSettings {
        let mut base = json!({
            "wineVersion": {"bin": "/p/GE/proton", "name": "GE-Proton9", "type": "proton"},
            "winePrefix": "/pfx", "launcherArgs": "-nolauncher",
            "enviromentOptions": [{"key": "PROTON_PREFER_SDL", "value": "1"}]
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        GameSettings::from_map(base.as_object().unwrap(), "/home/u")
    }

    #[test]
    fn the_games_arguments_are_split_the_windows_way() {
        // A backslash is a path separator, single quotes still group.
        let s = settings(json!({"launcherArgs": r"-userdir C:\Users\me\saves -x 'a b' -name Rafa's"}));
        let (launched, args) = launched_and_args("Calluna", &s, &LaunchOptions { extra_args: vec!["--go".into()], ..Default::default() });
        assert_eq!(launched, "Calluna");
        assert_eq!(args, ["--go", "-userdir", r"C:\Users\me\saves", "-x", "a b", "-name", "Rafa's"]);
    }

    fn process(probe: &FakeProbe, g: &GameInfo, s: &GameSettings, opts: &LaunchOptions) -> ProcessPlan {
        match build_plan_with_settings(&ctx(), probe, g, None, s, opts) {
            LaunchPlan::Wine { process, .. } | LaunchPlan::Native(process) => process,
            other => panic!("{other:?}"),
        }
    }

    fn epic_process(s: &GameSettings, opts: &LaunchOptions) -> ProcessPlan {
        process(&probe(), &game(), s, opts)
    }

    /// The user's arguments: what follows the program.
    fn user_args(p: &ProcessPlan) -> Vec<String> {
        match p.epic.as_ref().unwrap().exe_index {
            Some(i) => p.command.args[i + 1..].to_vec(),
            None => p.command.args.clone(),
        }
    }

    #[test]
    fn store_app_games_launch_without_an_install_folder() {
        let mut g = game();
        g.install.install_path = None;
        let missing = build_plan_with_settings(&ctx(), &probe(), &g, None, &settings(json!({})), &LaunchOptions::default());
        assert!(matches!(missing, LaunchPlan::Unavailable(ref why) if why.contains("Install folder not found")), "{missing:?}");

        g.rest.insert("thirdPartyManagedApp".into(), json!("UbisoftConnect"));
        g.rest.insert("isUbisoftManaged".into(), json!(true));
        let p = process(&probe(), &g, &settings(json!({})), &LaunchOptions::default());
        let launch = p.epic.as_ref().unwrap();
        assert_eq!(launch.store_link, Some(StoreLink::Ubisoft));
        if cfg!(unix) {
            // Wine's `start` opens the link in the prefix, with nothing after it.
            let i = launch.exe_index.unwrap();
            assert_eq!(&p.command.args[i..], ["start"]);
            assert!(p.stop.kill_patterns.is_empty());
        }
        assert!(!p.command.env.contains_key("STEAM_COMPAT_INSTALL_PATH"));
        assert!(p.prep.first_launch.unwrap().ubisoft_managed);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn the_plan_carries_the_first_launch_work() {
        let mut c = ctx();
        c.known_fixes = Some(json!({"epic": {"Calluna": {
            "winetricks": ["vcrun2022", ""], "runInPrefix": ["Redist/setup.exe"], "envVariables": {"A": "b"}
        }}}));
        let mut g = game();
        g.rest.insert("isUbisoftManaged".into(), json!(true));
        let LaunchPlan::Wine { process, .. } = build_plan_with_settings(&c, &probe(), &g, None, &settings(json!({})), &Default::default())
        else {
            panic!()
        };
        let fl = process.prep.first_launch.expect("first launch");
        assert_eq!(fl.app_name, "Calluna");
        assert_eq!(fl.install_path, "/games/Control");
        assert_eq!(fl.platform, "Windows");
        assert!(fl.ubisoft_managed && !fl.ea_managed);
        assert_eq!(fl.winetricks, ["vcrun2022"]);
        assert_eq!(fl.run_in_prefix, ["Redist/setup.exe"]);
        // Games without an entry still get the Epic setup.
        let fl = epic_process(&settings(json!({})), &Default::default()).prep.first_launch.unwrap();
        assert!(fl.winetricks.is_empty() && fl.run_in_prefix.is_empty());
    }

    // Linux paths; Epic is off on Windows (`Feature::EpicStore`).
    #[cfg(unix)]
    #[test]
    fn the_game_starts_through_umu_with_epic_completing_it_at_launch() {
        let s = settings(json!({"useGameMode": true, "targetExe": "Binaries/Alt.exe"}));
        let plan = build_plan_with_settings(
            &ctx(),
            &probe(),
            &game(),
            None,
            &s,
            &LaunchOptions { skip_version_check: true, ..Default::default() },
        );
        let LaunchPlan::Wine { process, runtime, .. } = plan else { panic!("{plan:?}") };
        assert_eq!(runtime, WineRuntime::Umu);
        let c = &process.command;
        assert_eq!(c.program, "/usr/bin/gamemoderun");
        assert_eq!(c.args, [UMU, "/games/Control/Binaries/Alt.exe", "-nolauncher"]);
        let launch = process.epic.as_ref().unwrap();
        assert_eq!(
            *launch,
            EpicLaunch {
                app_name: "Calluna".into(),
                game: "Calluna".into(),
                exe_index: Some(1),
                offline: false,
                check_update: false,
                language: Some("en".into()),
                override_exe: Some("Binaries/Alt.exe".into()),
                store_link: None,
            }
        );
        assert!(!c.env.contains_key("LEGENDARY_CONFIG_PATH"));
        assert_eq!(c.env["GAMEID"], "umu-870780");
        assert_eq!(c.env["PROTONPATH"], "/p/GE");
        assert_eq!(c.env["WINEPREFIX"], "/pfx");
        assert_eq!(c.env["STEAM_COMPAT_INSTALL_PATH"], "/games/Control");
        assert_eq!(c.env["SteamGameId"], "heroic-Control");
        assert_eq!(c.env["FOO"], "bar");
        assert_eq!(c.env["FLAG"], "true");
        assert_eq!(c.env["HEROIC_APP_RUNNER"], "legendary");
        assert_eq!(process.stop.kill_patterns, ["Alt.exe"]);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn proton_with_umu_disabled_starts_proton_itself() {
        let p = FakeProbe::new("/home/u").with_path("/games/Control").with_path(EXE).with_path("/p/GE/proton");
        let plan = process(&p, &game(), &settings(json!({"disableUMU": true})), &Default::default());
        assert_eq!(plan.command.program, "/p/GE/proton");
        assert_eq!(plan.command.args, ["waitforexitandrun", EXE, "-nolauncher"]);
        assert_eq!(plan.epic.unwrap().exe_index, Some(1));
        assert!(!plan.command.env.contains_key("GAMEID") || plan.command.env["GAMEID"] == "umu-0");
    }

    #[test]
    fn play_anyway_skips_the_update_check() {
        let s = settings(json!({}));
        assert!(epic_process(&s, &Default::default()).epic.unwrap().check_update);
        let anyway = LaunchOptions { skip_version_check: true, ..Default::default() };
        assert!(!epic_process(&s, &anyway).epic.unwrap().check_update);
    }

    #[test]
    fn the_chosen_launch_option_changes_what_starts() {
        let args = |s: Value| user_args(&epic_process(&settings(s), &Default::default()));
        assert_eq!(args(json!({"launcherArgs": "-dx11"})), ["-dx11"]);
        let vr =
            args(json!({"launcherArgs": "-dx11", "lastUsedLaunchOption": {"type": "basic", "name": "VR", "parameters": "--launchvr -x"}}));
        assert_eq!(vr, ["--launchvr", "-x", "-dx11"], "the option's arguments come first");
        let dlc = epic_process(
            &settings(json!({"lastUsedLaunchOption": {"type": "dlc", "dlcAppName": "Editor", "dlcTitle": "Editor"}})),
            &Default::default(),
        );
        let launch = dlc.epic.unwrap();
        assert_eq!((launch.app_name.as_str(), launch.game.as_str()), ("Editor", "Calluna"), "a playable DLC is launched itself");
    }

    #[test]
    fn language_and_offline_go_to_epic() {
        let launch = epic_process(&settings(json!({"language": "de", "offlineMode": true})), &Default::default()).epic.unwrap();
        assert_eq!(launch.language.as_deref(), Some("de"));
        assert!(launch.offline);
        let alt = LaunchOptions { alt_exe: Some("Binaries/Alt.exe".into()), ..Default::default() };
        let p = epic_process(&settings(json!({"targetExe": "Other.exe"})), &alt);
        assert_eq!(p.epic.unwrap().override_exe.as_deref(), Some("Binaries/Alt.exe"), "\"Play with\" wins over targetExe");
    }

    #[test]
    fn cloud_saves_need_autosync_and_a_folder() {
        let off = epic_process(&settings(json!({"savesPath": "/saves"})), &Default::default());
        assert_eq!(off.prep.cloud_saves, None);
        let no_path = epic_process(&settings(json!({"autoSyncSaves": true, "savesPath": "  "})), &Default::default());
        assert_eq!(no_path.prep.cloud_saves, None);
        let on = epic_process(&settings(json!({"autoSyncSaves": true, "savesPath": "/saves"})), &Default::default());
        assert_eq!(on.prep.cloud_saves, Some(CloudSaves::new("legendary", "/saves")));
        // The Wine side of the preparation travels with the plan (Linux).
        if cfg!(unix) {
            let wine = on.prep.wine.as_ref().unwrap();
            assert_eq!(wine.wine_prefix, "/pfx");
            assert!(!on.prep.umu);
        } else {
            assert!(on.prep.wine.is_none());
        }
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn a_missing_prefix_is_created_before_the_game_starts() {
        let first = epic_process(&settings(json!({})), &Default::default());
        assert_eq!(first.create_dirs, ["/pfx"]);
        assert_eq!(first.pre_commands[0].program, UMU);
        assert_eq!(first.pre_commands[0].args, ["createprefix"]);
        assert_eq!(first.pre_commands[0].env["PROTON_VERB"], "run");
        assert_eq!(first.pre_commands[0].env["WINEPREFIX"], "/pfx");
        // An existing prefix needs nothing.
        let mut ready = probe().with_path("/pfx");
        for f in ["dosdevices", "drive_c", "system.reg", "user.reg", "userdef.reg"] {
            ready = ready.with_path(&format!("/pfx/pfx/{f}"));
        }
        for f in ["pfx.lock", "tracked_files", "version", "config_info"] {
            ready = ready.with_path(&format!("/pfx/{f}"));
        }
        let p = process(&ready, &game(), &settings(json!({})), &Default::default());
        assert!(p.create_dirs.is_empty() && p.pre_commands.is_empty());
        // One whose creation was stopped midway is created again.
        let cut_short = ready.with_path(&format!("/pfx/{}", crate::wine::CREATING_MARKER));
        let p = process(&cut_short, &game(), &settings(json!({})), &Default::default());
        assert_eq!(p.pre_commands[0].args, ["createprefix"]);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn a_missing_umu_runtime_is_planned_for_download() {
        let p = FakeProbe::new("/home/u").with_path("/games/Control").with_path(EXE).with_path("/p/GE/proton");
        let plan = process(&p, &game(), &settings(json!({})), &Default::default());
        assert!(plan.prep.umu);
        assert_eq!(plan.command.program, UMU);
        assert_eq!(plan.command.args, [EXE, "-nolauncher"]);
        assert_eq!(plan.epic.unwrap().exe_index, Some(0));
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn plain_wine_starts_the_program_through_wine() {
        let p = FakeProbe::new("/home/u")
            .with_path("/games/Control")
            .with_path(EXE)
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_bin("mangohud", "/usr/bin/mangohud");
        let s = settings(json!({
            "wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"},
            "showMangohud": true, "offlineMode": true
        }));
        let plan = process(&p, &game(), &s, &Default::default());
        assert_eq!(plan.command.program, "/usr/bin/mangohud");
        assert_eq!(plan.command.args, ["--dlsym", "/w/bin/wine", EXE, "-nolauncher"]);
        assert_eq!(plan.epic.as_ref().unwrap().exe_index, Some(2));
        assert!(plan.epic.unwrap().offline);
        assert_eq!(plan.stop.commands[0].program, "/w/bin/wineserver");
    }

    #[test]
    fn unavailable_cases() {
        let c = ctx();
        let mut not_installed = game();
        not_installed.is_installed = false;
        assert!(build_plan_with_settings(&c, &probe(), &not_installed, None, &settings(json!({})), &Default::default())
            .unavailable_reason()
            .unwrap()
            .contains("not installed"));
        let no_dir = FakeProbe::new("/h");
        assert!(build_plan_with_settings(&c, &no_dir, &game(), None, &settings(json!({})), &Default::default())
            .unavailable_reason()
            .unwrap()
            .contains("Install folder"));
        let mut no_exe = game();
        no_exe.install.executable = None;
        assert!(build_plan_with_settings(&c, &probe(), &no_exe, None, &settings(json!({})), &Default::default())
            .unavailable_reason()
            .unwrap()
            .contains("Executable not found"));
    }

    /// Windows builds on Windows: the game's program as it is.
    #[cfg(windows)]
    mod windows {
        use super::*;

        fn native(s: &GameSettings, opts: &LaunchOptions) -> ProcessPlan {
            match build_plan_with_settings(&ctx(), &probe(), &game(), None, s, opts) {
                LaunchPlan::Native(process) => process,
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_windows_build_starts_its_program_without_wine() {
            let s =
                settings(json!({"targetExe": "Binaries/Alt.exe", "offlineMode": true, "autoSyncSaves": true, "savesPath": r"C:\Saves"}));
            let p = native(&s, &LaunchOptions { skip_version_check: true, ..Default::default() });
            assert_eq!(p.command.program, r"/games/Control\Binaries\Alt.exe");
            assert_eq!(p.command.args, ["-nolauncher"]);
            let launch = p.epic.clone().unwrap();
            assert_eq!(launch.exe_index, None);
            assert!(launch.offline && !launch.check_update);
            assert_eq!(launch.override_exe.as_deref(), Some("Binaries/Alt.exe"));
            let env = &p.command.env;
            assert_eq!(env["PROTON_PREFER_SDL"], "1", "the user's variables reach the game");
            for wine in ["WINEPREFIX", "PROTONPATH", "STEAM_COMPAT_INSTALL_PATH", "GAMEID", "FOO", "LD_PRELOAD", "LEGENDARY_CONFIG_PATH"] {
                assert!(!env.contains_key(wine), "{wine}");
            }
            assert!(p.prep.wine.is_none() && !p.prep.umu && p.pre_commands.is_empty() && p.create_dirs.is_empty());
            assert_eq!(p.prep.cloud_saves, Some(CloudSaves::new("legendary", r"C:\Saves")));
            let fl = p.prep.first_launch.unwrap();
            assert_eq!((fl.runner.as_str(), fl.install_path.as_str()), ("legendary", "/games/Control"));
            assert!(fl.winetricks.is_empty() && fl.run_in_prefix.is_empty(), "no known fixes without Wine");
            assert_eq!(p.stop.kill_patterns, ["Alt.exe"]);
            assert!(p.stop.commands.is_empty());
            assert_eq!(p.handoff.unwrap().folder.as_deref(), Some("/games/Control"));
            assert!(p.hold_awake, "the computer stays awake while the game runs");
        }

        #[test]
        fn store_app_games_ask_for_their_link() {
            let mut g = game();
            g.install.install_path = None;
            g.rest.insert("thirdPartyManagedApp".into(), json!("Origin"));
            g.rest.insert("isEAManaged".into(), json!(true));
            let plan = build_plan_with_settings(&ctx(), &probe(), &g, None, &settings(json!({})), &LaunchOptions::default());
            let LaunchPlan::Native(process) = plan else { panic!("{plan:?}") };
            assert_eq!(process.epic.as_ref().unwrap().store_link, Some(StoreLink::EaApp));
            assert!(process.handoff.is_none());
            assert!(process.prep.first_launch.unwrap().ea_managed);
        }

        #[test]
        fn builds_for_other_systems_do_not_run_here() {
            let mut g = game();
            g.install.platform = Some("Mac".into());
            let why = build_plan_with_settings(&ctx(), &probe(), &g, None, &settings(json!({})), &LaunchOptions::default())
                .unavailable_reason()
                .unwrap()
                .to_string();
            assert!(why.contains("macOS"), "{why}");
        }
    }
}
