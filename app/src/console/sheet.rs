//! Modal sheet for console mode: a title, an optional message and console
//! rows (confirmations, small editors such as the playtime). Values change
//! in place; the owner hears [`SheetEvent`]s and reads [`Sheet::value`].
//! B closes it (the host drops the overlay).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::kit::{self, u};
use super::list::{ListNav, Outcome, Row, Value};
use super::{hint, Btn, Hint, Pad, PadTarget};
use crate::theme;

pub enum SheetEvent {
    Activate(SharedString),
}

pub struct Sheet {
    title: SharedString,
    message: Option<SharedString>,
    rows: Vec<Row>,
    nav: ListNav,
}

impl EventEmitter<SheetEvent> for Sheet {}

impl Sheet {
    pub fn new(title: impl Into<SharedString>, message: Option<String>, rows: Vec<Row>) -> Self {
        Self { title: title.into(), message: message.map(Into::into), rows, nav: ListNav::default() }
    }

    pub fn value(&self, id: &str) -> Option<&Value> {
        self.rows.iter().find(|r| r.id.as_ref() == id).map(|r| &r.value)
    }

    pub fn number(&self, id: &str) -> i64 {
        match self.value(id) {
            Some(Value::Number { value, .. }) => *value,
            _ => 0,
        }
    }

    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.rows = rows;
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message.map(Into::into);
    }
}

impl PadTarget for Sheet {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.nav.on_pad(pad, &self.rows) {
            Outcome::Activate(id) => cx.emit(SheetEvent::Activate(id)),
            Outcome::Changed(id, value) => {
                if let Some(r) = self.rows.iter_mut().find(|r| r.id == id) {
                    r.value = value;
                }
            }
            Outcome::Moved | Outcome::Edge(_) => {}
            Outcome::Ignored => return false,
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        let changes = self.rows.iter().any(|r| matches!(r.value, Value::Toggle(_) | Value::Choice { .. } | Value::Number { .. }));
        let mut h = vec![hint(Btn::A, "Select")];
        if changes {
            h.push(hint(Btn::LeftRight, "Change"));
        }
        h.push(hint(Btn::B, "Cancel"));
        h
    }
}

impl Render for Sheet {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let list = self.nav.render("console-sheet", &self.rows, true);
        div().absolute().inset_0().child(kit::scrim()).child(
            div().absolute().inset_0().flex().items_center().justify_center().child(kit::panel_in(
                "sheet-panel",
                div()
                    .w(u(640.))
                    .max_h(u(620.))
                    .flex()
                    .flex_col()
                    .gap(u(14.))
                    .p(u(26.))
                    .rounded(u(24.))
                    .bg(theme::console_sidebar())
                    .border_1()
                    .border_color(theme::white_a(0.1))
                    .child(kit::title(self.title.clone(), 26.))
                    .children(self.message.clone().map(kit::body_text))
                    .shadow(kit::shadow(0.5, 18., 48.))
                    .child(list.min_h_0()),
            )),
        )
    }
}
