//! The section titles that are menus ("Details", "How Long To Beat"): which
//! one is pointed at and open, and "Edit rows…" of Details, a dialog of
//! switches for its rows (kept in `UiState::detail_rows_hidden`).

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::*;

use super::focus::TitleAction;
use super::vm::DetailField;
use super::CollectionView;
use crate::dialogs::detail_rows::DetailRowsDialog;

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
            TitleAction::EditDetails => {
                self.open_detail_rows(cx);
                titles
            }
        };
        if self.facts.titles != titles {
            cx.notify();
        }
    }

    /// Details → "Edit rows…": a switch per kind of row, applied and saved at
    /// once.
    fn open_detail_rows(&mut self, cx: &mut Context<Self>) {
        let rows = DetailField::ALL.iter().map(|f| (f.key(), SharedString::from(f.label()))).collect();
        let weak = cx.entity().downgrade();
        let on_change: Rc<dyn Fn(BTreeSet<String>, &mut App)> = Rc::new(move |hidden, cx| {
            weak.update(cx, |this, cx| {
                this.ui.detail_rows_hidden = hidden;
                this.save_ui();
                cx.notify();
            })
            .ok();
        });
        let hidden = self.ui.detail_rows_hidden.clone();
        let view = cx.new(|_| DetailRowsDialog::new(rows, hidden, on_change));
        self.open_dialog(view, cx);
    }
}
