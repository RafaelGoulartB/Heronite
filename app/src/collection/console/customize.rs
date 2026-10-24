//! ☰ → Customize: the console theme's options as a side panel over the live
//! screen (each change shows at once behind it).

use gpui_kit::*;

use super::super::CollectionView;
use crate::console::{Hint, Pad};

/// The panel's focus.
#[derive(Default)]
pub(super) struct CustomizeNav {}

impl CollectionView {
    pub(super) fn open_console_customize(&mut self) {
        self.console.customize = Some(CustomizeNav::default());
    }

    pub(super) fn render_console_customize(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    /// `false` lets the shell close it on B.
    pub(super) fn customize_pad(&mut self, _pad: Pad, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        false
    }

    pub(super) fn customize_hints(&self) -> Vec<Hint> {
        Vec::new()
    }
}
