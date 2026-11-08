//! The section titles that are menus ("How Long To Beat"): which one is
//! pointed at and which is open.

use gpui_kit::*;

use super::focus::TitleAction;
use super::CollectionView;

impl CollectionView {
    pub(super) fn on_title_action(&mut self, action: TitleAction, cx: &mut Context<Self>) {
        let titles = self.facts.titles;
        self.facts.titles = match action {
            TitleAction::Hovered(menu, on) => titles.hovered(menu, on),
            TitleAction::Toggle(menu) => {
                let mut t = titles;
                t.open = if titles.open == Some(menu) { None } else { Some(menu) };
                t
            }
            TitleAction::Close => {
                let mut t = titles;
                t.open = None;
                t
            }
        };
        if self.facts.titles != titles {
            cx.notify();
        }
    }
}
