//! The game's scripts and programs alongside, for a Steam game on Windows.
//!
//! Steam starts the game itself (`steam://rungameid/<id>`), exactly as from
//! its own library: nothing goes between Steam and the game. Heronite runs
//! the script before the game before it opens the URI, the programs
//! alongside once Steam's logs show the game running (closed when it ends),
//! and the script after the game once it is gone. A game started from Steam
//! itself gets none of them.
//!
//! Linux runs a Steam game's scripts through its launch options instead
//! (`heronite_steam::wrap`), and a program alongside would need Steam's
//! Proton prefix: there the plan stays bare.

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use heronite_platform::Feature;

use crate::plan::{CompanionPlan, LaunchPlan, SteamExtras};
use crate::probe::Probe;
use crate::scripts::{script_command, ScriptKind};
use crate::settings::GameSettings;
use crate::steam;

/// Whether Heronite runs them around Steam games here.
pub fn supported() -> bool {
    cfg!(windows) && heronite_platform::supported(Feature::LaunchExtras)
}

/// The plan that has Steam start `app_id`, with the game's extras.
pub fn plan(probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, app_id: &str) -> LaunchPlan {
    if steam::find_steam_binary(probe).is_none() {
        return LaunchPlan::Unavailable(tr!("Steam client was not found on PATH").into());
    }
    let mut plan =
        LaunchPlan::SteamUri { app_id: app_id.to_string(), uri: steam::steam_uri("rungameid", app_id), extras: SteamExtras::default() };
    attach(&mut plan, probe, game, settings);
    plan
}

/// Adds the game's scripts and programs to a Steam plan, where Heronite runs them.
pub fn attach(plan: &mut LaunchPlan, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    let LaunchPlan::SteamUri { extras, .. } = plan else { return };
    if !supported() {
        return;
    }
    *extras = SteamExtras {
        before_script: script_command(probe, game, settings, ScriptKind::Before),
        after_script: script_command(probe, game, settings, ScriptKind::After),
        companions: settings
            .companions
            .iter()
            .map(|c| CompanionPlan {
                path: c.path.clone(),
                command: crate::companions::direct_command(game, settings, &c.path),
                delay: std::time::Duration::from_secs(c.delay_secs.into()),
            })
            .collect(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use serde_json::json;

    fn game() -> GameInfo {
        serde_json::from_value(json!({
            "runner": "steam", "app_name": "steam_400", "title": "Portal",
            "install": {"install_path": "C:\\Steam\\steamapps\\common\\Portal"}
        }))
        .unwrap()
    }

    #[test]
    fn steam_starts_the_game_and_heronite_runs_what_is_around_it() {
        let settings = GameSettings::from_map(
            json!({
                "beforeLaunchScriptPath": "C:\\Scripts\\before.ps1",
                "afterLaunchScriptPath": "C:\\Scripts\\after.bat",
                "heroniteCompanions": [{"path": "C:\\Tools\\Trainer.exe", "delay": 15}],
            })
            .as_object()
            .unwrap(),
            "C:\\Users\\u",
        );
        // Not through `plan`: on Windows it looks for this machine's Steam.
        let probe = FakeProbe::new("C:\\Users\\u").with_path("C:\\Steam\\steamapps\\common\\Portal");
        let uri = steam::steam_uri("rungameid", "400");
        let mut plan = LaunchPlan::SteamUri { app_id: "400".into(), uri: uri.clone(), extras: SteamExtras::default() };
        attach(&mut plan, &probe, &game(), &settings);
        let LaunchPlan::SteamUri { uri: opened, extras, .. } = plan else { panic!() };
        assert_eq!(opened, uri, "Steam opens the game itself");
        if !supported() {
            assert_eq!(extras, SteamExtras::default(), "Linux runs them through Steam's launch options");
            return;
        }
        let before = extras.before_script.unwrap();
        assert_eq!(before.program, "C:\\Scripts\\before.ps1");
        assert_eq!(before.cwd.as_deref(), Some("C:\\Steam\\steamapps\\common\\Portal"));
        assert_eq!(before.env["HEROIC_GAME_TITLE"], "Portal");
        assert_eq!(extras.after_script.unwrap().env["HEROIC_GAME_SCRIPT_TYPE"], "after");
        let [trainer] = &extras.companions[..] else { panic!("{:?}", extras.companions) };
        assert_eq!((trainer.command.program.as_str(), trainer.command.args.len()), ("C:\\Tools\\Trainer.exe", 0));
        assert_eq!(trainer.delay, std::time::Duration::from_secs(15));
    }
}
