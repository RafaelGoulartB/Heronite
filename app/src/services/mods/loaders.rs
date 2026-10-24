//! Script extenders and mod loaders (SKSE, F4SE, ModEngine 2, SMAPI…): the
//! game must start through their executable. For Epic, GOG and Amazon games
//! and games added by hand that is the game's alternative executable
//! (`targetExe`); Steam starts the game itself, so the manager only shows the
//! line for its launch options.

use std::path::Path;

use gpui_kit::*;
use heronite_data::game_config::{game_overrides, set_game_setting};
use heronite_data::{CollectionGame, DataPaths};
use heronite_i18n::tr;
use heronite_mods::loaders::{self, Found};
use heronite_mods::{GameDirs, ModList};
use serde_json::Value;

use super::{LoaderCard, LoaderStart, ModsEvent, ModsService};

/// The key of the alternative executable in a game's settings.
const KEY: &str = "targetExe";

/// Epic's legendary takes the executable as a path inside the game's folder.
fn relative_target(game: &CollectionGame) -> bool {
    game.runner == "legendary"
}

fn current_target(paths: &DataPaths, app_name: &str) -> String {
    game_overrides(paths, app_name).get(KEY).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The loaders of the enabled mods and whether the game starts through them.
/// Blocking. Nothing for a Linux build (the Windows loaders do not apply) or
/// when the mods are not played with.
pub(super) fn cards(
    paths: &DataPaths,
    game: &CollectionGame,
    dirs: &GameDirs,
    list: &ModList,
    game_dir: &Path,
    through_steam: bool,
    native: bool,
) -> Vec<LoaderCard> {
    if native || !list.active {
        return Vec::new();
    }
    let found = loaders::find(dirs, list, game_dir);
    if found.is_empty() {
        return Vec::new();
    }
    let current = current_target(paths, &game.app_name);
    found.into_iter().map(|f| card(f, list, game_dir, through_steam, &current)).collect()
}

fn card(found: Found, list: &ModList, game_dir: &Path, through_steam: bool, current: &str) -> LoaderCard {
    let start = if through_steam {
        LoaderStart::Steam { line: loaders::steam_launch_line(found.loader) }
    } else if loaders::is_target(current, game_dir, &found.rel) {
        LoaderStart::On
    } else {
        LoaderStart::Off
    };
    LoaderCard {
        name: found.loader.name,
        file: found.rel.rsplit('/').next().unwrap_or(&found.rel).to_string(),
        mod_name: list.get(&found.mod_id).map(|m| m.name.clone()).unwrap_or(found.mod_id),
        rel: found.rel,
        needs_arguments: found.loader.needs_arguments,
        start,
    }
}

impl ModsService {
    /// Sets (or, with `on` false, puts back) the game's alternative
    /// executable. Games Steam starts have none: their line is only shown.
    pub(super) fn set_start_through(&mut self, game_key: &str, rel: &str, on: bool, cx: &mut Context<Self>) {
        let (Some(game), Some(source)) = (self.games.get(game_key), self.sources.get(game_key).cloned()) else { return };
        let Some(game_dir) = game.info.game_dir.clone() else { return };
        if game.info.through_steam {
            return;
        }
        let relative = relative_target(&source);
        let value = if on { loaders::target_exe(&game_dir, rel, relative) } else { self.previous_exe.remove(game_key).unwrap_or_default() };
        let (paths, key, app_name) = (self.data_paths.clone(), game_key.to_string(), source.app_name.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let before = current_target(&paths, &app_name);
                    let home = std::env::var("HOME").unwrap_or_default();
                    set_game_setting(&paths, &app_name, &home, KEY, Value::String(value)).map(|_| before).map_err(|e| format!("{e:#}"))
                })
                .await;
            this.update(cx, |s, cx| {
                match result {
                    Ok(before) => {
                        if on && !before.is_empty() {
                            s.previous_exe.entry(key.clone()).or_insert(before);
                        }
                    }
                    Err(e) => {
                        let message = tr!("Could not change the game's executable: {error}", error = e);
                        cx.emit(ModsEvent::Notice { game_key: key.clone(), message, error: true });
                    }
                }
                s.recheck(&key, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{card, loaders, LoaderStart};
    use heronite_mods::{GameDirs, ModList};
    use heronite_mods::{ModEntry, ModsRoot};

    fn setup() -> (tempfile::TempDir, GameDirs, ModList, std::path::PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let game = t.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("SkyrimSE.exe"), b"x").unwrap();
        std::fs::create_dir_all(dirs.mod_dir("skse")).unwrap();
        std::fs::write(dirs.mod_dir("skse").join("skse64_loader.exe"), b"x").unwrap();
        let mut list = ModList::new("g", "Skyrim");
        list.mods.push(ModEntry { id: "skse".into(), name: "SKSE64".into(), enabled: true, ..Default::default() });
        (t, dirs, list, game)
    }

    #[test]
    fn a_game_that_starts_through_the_loader_is_told_from_one_that_does_not() {
        let (_t, dirs, list, game_dir) = setup();
        let found = loaders::find(&dirs, &list, &game_dir);
        assert_eq!(found.len(), 1);
        let off = card(found[0].clone(), &list, &game_dir, false, "");
        assert_eq!(
            (off.name, off.file.as_str(), off.mod_name.as_str(), off.start.clone()),
            ("SKSE", "skse64_loader.exe", "SKSE64", LoaderStart::Off)
        );
        let on = card(found[0].clone(), &list, &game_dir, false, &game_dir.join("skse64_loader.exe").to_string_lossy());
        assert_eq!(on.start, LoaderStart::On);
        let steam = card(found[0].clone(), &list, &game_dir, true, "");
        let LoaderStart::Steam { line } = steam.start else { panic!() };
        assert!(line.unwrap().contains("SkyrimSELauncher.exe/skse64_loader.exe"));
    }
}
