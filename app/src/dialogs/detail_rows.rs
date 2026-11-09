//! Details → "Edit rows…": a switch per kind of row of the focus panel's
//! "Details". Each change is applied (and saved by the Collection) at once.

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::DialogEvent;
use crate::ui::controls::{button, checkbox, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};

pub struct DetailRowsDialog {
    /// Key and name of every kind of row, in the order Details shows them.
    rows: Vec<(&'static str, SharedString)>,
    /// Keys of the rows turned off.
    hidden: BTreeSet<String>,
    on_change: Rc<dyn Fn(BTreeSet<String>, &mut App)>,
}

impl EventEmitter<DialogEvent> for DetailRowsDialog {}

impl DetailRowsDialog {
    pub fn new(
        rows: Vec<(&'static str, SharedString)>,
        hidden: BTreeSet<String>,
        on_change: Rc<dyn Fn(BTreeSet<String>, &mut App)>,
    ) -> Self {
        Self { rows, hidden, on_change }
    }

    fn toggle(&mut self, key: &'static str, cx: &mut Context<Self>) {
        if !self.hidden.remove(key) {
            self.hidden.insert(key.to_owned());
        }
        (self.on_change)(self.hidden.clone(), cx);
        cx.notify();
    }
}

impl Render for DetailRowsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col();
        for (key, label) in &self.rows {
            let key = *key;
            list = list.child(
                checkbox(SharedString::from(format!("detail-row-{key}")), !self.hidden.contains(key), label.clone())
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(key, cx))),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(help_text(tr!("Pick the rows Details shows. A row a game has no value for stays out anyway.")))
            .child(list);
        let footer = div().flex().child(
            button("detail-rows-done", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
        );
        let entity = cx.entity().downgrade();
        let vh = f32::from(crate::ui::content_size(window).height);
        dialog(
            DialogProps { id: "detail-rows".into(), title: tr!("Details rows").into(), width: 460., height: Some(560f32.min(vh * 0.75)) },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
