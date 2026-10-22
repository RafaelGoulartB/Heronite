//! Where a game's files are on this computer: the folders the Details panel
//! opens and the Mods service mounts over.

use std::path::{Path, PathBuf};

use heronite_data::{CollectionGame, DataPaths};
use heronite_i18n::tr;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

/// The Steam install the settings point at (read fresh: the Steam folder setting may change).
pub(crate) fn steam_locator(paths: &DataPaths) -> SteamLocator {
    SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe)
}

/// What a folder row of the Details panel holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderKind {
    Install,
    Emulator,
    Rom,
}

impl FolderKind {
    pub fn label(self) -> &'static str {
        match self {
            FolderKind::Install => tr!("Installation Folder"),
            FolderKind::Emulator => tr!("Emulator Folder"),
            FolderKind::Rom => tr!("ROM Folder"),
        }
    }
}

/// The folders of `game` that exist on this computer. Emulated games: the
/// emulator's folder and the ROM's. Steam games: the folder Steam installed
/// them in. Others: the stored install path. Paths imported from Playnite are
/// often Windows paths (`C:\...`) that do not exist here; those are left out.
pub(crate) fn game_folders(paths: &DataPaths, game: &CollectionGame) -> Vec<(FolderKind, PathBuf)> {
    let meta = game.meta.as_deref();
    if game.launch_kind.as_deref() == Some("emulator") {
        let emulator = [meta.and_then(|m| m.remapped_executable.as_deref()), game.game.install.executable.as_deref()]
            .into_iter()
            .flatten()
            .find_map(local_dir);
        let rom = meta
            .and_then(|m| m.selected_rom_path.as_deref())
            .into_iter()
            .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
            .chain(game.game.folder_name.as_deref())
            .find_map(local_dir);
        return [emulator.map(|p| (FolderKind::Emulator, p)), rom.map(|p| (FolderKind::Rom, p))].into_iter().flatten().collect();
    }
    let steam = game.steam_app_id.as_deref().filter(|s| !s.is_empty());
    let dir = if game.launch_kind.as_deref() == Some("steam-uri") && steam.is_some() {
        steam_locator(paths).install_dir(steam.unwrap_or_default())
    } else {
        [game.details.install_path.as_deref(), game.game.install.install_path.as_deref(), game.game.folder_name.as_deref()]
            .into_iter()
            .flatten()
            .find_map(local_dir)
    };
    dir.map(|d| vec![(FolderKind::Install, d)]).unwrap_or_default()
}

/// An absolute path that exists here, as a folder (a file gives its parent).
fn local_dir(raw: &str) -> Option<PathBuf> {
    let p = Path::new(raw.trim());
    if !p.is_absolute() {
        return None;
    }
    if p.is_dir() {
        Some(p.to_path_buf())
    } else if p.is_file() {
        p.parent().map(Path::to_path_buf)
    } else {
        None
    }
}
