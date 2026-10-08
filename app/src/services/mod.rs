//! Long-lived services shared by pages, dialogs and the Collection. They are
//! GPUI entities kept in a global, created on first use.

pub mod downloads;
pub mod widgets;

use gpui_kit::*;
use heronite_data::HeroicPaths;

pub use downloads::{Downloads, DownloadsEvent};

struct DownloadsGlobal(Entity<Downloads>);
impl Global for DownloadsGlobal {}

/// The Epic service: download queue, account, library refresh and game
/// operations (see [`Downloads`]).
pub fn downloads(cx: &mut App) -> Entity<Downloads> {
    if let Some(g) = cx.try_global::<DownloadsGlobal>() {
        return g.0.clone();
    }
    let heroic = HeroicPaths::detect();
    let entity = cx.new(|cx| Downloads::new(heroic, cx));
    cx.set_global(DownloadsGlobal(entity.clone()));
    entity
}
