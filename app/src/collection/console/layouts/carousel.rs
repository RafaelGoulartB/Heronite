//! The carousel layout (not built yet: shows the shelves meanwhile).

use gpui_kit::*;
use heronite_data::console_theme::TabSpec;

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use crate::console::{Hint, Pad};

#[derive(Default)]
pub(crate) struct Nav {}

pub(super) const SCRIM: Scrim = Scrim::Classic;

impl CollectionView {
    pub(super) fn render_carousel(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.render_shelves(tab, spec, window, cx)
    }

    pub(super) fn carousel_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.shelves_pad(tab, spec, pad, window, cx)
    }

    pub(super) fn carousel_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        self.shelves_hints(tab, spec)
    }
}
