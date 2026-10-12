//! Add / edit an emulator in console mode: the fields as rows (text typed on
//! the on-screen keyboard, the executable and folders picked with the console
//! browser, choices with ◀ ▶), then the profiles. A on a profile opens its
//! own page of rows; B goes back to the emulator, B again closes the dialog.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::catalog::emulator_definition;
use heronite_emulation::detect::profiles_for_definition;
use heronite_emulation::{emulator_definitions, SystemEnv};
use heronite_i18n::{tr, trn};

use super::super::scan_roms::profile_label;
use super::super::DialogEvent;
use super::{text_of, EmulatorEditDialog, KINDS, WORKING_DIRS};
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, open_osk, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

impl HasLayer for EmulatorEditDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

/// Ids of the catalog definitions in the order of the "Emulator" choice.
fn definition_ids() -> Vec<String> {
    std::iter::once("custom".to_string()).chain(emulator_definitions().iter().filter(|d| d.id != "custom").map(|d| d.id.clone())).collect()
}

impl EmulatorEditDialog {
    fn console_main_rows(&self, cx: &App) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.create {
            let ids = definition_ids();
            let names: Vec<SharedString> = ids
                .iter()
                .map(|id| {
                    if id == "custom" {
                        tr!("Custom").into()
                    } else {
                        emulator_definition(id).map(|d| d.name.clone()).unwrap_or_default().into()
                    }
                })
                .collect();
            let index = ids.iter().position(|i| *i == self.definition_id).unwrap_or(0);
            rows.push(
                Row::choice("def", tr!("Emulator"), names, index)
                    .desc(tr!("Pick one from the catalog to start with its platforms and arguments, or Custom to describe any program.")),
            );
        }
        rows.push(Row::text("name", tr!("Name"), text_of(&self.name, cx)).icon(Icon::Edit));
        let kind = KINDS.iter().position(|k| k.0 == self.launch_kind).unwrap_or(0);
        rows.push(Row::choice("kind", tr!("Run as"), KINDS.iter().map(|k| SharedString::from(tr(k.1))).collect(), kind));
        if self.launch_kind == "flatpak" {
            rows.push(Row::text("flatpak", tr!("Flatpak application id"), text_of(&self.flatpak_id, cx)).icon(Icon::Edit));
        } else {
            rows.push(
                Row::text("exe", tr!("Executable"), text_of(&self.executable, cx))
                    .icon(Icon::FolderOpen)
                    .desc(tr!("A picks the file. X clears it.")),
            );
        }
        rows.push(
            Row::text("dir", tr!("Install folder (optional)"), text_of(&self.install_dir, cx))
                .icon(Icon::FolderOpen)
                .desc(tr!("{EmulatorDir} in arguments. Empty uses the executable's folder. X clears it.")),
        );
        rows.push(Row::header(tr!("Profiles")));
        for (i, p) in self.profiles.iter().enumerate() {
            let args = if p.arguments.is_empty() { tr!("(catalog arguments)").to_string() } else { p.arguments.clone() };
            let ext = if p.extensions.is_empty() { tr!("any extension").to_string() } else { p.extensions.join(", ") };
            rows.push(Row::action(format!("p:{i}"), profile_label(p)).desc(format!("{args}  \u{b7}  {ext}")));
        }
        rows.push(Row::action("padd", tr!("Add profile")).icon(Icon::Add));
        let missing = self.missing_catalog_profiles().len();
        if missing > 0 {
            rows.push(Row::action("pcat", trn!(missing, "Add {n} catalog profile", "Add {n} catalog profiles")).icon(Icon::Add));
        }
        rows.push(Row::action("cancel", tr!("Cancel")));
        let label = if self.saving {
            tr!("Saving\u{2026}")
        } else if self.create {
            tr!("Add emulator")
        } else {
            tr!("Save")
        };
        rows.push(Row::action("save", label).enabled(!self.saving));
        rows
    }

    fn console_profile_rows(&self, cx: &App) -> Vec<Row> {
        let (Some(i), Some(f)) = (self.open_profile, self.form.as_ref()) else { return vec![] };
        let Some(p) = self.profiles.get(i) else { return vec![] };
        let mut rows = vec![
            Row::text("pf-name", tr!("Profile name"), text_of(&f.name, cx)).icon(Icon::Edit),
            Row::text("pf-platform", tr!("Platform"), text_of(&f.platform, cx)).icon(Icon::Edit),
            Row::text("pf-args", tr!("Arguments"), text_of(&f.arguments, cx)).icon(Icon::Edit).desc(tr!(
                "May use {ImagePath}, {ImageName}, {ImageNameNoExt}, {ImageDir}, {EmulatorDir}, {CorePath}, {Name} and {InstallDir}."
            )),
            Row::text("pf-ext", tr!("ROM extensions"), text_of(&f.extensions, cx))
                .icon(Icon::Edit)
                .desc(tr!("Comma-separated, for example iso, cue, chd.")),
        ];
        if self.core_relevant(p) {
            rows.push(
                Row::text("pf-core", tr!("RetroArch core (.so)"), text_of(&f.core, cx))
                    .icon(Icon::FolderOpen)
                    .desc(tr!("A picks the file. X clears it.")),
            );
        }
        let wd = WORKING_DIRS.iter().position(|w| Some(w.0) == f.working_dir.as_deref().or(Some(""))).unwrap_or(0);
        rows.push(Row::choice("pf-wd", tr!("Working folder"), WORKING_DIRS.iter().map(|w| SharedString::from(tr(w.1))).collect(), wd));
        let users = self.profile_users(p);
        let remove = if self.profiles.len() <= 1 {
            Row::action("pf-rm", tr!("Remove profile")).desc(tr!("An emulator needs at least one profile.")).enabled(false)
        } else if users > 0 {
            Row::action("pf-rm", tr!("Remove profile")).desc(tr!("A ROM folder uses this profile.")).enabled(false)
        } else {
            Row::action("pf-rm", tr!("Remove profile")).icon(Icon::FaTrash).danger()
        };
        rows.push(remove);
        rows.push(Row::action("pf-done", tr!("Done")));
        rows
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        if self.form.is_some() {
            self.console_profile_rows(cx)
        } else {
            self.console_main_rows(cx)
        }
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let profile = self.form.is_some();
        let mut body = div().flex().flex_col().flex_1().min_h_0().gap(u(8.));
        if !profile && self.launch_kind != "flatpak" {
            let exe = text_of(&self.executable, cx);
            if !exe.is_empty() && exe.contains('/') && !std::path::Path::new(&exe).exists() {
                body = body.child(kit::body_text(tr!("This file does not exist.")).text_color(theme::danger_text()));
            }
        }
        if !self.error.is_empty() {
            body = body.child(kit::body_text(self.error.clone()).text_color(theme::danger_text()));
        }
        let active = !self.layer.is_open();
        let list = if profile {
            self.console_profile_list.render("console-emu-profile", &rows, active)
        } else {
            self.console_list.render("console-emu-edit", &rows, active)
        };
        let title = match (profile, self.create) {
            (true, _) => tr!("Edit profile"),
            (false, true) => tr!("Add emulator"),
            (false, false) => tr!("Edit emulator"),
        };
        div()
            .absolute()
            .inset_0()
            .child(kit::modal(title, 820., body.child(list.flex_1().min_h_0())))
            .children(self.layer.view())
            .into_any_element()
    }

    fn console_type(&mut self, title: &str, input: Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let initial = text_of(&input, cx);
        open_osk(&mut self.layer, title, &initial, window, cx, move |_: &mut Self, text, window, cx| {
            input.update(cx, |s, cx| s.set_value(text.trim().to_string(), window, cx));
        });
    }

    fn console_pick(
        &mut self,
        title: &str,
        kind: PickKind,
        input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: fn(&mut Self, &str, &mut Context<Self>),
    ) {
        let current = text_of(&input, cx);
        let start = (!current.is_empty()).then(|| std::path::PathBuf::from(&current));
        open_browser(&mut self.layer, title, kind, start.as_deref(), window, cx, move |this: &mut Self, p, window, cx| {
            let value = p.display().to_string();
            input.update(cx, |s, cx| s.set_value(value.clone(), window, cx));
            after(this, &value, cx);
        });
    }

    fn console_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "name" => self.console_type(tr!("Name"), self.name.clone(), window, cx),
            "flatpak" => self.console_type(tr!("Flatpak application id"), self.flatpak_id.clone(), window, cx),
            "exe" => {
                self.console_pick(tr!("Emulator executable"), PickKind::File(vec![]), self.executable.clone(), window, cx, |this, p, _| {
                    if p.to_lowercase().ends_with(".exe") {
                        this.launch_kind = "wine".into();
                    }
                    if this.create {
                        let (def, kind) = (this.definition_id.clone(), this.launch_kind.clone());
                        if emulator_definition(&def).is_some_and(|d| d.profiles.iter().any(|p| p.core.is_some())) {
                            this.profiles = profiles_for_definition(&SystemEnv, &def, p, &kind);
                            this.open_profile = None;
                            this.form = None;
                        }
                    }
                })
            }
            "dir" => self.console_pick(tr!("Install folder"), PickKind::Folder, self.install_dir.clone(), window, cx, |_, _, _| {}),
            "padd" => self.add_profile(window, cx),
            "pcat" => self.add_catalog_profiles(cx),
            "cancel" => cx.emit(DialogEvent::Close),
            "save" => self.save(cx),
            "pf-name" | "pf-platform" | "pf-args" | "pf-ext" => {
                let Some(f) = self.form.as_ref() else { return };
                let (title, input) = match id {
                    "pf-name" => (tr!("Profile name"), f.name.clone()),
                    "pf-platform" => (tr!("Platform"), f.platform.clone()),
                    "pf-args" => (tr!("Arguments"), f.arguments.clone()),
                    _ => (tr!("ROM extensions"), f.extensions.clone()),
                };
                self.console_type(title, input, window, cx);
            }
            "pf-core" => {
                if let Some(core) = self.form.as_ref().map(|f| f.core.clone()) {
                    self.console_pick(tr!("Libretro core"), PickKind::File(vec!["so".into()]), core, window, cx, |_, _, _| {});
                }
            }
            "pf-rm" => {
                if let Some(i) = self.open_profile {
                    self.remove_profile(i, cx);
                }
            }
            "pf-done" => self.close_form(cx),
            _ => {
                if let Some(i) = id.strip_prefix("p:").and_then(|n| n.parse::<usize>().ok()) {
                    self.edit_profile(i, window, cx);
                    self.console_profile_list.reset();
                }
            }
        }
    }

    fn console_changed(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        match (id, value) {
            ("def", Value::Choice { index, .. }) => {
                if let Some(id) = definition_ids().get(index) {
                    self.select_definition(&id.clone(), cx);
                }
            }
            ("kind", Value::Choice { index, .. }) => {
                self.launch_kind = KINDS[index.min(KINDS.len() - 1)].0.to_string();
                if self.launch_kind == "flatpak" && text_of(&self.flatpak_id, cx).is_empty() {
                    if let Some(fid) = emulator_definition(&self.definition_id).and_then(|d| d.flatpak_id.clone()) {
                        let input = self.flatpak_id.clone();
                        self.set_later(&input, fid);
                    }
                }
            }
            ("pf-wd", Value::Choice { index, .. }) => {
                if let Some(f) = self.form.as_mut() {
                    let wd = WORKING_DIRS[index.min(WORKING_DIRS.len() - 1)].0;
                    f.working_dir = (!wd.is_empty()).then(|| wd.to_string());
                }
            }
            _ => {}
        }
    }

    /// The input a path row edits (X clears it).
    fn console_path_input(&self, id: &str) -> Option<Entity<InputState>> {
        match id {
            "exe" => Some(self.executable.clone()),
            "dir" => Some(self.install_dir.clone()),
            "pf-core" => self.form.as_ref().map(|f| f.core.clone()),
            _ => None,
        }
    }
}

impl PadTarget for EmulatorEditDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        // B on a profile goes back to the emulator.
        if pad == Pad::B && self.form.is_some() {
            self.close_form(cx);
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        if pad == Pad::X {
            let focused = if self.form.is_some() { self.console_profile_list.focused(&rows) } else { self.console_list.focused(&rows) };
            if let Some(input) = focused.and_then(|r| self.console_path_input(&r.id)) {
                input.update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
            }
            return true;
        }
        let outcome = if self.form.is_some() { self.console_profile_list.on_pad(pad, &rows) } else { self.console_list.on_pad(pad, &rows) };
        match outcome {
            Outcome::Changed(id, value) => self.console_changed(&id, value, cx),
            Outcome::Activate(id) => self.console_activate(&id, window, cx),
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
        let rows = self.console_rows(cx);
        let focused = if self.form.is_some() { self.console_profile_list.focused(&rows) } else { self.console_list.focused(&rows) };
        let mut h = vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))];
        if focused.is_some_and(|r| self.console_path_input(&r.id).is_some()) {
            h.push(hint(Btn::X, tr!("Clear")));
        }
        h.push(hint(Btn::B, if self.form.is_some() { tr!("Back") } else { tr!("Cancel") }));
        h
    }
}
