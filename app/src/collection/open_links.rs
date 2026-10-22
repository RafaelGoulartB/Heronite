//! What the focus panel opens outside the app: the install folder in the file
//! manager and the game's ProtonDB page in the browser (both via `xdg-open`,
//! off the UI thread).

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use gpui_kit::*;

use super::CollectionView;
pub(super) use crate::services::game_folders::{game_folders, FolderKind};
use heronite_i18n::tr;

pub(super) fn protondb_url(steam_app_id: &str) -> String {
    format!("https://www.protondb.com/app/{steam_app_id}")
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
    match Command::new("xdg-open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(tr!("Could not open {target}: xdg-open exited with {s}", target = target.to_string_lossy(), s)),
        Err(err) => Err(tr!("Could not open {target}: {err}", target = target.to_string_lossy(), err)),
    }
}
