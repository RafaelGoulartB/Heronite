//! The console Achievements screen: a game's achievements over its art,
//! full screen, with filters (LB / RB), a sort (Y) and a detail card (A).

use gpui_kit::*;

use super::super::CollectionView;

/// The screen's state while it is open.
#[derive(Default)]
pub(super) struct AchNav {
    /// The game whose achievements show.
    pub key: SharedString,
}

impl CollectionView {
    /// Opens the Achievements screen of the game `key` (over the current screen;
    /// B goes back to it).
    pub(in crate::collection) fn open_console_achievements(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.console.achievements = Some(AchNav { key });
        cx.notify();
    }
}
