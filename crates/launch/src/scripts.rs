//! The scripts of the game settings run before the game starts and after it
//! exits (port of `runBeforeLaunchScript`, `runAfterLaunchScript` and
//! `runScriptForGame` in Heroic's `backend/launcher.ts`). Only builds the
//! commands; the runtime runs them.

use heronite_data::models::GameInfo;

use crate::env::Env;
use crate::plan::{Command, LaunchPlan};
use crate::probe::Probe;
use crate::settings::GameSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Before,
    After,
}

impl ScriptKind {
    fn label(self) -> &'static str {
        match self {
            ScriptKind::Before => "before",
            ScriptKind::After => "after",
        }
    }
}

/// What the script is told about the game. Heroic's `HEROIC_GAME_*` names,
/// so scripts written for Heroic keep working, plus the same values under
/// `HERONITE_GAME_*`.
fn script_env(game: &GameInfo, settings: &GameSettings, kind: ScriptKind) -> Env {
    let values = [
        ("APP_NAME", game.app_name.clone()),
        ("EXEC", game.install.executable.clone().unwrap_or_default()),
        ("PLATFORM", game.install.platform.clone().unwrap_or_default()),
        ("PREFIX", settings.wine_prefix.clone()),
        ("RUNNER", game.runner.clone()),
        ("SCRIPT_TYPE", kind.label().to_string()),
        ("TITLE", game.title.clone()),
    ];
    let mut env = Env::new();
    for (key, value) in values {
        env.insert(format!("HEROIC_GAME_{key}"), value.clone());
        env.insert(format!("HERONITE_GAME_{key}"), value);
    }
    env
}

/// The command for the script of `kind`, `None` when none is set. Runs in
/// the install folder, like Heroic's.
pub fn script_command(probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, kind: ScriptKind) -> Option<Command> {
    let path = match kind {
        ScriptKind::Before => &settings.before_launch_script,
        ScriptKind::After => &settings.after_launch_script,
    };
    if path.is_empty() {
        return None;
    }
    let cwd = game.install.install_path.clone().filter(|p| !p.is_empty() && probe.exists(p));
    Some(Command { program: path.clone(), args: vec![], env: script_env(game, settings, kind), cwd })
}

/// Adds the game's scripts to a plan that spawns a process.
pub fn attach(plan: &mut LaunchPlan, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings) {
    if let Some(process) = plan.process_mut() {
        process.before_script = script_command(probe, game, settings, ScriptKind::Before);
        process.after_script = script_command(probe, game, settings, ScriptKind::After);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::ProcessPlan;
    use crate::probe::FakeProbe;
    use serde_json::json;

    fn game() -> GameInfo {
        serde_json::from_value(json!({
            "runner": "sideload", "app_name": "g1", "title": "Game One",
            "install": {"executable": "/games/one/run.exe", "platform": "Windows", "install_path": "/games/one"}
        }))
        .unwrap()
    }

    fn settings(v: serde_json::Value) -> GameSettings {
        GameSettings::from_map(v.as_object().unwrap(), "/home/u")
    }

    #[test]
    fn scripts_get_heroics_variables_and_run_in_the_install_folder() {
        let s = settings(json!({"beforeLaunchScriptPath": " /s/before.sh ", "winePrefix": "/pfx"}));
        let probe = FakeProbe::new("/home/u").with_path("/games/one");
        let cmd = script_command(&probe, &game(), &s, ScriptKind::Before).unwrap();
        assert_eq!(cmd.program, "/s/before.sh");
        assert_eq!(cmd.cwd.as_deref(), Some("/games/one"));
        assert_eq!(cmd.env["HEROIC_GAME_APP_NAME"], "g1");
        assert_eq!(cmd.env["HEROIC_GAME_EXEC"], "/games/one/run.exe");
        assert_eq!(cmd.env["HEROIC_GAME_PLATFORM"], "Windows");
        assert_eq!(cmd.env["HEROIC_GAME_PREFIX"], "/pfx");
        assert_eq!(cmd.env["HEROIC_GAME_RUNNER"], "sideload");
        assert_eq!(cmd.env["HEROIC_GAME_SCRIPT_TYPE"], "before");
        assert_eq!(cmd.env["HERONITE_GAME_TITLE"], "Game One");
        assert!(script_command(&probe, &game(), &s, ScriptKind::After).is_none());
        // A missing install folder leaves the working directory alone.
        assert!(script_command(&FakeProbe::new("/home/u"), &game(), &s, ScriptKind::Before).unwrap().cwd.is_none());
    }

    #[test]
    fn only_process_plans_get_scripts() {
        let s = settings(json!({"beforeLaunchScriptPath": "/s/b", "afterLaunchScriptPath": "/s/a"}));
        let probe = FakeProbe::new("/home/u");
        let mut plan = LaunchPlan::Native(ProcessPlan::default());
        attach(&mut plan, &probe, &game(), &s);
        let p = plan.process().unwrap();
        assert_eq!(p.before_script.as_ref().unwrap().program, "/s/b");
        assert_eq!(p.after_script.as_ref().unwrap().env["HEROIC_GAME_SCRIPT_TYPE"], "after");
        let mut steam = LaunchPlan::SteamUri { app_id: "1".into(), uri: "steam://rungameid/1".into() };
        attach(&mut steam, &probe, &game(), &s);
        assert!(steam.process().is_none());
    }
}
