//! Long-lived services shared by pages, dialogs and the Collection. They are
//! GPUI entities kept in a global, created on first use.

pub mod compression;
pub mod downloads;
pub mod session;
pub mod widgets;

use gpui_kit::*;
use heronite_data::DataPaths;

pub use downloads::Downloads;

struct DownloadsGlobal(Entity<Downloads>);
impl Global for DownloadsGlobal {}

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

/// The Epic service if something already created it.
pub fn try_downloads(cx: &App) -> Option<Entity<Downloads>> {
    cx.try_global::<DownloadsGlobal>().map(|g| g.0.clone())
}
