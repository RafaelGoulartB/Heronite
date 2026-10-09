//! The log viewer in console mode: the logs as console rows on the left
//! (moving selects and loads one), its text on the right scrolled with the
//! right stick; X shows the previous run, Y loads earlier lines.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::LogViewer;
use crate::console::kit::u;
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};

impl LogViewer {
    fn console_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut last = None;
        for (i, e) in self.entries.iter().enumerate() {
            let (g, caption) = Self::group_of(e);
            if last != Some(g) {
                rows.push(Row::header(caption));
                last = Some(g);
            }
            let (name, detail) = self.label(e);
            rows.push(Row::action(SharedString::from(format!("l{i}")), name).desc(detail));
        }
        if rows.is_empty() {
            rows.push(Row::info("none", if self.entries_loaded { "No logs yet" } else { "Looking for logs\u{2026}" }, ""));
        }
        rows
    }

    fn console_select_focused(&mut self, rows: &[Row], cx: &mut Context<Self>) {
        if let Some(i) = self.console_list.focused(rows).and_then(|r| r.id.strip_prefix('l')).and_then(|n| n.parse::<usize>().ok()) {
            self.select(i, cx);
        }
    }

    pub fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        if self.selected.is_none() && !self.entries.is_empty() {
            self.console_select_focused(&rows, cx);
        }
        let list = self.console_list.render("console-logs", &rows, true);
        div()
            .flex()
            .gap(u(24.))
            .size_full()
            .child(div().w(u(360.)).flex_none().flex().flex_col().child(list.flex_1().min_h_0()))
            .child(div().flex_1().min_w_0().h_full().child(self.render_pane(cx)))
            .into_any_element()
    }
}

impl PadTarget for LogViewer {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match pad {
            Pad::ScrollUp | Pad::ScrollDown => {
                let lines = if pad == Pad::ScrollDown { 12. } else { -12. };
                self.list_state.scroll_by(px(lines * 18.));
            }
            Pad::X => {
                let has_previous = self.selected.and_then(|i| self.entries.get(i)).is_some_and(|e| e.previous.is_some());
                if has_previous {
                    self.set_previous(!self.previous, cx);
                }
            }
            Pad::Y if self.can_load_earlier() => self.load_earlier(cx),
            _ => match self.console_list.on_pad(pad, &rows) {
                Outcome::Moved | Outcome::Activate(_) => self.console_select_focused(&rows, cx),
                Outcome::Ignored | Outcome::Edge(_) => return false,
                _ => {}
            },
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, "Show"), hint(Btn::X, "Previous run"), hint(Btn::Y, "Earlier lines")]
    }
}
