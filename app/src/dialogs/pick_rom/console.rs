//! "Choose disc" in console mode: one row per ROM, A launches that one.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::DialogEvent;
use super::PickRomDialog;
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;

impl PickRomDialog {
    fn console_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .roms
            .iter()
            .enumerate()
            .map(|(i, (name, path))| {
                let label = if name.is_empty() { self.title.clone() } else { name.clone() };
                Row::action(format!("r:{i}"), label).desc(path.clone()).icon(Icon::PlayArrow)
            })
            .collect();
        rows.push(Row::action("cancel", "Cancel"));
        rows
    }

    /// Starts with the focus on the ROM that was chosen last time.
    pub(super) fn console_focus_selected(&mut self) {
        if let Some(i) = self.roms.iter().position(|(_, p)| Some(p) == self.selected.as_ref()) {
            self.console_list.focus = i;
        }
    }

    pub(super) fn render_console(&mut self) -> AnyElement {
        let rows = self.console_rows();
        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(u(12.))
            .child(kit::body_text("This title has more than one ROM. Pick which disc to launch."))
            .child(self.console_list.render("console-pick-rom", &rows, true).flex_1().min_h_0());
        kit::modal("Choose disc", 760., body).into_any_element()
    }
}

impl PadTarget for PickRomDialog {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                if id.as_ref() == "cancel" {
                    cx.emit(DialogEvent::Close);
                } else if let Some((_, path)) = id.strip_prefix("r:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.roms.get(i)) {
                    self.selected = Some(path.clone());
                    (self.on_play)(path.clone(), cx);
                }
            }
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, "Play"), hint(Btn::B, "Cancel")]
    }
}
