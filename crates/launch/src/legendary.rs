//! Epic games through the `legendary` CLI (port of Heroic's
//! `LegendaryGame.launch` and `getWineFlags`).

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::env::{build_wrappers_in, setup_env_vars, setup_wine_env_vars, setup_wrapper_env_vars, Env};
use crate::host::{run_mode, unavailable_reason, Os, RunMode};
use crate::plan::{CloudSaves, Command, FirstLaunch, Handoff, LaunchOptions, LaunchPlan, PlanContext, PrepSpec, ProcessPlan, StopSpec};
use crate::probe::Probe;
use crate::sandbox;
use crate::settings::GameSettings;
use crate::wine;

/// Where the legendary binary may live, in priority order
/// (`HERONITE_LEGENDARY_BIN`, `altLegendaryBin`, the data folder's copy).
pub fn legendary_candidates(ctx: &PlanContext, probe: &dyn Probe) -> Vec<String> {
    crate::store_tools::LEGENDARY.candidates(ctx, probe)
}

/// First existing candidate, else `legendary` on `PATH`.
pub fn find_legendary(ctx: &PlanContext, probe: &dyn Probe) -> Option<String> {
    crate::store_tools::LEGENDARY.find(ctx, probe)
}

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

/// What `legendary launch` starts and the arguments up to the user's own:
/// a playable DLC is launched itself; a basic launch option adds its
/// arguments before the user's (the order legendary's command object is
/// flattened in).
fn launch_args(app: &str, s: &GameSettings, opts: &LaunchOptions) -> (String, Vec<String>) {
    use crate::launch_option::LaunchOption;
    let (launched, option_args) = match &s.launch_option {
        Some(LaunchOption::Dlc { app_name, .. }) => (app_name.as_str(), String::new()),
        Some(LaunchOption::Basic { parameters, .. }) => (app, parameters.clone()),
        None => (app, String::new()),
    };
    let mut args: Vec<String> = vec!["launch".into(), launched.into()];
    let extra = Some(option_args)
        .into_iter()
        .chain(opts.extra_args.iter().cloned())
        .chain(Some(s.launcher_args.clone()))
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !extra.is_empty() {
        args.extend(shlex::split(&extra).unwrap_or_else(|| extra.split_whitespace().map(str::to_string).collect()));
    }
    (launched.to_string(), args)
}

/// legendary's own options after the runner's: the update check, the
/// language, another executable, offline, the store app of the game.
fn option_flags(ctx: &PlanContext, game: &GameInfo, s: &GameSettings, opts: &LaunchOptions) -> Vec<String> {
    let mut args = Vec::new();
    if opts.skip_version_check {
        args.push("--skip-version-check".into());
    }
    let language = if !s.language.is_empty() { Some(s.language.clone()) } else { ctx.ui_language.clone() };
    if let Some(l) = language {
        args.extend(["--language".into(), l]);
    }
    if let Some(exe) = opts.alt_exe.clone().filter(|e| !e.is_empty()) {
        args.extend(["--override-exe".into(), exe]);
    } else if !s.target_exe.is_empty() {
        args.extend(["--override-exe".into(), s.target_exe.clone()]);
    }
    if s.offline_mode {
        args.push("--offline".into());
    }
    if flag(&game.rest, "isEAManaged") {
        args.push("--origin".into());
    }
    if flag(&game.rest, "isUbisoftManaged") {
        args.push("--ubisoft".into());
    }
    args
}

