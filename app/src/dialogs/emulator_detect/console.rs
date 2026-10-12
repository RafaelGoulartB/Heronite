//! Detect emulators in console mode: the emulators found as switches (Y adds
//! the ticked ones), plus scanning a folder with the console browser.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::DialogEvent;
use super::{add_label, profiles_summary, DetectEmulatorsDialog};
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl HasLayer for DetectEmulatorsDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl DetectEmulatorsDialog {
    fn busy(&self) -> bool {
        self.scanning || self.folder_scanning.is_some() || self.adding
    }

    /// Opens the console browser; the folder picked is searched for emulators.
    pub fn console_pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        open_browser(&mut self.layer, tr!("Folder with emulators"), PickKind::Folder, None, window, cx, |this: &mut Self, p, _, cx| {
            this.scan_folder(p.display().to_string(), cx);
        });
    }

    fn console_rows(&self) -> Vec<Row> {
        let busy = self.busy();
        let selected = self.selected_count();
        let mut rows = vec![Row::action(
            "folder",
            if self.folder_scanning.is_some() { tr!("Searching\u{2026}") } else { tr!("Scan a folder\u{2026}") },
        )
        .desc(tr!("Searches a folder you pick, for example your Emulation folder"))
        .icon(Icon::FolderOpen)
        .enabled(!busy)];
        if self.rows.iter().any(|r| !r.item.already_added) {
            rows.push(Row::action("all", tr!("Select all")).icon(Icon::Check));
            rows.push(Row::action("none", tr!("Select none")).icon(Icon::Close));
        }
        if !self.rows.is_empty() {
            rows.push(Row::header(tr!("Found")));
        }
        for (i, r) in self.rows.iter().enumerate() {
            let loc = if r.item.launch_kind == "flatpak" {
                format!("Flatpak \u{b7} {}", r.item.flatpak_id.clone().unwrap_or_default())
            } else {
                r.item.executable.clone()
            };
            let desc = format!("{loc}  \u{b7}  {}", profiles_summary(&r.item));
            let id = format!("d:{i}");
            rows.push(if r.item.already_added {
                Row::info(id, r.item.name.clone(), tr!("Already added")).desc(desc)
            } else {
                Row::toggle(id, r.item.name.clone(), r.selected).desc(desc)
            });
        }
        rows.push(Row::action("add", add_label(self.adding, selected)).enabled(selected > 0 && !busy));
        rows.push(Row::action("close", tr!("Close")));
        rows
    }

    pub(super) fn render_console(&mut self) -> AnyElement {
        let rows = self.console_rows();
        let status = self.status_text();
        let mut body = div().flex().flex_col().flex_1().min_h_0().gap(u(8.)).child(kit::body_text(status));
        if !self.error.is_empty() {
            body = body.child(kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        }
        let list = self.console_list.render("console-detect", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .child(kit::modal(tr!("Detect emulators"), 820., body.child(list.flex_1().min_h_0())))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for DetectEmulatorsDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if pad == Pad::Y {
            if self.selected_count() > 0 && !self.busy() {
                self.add(cx);
            }
            return true;
        }
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Toggle(on)) => {
                if let Some(r) = id.strip_prefix("d:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.rows.get_mut(i)) {
                    r.selected = on;
                }
            }
            Outcome::Activate(id) => match id.as_ref() {
                "folder" => self.console_pick_folder(window, cx),
                "all" | "none" => {
                    let on = id.as_ref() == "all";
                    for r in self.rows.iter_mut().filter(|r| !r.item.already_added) {
                        r.selected = on;
                    }
                }
                "add" => self.add(cx),
                "close" => cx.emit(DialogEvent::Close),
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
        let mut h = vec![hint(Btn::A, tr!("Select"))];
        if self.selected_count() > 0 && !self.busy() {
            h.push(hint(Btn::Y, tr!("Add {n}", n = self.selected_count())));
        }
        h.push(hint(Btn::B, tr!("Close")));
        h
    }
}
