//! A game's own actions (`heronite_data::game_actions`) as something to
//! start. Builds only; the app opens, spawns or launches the result.
//!
//! An action that counts as playing is launched like a local game that
//! starts this program or command instead of the game's: the plan gets the
//! game's settings (Wine and prefix, environment, wrappers, MangoHud, the FPS
//! limit, gamescope, scripts and programs alongside) and the runtime tracks it
//! as the game's session. Other actions are plain processes: a Linux program
//! or a command with the game's `HEROIC_GAME_*` variables, a Windows program
//! in the game's prefix with its Wine.

use heronite_data::game_actions::{Action, ActionKind};
use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;
use serde_json::Value;

use crate::emulators::node_dirname;
use crate::native::{detect_executable_kind, ExecutableKind};
use crate::plan::{build_plan_with_settings, Command, LaunchOptions, LaunchPlan, PlanContext};
use crate::probe::Probe;
use crate::scripts::game_env;
use crate::settings::GameSettings;

/// What starting an action means.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionRun {
    /// Open with the desktop's default app (`xdg-open`).
    Open(String),
    /// A process nobody waits for.
    Spawn(Command),
    /// Launched as the game's session.
    Play(Box<LaunchPlan>),
}

/// The folder a command runs in: the game's, else the home folder.
pub fn game_folder(game: &GameInfo, probe: &dyn Probe) -> String {
    [game.install.install_path.as_deref(), game.folder_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|f| !f.is_empty() && probe.exists(f))
        .map(str::to_string)
        .unwrap_or_else(|| probe.home())
}

/// `settings`: the game's effective settings; a Steam game's Windows programs
/// need Steam's prefix and Proton in them (see the app's runtime).
pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, action: &Action) -> Result<ActionRun, String> {
    let target = action.target.trim();
    if target.is_empty() {
        return Err(tr!("This action has nothing to start.").into());
    }
    if action.kind == ActionKind::Open {
        return Ok(ActionRun::Open(target.to_string()));
    }
    if action.kind == ActionKind::Program && !probe.exists(target) {
        return Err(tr!("File not found: {exe}", exe = target));
    }
    if action.plays() {
        return play_plan(ctx, probe, game, settings, action).map(|p| ActionRun::Play(Box::new(p)));
    }
    let args = crate::plan::split_args(&action.args);
    match action.kind {
        ActionKind::Program if detect_executable_kind(target, probe) == ExecutableKind::Windows => {
            crate::wine_tools::run_exe_command(ctx, probe, settings, target, &args).map(|c| ActionRun::Spawn(c.command))
        }
        ActionKind::Program => Ok(ActionRun::Spawn(Command {
            program: target.to_string(),
            args,
            env: game_env(game, settings, &[]),
            cwd: Some(node_dirname(target)),
        })),
        _ => Ok(ActionRun::Spawn(Command {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), target.to_string()],
            env: game_env(game, settings, &[]),
            cwd: Some(game_folder(game, probe)),
        })),
    }
}

