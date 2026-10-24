//! What the mods need to know about a Collection game: its folder, its Wine
//! prefix and how it starts. Blocking (reads Steam's files and the game's
//! settings): call it off the UI thread.

use std::path::{Path, PathBuf};

use heronite_data::{CollectionGame, DataPaths, Library};
use heronite_i18n::tr;
use heronite_launch::native::{effective_platform, Platform};
use heronite_launch::settings::{load_game_settings, GlobalSettings};
use heronite_launch::SystemProbe;

use crate::services::game_folders::{game_folders, steam_locator, FolderKind};

#[derive(Debug, Clone, Default)]
pub(super) struct Facts {
    /// The folder the overlay mounts over.
    pub dir: Option<PathBuf>,
    /// The folder holding `drive_c`.
    pub prefix: Option<PathBuf>,
    pub through_steam: bool,
    pub native: bool,
}

pub(super) fn resolve(paths: &DataPaths, game: &CollectionGame) -> Facts {
    let through_steam = game.launch_kind.as_deref() == Some("steam-uri");
    let dir = game_folders(paths, game).into_iter().find(|(kind, _)| *kind == FolderKind::Install).map(|(_, dir)| dir);
    let steam_id = game.steam_app_id.as_deref().filter(|id| !id.is_empty());
    let prefix = match (through_steam, steam_id) {
        // Steam keeps the prefix beside the library the game is installed in.
        (true, Some(id)) => heronite_steam::files::prefix(&steam_locator(paths), id).map(|p| p.compatdata.join("pfx")),
        _ => wine_prefix(paths, game),
    };
    let native = is_native(game, through_steam, dir.as_deref(), prefix.as_deref());
    Facts { dir, prefix, through_steam, native }
}

/// The game's prefix from its settings: Proton keeps the files in `pfx`
/// inside the folder, umu and Wine use the folder itself.
fn wine_prefix(paths: &DataPaths, game: &CollectionGame) -> Option<PathBuf> {
    let settings = load_game_settings(paths, &game.app_name, &GlobalSettings::load(paths), &SystemProbe);
    let raw = settings.wine_prefix.trim();
    if raw.is_empty() {
        return None;
    }
    let base = PathBuf::from(raw);
    let nested = base.join("pfx");
    let proton_alone = settings.wine_version.kind == "proton" && settings.disable_umu == Some(true);
    if nested.join("drive_c").is_dir() || (!base.join("drive_c").is_dir() && proton_alone) {
        Some(nested)
    } else {
        Some(base)
    }
}

fn is_native(game: &CollectionGame, through_steam: bool, dir: Option<&Path>, prefix: Option<&Path>) -> bool {
    if game.game.is_linux_native == Some(true) {
        return true;
    }
    if through_steam {
        // A Windows game Steam never ran has no prefix either: tell by the files.
        return prefix.is_none_or(|p| !p.is_dir()) && dir.is_some_and(|d| !has_windows_files(d));
    }
    let exe = game.game.install.executable.as_deref().unwrap_or_default();
    effective_platform(&game.game, game.meta.as_deref(), exe, &SystemProbe) == Platform::Linux
}

/// An `.exe` or `.dll` in the folder or one level down.
fn has_windows_files(dir: &Path) -> bool {
    let windows = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll"));
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let path = e.path();
        if path.is_dir() {
            std::fs::read_dir(&path).into_iter().flatten().flatten().any(|c| windows(&c.path()))
        } else {
            windows(&path)
        }
    })
}

/// The game of the launch hook (`runner`, `app_name`), read from the library.
pub(super) fn find_game(paths: &DataPaths, key: &str) -> Result<CollectionGame, String> {
    let library = Library::load_with(paths, true).map_err(|e| tr!("Could not read the Collection: {error}", error = format!("{e:#}")))?;
    library.get(key).map(|g| (**g).clone()).ok_or_else(|| tr!("The game {key} is not in the Collection", key = key))
}

#[cfg(test)]
mod tests {
    use super::has_windows_files;

    #[test]
    fn windows_files_are_found_one_level_down() {
        let t = tempfile::tempdir().unwrap();
        assert!(!has_windows_files(t.path()));
        std::fs::create_dir_all(t.path().join("bin")).unwrap();
        std::fs::write(t.path().join("bin/Game.EXE"), b"x").unwrap();
        assert!(has_windows_files(t.path()));
        let native = tempfile::tempdir().unwrap();
        std::fs::write(native.path().join("game.x86_64"), b"x").unwrap();
        assert!(!has_windows_files(native.path()));
    }
}
