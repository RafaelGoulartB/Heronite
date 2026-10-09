//! Add / edit a saved ROM folder (a "scanner"): emulator, profile, folder and
//! the scan options. Saving with "scan on refresh" on also scans the folder
//! right away. Used by the Emulators page.

mod console;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_emulation::{run_rom_scanners, upsert_rom_scanner, RomScanner, UpsertRomScannerArgs};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::scan_roms::profile_label;
use super::text_of;
use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, new_input, panel, paragraph, path_field, select_field};

/// What saving did (read by the page once the dialog closes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveOutcome {
    pub directory: String,
    /// The folder was scanned after saving.
    pub scanned: bool,
    pub imported: usize,
    pub errors: Vec<String>,
}

pub struct RomFolderDialog {
    paths: HeroicPaths,
    scanner: Option<RomScanner>,
    emulators: Vec<UserEmulator>,
    emulator_id: String,
    profile_id: String,
    directory: Entity<InputState>,
    scan_subfolders: bool,
    merge_related: bool,
    auto_scan: bool,
    emu_open: bool,
    profile_open: bool,
    saving: bool,
    error: String,
    pub outcome: Option<SaveOutcome>,
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for RomFolderDialog {}

/// The name the store gives a folder: "Emulator \u{b7} Profile".
fn default_name(e: &UserEmulator, p: Option<&UserEmulatorProfile>) -> String {
    [Some(e.name.as_str()), p.map(|p| p.name.as_str())]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

impl RomFolderDialog {
    /// `scanner` = `Some` edits that folder; otherwise a new one for
    /// `emulator_id` (or the first emulator).
    pub fn new(
        paths: HeroicPaths,
        emulators: Vec<UserEmulator>,
        emulator_id: Option<String>,
        scanner: Option<RomScanner>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let wanted = scanner.as_ref().map(|s| s.emulator_id.clone()).or(emulator_id);
        let emulator = wanted
            .as_ref()
            .and_then(|id| emulators.iter().find(|e| &e.id == id))
            .or_else(|| emulators.first());
        let profile_id = scanner
            .as_ref()
            .map(|s| s.profile_id.clone())
            .filter(|p| emulator.is_some_and(|e| e.profiles.iter().any(|x| &x.id == p)))
            .or_else(|| emulator.and_then(|e| e.profiles.first().map(|p| p.id.clone())))
            .unwrap_or_default();
        Self {
            paths,
            emulator_id: emulator.map(|e| e.id.clone()).unwrap_or_default(),
            profile_id,
            directory: new_input(window, cx, scanner.as_ref().map(|s| s.directory.as_str()).unwrap_or(""), "/path/to/roms"),
            scan_subfolders: scanner.as_ref().map(|s| s.scan_subfolders).unwrap_or(true),
            merge_related: scanner.as_ref().map(|s| s.merge_related_files).unwrap_or(true),
            auto_scan: scanner.as_ref().map(|s| s.auto_scan_on_refresh).unwrap_or(true),
            scanner,
            emulators,
            emu_open: false,
            profile_open: false,
            saving: false,
            error: String::new(),
            outcome: None,
            console_list: Default::default(),
            layer: Default::default(),
        }
    }

    /// Debug helper: types the folder and saves.
    pub fn debug_fill_and_save(&mut self, directory: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.directory.update(cx, |s, cx| s.set_value(directory.to_string(), window, cx));
        self.save(cx);
    }

    fn emulator(&self) -> Option<&UserEmulator> {
        self.emulators.iter().find(|e| e.id == self.emulator_id)
    }

    fn profile(&self) -> Option<&UserEmulatorProfile> {
        let e = self.emulator()?;
        e.profiles.iter().find(|p| p.id == self.profile_id).or_else(|| e.profiles.first())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let directory = text_of(&self.directory, cx);
        let Some(profile) = self.profile().map(|p| p.id.clone()) else {
            self.error = "Add an emulator first.".into();
            cx.notify();
            return;
        };
        if directory.is_empty() {
            self.error = "Choose the folder with the ROMs.".into();
            cx.notify();
            return;
        }
        // A folder named after its old target follows the new one.
        let name = self.scanner.as_ref().and_then(|s| {
            let old = self
                .emulators
                .iter()
                .find(|e| e.id == s.emulator_id)
                .map(|e| default_name(e, e.profile(&s.profile_id)));
            (old.as_deref() == Some(s.name.as_str())).then(|| self.emulator().map(|e| default_name(e, e.profile(&profile)))).flatten()
        });
        let args = UpsertRomScannerArgs {
            id: self.scanner.as_ref().map(|s| s.id.clone()),
            name,
            emulator_id: self.emulator_id.clone(),
            profile_id: profile,
            directory: directory.clone(),
            scan_subfolders: Some(self.scan_subfolders),
            merge_related_files: Some(self.merge_related),
            auto_scan_on_refresh: Some(self.auto_scan),
        };
        let (paths, auto) = (self.paths.clone(), self.auto_scan);
        self.saving = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let saved = upsert_rom_scanner(&paths, args)?;
                    let mut outcome = SaveOutcome { directory, ..Default::default() };
                    if auto {
                        let report = run_rom_scanners(&paths, Some(&saved.id))?;
                        outcome.scanned = true;
                        outcome.imported = report.imported;
                        outcome.errors = report.errors;
                    }
                    anyhow::Ok(outcome)
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(o) => {
                        this.outcome = Some(o);
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(e) => this.error = format!("{e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for RomFolderDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let edit = self.scanner.is_some();
        let dir_value = text_of(&self.directory, cx);
        let has_dir = !dir_value.is_empty();
        let emu_name = self.emulator().map(|e| e.name.clone()).unwrap_or_else(|| "No emulators yet".into());
        let emu_opts: Vec<(SharedString, bool)> =
            self.emulators.iter().map(|e| (e.name.clone().into(), e.id == self.emulator_id)).collect();
        let profile_name = self.profile().map(profile_label).unwrap_or_default();
        let profile_opts: Vec<(SharedString, bool)> = self
            .emulator()
            .map(|e| e.profiles.iter().map(|p| (profile_label(p).into(), p.id == self.profile_id)).collect())
            .unwrap_or_default();

        let emu_select = select_field(
            "romf-emulator",
            Some("Emulator"),
            emu_name,
            self.emulators.is_empty(),
            self.emu_open,
            emu_opts,
            cx,
            |this: &mut Self, _, cx| {
                this.emu_open = !this.emu_open;
                this.profile_open = false;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                if let Some(e) = this.emulators.get(i) {
                    this.emulator_id = e.id.clone();
                    this.profile_id = e.profiles.first().map(|p| p.id.clone()).unwrap_or_default();
                }
                this.emu_open = false;
                cx.notify();
            },
        );
        let profile_select = select_field(
            "romf-profile",
            Some("Platform / profile"),
            profile_name,
            profile_opts.is_empty(),
            self.profile_open,
            profile_opts,
            cx,
            |this: &mut Self, _, cx| {
                this.profile_open = !this.profile_open;
                this.emu_open = false;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                if let Some(p) = this.emulator().and_then(|e| e.profiles.get(i)).map(|p| p.id.clone()) {
                    this.profile_id = p;
                }
                this.profile_open = false;
                cx.notify();
            },
        );
        let folder = path_field(
            "romf-folder-pick",
            Some("ROM folder"),
            &self.directory,
            has_dir,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_dir {
                    this.directory.update(cx, |s, cx| s.set_value("", window, cx));
                    cx.notify();
                } else {
                    choose_path(window, cx, true, "ROM folder", |this: &mut Self, p, window, cx| {
                        this.directory.update(cx, |s, cx| s.set_value(p, window, cx));
                        cx.notify();
                    });
                }
            }),
        );
        let toggles = panel()
            .child(checkbox("romf-subfolders", self.scan_subfolders, "Include subfolders").on_click(cx.listener(|this, _, _, cx| {
                this.scan_subfolders = !this.scan_subfolders;
                cx.notify();
            })))
            .child(checkbox("romf-merge", self.merge_related, "Merge discs into one game").on_click(cx.listener(|this, _, _, cx| {
                this.merge_related = !this.merge_related;
                cx.notify();
            })))
            .child(checkbox("romf-auto", self.auto_scan, "Scan this folder when Collection refreshes").on_click(cx.listener(
                |this, _, _, cx| {
                    this.auto_scan = !this.auto_scan;
                    cx.notify();
                },
            )))
            .child(meta_text("With this on, saving also scans the folder right away and adds the games it finds."));

        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph("Games found in this folder are added to the Collection and launched with the chosen emulator profile."))
            .child(emu_select)
            .child(profile_select)
            .child(folder)
            .child(toggles)
            .children((!self.error.is_empty()).then(|| div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())));
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("romf-cancel", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "romf-save",
                ButtonKind::Primary,
                if self.saving {
                    if self.auto_scan { "Scanning\u{2026}" } else { "Saving\u{2026}" }
                } else if edit {
                    "Save"
                } else {
                    "Add ROM folder"
                },
                !self.saving && !self.emulators.is_empty(),
                cx.listener(|this, _, _, cx| this.save(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "rom-folder".into(),
                title: if edit { "Edit ROM folder" } else { "Add ROM folder" }.into(),
                width: 640f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