/// The plan of a local game that starts the action: its program (Linux or
/// Windows, from the file) or its command in the game's folder.
fn play_plan(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    action: &Action,
) -> Result<LaunchPlan, String> {
    let mut info = game.clone();
    info.runner = "sideload".into();
    info.is_installed = true;
    info.rest.shift_remove(heronite_data::sideload::COMMAND_KEY);
    let target = action.target.trim().to_string();
    match action.kind {
        ActionKind::Command => {
            info.install.install_path = Some(game_folder(game, probe));
            info.rest.insert(heronite_data::sideload::COMMAND_KEY.into(), Value::String(target));
        }
        _ => {
            info.install.install_path = Some(node_dirname(&target));
            info.install.executable = Some(target);
            info.install.platform = None;
            info.is_linux_native = None;
        }
    }
    let meta = LocalGameMeta { launch_kind: "executable".into(), run_as: Some("auto".into()), ..Default::default() };
    let mut s = settings.clone();
    s.target_exe.clear();
    s.launcher_args = action.args.clone();
    match build_plan_with_settings(ctx, probe, &info, Some(&meta), &s, &LaunchOptions::default()) {
        LaunchPlan::Unavailable(why) => Err(why),
        plan => Ok(plan),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use heronite_data::DataPaths;
    use serde_json::json;

    fn game() -> GameInfo {
        serde_json::from_value(json!({
            "runner": "legendary", "app_name": "Calluna", "title": "Control", "is_installed": true,
            "install": {"executable": "Control.exe", "platform": "Windows", "install_path": "/games/control"}
        }))
        .unwrap()
    }

    fn ctx() -> (tempfile::TempDir, PlanContext) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = PlanContext::load(&DataPaths::at(dir.path()));
        (dir, ctx)
    }

    fn action(kind: ActionKind, target: &str) -> Action {
        Action { id: "1".into(), name: "A".into(), kind, target: target.into(), ..Default::default() }
    }

    #[test]
    fn links_open_and_commands_run_in_the_game_folder() {
        let (_d, ctx) = ctx();
        let probe = FakeProbe::new("/home/u").with_path("/games/control");
        let s = GameSettings::default();
        let open = build(&ctx, &probe, &game(), &s, &action(ActionKind::Open, " https://wiki.example "));
        assert_eq!(open, Ok(ActionRun::Open("https://wiki.example".into())));
        let Ok(ActionRun::Spawn(c)) = build(&ctx, &probe, &game(), &s, &action(ActionKind::Command, "./tool.sh --x")) else { panic!() };
        assert_eq!(
            (c.program.as_str(), c.args.clone(), c.cwd.as_deref()),
            ("/bin/sh", vec!["-c".to_string(), "./tool.sh --x".into()], Some("/games/control"))
        );
        assert_eq!(c.env["HERONITE_GAME_APP_NAME"], "Calluna");
        // Without the game's folder, the home folder.
        let Ok(ActionRun::Spawn(c)) = build(&ctx, &FakeProbe::new("/home/u"), &game(), &s, &action(ActionKind::Command, "ls")) else {
            panic!()
        };
        assert_eq!(c.cwd.as_deref(), Some("/home/u"));
    }

    #[test]
    fn a_linux_program_runs_from_its_folder_with_its_arguments() {
        let (_d, ctx) = ctx();
        let probe = FakeProbe::new("/h").with_path("/tools/mods.sh");
        let a = Action { args: "--profile 'My mods'".into(), ..action(ActionKind::Program, "/tools/mods.sh") };
        let Ok(ActionRun::Spawn(c)) = build(&ctx, &probe, &game(), &GameSettings::default(), &a) else { panic!() };
        assert_eq!(
            (c.program.as_str(), c.args.clone(), c.cwd.as_deref()),
            ("/tools/mods.sh", vec!["--profile".to_string(), "My mods".into()], Some("/tools"))
        );
        let missing = build(&ctx, &FakeProbe::new("/h"), &game(), &GameSettings::default(), &a);
        assert!(missing.unwrap_err().contains("/tools/mods.sh"));
    }

    #[test]
    fn playing_starts_the_program_like_the_game() {
        let (_d, ctx) = ctx();
        let probe = FakeProbe::new("/h").with_path("/tools/mods.sh").with_bin("mangohud", "/usr/bin/mangohud");
        let s = GameSettings { show_mangohud: true, launcher_args: "-game-args".into(), ..Default::default() };
        let a = Action { counts_as_playing: true, args: "--go".into(), ..action(ActionKind::Program, "/tools/mods.sh") };
        let Ok(ActionRun::Play(plan)) = build(&ctx, &probe, &game(), &s, &a) else { panic!() };
        let LaunchPlan::Native(p) = *plan else { panic!() };
        // The game's wrappers, the action's arguments (not the game's).
        assert_eq!(p.command.program, "/usr/bin/mangohud");
        assert!(p.command.args.ends_with(&["/tools/mods.sh".to_string(), "--go".into()]), "{:?}", p.command.args);
    }
}
