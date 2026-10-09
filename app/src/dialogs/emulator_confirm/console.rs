//! The emulator / ROM folder removal confirmation in console mode: the
//! message, then Cancel (focused first) and Remove.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::DialogEvent;
use super::EmulatorConfirmDialog;
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::theme;

impl EmulatorConfirmDialog {
    fn console_rows(&self) -> Vec<Row> {
        vec![
            Row::action("cancel", "Cancel"),
            Row::action("confirm", if self.working { "Removing\u{2026}" } else { "Remove" }).danger().enabled(!self.working),
        ]
    }

    pub(super) fn render_console(&mut self) -> AnyElement {
        let rows = self.console_rows();
        let mut body = div().flex().flex_col().flex_1().min_h_0().gap(u(12.)).child(kit::body_text(self.message.clone()));
        if let Some(e) = self.error.clone() {
            body = body.child(kit::body_text(e).text_color(theme::danger_text()));
        }
        let list = self.console_list.render("console-emu-confirm", &rows, true);
        kit::modal(self.title.clone(), 720., body.child(list.flex_1().min_h_0())).into_any_element()
    }
}

impl PadTarget for EmulatorConfirmDialog {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => match id.as_ref() {
                "cancel" => cx.emit(DialogEvent::Close),
                "confirm" => self.confirm(cx),
                _ => {}
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, "Select"), hint(Btn::B, "Cancel")]
    }
}
