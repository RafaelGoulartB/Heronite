//! "Add emulated games" in console mode: emulator, profile and folder as
//! rows, "Scan folder", then the games found as switches (Y imports the
//! ticked ones). Adding a custom emulator by hand stays on the desktop (the
//! Emulators page adds one with its own console form).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{profile_label, text_of, ScanRomsDialog};
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl HasLayer for ScanRomsDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl ScanRomsDialog {
    fn selected_games(&self) -> usize {
        self.preview.as_ref().map(|p| p.games.iter().filter(|g| g.import).count()).unwrap_or(0)
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let dir = text_of(&self.directory, cx);
        let busy = self.loading;
        let names: Vec<SharedString> = self.state.emulators.iter().map(|e| SharedString::from(e.name.clone())).collect();
        let emu_index = self.state.emulators.iter().position(|e| e.id == self.emulator_id).unwrap_or(0);
        let profiles: Vec<SharedString> =
            self.emulator().map(|e| e.profiles.iter().map(|p| SharedString::from(profile_label(p))).collect()).unwrap_or_default();
        let profile_index = self.emulator().and_then(|e| e.profiles.iter().position(|p| p.id == self.profile_id)).unwrap_or(0);

        let mut rows = Vec::new();
        rows.push(if names.is_empty() {
            Row::choice("emulator", "Emulator", vec!["No emulators yet".into()], 0).enabled(false)
        } else {
            Row::choice("emulator", "Emulator", names, emu_index)
        });
        rows.push(if profiles.is_empty() {
            Row::choice("profile", "Platform / profile", vec!["\u{2014}".into()], 0).enabled(false)
        } else {
            Row::choice("profile", "Platform / profile", profiles, profile_index)
        });
        rows.push(Row::text("dir", "ROM folder", dir.clone()).icon(Icon::FolderOpen));
        rows.push(Row::toggle("sub", "Include subfolders", self.scan_subfolders));
        rows.push(Row::toggle("merge", "Merge discs into one game", self.merge_related));
        rows.push(Row::toggle("auto", "Scan this folder when the Collection refreshes", self.auto_scan));
        let can_scan = !self.emulator_id.is_empty() && !dir.is_empty() && !busy;
        rows.push(
            Row::action("scan", if busy && self.preview.is_none() { "Scanning\u{2026}" } else { "Scan folder" })
                .icon(Icon::Refresh)
                .enabled(can_scan),
        );
        let missing = self.state.detected.iter().filter(|d| !d.already_added).count();
        let detect = Row::action("detect", if self.detecting { "Detecting\u{2026}" } else { "Detect emulators on this PC" }).icon(Icon::TravelExplore).enabled(!self.detecting);
        rows.push(if missing > 0 { detect.desc(format!("{missing} emulator(s) found on this PC.")) } else { detect });

        if let Some(p) = &self.preview {
            rows.push(Row::header(format!("{} game(s) found", p.games.len())));
            rows.push(Row::action("all", "Select all").icon(Icon::Check));
            rows.push(Row::action("none", "Select none").icon(Icon::Close));
            for (i, g) in p.games.iter().enumerate() {
                let sub = if g.already_imported {
                    "Already in the Collection".to_string()
                } else if g.roms.len() > 1 {
                    format!("{} discs", g.roms.len())
                } else {
                    g.roms.first().map(|r| r.path.clone()).unwrap_or_default()
                };
                let id = format!("g:{i}");
                rows.push(if g.already_imported {
                    Row::info(id, g.title.clone(), "In the Collection").desc(sub)
                } else {
                    Row::toggle(id, g.title.clone(), g.import).desc(sub)
                });
            }
            let n = self.selected_games();
            rows.push(Row::action("import", format!("Add {n} to the Collection")).icon(Icon::Add).enabled(n > 0 && !busy));
        }
        rows
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let mut body = div().flex().flex_col().flex_1().min_h_0().gap(u(8.)).child(kit::body_text(
            "Pick an emulator, its platform profile and the folder with the ROMs. The Collection scans that folder and adds the games.",
        ));
        if !self.error.is_empty() {
            body = body.child(kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        }
        let list = self.console_list.render("console-scan-roms", &rows, !self.layer.is_open());
        div()
            .absolute()
            .inset_0()
            .child(kit::modal("Add emulated games", 820., body.child(list.flex_1().min_h_0())))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for ScanRomsDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if pad == Pad::Y {
            if self.preview.is_some() && self.selected_games() > 0 && !self.loading {
                self.import(cx);
            }
            return true;
        }
        let rows = self.console_rows(cx);
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("emulator", Value::Choice { index, .. }) => {
                    if let Some(e) = self.state.emulators.get(index) {
                        let (id, p) = (e.id.clone(), e.profiles.first().map(|p| p.id.clone()).unwrap_or_default());
                        self.select_target(&id, &p, None, cx);
                    }
                }
                ("profile", Value::Choice { index, .. }) => {
                    if let Some(p) = self.emulator().and_then(|e| e.profiles.get(index)).map(|p| p.id.clone()) {
                        let e = self.emulator_id.clone();
                        self.select_target(&e, &p, None, cx);
                    }
                }
                ("sub", Value::Toggle(on)) => self.scan_subfolders = on,
                ("merge", Value::Toggle(on)) => self.merge_related = on,
                ("auto", Value::Toggle(on)) => self.auto_scan = on,
                (id, Value::Toggle(on)) => {
                    if let Some(g) = id.strip_prefix("g:").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.preview.as_mut().and_then(|p| p.games.get_mut(i))) {
                        g.import = on;
                    }
                }
                _ => {}
            },
            Outcome::Activate(id) => match id.as_ref() {
                "dir" => {
                    let current = text_of(&self.directory, cx);
                    let start = (!current.is_empty()).then(|| std::path::PathBuf::from(current));
                    let input = self.directory.clone();
                    open_browser(&mut self.layer, "Choose the ROM folder", PickKind::Folder, start.as_deref(), window, cx, move |this: &mut Self, p, window, cx| {
                        input.update(cx, |s, cx| s.set_value(p.display().to_string(), window, cx));
                        this.preview = None;
                    });
                }
                "scan" => self.scan(cx),
                "detect" => self.detect(cx),
                "all" => self.toggle_all(true),
                "none" => self.toggle_all(false),
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
        let mut h = vec![hint(Btn::A, "Select"), hint(Btn::LeftRight, "Change")];
        let n = self.selected_games();
        if self.preview.is_some() && n > 0 && !self.loading {
            h.push(hint(Btn::Y, format!("Add {n}")));
        }
        h.push(hint(Btn::B, "Close"));
        h
    }
}
