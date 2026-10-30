//! What the focus panel opens outside the app: the install folder in the file
//! manager and the game's ProtonDB page in the browser (both via `xdg-open`,
//! the shell on Windows, off the UI thread).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::*;
use heronite_data::{CollectionGame, DataPaths};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

use super::CollectionView;
use heronite_i18n::tr;

pub(super) fn protondb_url(steam_app_id: &str) -> String {
    format!("https://www.protondb.com/app/{steam_app_id}")
}

/// What a folder row of the Details panel holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderKind {
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
pub(super) fn game_folders(paths: &DataPaths, game: &CollectionGame) -> Vec<(FolderKind, PathBuf)> {
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
        SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe).install_dir(steam.unwrap_or_default())
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

impl CollectionView {
    /// Opens a folder of `key` in the file manager (the context menu's "Open
    /// install folder": the ROM's folder for emulated games).
    pub(super) fn open_install_folder(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let paths = self.data_paths.clone();
        self.open_external(
            move || {
                let folders = game_folders(&paths, &game);
                let dir = folders
                    .iter()
                    .find(|(k, _)| *k != FolderKind::Emulator)
                    .or(folders.first())
                    .map(|(_, d)| d.clone())
                    .ok_or_else(|| tr!("The install folder was not found on this computer.").to_string())?;
                xdg_open(&dir)
            },
            cx,
        );
    }

    /// Opens `dir` (a folder found by [`game_folders`]) in the file manager.
    pub(super) fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.open_external(move || xdg_open(&dir), cx);
    }

    /// Plays a saved replay in the desktop's video player.
    pub(crate) fn open_replay(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_external(
            move || {
                if path.is_file() {
                    xdg_open(&path)
                } else {
                    Err(tr!("The replay is no longer at {path}", path = path.to_string_lossy()))
                }
            },
            cx,
        );
    }

    /// Opens the ProtonDB page of a Steam game in the browser.
    pub(super) fn open_protondb(&mut self, steam_app_id: &str, cx: &mut Context<Self>) {
        let url = protondb_url(steam_app_id);
        self.open_external(move || xdg_open(&url), cx);
    }

    /// Runs `open` off the UI thread; an error becomes a toast.
    fn open_external(&mut self, open: impl FnOnce() -> Result<(), String> + Send + 'static, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if let Err(err) = cx.background_spawn(async move { open() }).await {
                this.update(cx, |this, cx| this.show_toast(err, true, cx)).ok();
            }
        })
        .detach();
    }
}

fn xdg_open(target: impl AsRef<OsStr>) -> Result<(), String> {
    let target = target.as_ref();
    if cfg!(windows) {
        return heronite_platform::shell::open(target)
            .map_err(|err| tr!("Could not open {target}: {err}", target = target.to_string_lossy(), err));
    }
    match Command::new("xdg-open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(tr!("Could not open {target}: xdg-open exited with {s}", target = target.to_string_lossy(), s)),
        Err(err) => Err(tr!("Could not open {target}: {err}", target = target.to_string_lossy(), err)),
    }
}
