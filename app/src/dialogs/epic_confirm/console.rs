//! The confirmation dialog in console mode: the message, its switches and
//! folder as console rows (the folder is picked with the console browser),
//! then Cancel / the confirm action.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::DialogEvent;
use super::EpicConfirmDialog;
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl EpicConfirmDialog {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, t) in self.spec.toggles.iter().enumerate() {
            let mut r = Row::toggle(SharedString::from(format!("t{i}")), t.label.clone(), self.toggles[i]).enabled(!t.disabled);
            if let Some(n) = &t.note {
                r = r.desc(n.clone());
            }
            rows.push(r);
        }
        if let (Some((label, _, _)), Some(input)) = (&self.spec.path, &self.path_input) {
            rows.push(Row::text("path", label.clone(), input.read(cx).value().to_string()).icon(Icon::FolderOpen));
        }
        match &self.spec.confirm {
            Some(label) => {
                rows.push(Row::action("cancel", "Cancel"));
                if let Some((label, _)) = &self.spec.secondary {
                    rows.push(Row::action("secondary", label.clone()));
                }
                let ready = self.spec.path.is_none() || self.path_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty());
                let r = Row::action("confirm", label.clone()).enabled(ready);
                rows.push(if self.spec.danger { r.danger() } else { r });
            }
            None => rows.push(Row::action("cancel", "Close")),
        }
        rows
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let mut body = div().flex().flex_col().gap(u(12.));
        for line in self.spec.message.lines().filter(|l| !l.trim().is_empty()) {
            body = body.child(kit::body_text(line.to_string()));
        }
        if let Some(text) = &self.spec.text {
            body = body.child(
                div()
                    .id("console-confirm-text")
                    .max_h(u(260.))
                    .overflow_y_scroll()
                    .p(u(12.))
                    .rounded(u(12.))
                    .bg(theme::black_a(0.35))
                    .font_family(theme::FONT_MONO)
                    .text_size(u(13.))
                    .child(text.clone()),
            );
        }
        let list = self.console_list.render("console-confirm", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .child(kit::modal(self.spec.title.clone(), 720., body.child(list.min_h_0())))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for EpicConfirmDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Toggle(on)) => {
                if let Some(i) = id.strip_prefix('t').and_then(|n| n.parse::<usize>().ok()) {
                    self.toggles[i] = on;
                }
            }
            Outcome::Activate(id) => match id.as_ref() {
                "cancel" => cx.emit(DialogEvent::Close),
                "secondary" => {
                    if let Some((_, action)) = self.spec.secondary.clone() {
                        action(cx);
                        cx.emit(DialogEvent::Close);
                    }
                }
                "confirm" => self.confirm(cx),
                "path" => {
                    let start = self.path_input.as_ref().map(|i| std::path::PathBuf::from(i.read(cx).value().to_string()));
                    let browser = cx.new(|cx| Browser::new("Choose a folder", PickKind::Folder, start.as_deref(), cx));
                    let sub = cx.subscribe_in(&browser, window, |this, _, ev: &BrowserEvent, window, cx| {
                        if let BrowserEvent::Picked(p) = ev {
                            if let Some(input) = this.path_input.clone() {
                                let v = p.display().to_string();
                                input.update(cx, |s, cx| s.set_value(v, window, cx));
                            }
                        }
                        this.layer.close();
                        cx.notify();
                    });
                    self.layer.open(&browser, sub);
                }
                _ => {}
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, "Select"), hint(Btn::B, "Cancel")]
    }
}
