//! Games a Windows store client starts (`heronite_data::clients`: Battle.net,
//! the EA app, Ubisoft Connect, the Rockstar Games Launcher).
//!
//! On Linux the client runs under Wine in a prefix of its own, with its own
//! settings (`GamesConfig/<config app>.json`); on Windows it runs as it is,
//! from where it is installed. It starts, updates and signs in to its games.
//! The plan of such a game runs the client's program with `launcherArgs`
//! (what the client is asked to do), so the game's settings do not apply.
//! The session follows the game, not the client, which may have been open
//! for hours and stays open afterwards (see `runtime::client`).
//!
//! The sandbox does not apply: the game starts from inside the client, so the
//! client would have to run in it.

use std::path::{Path, PathBuf};

use heronite_data::clients::{machine_drive, prefix_of, Client};
use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use crate::env::join;
use crate::host::{run_mode, Os, RunMode};
use crate::plan::{split_args, ClientHandoff, Command, Handoff, LaunchPlan, PlanContext, ProcessPlan, StopSpec, WineRuntime};
use crate::probe::Probe;
use crate::settings::{load_game_settings, GameSettings};
use crate::wine_tools::{alongside_command, UMU_NSENTER};

/// The plan of a game the client named in `meta.source` starts.
pub(crate) fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, meta: &LocalGameMeta) -> LaunchPlan {
    build_as(ctx, probe, game, meta, run_mode(Os::Windows))
}

/// [`build`] for a client that runs as `mode` says (natively on Windows,
/// under Wine elsewhere).
fn build_as(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, meta: &LocalGameMeta, mode: RunMode) -> LaunchPlan {
    let no_method = || LaunchPlan::Unavailable(tr!("This game has no launch method").into());
    let Some(client) = Client::from_id(&meta.source) else { return no_method() };
    let args = split_args(meta.launcher_args.as_deref().unwrap_or_default());
    if args.is_empty() {
        return no_method();
    }
    match mode {
        RunMode::Native => native_plan(probe, game, client, args),
        RunMode::Wine | RunMode::Unavailable => wine_plan(ctx, probe, game, client, args),
    }
}

fn not_set_up(client: Client) -> LaunchPlan {
    LaunchPlan::Unavailable(tr!("{client} is not set up. Set it up in Stores → {client}.", client = client.name()))
}

/// The game's folder, when it is there: what tells the game from the client.
fn game_folder(probe: &dyn Probe, game: &GameInfo) -> Option<String> {
    game.install.install_path.clone().filter(|f| !f.is_empty() && probe.exists(f))
}

/// What the session follows: the game from `folder`, not the client's
/// programs; `join` asks the client that runs already.
fn handoff(client: Client, folder: Option<String>, join: Command) -> Handoff {
    let client = ClientHandoff {
        name: client.name().to_string(),
        programs: client.programs().iter().map(|p| p.to_string()).collect(),
        main: client.main_program().to_string(),
        join,
    };
    Handoff { folder, client: Some(client), ..Default::default() }
}

/// Windows: the client's program run as it is with `args`. A client that
/// runs already takes the same command as a request (its new instance
/// hands it over and exits), so `join` is the command itself. The session
/// tells the game by its folder, so a game whose folder is not there cannot
/// be followed.
fn native_plan(probe: &dyn Probe, game: &GameInfo, client: Client, args: Vec<String>) -> LaunchPlan {
    let Some(exe) = native_exe(probe, client) else { return not_set_up(client) };
    let Some(folder) = game_folder(probe, game) else {
        return LaunchPlan::Unavailable(tr!(
            "The folder of this game was not found. Refresh the games of {client} in Stores → {client}.",
            client = client.name()
        ));
    };
    let cwd = Path::new(&exe).parent().map(|d| d.to_string_lossy().into_owned()).filter(|d| !d.is_empty());
    let command = Command { program: exe, args, env: Default::default(), cwd };
    LaunchPlan::Native(ProcessPlan {
        command: command.clone(),
        notes: vec![format!("{} starts the game", client.name())],
        handoff: Some(handoff(client, Some(folder), command)),
        ..Default::default()
    })
}

