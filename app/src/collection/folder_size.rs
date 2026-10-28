//! Size under a folder row of the Details panel for games whose store gives
//! none: Steam's manifest, the ROM files of an emulated game, the file of an
//! AppImage, else a walk of the game folder (`heronite_data::install_size`).
//! It loads after the other facts, a moment after the focus settles, and a
//! walk stops as soon as another game gets the focus.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use heronite_data::install_size::{folder_bytes, rom_bytes, walkable, SizeCache};
use heronite_data::{CollectionGame, DataPaths, Library};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

use super::open_links::FolderKind;
use super::CollectionView;

/// Wait before measuring, so moving through the list does not start a walk
/// for every game passed.
const SETTLE: Duration = Duration::from_millis(300);

impl CollectionView {
    /// Measures the focused game when its store gave no install size.
    pub(super) fn load_folder_size(&mut self, key: SharedString, game: Arc<CollectionGame>, cx: &mut Context<Self>) {
        if game.details.install_size.as_deref().is_some_and(|s| !s.is_empty() && s != "0") || self.facts.folders.is_empty() {
            return;
        }
        let generation = self.facts.size_generation.clone();
        let mine = generation.load(Ordering::Relaxed);
        let folders = self.facts.folders.clone();
        let (lib, paths, cache) = (self.lib.clone(), self.data_paths.clone(), self.app_paths.cache.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            if generation.load(Ordering::Relaxed) != mine {
                return;
            }
            let measured = cx
                .background_spawn(async move {
                    let stop = || generation.load(Ordering::Relaxed) != mine;
                    measure(&game, &folders, lib.as_deref(), &paths, &cache, &stop)
                })
                .await;
            let Some((kind, bytes)) = measured else { return };
            this.update(cx, |this, cx| {
                if this.facts.key() == Some(&key) {
                    this.facts.folder_size = Some((kind, heronite_epic::util::file_size_local(bytes as f64)));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// The size of the game and the folder row it goes under.
fn measure(
    game: &CollectionGame,
    folders: &[(FolderKind, PathBuf)],
    lib: Option<&Library>,
    paths: &DataPaths,
    cache: &Path,
    stop: &dyn Fn() -> bool,
) -> Option<(FolderKind, u64)> {
    let meta = game.meta.as_deref();
    if game.launch_kind.as_deref() == Some("emulator") {
        let roms: Vec<PathBuf> = meta
            .and_then(|m| m.selected_rom_path.as_deref())
            .into_iter()
            .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        return Some((FolderKind::Rom, rom_bytes(&roms))).filter(|(_, b)| *b > 0);
    }
    if game.launch_kind.as_deref() == Some("steam-uri") {
        let id = game.steam_app_id.as_deref()?;
        return SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe).size_on_disk(id).map(|b| (FolderKind::Install, b));
    }
    // An AppImage is the whole game; its folder may hold anything.
    if let Some(exe) = game.game.install.executable.as_deref().filter(|e| e.to_ascii_lowercase().ends_with(".appimage")) {
        return std::fs::metadata(exe).ok().filter(|m| m.is_file()).map(|m| (FolderKind::Install, m.len()));
    }
    let dir = folders.iter().find(|(k, _)| *k == FolderKind::Install).map(|(_, d)| d.as_path())?;
    let home = dirs::home_dir();
    if !walkable(dir, home.as_deref()) || holds_other_games(dir, game, lib) {
        return None;
    }
    if let Some(bytes) = SizeCache::load(cache).get(dir) {
        return Some((FolderKind::Install, bytes));
    }
    let bytes = folder_bytes(dir, stop)?;
    if let Err(err) = SizeCache::put(cache, dir, bytes) {
        log::warn!("saving the size of {}: {err:#}", dir.display());
    }
    Some((FolderKind::Install, bytes))
}

/// A folder that holds (or is) another game's folder is a games folder, not
/// this game's.
fn holds_other_games(dir: &Path, game: &CollectionGame, lib: Option<&Library>) -> bool {
    let Some(lib) = lib else { return false };
    lib.games.iter().filter(|g| g.key != game.key).any(|g| {
        [g.details.install_path.as_deref(), g.game.install.install_path.as_deref()]
            .into_iter()
            .flatten()
            .filter(|p| !p.trim().is_empty())
            .any(|p| Path::new(p.trim()).starts_with(dir))
    })
}

/// Bumped when the focus moves: a measure of the game before stops.
pub(super) type SizeGeneration = Arc<AtomicU64>;
