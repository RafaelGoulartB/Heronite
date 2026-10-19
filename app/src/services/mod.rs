//! Long-lived services shared by pages, dialogs and the Collection. They are
//! GPUI entities kept in a global, created on first use.

pub mod compression;
pub mod downloads;
pub mod launch_blocker;
pub mod script_install;
pub mod session;
pub mod steam_library;
pub mod update;
pub mod upkeep;
pub mod widgets;

use gpui_kit::*;
use heronite_data::DataPaths;

pub use downloads::Downloads;

struct DownloadsGlobal(Entity<Downloads>);
impl Global for DownloadsGlobal {}

/// The Epic service when something started it already (read-only callers
/// that must not start it, e.g. the command palette's listing).
pub fn downloads_started(cx: &App) -> Option<Entity<Downloads>> {
    cx.try_global::<DownloadsGlobal>().map(|g| g.0.clone())
}

/// The Epic service: download queue, account, library refresh and game
/// operations (see [`Downloads`]).
pub fn downloads(cx: &mut App) -> Entity<Downloads> {
    if let Some(g) = cx.try_global::<DownloadsGlobal>() {
        return g.0.clone();
    }
    let data_paths = DataPaths::detect();
    let entity = cx.new(|cx| Downloads::new(data_paths, cx));
    cx.set_global(DownloadsGlobal(entity.clone()));
    entity
}

struct SteamLibraryGlobal(Entity<steam_library::SteamLibrary>);
impl Global for SteamLibraryGlobal {}

/// The Steam library service when something started it already.
pub fn steam_library_started(cx: &App) -> Option<Entity<steam_library::SteamLibrary>> {
    cx.try_global::<SteamLibraryGlobal>().map(|g| g.0.clone())
}

/// The Steam library service (created, and its timer started, on first use).
pub fn steam_library(cx: &mut App) -> Entity<steam_library::SteamLibrary> {
    if let Some(g) = cx.try_global::<SteamLibraryGlobal>() {
        return g.0.clone();
    }
    let entity = cx.new(|cx| steam_library::SteamLibrary::new(DataPaths::detect(), cx));
    cx.set_global(SteamLibraryGlobal(entity.clone()));
    entity
}

/// The Epic service if something already created it.
pub fn try_downloads(cx: &App) -> Option<Entity<Downloads>> {
    cx.try_global::<DownloadsGlobal>().map(|g| g.0.clone())
}
