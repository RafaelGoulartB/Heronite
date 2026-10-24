//! Art console mode uses beyond covers and the wide art: game logos.
//! Built only while console mode is on.

use gpui_kit::*;

use super::super::CollectionView;

/// Logo sources found per game (filled lazily, dropped with console mode).
#[derive(Default)]
pub(super) struct LogoCache {}

impl CollectionView {
    /// The game's logo fitted inside `w`×`h` design px (transparent around
    /// it, never cropped); `None` while it loads or when the game has none
    /// (callers then show the name as text).
    pub(super) fn console_logo(&mut self, _key: &SharedString, _w: f32, _h: f32, _window: &Window) -> Option<ImageSource> {
        None
    }
}
