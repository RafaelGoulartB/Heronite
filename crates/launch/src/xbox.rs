//! Microsoft Store and PC Game Pass games (`heronite_data::xbox`): Windows
//! starts them from their package, by the application's id (`launcherArgs`
//! holds the AUMID, `<family name>!<app id>`).
//!
//! `explorer.exe shell:AppsFolder\<AUMID>` asks the shell to start the
//! application and returns at once (with exit code 1, whatever happened),
//! so the session follows the programs running from the package's folder
//! (a GDK game starts through `gamelaunchhelper.exe` there, which starts the
//! game). Stopping ends those programs.

use heronite_data::models::{GameInfo, LocalGameMeta};
use heronite_i18n::tr;

use crate::plan::{Command, Handoff, LaunchPlan, ProcessPlan};
use crate::probe::Probe;

/// The plan of a game the Xbox app installed.
pub(crate) fn build(probe: &dyn Probe, game: &GameInfo, meta: &LocalGameMeta) -> LaunchPlan {
    if !cfg!(windows) {
        return LaunchPlan::Unavailable(tr!("Microsoft Store games run on Windows only").into());
    }
    let Some(aumid) = meta.launcher_args.as_deref().map(str::trim).filter(|a| valid_aumid(a)) else {
        return LaunchPlan::Unavailable(tr!("This game has no launch method").into());
    };
    let folder = game.install.install_path.clone().filter(|f| !f.is_empty() && probe.exists(f));
    LaunchPlan::Native(ProcessPlan {
        command: Command { program: "explorer.exe".into(), args: vec![format!(r"shell:AppsFolder\{aumid}")], ..Default::default() },
        // Without the folder there is nothing to follow: the session ends
        // with the request.
        handoff: folder.is_some().then(|| Handoff { folder, exit_ignored: true, ..Default::default() }),
        notes: vec![format!("Windows starts the game from its package ({aumid})")],
        ..Default::default()
    })
}

/// `<family name>!<app id>`: package names, publisher ids and application
/// ids are made of ASCII letters, digits, `.`, `-` and `_` only, so nothing
/// in it means anything else to the shell.
fn valid_aumid(aumid: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    matches!(aumid.split_once('!'), Some((family, app)) if ok(family) && ok(app) && family.contains('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan_with_settings, LaunchOptions, PlanContext};
    use crate::probe::FakeProbe;
    use crate::settings::{GameSettings, GlobalSettings};
    use heronite_data::DataPaths;

    #[test]
    fn aumids_are_checked() {
        assert!(valid_aumid("Microsoft.624F8B84B80_8wekyb3d8bbwe!Game"));
        assert!(valid_aumid("Studio.Game-2_abc123!App.Main"));
        assert!(!valid_aumid("Microsoft.624F8B84B80_8wekyb3d8bbwe"));
        assert!(!valid_aumid("NoPublisher!Game"));
        assert!(!valid_aumid("Game_abc!App & calc"));
        assert!(!valid_aumid("Game_abc!App\" x"));
        assert!(!valid_aumid("!App"));
    }

    fn game(folder: &str, aumid: &str) -> (GameInfo, LocalGameMeta) {
        let mut game = GameInfo { runner: "sideload".into(), app_name: "xbox_game_abc".into(), title: "Game".into(), ..Default::default() };
        game.install.install_path = Some(folder.into());
        game.install.platform = Some("windows".into());
        let meta = LocalGameMeta {
            app_name: "xbox_game_abc".into(),
            runner: "sideload".into(),
            source: "xbox".into(),
            launch_kind: heronite_data::xbox::LAUNCH_KIND.into(),
            launcher_args: Some(aumid.into()),
            ..Default::default()
        };
        (game, meta)
    }

    fn plan(probe: &FakeProbe, folder: &str, aumid: &str) -> LaunchPlan {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = PlanContext::load(&DataPaths::at(dir.path()));
        ctx.global = GlobalSettings::from_defaults(Default::default());
        let (game, meta) = game(folder, aumid);
        let settings = GameSettings::from_map(&Default::default(), "/h");
        build_plan_with_settings(&ctx, probe, &game, Some(&meta), &settings, &LaunchOptions::default())
    }

    #[cfg(windows)]
    #[test]
    fn the_shell_starts_the_package_and_the_session_follows_its_folder() {
        let folder = r"C:\XboxGames\Game\Content";
        let probe = FakeProbe::new("/h").with_path(folder);
        let LaunchPlan::Native(p) = plan(&probe, folder, "Studio.Game_abc!Game") else { panic!("not a native plan") };
        assert_eq!(p.command.program, "explorer.exe");
        assert_eq!(p.command.args, [r"shell:AppsFolder\Studio.Game_abc!Game"]);
        let h = p.handoff.unwrap();
        assert_eq!(h.folder.as_deref(), Some(folder));
        assert!(h.exit_ignored && h.client.is_none() && h.prefix.is_none());

        // A folder that is gone: the request still goes, nothing to follow.
        let LaunchPlan::Native(p) = plan(&FakeProbe::new("/h"), folder, "Studio.Game_abc!Game") else { panic!() };
        assert!(p.handoff.is_none());
        assert!(matches!(plan(&probe, folder, "Studio.Game_abc"), LaunchPlan::Unavailable(_)));
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_cannot_start_them() {
        let probe = FakeProbe::new("/h").with_path("/games/x");
        assert!(matches!(plan(&probe, "/games/x", "Studio.Game_abc!Game"), LaunchPlan::Unavailable(_)));
    }
}
