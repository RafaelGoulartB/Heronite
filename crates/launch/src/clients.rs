//! Games a Windows store client starts (`heronite_data::clients`: Battle.net,
//! the EA app, Ubisoft Connect, the Rockstar Games Launcher).
//!
//! The client runs under Wine in a prefix of its own, with its own settings
//! (`GamesConfig/<config app>.json`), and starts, updates and signs in to its
//! games. The plan of such a game runs the client's program with
//! `launcherArgs` (what the client is asked to do), so the game's settings do
//! not apply. The session follows the game, not the client, which may have
//! been open for hours and stays open afterwards (see `runtime::client`).
//!
//! The sandbox does not apply: the game starts from inside the client, so the
//! client would have to run in it.

use heronite_data::clients::{prefix_of, Client};
use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use crate::env::join;
use crate::plan::{split_args, ClientHandoff, Handoff, LaunchPlan, PlanContext, StopSpec, WineRuntime};
use crate::probe::Probe;
use crate::settings::{load_game_settings, GameSettings};
use crate::wine_tools::{alongside_command, UMU_NSENTER};

/// The plan of a game the client named in `meta.source` starts.
pub(crate) fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, meta: &LocalGameMeta) -> LaunchPlan {
    let no_method = || LaunchPlan::Unavailable(tr!("This game has no launch method").into());
    let Some(client) = Client::from_id(&meta.source) else { return no_method() };
    let args = split_args(meta.launcher_args.as_deref().unwrap_or_default());
    if args.is_empty() {
        return no_method();
    }
    let not_set_up = || LaunchPlan::Unavailable(tr!("{client} is not set up. Set it up in Stores → {client}.", client = client.name()));
    let Some(prefix) = prefix_of(&ctx.paths, client, &probe.home()) else { return not_set_up() };
    let Some(exe) = client_exe(probe, client, &prefix.to_string_lossy()) else { return not_set_up() };

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
    process.handoff = Some(Handoff {
        prefix: Some(prefix.clone()),
        folder: game.install.install_path.clone().filter(|f| !f.is_empty() && probe.exists(f)),
        client: Some(ClientHandoff {
            name: client.name().to_string(),
            programs: client.programs().iter().map(|p| p.to_string()).collect(),
            main: client.main_program().to_string(),
            join,
        }),
    });
    LaunchPlan::Wine { process, runtime, prefix }
}

/// Adds the game's programs to the plan. They run beside the game, so with
/// the client's Wine and prefix, not the game's own.
pub(crate) fn attach_companions(plan: &mut LaunchPlan, ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    let Some(wine) = plan.process().and_then(|p| p.prep.wine.clone()) else { return };
    let settings = GameSettings { companions: settings.companions.clone(), ..wine };
    crate::companions::attach(plan, ctx, probe, game, &settings);
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

    fn plan_of(r: &Rig, probe: &FakeProbe, game_settings: &GameSettings) -> LaunchPlan {
        let (game, meta) = wow();
        build_plan_with_settings(&r.ctx, probe, &game, Some(&meta), game_settings, &LaunchOptions::default())
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
        let plan = build_plan_with_settings(&none.ctx, &none.probe(), &game, Some(&meta), &own, &LaunchOptions::default());
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
