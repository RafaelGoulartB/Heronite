//! Finding a game's `GameInfo` and Collection metadata on disk.

use heronite_data::json_store;
use heronite_data::models::{
    GameInfo, LocalGameMeta, LocalMetaFile, SideloadLibraryFile, StoreLibraryFile,
};
use heronite_data::DataPaths;

#[derive(Debug, Clone)]
pub struct LoadedGame {
    pub info: GameInfo,
    pub meta: Option<LocalGameMeta>,
}

/// Looks `app_name` up in the Collection metadata, then in the library of the
/// runner it names (sideload first when there is no metadata).
pub fn load_game(paths: &DataPaths, app_name: &str) -> Option<LoadedGame> {
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap_or_default();
    let meta = metas.games.get(app_name).cloned();
    let runners: Vec<String> = match &meta {
        Some(m) => vec![m.runner.clone()],
        None => vec![
            "sideload".into(),
            "legendary".into(),
            "gog".into(),
            "nile".into(),
        ],
    };
    for runner in runners {
        if let Some(info) = find_in_library(paths, &runner, app_name) {
            return Some(LoadedGame { info, meta });
        }
    }
    None
}

pub fn find_in_library(paths: &DataPaths, runner: &str, app_name: &str) -> Option<GameInfo> {
    match runner {
        "sideload" => {
            let lib: SideloadLibraryFile = json_store::read(&paths.sideload_library()).ok()?;
            lib.games.into_iter().find(|g| g.app_name == app_name)
        }
        "legendary" | "gog" | "nile" => {
            let path = match runner {
                "legendary" => paths.legendary_library(),
                "gog" => paths.gog_library(),
                _ => paths.nile_library(),
            };
            let lib: StoreLibraryFile = json_store::read(&path).ok()?;
            lib.library
                .into_iter()
                .chain(lib.games)
                .find(|g| g.app_name == app_name)
        }
        _ => None,
    }
}

/// Loads the game and builds its launch plan in one go. `None` if the game is
/// not in any library.
pub fn plan_for_app(
    ctx: &crate::plan::PlanContext,
    probe: &dyn crate::probe::Probe,
    app_name: &str,
    opts: &crate::plan::LaunchOptions,
) -> Option<crate::plan::LaunchPlan> {
    let game = load_game(&ctx.paths, app_name)?;
    Some(crate::plan::build_plan(
        ctx,
        probe,
        &game.info,
        game.meta.as_ref(),
        opts,
    ))
}
