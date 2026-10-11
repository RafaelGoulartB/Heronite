//! The ROM folder dialog in console mode: emulator and profile as choices,
//! the folder picked with the console browser, the scan options as switches.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::scan_roms::profile_label;
use super::super::text_of;
use super::RomFolderDialog;
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl HasLayer for RomFolderDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl RomFolderDialog {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let emu_names: Vec<SharedString> = self.emulators.iter().map(|e| SharedString::from(e.name.clone())).collect();
        let emu_index = self.emulators.iter().position(|e| e.id == self.emulator_id).unwrap_or(0);
        let profiles: Vec<SharedString> =
            self.emulator().map(|e| e.profiles.iter().map(|p| SharedString::from(profile_label(p))).collect()).unwrap_or_default();
        let profile_index = self.emulator().and_then(|e| e.profiles.iter().position(|p| p.id == self.profile_id)).unwrap_or(0);
        let mut rows = Vec::new();
        rows.push(if emu_names.is_empty() {
            Row::choice("emulator", "Emulator", vec!["No emulators yet".into()], 0).enabled(false)
        } else {
            Row::choice("emulator", "Emulator", emu_names, emu_index)
        });
        rows.push(if profiles.is_empty() {
            Row::choice("profile", "Platform / profile", vec!["\u{2014}".into()], 0).enabled(false)
        } else {
            Row::choice("profile", "Platform / profile", profiles, profile_index)
        });
        rows.push(Row::text("dir", "ROM folder", text_of(&self.directory, cx)).icon(Icon::FolderOpen));
        rows.push(Row::toggle("sub", "Include subfolders", self.scan_subfolders));
        rows.push(Row::toggle("merge", "Merge discs into one game", self.merge_related));
        rows.push(
            Row::toggle("auto", "Scan this folder when the Collection refreshes", self.auto_scan)
                .desc("With this on, saving also scans the folder right away and adds the games it finds."),
        );
        let label = if self.saving {
            if self.auto_scan {
                "Scanning\u{2026}"
            } else {
                "Saving\u{2026}"
            }
        } else if self.scanner.is_some() {
            "Save"
        } else {
            "Add ROM folder"
        };
        rows.push(Row::action("save", label).enabled(!self.saving && !self.emulators.is_empty()));
        rows
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let mut body =
            div().flex().flex_col().flex_1().min_h_0().gap(u(12.)).child(kit::body_text(
                "Games found in this folder are added to the Collection and launched with the chosen emulator profile.",
            ));
        if !self.error.is_empty() {
            body = body.child(kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        }
        let list = self.console_list.render("console-rom-folder", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .child(kit::modal(
                if self.scanner.is_some() { "Edit ROM folder" } else { "Add ROM folder" },
                760.,
                body.child(list.flex_1().min_h_0()),
            ))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for RomFolderDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("emulator", Value::Choice { index, .. }) => {
                    if let Some(e) = self.emulators.get(index) {
                        self.emulator_id = e.id.clone();
                        self.profile_id = e.profiles.first().map(|p| p.id.clone()).unwrap_or_default();
                    }
                }
                ("profile", Value::Choice { index, .. }) => {
                    if let Some(p) = self.emulator().and_then(|e| e.profiles.get(index)).map(|p| p.id.clone()) {
                        self.profile_id = p;
                    }
                }
                ("sub", Value::Toggle(on)) => self.scan_subfolders = on,
                ("merge", Value::Toggle(on)) => self.merge_related = on,
                ("auto", Value::Toggle(on)) => self.auto_scan = on,
                _ => {}
            },
            Outcome::Activate(id) => match id.as_ref() {
                "save" => self.save(cx),
                "dir" => {
                    let current = text_of(&self.directory, cx);
                    let start = (!current.is_empty()).then(|| std::path::PathBuf::from(current));
                    let input = self.directory.clone();
                    open_browser(
                        &mut self.layer,
                        "Choose the ROM folder",
                        PickKind::Folder,
                        start.as_deref(),
                        window,
                        cx,
                        move |_: &mut Self, p, window, cx| {
                            input.update(cx, |s, cx| s.set_value(p.display().to_string(), window, cx));
                        },
                    );
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
        vec![hint(Btn::A, "Select"), hint(Btn::LeftRight, "Change"), hint(Btn::B, "Cancel")]
    }
}
