//! Console mode behaviour around the controller: navigation sounds, the idle
//! screen, and what happens at start or when a controller connects.

use gpui_kit::*;

use super::super::CollectionView;
use crate::console::Pad;

#[derive(Default)]
pub(super) struct ControlsState {}

impl CollectionView {
    /// At app start, before the first frame (console mode may be off).
    pub(super) fn console_startup(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Console mode just started, its theme installed.
    pub(super) fn console_controls_started(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Every press, before it is routed.
    pub(super) fn console_controls_press(&mut self, _pad: Pad, _cx: &mut Context<Self>) {}

    /// Drawn over everything when shown (the idle screen).
    pub(super) fn render_console_idle(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }
}
