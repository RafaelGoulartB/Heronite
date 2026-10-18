//! The installed games and their size: the store's number, Steam's manifest,
//! the ROM files of an emulated game, an AppImage's file, a size walked
//! before (`install-sizes.json`), else a walk of the game folder later.

use std::path::{Path, PathBuf};

use heronite_data::install_size::{parse_store_size, rom_bytes, walkable, SizeCache};
use heronite_data::{CollectionGame, Library};
use heronite_launch::steam::SteamLocator;

use crate::{Group, Item, Kind, State};

pub(crate) fn items(lib: &Library, steam: &SteamLocator, walked: &SizeCache) -> Vec<Item> {
    let home = dirs::home_dir();
    let mut out = Vec::new();
    for g in lib.games.iter().filter(|g| g.installed && g.game.install.is_dlc != Some(true)) {
        let Some((path, size, walk)) = locate(g, lib, steam, walked, home.as_deref()) else { continue };
        let mut item = Item::new(Group::Games, Kind::Game { key: g.key.clone() }, State::InUse, g.title.clone(), path);
        item.size = size;
        item.needs_walk = walk && size.is_none();
        item.games = vec![g.key.clone()];
        item.source = Some(g.source_label.to_string());
        item.last_played = g.last_played.clone();
        out.push(item);
    }
    out
}

/// The game's folder (a ROM's folder, an AppImage), its size when known
/// without walking, and whether the folder may be walked.
fn locate(
    g: &CollectionGame,
    lib: &Library,
    steam: &SteamLocator,
    walked: &SizeCache,
    home: Option<&Path>,
) -> Option<(PathBuf, Option<u64>, bool)> {
    if g.launch_kind.as_deref() == Some("emulator") {
        let roms = rom_paths(g);
        let folder = roms.first()?.parent()?.to_path_buf();
        return Some((folder, Some(rom_bytes(&roms)).filter(|b| *b > 0), false));
    }
    if g.launch_kind.as_deref() == Some("steam-uri") {
        let id = g.steam_launch_id.as_deref().or(g.steam_app_id.as_deref())?;
        let dir = steam.install_dir(id)?;
        return Some((dir, steam.size_on_disk(id), true));
    }
    if let Some(exe) = g.game.install.executable.as_deref().filter(|e| e.to_ascii_lowercase().ends_with(".appimage")) {
        let meta = std::fs::metadata(exe).ok().filter(|m| m.is_file())?;
        return Some((PathBuf::from(exe), Some(meta.len()), false));
    }
    let dir = install_dir(g)?;
    let store = g.details.install_size.as_deref().and_then(parse_store_size);
    let size = store.or_else(|| walked.get(&dir));
    let walk = walkable(&dir, home) && !holds_other_games(&dir, g, lib);
    Some((dir, size, walk))
}

fn rom_paths(g: &CollectionGame) -> Vec<PathBuf> {
    let meta = g.meta.as_deref();
    meta.and_then(|m| m.selected_rom_path.as_deref())
        .into_iter()
        .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

/// The game's install folder, when it exists.
pub(crate) fn install_dir(g: &CollectionGame) -> Option<PathBuf> {
    [g.details.install_path.as_deref(), g.game.install.install_path.as_deref()]
        .into_iter()
        .flatten()
        .map(|p| PathBuf::from(p.trim()))
        .find(|p| p.is_absolute() && p.is_dir())
}

/// A folder that holds (or is) another game's folder is a games folder, not
/// this game's.
fn holds_other_games(dir: &Path, game: &CollectionGame, lib: &Library) -> bool {
    lib.games.iter().filter(|g| g.key != game.key).any(|g| {
        [g.details.install_path.as_deref(), g.game.install.install_path.as_deref()]
            .into_iter()
            .flatten()
            .filter(|p| !p.trim().is_empty())
            .any(|p| Path::new(p.trim()).starts_with(dir))
    })
}