/// A Windows build on Windows: no Wine, prefix, wrappers, winetricks or
/// known fixes. legendary only signs in and names the game's program
/// (`--json`), which the launch starts itself ([`crate::legendary_windows`]);
/// the first launch runs the game's prerequisites
/// ([`crate::epic_native_setup`]).
fn build_native(
    ctx: &PlanContext,
    game: &GameInfo,
    s: &GameSettings,
    opts: &LaunchOptions,
    legendary: String,
    install_path: &str,
    third_party: bool,
) -> LaunchPlan {
    let app = game.app_name.as_str();
    let mut env = setup_wrapper_env_vars(app, "legendary");
    env.extend(s.env_options.iter().cloned());
    env.insert("LEGENDARY_CONFIG_PATH".into(), ctx.paths.legendary_config_dir().to_string_lossy().into_owned());
    let (_, mut args) = launch_args(app, s, opts);
    args.extend(option_flags(ctx, game, s, opts));
    args.push(crate::legendary_windows::JSON.into());
    let mut prep = PrepSpec::default();
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
    let exe = game.install.executable.as_deref().map(crate::emulators::node_basename).filter(|n| !n.is_empty());
    let stop = StopSpec { kill_patterns: exe.into_iter().collect(), commands: Vec::new() };
    // A game that hands itself to another process at once is followed by the
    // programs running from its folder.
    let handoff =
        (!third_party && !install_path.is_empty()).then(|| Handoff { folder: Some(install_path.to_string()), ..Default::default() });
    LaunchPlan::Legendary {
        app_name: app.to_string(),
        process: ProcessPlan {
            command: Command { program: legendary.clone(), args, env, cwd: Some(crate::emulators::node_dirname(&legendary)) },
            stop,
            prep,
            handoff,
            ask_legendary: true,
            ..Default::default()
        },
    }
}

pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, opts: &LaunchOptions) -> LaunchPlan {
    let app = game.app_name.as_str();
    if !game.is_installed {
        return LaunchPlan::Unavailable(tr!("This Epic game is not installed").into());
    }
    let install_path = game.install.install_path.as_deref().unwrap_or("");
    // EA app / Ubisoft Connect games live inside the prefix, with their store
    // app; legendary only opens them through it (`--origin` / `--ubisoft`).
    let third_party = game.rest.get("thirdPartyManagedApp").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if !third_party && (install_path.is_empty() || !probe.exists(install_path)) {
        return LaunchPlan::Unavailable(tr!("Install folder not found: {install_path}", install_path));
    }
    let Some(legendary) = find_legendary(ctx, probe) else {
        return LaunchPlan::Unavailable(tr!("The legendary binary was not found").into());
    };

    // A Windows build on Windows runs as it is; on Linux every Epic game runs
    // through Wine/Proton.
    let os = Os::from_name(game.install.platform.as_deref().unwrap_or("")).unwrap_or(Os::Windows);
    match run_mode(os) {
        RunMode::Native => return build_native(ctx, game, settings, opts, legendary, install_path, third_party),
        RunMode::Unavailable if Os::HOST == Os::Windows => return LaunchPlan::Unavailable(unavailable_reason(os)),
        _ => {}
    }

    let wine = match wine::prepare_for_launch(ctx, probe, settings) {
        Ok(w) => w,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    let s = &wine.settings;

    // umu goes into the wrapper list so legendary starts the game through it.
    let through_umu = wine.umu.is_some();

    // Environment, later entries win (same order as Heroic).
    let mut env = setup_wrapper_env_vars(app, "legendary");
    env.extend(setup_env_vars(s, Some(install_path), probe));
    env.extend(known_fixes_env(ctx, "epic", app));
    env.extend(setup_wine_env_vars(s, game.folder_name.as_deref().unwrap_or("0"), &ctx.runtime_dir(), probe));
    if wine.umu.is_some() {
        if let Some(id) = cached_umu_id(ctx, "legendary", app) {
            env.insert("GAMEID".into(), id);
        }
    }
    if let Err(err) = crate::lsfg::apply(&mut env, &s.lsfg, &ctx.global, probe) {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    env.insert("LEGENDARY_CONFIG_PATH".into(), ctx.paths.legendary_config_dir().to_string_lossy().into_owned());

    // legendary stays outside the sandbox (it signs in to Epic); the game
    // it starts through the wrappers runs inside.
    let req = sandbox::Request { folder: Some(install_path).filter(|p| !p.is_empty()), read: Vec::new(), env: Some(&env) };
    let prelude = match sandbox::prelude(ctx, probe, s, sandbox::Owner::Prefix(&s.wine_prefix), &req) {
        Ok(p) => p,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    let wp = build_wrappers_in(s, probe, wine.umu.as_deref(), prelude.as_deref());
    if let Some(err) = wp.error {
        return LaunchPlan::Unavailable(tr!("Launch aborted: {error}", error = err));
    }
    let wrappers = wp.wrappers;
    let wrapper_joined = shlex::try_join(wrappers.iter().map(String::as_str)).unwrap_or_else(|_| wrappers.join(" "));

    let (launched, mut args) = launch_args(app, s, opts);
    match s.wine_version.kind.as_str() {
        "proton" => {
            args.push("--no-wine".into());
            let wrapper = if wine.umu.is_some() {
                wrapper_joined.clone()
            } else {
                format!("{wrapper_joined} \"{}\" waitforexitandrun", s.wine_version.bin)
            };
            args.extend(["--wrapper".into(), wrapper.trim().to_string()]);
        }
        _ => {
            args.extend(["--wine".into(), s.wine_version.bin.clone()]);
            if !wrapper_joined.is_empty() {
                args.extend(["--wrapper".into(), wrapper_joined.clone()]);
            }
        }
    }
    args.extend(option_flags(ctx, game, s, opts));

    let stop = StopSpec { kill_patterns: vec![launched], commands: vec![wine.shutdown_command(&env)] };
    // Heroic creates a missing prefix (`verifyWinePrefix`) before anything
    // else touches it, so DXVK can be installed into it.
    let (create_dirs, pre_commands) =
        if wine.prefix_missing(probe) { (vec![s.wine_prefix.clone()], wine.prefix_creation(&env)) } else { (vec![], vec![]) };
    let (pre_commands, _, stop) = sandbox::confine(prelude.as_deref(), pre_commands, None, stop);
    // Cloud saves are synced around the launch when `autoSyncSaves` is on
    // and there is a folder to sync.
    let mut prep = wine.prep_spec();
    if s.auto_sync_saves && !s.saves_path.trim().is_empty() {
        prep.cloud_saves = Some(CloudSaves::new("legendary", &s.saves_path));
    }
    // Redistributables, EOS overlay and known fixes: once per game and prefix.
    let (winetricks, run_in_prefix) = known_fixes_steps(ctx, "epic", app);
    prep.first_launch = Some(FirstLaunch {
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
    LaunchPlan::Legendary {
        app_name: app.to_string(),
        process: ProcessPlan {
            command: Command { program: legendary.clone(), args, env, cwd: Some(crate::emulators::node_dirname(&legendary)) },
            stop,
            create_dirs,
            pre_commands,
            prep,
            notes: wine.notes,
            through_umu,
            sandboxed: prelude.is_some(),
            ..Default::default()
        },
    }
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
            install: InstallInfo { install_path: Some("/games/Control".into()), platform: Some("Windows".into()), ..Default::default() },
            ..Default::default()
        };
        g.rest.insert("isEAManaged".into(), json!(false));
        g
    }

    fn probe() -> FakeProbe {
        FakeProbe::new("/home/u")
            .with_env("HERONITE_LEGENDARY_BIN", "/lg/legendary")
            .with_path("/lg/legendary")
            .with_path("/games/Control")
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
    fn store_app_games_launch_without_an_install_folder() {
        let mut g = game();
        g.install.install_path = None;
        let missing = build_plan_with_settings(&ctx(), &probe(), &g, None, &settings(json!({})), &LaunchOptions::default());
        assert!(matches!(missing, LaunchPlan::Unavailable(ref why) if why.contains("Install folder not found")), "{missing:?}");

        g.rest.insert("thirdPartyManagedApp".into(), json!("UbisoftConnect"));
        g.rest.insert("isUbisoftManaged".into(), json!(true));
        let plan = build_plan_with_settings(&ctx(), &probe(), &g, None, &settings(json!({})), &LaunchOptions::default());
        let LaunchPlan::Legendary { process, .. } = plan else { panic!("{plan:?}") };
        assert!(process.command.args.iter().any(|a| a == "--ubisoft"), "{:?}", process.command.args);
        assert!(!process.command.env.contains_key("STEAM_COMPAT_INSTALL_PATH"));
        assert!(process.prep.first_launch.unwrap().ubisoft_managed);
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
        let plan = build_plan_with_settings(&c, &probe(), &g, None, &settings(json!({})), &LaunchOptions::default());
        let LaunchPlan::Legendary { process, .. } = plan else { panic!("{plan:?}") };
        let fl = process.prep.first_launch.expect("first launch");
        assert_eq!(fl.app_name, "Calluna");
        assert_eq!(fl.install_path, "/games/Control");
        assert_eq!(fl.platform, "Windows");
        assert!(fl.ubisoft_managed && !fl.ea_managed);
        assert_eq!(fl.winetricks, ["vcrun2022"]);
        assert_eq!(fl.run_in_prefix, ["Redist/setup.exe"]);
        // Games without an entry still get the Epic setup.
        let plain = build_plan_with_settings(&ctx(), &probe(), &game(), None, &settings(json!({})), &LaunchOptions::default());
        let LaunchPlan::Legendary { process, .. } = plain else { panic!() };
        let fl = process.prep.first_launch.unwrap();
        assert!(fl.winetricks.is_empty() && fl.run_in_prefix.is_empty());
    }

    // Linux paths; Epic is off on Windows (`Feature::EpicStore`).
    #[cfg(unix)]
    #[test]
    fn proton_through_umu_goes_in_the_wrapper() {
        let s = settings(json!({"useGameMode": true, "targetExe": "Binaries/Alt.exe"}));
        let plan = build_plan_with_settings(
            &ctx(),
            &probe(),
            &game(),
            None,
            &s,
            &LaunchOptions { skip_version_check: true, ..Default::default() },
        );
        let LaunchPlan::Legendary { process, app_name } = plan else { panic!("{plan:?}") };
        assert_eq!(app_name, "Calluna");
        let c = &process.command;
        assert_eq!(c.program, "/lg/legendary");
        assert_eq!(
            c.args,
            [
                "launch",
                "Calluna",
                "-nolauncher",
                "--no-wine",
                "--wrapper",
                &format!("/usr/bin/gamemoderun {UMU}"),
                "--skip-version-check",
                "--language",
                "en",
                "--override-exe",
                "Binaries/Alt.exe"
            ]
        );
        assert_eq!(c.env["LEGENDARY_CONFIG_PATH"], "/hr/legendaryConfig/legendary");
        assert_eq!(c.env["GAMEID"], "umu-870780");
        assert_eq!(c.env["PROTONPATH"], "/p/GE");
        assert_eq!(c.env["WINEPREFIX"], "/pfx");
        assert_eq!(c.env["STEAM_COMPAT_INSTALL_PATH"], "/games/Control");
        assert_eq!(c.env["SteamGameId"], "heroic-Control");
        assert_eq!(c.env["FOO"], "bar");
        assert_eq!(c.env["FLAG"], "true");
        assert_eq!(c.env["HEROIC_APP_RUNNER"], "legendary");
        assert_eq!(process.stop.kill_patterns, ["Calluna"]);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn proton_with_umu_disabled_names_proton_in_the_wrapper() {
        let p = FakeProbe::new("/home/u")
            .with_env("HERONITE_LEGENDARY_BIN", "/lg/legendary")
            .with_path("/lg/legendary")
            .with_path("/games/Control")
            .with_path("/p/GE/proton");
        let plan = build_plan_with_settings(&ctx(), &p, &game(), None, &settings(json!({"disableUMU": true})), &Default::default());
        let LaunchPlan::Legendary { process, .. } = plan else { panic!("{plan:?}") };
        let args = &process.command.args;
        let i = args.iter().position(|a| a == "--wrapper").unwrap();
        assert_eq!(args[i + 1], "\"/p/GE/proton\" waitforexitandrun");
        assert!(args.contains(&"--no-wine".to_string()));
        assert!(!process.command.env.contains_key("GAMEID") || process.command.env["GAMEID"] == "umu-0");
    }

    fn legendary_process(probe: &FakeProbe, s: &GameSettings, opts: &LaunchOptions) -> ProcessPlan {
        match build_plan_with_settings(&ctx(), probe, &game(), None, s, opts) {
            LaunchPlan::Legendary { process, .. } => process,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn play_anyway_passes_skip_version_check() {
        let p = probe();
        let s = settings(json!({}));
        let normal = legendary_process(&p, &s, &Default::default());
        assert!(!normal.command.args.contains(&"--skip-version-check".to_string()));
        let anyway = LaunchOptions { skip_version_check: true, ..Default::default() };
        let process = legendary_process(&p, &s, &anyway);
        assert!(process.command.args.contains(&"--skip-version-check".to_string()));
    }

    #[test]
    fn the_chosen_launch_option_changes_what_starts() {
        let p = probe();
        let args = |s: Value| legendary_process(&p, &settings(s), &Default::default()).command.args;
        let plain = args(json!({"launcherArgs": "-dx11"}));
        assert_eq!(&plain[..3], ["launch", "Calluna", "-dx11"]);
        let vr =
            args(json!({"launcherArgs": "-dx11", "lastUsedLaunchOption": {"type": "basic", "name": "VR", "parameters": "--launchvr -x"}}));
        assert_eq!(&vr[..5], ["launch", "Calluna", "--launchvr", "-x", "-dx11"], "the option's arguments come first");
        let dlc = legendary_process(
            &p,
            &settings(json!({"lastUsedLaunchOption": {"type": "dlc", "dlcAppName": "Editor", "dlcTitle": "Editor"}})),
            &Default::default(),
        );
        assert_eq!(&dlc.command.args[..2], ["launch", "Editor"], "a playable DLC is launched itself");
        if cfg!(unix) {
            assert_eq!(dlc.stop.kill_patterns, ["Editor"]);
        }
    }

    #[test]
    fn cloud_saves_need_autosync_and_a_folder() {
        let p = probe();
        let off = legendary_process(&p, &settings(json!({"savesPath": "/saves"})), &Default::default());
        assert_eq!(off.prep.cloud_saves, None);
        let no_path = legendary_process(&p, &settings(json!({"autoSyncSaves": true, "savesPath": "  "})), &Default::default());
        assert_eq!(no_path.prep.cloud_saves, None);
        let on = legendary_process(&p, &settings(json!({"autoSyncSaves": true, "savesPath": "/saves"})), &Default::default());
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
        let p = probe();
        let process = legendary_process(&p, &settings(json!({})), &Default::default());
        assert_eq!(process.create_dirs, ["/pfx"]);
        assert_eq!(process.pre_commands[0].program, UMU);
        assert_eq!(process.pre_commands[0].args, ["createprefix"]);
        assert_eq!(process.pre_commands[0].env["PROTON_VERB"], "run");
        assert_eq!(process.pre_commands[0].env["WINEPREFIX"], "/pfx");
        // An existing prefix needs nothing.
        let mut ready = probe().with_path("/pfx");
        for f in ["dosdevices", "drive_c", "system.reg", "user.reg", "userdef.reg"] {
            ready = ready.with_path(&format!("/pfx/pfx/{f}"));
        }
        for f in ["pfx.lock", "tracked_files", "version", "config_info"] {
            ready = ready.with_path(&format!("/pfx/{f}"));
        }
        let process = legendary_process(&ready, &settings(json!({})), &Default::default());
        assert!(process.create_dirs.is_empty() && process.pre_commands.is_empty());
        // One whose creation was stopped midway is created again.
        let cut_short = ready.with_path(&format!("/pfx/{}", crate::wine::CREATING_MARKER));
        let process = legendary_process(&cut_short, &settings(json!({})), &Default::default());
        assert_eq!(process.pre_commands[0].args, ["createprefix"]);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn a_missing_umu_runtime_is_planned_for_download() {
        let p = FakeProbe::new("/home/u")
            .with_env("HERONITE_LEGENDARY_BIN", "/lg/legendary")
            .with_path("/lg/legendary")
            .with_path("/games/Control")
            .with_path("/p/GE/proton");
        let process = legendary_process(&p, &settings(json!({})), &Default::default());
        assert!(process.prep.umu);
        let args = &process.command.args;
        let i = args.iter().position(|a| a == "--wrapper").unwrap();
        assert_eq!(args[i + 1], UMU);
    }

    // Wine and its prefix are Linux's (Windows runs the game as it is).
    #[cfg(unix)]
    #[test]
    fn plain_wine_uses_wine_flag() {
        let p = FakeProbe::new("/home/u")
            .with_env("HERONITE_LEGENDARY_BIN", "/lg/legendary")
            .with_path("/lg/legendary")
            .with_path("/games/Control")
            .with_path("/w/bin/wine")
            .with_path("/w/bin/wineserver")
            .with_bin("mangohud", "/usr/bin/mangohud");
        let s = settings(json!({
            "wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"},
            "showMangohud": true, "offlineMode": true
        }));
        let LaunchPlan::Legendary { process, .. } = build_plan_with_settings(&ctx(), &p, &game(), None, &s, &Default::default()) else {
            panic!()
        };
        let a = &process.command.args;
        assert_eq!(a[3..7], ["--wine", "/w/bin/wine", "--wrapper", "/usr/bin/mangohud --dlsym"]);
        assert!(a.contains(&"--offline".to_string()));
        assert_eq!(process.stop.commands[0].program, "/w/bin/wineserver");
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
        let no_dir = FakeProbe::new("/h").with_env("HERONITE_LEGENDARY_BIN", "/lg/legendary").with_path("/lg/legendary");
        assert!(build_plan_with_settings(&c, &no_dir, &game(), None, &settings(json!({})), &Default::default())
            .unavailable_reason()
            .unwrap()
            .contains("Install folder"));
        let no_bin = FakeProbe::new("/h").with_path("/games/Control").with_path("/p/GE/proton");
        assert!(build_plan_with_settings(&c, &no_bin, &game(), None, &settings(json!({})), &Default::default())
            .unavailable_reason()
            .unwrap()
            .contains("legendary binary"));
    }

    // Linux paths; Epic is off on Windows (`Feature::EpicStore`).
    #[cfg(unix)]
    #[test]
    fn legendary_binary_lookup_order() {
        let c = ctx();
        assert_eq!(legendary_candidates(&c, &FakeProbe::new("/home/u")), ["/hr/tools/bin/legendary"]);
        let probe = FakeProbe::new("/home/u").with_path("/hr/tools/bin/legendary").with_bin("legendary", "/usr/bin/legendary");
        assert_eq!(find_legendary(&c, &probe).as_deref(), Some("/hr/tools/bin/legendary"));
        let probe = FakeProbe::new("/home/u").with_bin("legendary", "/usr/bin/legendary");
        assert_eq!(find_legendary(&c, &probe).as_deref(), Some("/usr/bin/legendary"));
    }

    /// Windows builds on Windows: legendary only names the program.
    #[cfg(windows)]
    mod windows {
        use super::*;

        fn native(s: &GameSettings, opts: &LaunchOptions) -> ProcessPlan {
            let mut g = game();
            g.install.executable = Some(r"Binaries\Win64\Control.exe".into());
            match build_plan_with_settings(&ctx(), &probe(), &g, None, s, opts) {
                LaunchPlan::Legendary { process, .. } => process,
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_windows_build_asks_legendary_without_wine() {
            let s =
                settings(json!({"targetExe": "Binaries/Alt.exe", "offlineMode": true, "autoSyncSaves": true, "savesPath": r"C:\Saves"}));
            let p = native(&s, &LaunchOptions { skip_version_check: true, ..Default::default() });
            assert!(p.ask_legendary);
            assert_eq!(p.command.program, "/lg/legendary");
            assert_eq!(
                p.command.args,
                [
                    "launch",
                    "Calluna",
                    "-nolauncher",
                    "--skip-version-check",
                    "--language",
                    "en",
                    "--override-exe",
                    "Binaries/Alt.exe",
                    "--offline",
                    "--json"
                ]
            );
            let env = &p.command.env;
            assert_eq!(env["LEGENDARY_CONFIG_PATH"], ctx().paths.legendary_config_dir().to_string_lossy());
            assert_eq!(env["PROTON_PREFER_SDL"], "1", "the user's variables reach the game");
            for wine in ["WINEPREFIX", "PROTONPATH", "STEAM_COMPAT_INSTALL_PATH", "GAMEID", "FOO", "LD_PRELOAD"] {
                assert!(!env.contains_key(wine) || wine == "GAMEID", "{wine}");
            }
            assert!(p.prep.wine.is_none() && !p.prep.umu && p.pre_commands.is_empty() && p.create_dirs.is_empty());
            assert_eq!(p.prep.cloud_saves, Some(CloudSaves::new("legendary", r"C:\Saves")));
            let fl = p.prep.first_launch.unwrap();
            assert_eq!((fl.runner.as_str(), fl.install_path.as_str()), ("legendary", "/games/Control"));
            assert!(fl.winetricks.is_empty() && fl.run_in_prefix.is_empty(), "no known fixes without Wine");
            assert_eq!(p.stop.kill_patterns, ["Control.exe"]);
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
            let LaunchPlan::Legendary { process, .. } = plan else { panic!("{plan:?}") };
            let args = &process.command.args;
            assert!(args.ends_with(&["--origin".to_string(), "--json".to_string()]), "{args:?}");
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