/// The client's program on this PC: where its installer puts it
/// (`Client::exe_candidates` in this PC's `Program Files`), else in the
/// folder its installer wrote in the registry. Looked up through the probe.
fn native_exe(probe: &dyn Probe, client: Client) -> Option<String> {
    let drive = machine_drive(|k| probe.env_var(k));
    let exists = |p: &Path| probe.exists(&p.to_string_lossy());
    let usual = client.exe_candidates().iter().map(|rel| drive.path(rel)).find(|p| exists(p));
    let registered = || registered_folders(client).into_iter().find_map(|named| client.exe_in_folder(&named, exists));
    usual.or_else(registered).map(|p| p.to_string_lossy().into_owned())
}

/// The folders (or programs) the client's installer wrote in this PC's
/// registry (`Client::install_keys`).
#[cfg(windows)]
fn registered_folders(client: Client) -> Vec<PathBuf> {
    use heronite_platform::registry::{string, Hive};
    client
        .install_keys()
        .iter()
        .filter_map(|(key, value)| string(Hive::LocalMachine, key, value))
        .map(|v| v.trim().trim_matches('"').replace('/', "\\"))
        .filter(|v| !v.is_empty())
        .map(|v| PathBuf::from(v.trim_end_matches('\\')))
        .collect()
}

/// No registry here.
#[cfg(not(windows))]
fn registered_folders(_client: Client) -> Vec<PathBuf> {
    Vec::new()
}

/// Under Wine: the client's program in its prefix, with its Wine settings.
fn wine_plan(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, client: Client, args: Vec<String>) -> LaunchPlan {
    let Some(prefix) = prefix_of(&ctx.paths, client, &probe.home()) else { return not_set_up(client) };
    let Some(exe) = client_exe(probe, client, &prefix.to_string_lossy()) else { return not_set_up(client) };

    let settings = load_game_settings(&ctx.paths, client.config_app(), &ctx.global, probe);
    if settings.sandbox.enabled {
        return LaunchPlan::Unavailable(tr!(
            "The sandbox does not run {client}. Turn it off in Stores → {client}.",
            client = client.name()
        ));
    }
    let (mut process, runtime, prefix) = match crate::wine::build_wine_plan(ctx, probe, game, &settings, &exe, &args) {
        LaunchPlan::Wine { process, runtime, prefix } => (process, runtime, prefix),
        other => return other,
    };
    let mut join = match alongside_command(ctx, probe, &game.app_name, &game.runner, &settings, &exe, true) {
        Ok(cmd) => cmd,
        Err(e) => return LaunchPlan::Unavailable(e),
    };
    join.args.extend(args);

    // `waitforexitandrun` first waits for the prefix's wineserver to exit,
    // that is for the client to close.
    process.command.env.insert("PROTON_VERB".into(), "run".into());
    match runtime {
        WineRuntime::Proton => {
            if let Some(verb) = process.command.args.iter_mut().find(|a| *a == "waitforexitandrun") {
                *verb = "run".into();
            }
        }
        // A later game request joins this container (see `join`).
        WineRuntime::Umu => {
            process.command.env.insert(UMU_NSENTER.into(), "1".into());
        }
        WineRuntime::Wine => {}
    }
    // The client keeps the wineserver alive, and stopping the game must not
    // take the client down with it.
    process.wait_after_exit = None;
    process.stop = StopSpec::default();
    process.notes.push(format!("{} starts the game: its prefix and settings are used", client.name()));
    let mut handoff = handoff(client, game_folder(probe, game), join);
    handoff.prefix = Some(prefix.clone());
    process.handoff = Some(handoff);
    LaunchPlan::Wine { process, runtime, prefix }
}

/// Adds the game's programs to the plan. Under Wine they run beside the
/// game, so with the client's Wine and prefix, not the game's own; on
/// Windows as they are.
pub(crate) fn attach_companions(plan: &mut LaunchPlan, ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    match plan.process().map(|p| p.prep.wine.clone()) {
        Some(Some(wine)) => {
            let settings = GameSettings { companions: settings.companions.clone(), ..wine };
            crate::companions::attach(plan, ctx, probe, game, &settings);
        }
        Some(None) if matches!(plan, LaunchPlan::Native(_)) => crate::companions::attach(plan, ctx, probe, game, settings),
        _ => {}
    }
}

