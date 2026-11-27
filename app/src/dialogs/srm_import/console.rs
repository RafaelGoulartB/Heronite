//! Console-mode layout of the Steam ROM Manager importer dialog.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::SrmImportDialog;
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::dialogs::text_of;
use crate::icons::Icon;
use crate::theme;

impl HasLayer for SrmImportDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl SrmImportDialog {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let busy = self.loading || self.importing;
        let mut rows = vec![Row::text("file", tr!("Steam ROM Manager config"), text_of(&self.config_path, cx)).icon(Icon::FolderOpen)];
        if !self.parsers.is_empty() {
            rows.push(Row::toggle("scan-now", tr!("Scan imported folders right away"), self.scan_now));
            rows.push(Row::header(trn!(self.parsers.len(), "{n} Glob parser found", "{n} Glob parsers found")));
            for (i, p) in self.parsers.iter().enumerate() {
                rows.push(
                    Row::toggle(format!("p:{i}"), p.title.clone(), p.selected).desc(format!("{}  \u{b7}  {}", p.rom_directory, p.rom_glob)),
                );
            }
            let selected = self.selected_count();
            rows.push(
                Row::action(
                    "import",
                    if self.importing {
                        tr!("Importing\u{2026}").to_string()
                    } else {
                        trn!(selected, "Import {n} parser", "Import {n} parsers")
                    },
                )
                .icon(Icon::Add)
                .enabled(selected > 0 && !busy),
            );
        }
        rows
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let mut body = div().flex().flex_col().flex_1().min_h_0().gap(u(8.)).child(kit::body_text(self.status_text()));
        if !self.error.is_empty() {
            body = body.child(kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        }
        let list = self.console_list.render("console-srm-import", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .child(kit::modal(tr!("Import from Steam ROM Manager"), 820., body.child(list.flex_1().min_h_0())))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for SrmImportDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Toggle(on)) => {
                if id == "scan-now" {
                    self.scan_now = on;
                } else if let Some(p) = id.strip_prefix("p:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.parsers.get_mut(i)) {
                    p.selected = on;
                }
            }
            Outcome::Activate(id) => match id.as_ref() {
                "file" => {
                    let current = text_of(&self.config_path, cx);
                    let start = (!current.is_empty()).then(|| std::path::PathBuf::from(current));
                    open_browser(
                        &mut self.layer,
                        tr!("Steam ROM Manager config"),
                        PickKind::File(vec!["json".into()]),
                        start.as_deref(),
                        window,
                        cx,
                        |this: &mut Self, p, window, cx| {
                            let s = p.display().to_string();
                            this.config_path.update(cx, |i, cx| i.set_value(s.clone(), window, cx));
                            this.load_file(s, cx);
                        },
                    );
                }
                "import" => self.import(cx),
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
        vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Close"))]
    }
}
