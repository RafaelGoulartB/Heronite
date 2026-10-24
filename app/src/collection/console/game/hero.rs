//! The game page as Hero: the art full screen with the logo, the actions as a row of buttons and tabs (LB / RB) for Overview, Details, Screenshots, Journal (not built yet: shows the Classic page meanwhile).

use gpui_kit::*;

use super::super::super::CollectionView;
use crate::console::{Hint, Pad};

#[derive(Default)]
pub(super) struct Nav {}

impl CollectionView {
    pub(super) fn render_game_hero(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.render_game_classic(key, window, cx)
    }

    pub(super) fn game_pad_hero(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.game_pad_classic(key, pad, window, cx)
    }

    pub(super) fn game_hints_hero(&self, key: &SharedString) -> Vec<Hint> {
        self.game_hints_classic(key)
    }
}