/// The client's program in `prefix` (Proton keeps the drive in `pfx/`),
/// looked up through the probe.
fn client_exe(probe: &dyn Probe, client: Client, prefix: &str) -> Option<String> {
    ["pfx/drive_c", "drive_c"]
        .iter()
        .flat_map(|drive| client.exe_candidates().iter().map(move |rel| join(&join(prefix, drive), rel)))
        .find(|exe| probe.exists(exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan_with_settings, LaunchOptions};
    use crate::probe::FakeProbe;
    use crate::settings::{GameSettings, GlobalSettings};
    use heronite_data::DataPaths;
    use serde_json::{json, Value};

    const PROTON: &str = "/p/GE/proton";
    const BNET_EXE: &str = "/pfx/bnet/pfx/drive_c/Program Files (x86)/Battle.net/Battle.net.exe";

    struct Rig {
        _dir: tempfile::TempDir,
        ctx: PlanContext,
    }

    /// The client's settings file in a data folder of its own.
    fn rig(client_settings: Value) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.games_config_dir()).unwrap();
        let app = Client::BattleNet.config_app();
        std::fs::write(paths.game_config(app), json!({ app: client_settings, "version": "v0" }).to_string()).unwrap();
        let mut ctx = PlanContext::load(&paths);
        ctx.global = GlobalSettings::from_defaults(Default::default());
        Rig { _dir: dir, ctx }
    }

    impl Rig {
        fn umu(&self) -> String {
            self.ctx.runtime_dir() + "/umu/umu_run.py"
        }

        /// A machine with the client set up and Proton and umu installed.
        fn probe(&self) -> FakeProbe {
            FakeProbe::new("/home/u")
                .with_path(PROTON)
                .with_path(&self.umu())
                .with_path(BNET_EXE)
                .with_path("/pfx/bnet")
                .with_path("/games/wow")
        }
    }

    fn set_up() -> Value {
        json!({"winePrefix": "/pfx/bnet", "wineVersion": {"bin": PROTON, "name": "GE-Proton9", "type": "proton"}})
    }

    fn wow() -> (GameInfo, LocalGameMeta) {
        let mut game = GameInfo {
            runner: "sideload".into(),
            app_name: "battlenet_wow".into(),
            title: "World of Warcraft".into(),
            ..Default::default()
        };
        game.install.install_path = Some("/games/wow".into());
        let meta = LocalGameMeta {
            app_name: "battlenet_wow".into(),
            runner: "sideload".into(),
            launch_kind: heronite_data::clients::LAUNCH_KIND.into(),
            source: "battlenet".into(),
            launcher_args: Some(r#"--exec="launch WoW""#.into()),
            ..Default::default()
        };
        (game, meta)
    }

    /// What `build_plan_with_settings` does for a client's game, with the
    /// client running as `mode` says whatever this system is.
    fn plan_as(
        mode: RunMode,
        ctx: &PlanContext,
        probe: &dyn Probe,
        game: &GameInfo,
        meta: &LocalGameMeta,
        settings: &GameSettings,
    ) -> LaunchPlan {
        let mut plan = build_as(ctx, probe, game, meta, mode);
        crate::keep_awake::attach(&mut plan, probe, &game.title, settings);
        crate::scripts::attach(&mut plan, probe, game, settings);
        attach_companions(&mut plan, ctx, probe, game, settings);
        plan
    }

    /// The plan under Wine.
    fn plan_of(r: &Rig, probe: &FakeProbe, game_settings: &GameSettings) -> LaunchPlan {
        let (game, meta) = wow();
        plan_as(RunMode::Wine, &r.ctx, probe, &game, &meta, game_settings)
    }

    #[test]
    fn the_client_runs_under_wine_on_linux_and_as_it_is_on_windows() {
        let r = rig(set_up());
        let (game, meta) = wow();
        let probe = on_this_pc(r.probe());
        let plan = build_plan_with_settings(&r.ctx, &probe, &game, Some(&meta), &game_settings(json!({})), &LaunchOptions::default());
        assert_eq!(plan.kind(), if cfg!(windows) { "native" } else { "wine" }, "{plan:?}");
    }

    /// Where Battle.net's installer puts it on a PC whose `Program Files
    /// (x86)` is on `D:`.
    fn bnet_pc() -> String {
        Path::new(r"D:\Apps (x86)").join("Battle.net").join("Battle.net.exe").to_string_lossy().into_owned()
    }

    /// `probe` with Battle.net installed on the PC.
    fn on_this_pc(probe: FakeProbe) -> FakeProbe {
        probe.with_env("ProgramFiles(x86)", r"D:\Apps (x86)").with_path(&bnet_pc())
    }

    fn native_plan_of(probe: &FakeProbe, game: &GameInfo, meta: &LocalGameMeta, settings: &GameSettings) -> LaunchPlan {
        // No settings file: the client has none on Windows.
        let r = rig(json!({}));
        plan_as(RunMode::Native, &r.ctx, probe, game, meta, settings)
    }

    #[test]
    fn on_windows_the_clients_program_runs_as_it_is_and_the_game_is_followed_by_its_folder() {
        let (game, meta) = wow();
        let probe = on_this_pc(FakeProbe::new("/home/u")).with_path("/games/wow");
        let LaunchPlan::Native(process) = native_plan_of(&probe, &game, &meta, &game_settings(json!({}))) else { panic!() };
        let c = &process.command;
        assert_eq!((c.program.as_str(), c.args.as_slice()), (bnet_pc().as_str(), &["--exec=launch WoW".to_string()][..]));
        assert_eq!(c.cwd.as_deref(), Path::new(&bnet_pc()).parent().and_then(Path::to_str));
        assert!(c.env.is_empty(), "the client may outlive the game: nothing of the game's in its environment");
        assert_eq!(process.stop, StopSpec::default(), "stopping the game never touches the client");
        assert!(process.prep.wine.is_none() && !process.sandboxed);
        let h = process.handoff.unwrap();
        assert_eq!((h.prefix.as_deref(), h.folder.as_deref()), (None, Some("/games/wow")));
        let client = h.client.unwrap();
        assert_eq!((client.name.as_str(), client.main.as_str()), ("Battle.net", "battle.net.exe"));
        // A Battle.net that runs takes the same command as a request.
        assert_eq!(client.join, process.command);
    }

    #[test]
    fn on_windows_a_link_is_one_argument_and_the_games_extras_come_with_it() {
        let (mut game, meta) = wow();
        game.app_name = "ea_71067".into();
        let meta = LocalGameMeta {
            source: "ea".into(),
            launcher_args: Some("\"origin2://game/launch?offerIds=71067,71068&autoDownload=1\"".into()),
            ..meta
        };
        let parts = ["Electronic Arts", "EA Desktop", "EA Desktop", "EALauncher.exe"];
        let ea = parts.iter().fold(PathBuf::from(r"C:\Program Files"), |p, part| p.join(part));
        let probe = FakeProbe::new("/home/u").with_env("ProgramW6432", r"C:\Program Files").with_path(&ea.to_string_lossy());
        let probe = probe.with_path("/games/wow").with_path("/t/trainer.exe");
        let own = game_settings(json!({"beforeLaunchScriptPath": "/s/before.bat", "heroniteCompanions": [{"path": "/t/trainer.exe"}]}));
        let LaunchPlan::Native(process) = native_plan_of(&probe, &game, &meta, &own) else { panic!() };
        assert_eq!(process.command.program, ea.to_string_lossy());
        assert_eq!(process.command.args, ["origin2://game/launch?offerIds=71067,71068&autoDownload=1"]);
        assert_eq!(process.before_script.as_ref().unwrap().program, "/s/before.bat");
        assert_eq!(process.handoff.as_ref().unwrap().client.as_ref().unwrap().main, "eadesktop.exe");
        if cfg!(windows) {
            assert_eq!(process.companions.len(), 1);
            assert_eq!(process.companions[0].command.program, "/t/trainer.exe", "as it is, beside the game");
        }
    }

    #[test]
    fn on_windows_a_client_or_a_folder_that_is_not_there_cannot_start_the_game() {
        let (game, meta) = wow();
        let own = game_settings(json!({}));
        // The registry of this PC may name a real Battle.net: the probe has none.
        let none = FakeProbe::new("/home/u").with_env("ProgramFiles(x86)", r"D:\Apps (x86)").with_path("/games/wow");
        let msg = "Battle.net is not set up. Set it up in Stores → Battle.net.";
        assert_eq!(native_plan_of(&none, &game, &meta, &own), LaunchPlan::Unavailable(msg.into()));
        let gone = on_this_pc(FakeProbe::new("/home/u"));
        let msg = "The folder of this game was not found. Refresh the games of Battle.net in Stores → Battle.net.";
        assert_eq!(native_plan_of(&gone, &game, &meta, &own), LaunchPlan::Unavailable(msg.into()));
        let bare = LocalGameMeta { launcher_args: Some("  ".into()), ..meta };
        let probe = on_this_pc(FakeProbe::new("/home/u")).with_path("/games/wow");
        assert_eq!(native_plan_of(&probe, &game, &bare, &own), LaunchPlan::Unavailable("This game has no launch method".into()));
    }

    fn game_settings(v: Value) -> GameSettings {
        GameSettings::from_map(v.as_object().unwrap(), "/home/u")
    }

    #[test]
    fn the_client_runs_in_its_prefix_with_its_args_and_leaves_the_prefix_alone() {
        let r = rig(set_up());
        // The game's own settings (sandbox on) are not the client's.
        let own = game_settings(json!({"heroniteSandbox": {"enabled": true}, "heroniteKeepAwake": false}));
        let LaunchPlan::Wine { process, runtime, prefix } = plan_of(&r, &r.probe(), &own) else { panic!() };
        assert_eq!((runtime, prefix.as_str()), (WineRuntime::Umu, "/pfx/bnet"));
        let c = &process.command;
        assert_eq!(c.program, r.umu());
        assert_eq!(c.args, [BNET_EXE, "--exec=launch WoW"]);
        assert_eq!(c.env["PROTON_VERB"], "run");
        assert_eq!(c.env[UMU_NSENTER], "1");
        assert_eq!(c.env["WINEPREFIX"], "/pfx/bnet");
        assert_eq!(c.env["GAMEID"], "umu-0");
        assert!(process.wait_after_exit.is_none());
        assert_eq!(process.stop, StopSpec::default(), "stopping the game never touches the client or its prefix");
        assert!(!process.sandboxed);
        assert_eq!(process.prep.wine.as_ref().unwrap().wine_prefix, "/pfx/bnet");

        let h = process.handoff.unwrap();
        assert_eq!((h.prefix.as_deref(), h.folder.as_deref()), (Some("/pfx/bnet"), Some("/games/wow")));
        let client = h.client.unwrap();
        assert_eq!((client.name.as_str(), client.main.as_str()), ("Battle.net", "battle.net.exe"));
        assert!(client.programs.iter().any(|p| p == "battle.net.exe" || p == "agent.exe"));
        assert_eq!(client.join.program, r.umu());
        assert_eq!(client.join.args, [BNET_EXE, "--exec=launch WoW"]);
        assert_eq!(client.join.env["PROTON_VERB"], "runinprefix");
        assert_eq!(client.join.env[UMU_NSENTER], "1");
    }

    #[test]
    fn plain_proton_asks_for_run_and_joins_with_runinprefix() {
        let r = rig(json!({"winePrefix": "/pfx/bnet", "disableUMU": true, "wineVersion": {"bin": PROTON, "name": "GE", "type": "proton"}}));
        let LaunchPlan::Wine { process, runtime, .. } = plan_of(&r, &r.probe(), &game_settings(json!({}))) else { panic!() };
        assert_eq!(runtime, WineRuntime::Proton);
        assert_eq!(process.command.program, PROTON);
        assert_eq!(process.command.args, ["run", BNET_EXE, "--exec=launch WoW"]);
        assert_eq!(process.command.env["PROTON_VERB"], "run");
        assert!(!process.command.env.contains_key(UMU_NSENTER));
        let join = process.handoff.unwrap().client.unwrap().join;
        assert_eq!(join.args, ["runinprefix", BNET_EXE, "--exec=launch WoW"]);
    }

    #[test]
    fn plain_wine_is_not_waited_for_and_never_shut_down() {
        let r = rig(json!({"winePrefix": "/pfx/bnet", "wineVersion": {"bin": "/w/bin/wine", "name": "Wine", "type": "wine"}}));
        let wine = r.probe().with_path("/w/bin/wine").with_path("/w/bin/wineserver");
        let LaunchPlan::Wine { process, runtime, .. } = plan_of(&r, &wine, &game_settings(json!({}))) else { panic!() };
        assert_eq!(runtime, WineRuntime::Wine);
        assert!(process.wait_after_exit.is_none() && process.stop.commands.is_empty() && process.stop.kill_patterns.is_empty());
    }

    #[test]
    fn the_games_scripts_and_programs_still_come_with_it() {
        let r = rig(set_up());
        let own = game_settings(json!({
            "beforeLaunchScriptPath": "/s/before.sh", "heroniteKeepAwake": true,
            "heroniteCompanions": [{"path": "/t/trainer.exe", "delay": 5}],
            // Not the prefix the game runs in.
            "winePrefix": "/pfx/other"
        }));
        let probe = r.probe().with_bin("systemd-inhibit", "/usr/bin/systemd-inhibit").with_path("/t/trainer.exe");
        let LaunchPlan::Wine { process, .. } = plan_of(&r, &probe, &own) else { panic!() };
        assert_eq!(process.before_script.as_ref().unwrap().program, "/s/before.sh");
        // Linux holds logind's lock with a command, Windows asks the system itself.
        assert!(if cfg!(windows) { process.hold_awake } else { process.keep_awake.is_some() });
        assert_eq!(process.companions.len(), 1);
        let trainer = &process.companions[0].command;
        if cfg!(windows) {
            // Windows runs the program as it is.
            assert_eq!(trainer.program, "/t/trainer.exe");
            return;
        }
        assert_eq!(trainer.env["WINEPREFIX"], "/pfx/bnet", "beside the game, in the client's prefix");
        assert_eq!(trainer.env["PROTON_VERB"], "runinprefix");
    }

    #[test]
    fn a_client_that_is_not_set_up_cannot_start_games() {
        let (game, meta) = wow();
        let own = game_settings(json!({}));
        let msg = "Battle.net is not set up. Set it up in Stores → Battle.net.";
        // No settings file: no prefix.
        let none = rig(json!({}));
        let plan = plan_as(RunMode::Wine, &none.ctx, &none.probe(), &game, &meta, &own);
        assert_eq!(plan, LaunchPlan::Unavailable(msg.into()));
        // A prefix without the client in it.
        let r = rig(set_up());
        let empty = FakeProbe::new("/home/u").with_path(PROTON).with_path(&r.umu());
        assert_eq!(plan_of(&r, &empty, &own), LaunchPlan::Unavailable(msg.into()));
        // Nothing to ask the client.
        let bare = LocalGameMeta { launcher_args: None, ..meta.clone() };
        let plan = build_plan_with_settings(&r.ctx, &r.probe(), &game, Some(&bare), &own, &LaunchOptions::default());
        assert_eq!(plan, LaunchPlan::Unavailable("This game has no launch method".into()));
        let other = LocalGameMeta { source: "gog".into(), ..meta };
        let plan = build_plan_with_settings(&r.ctx, &r.probe(), &game, Some(&other), &own, &LaunchOptions::default());
        assert_eq!(plan, LaunchPlan::Unavailable("This game has no launch method".into()));
    }

    #[test]
    fn the_client_in_the_sandbox_is_refused_whatever_the_game_says() {
        let r = rig(json!({"winePrefix": "/pfx/bnet", "wineVersion": {"bin": PROTON, "name": "GE", "type": "proton"},
            "heroniteSandbox": {"enabled": true}}));
        let plan = plan_of(&r, &r.probe(), &game_settings(json!({"heroniteSandbox": {"enabled": false}})));
        assert_eq!(plan, LaunchPlan::Unavailable("The sandbox does not run Battle.net. Turn it off in Stores → Battle.net.".into()));
    }

    #[test]
    fn the_client_finds_its_program_in_either_prefix_layout() {
        let p = FakeProbe::new("/h").with_path("/w/drive_c/Program Files/Rockstar Games/Launcher/Launcher.exe");
        assert_eq!(
            client_exe(&p, Client::Rockstar, "/w").as_deref(),
            Some("/w/drive_c/Program Files/Rockstar Games/Launcher/Launcher.exe")
        );
        assert_eq!(client_exe(&p, Client::Ea, "/w"), None);
        let p = FakeProbe::new("/h").with_path(BNET_EXE);
        assert_eq!(client_exe(&p, Client::BattleNet, "/pfx/bnet").as_deref(), Some(BNET_EXE));
    }
}
