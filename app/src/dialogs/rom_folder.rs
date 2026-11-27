//! Add / edit a saved ROM folder (a "scanner"): emulator, profile, folder and
//! the scan options. Saving with "scan on refresh" on also scans the folder
//! right away. Used by the Emulators page.

mod console;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_emulation::{run_rom_scanners, upsert_rom_scanner, RomScanner, UpsertRomScannerArgs};
use heronite_i18n::tr;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::scan_roms::profile_label;
use super::text_of;
use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, button_sm, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, new_input, panel, paragraph, path_field, select_field, text_field};

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
    paths: DataPaths,
    scanner: Option<RomScanner>,
    emulators: Vec<UserEmulator>,
    emulator_id: String,
    profile_id: String,
    directory: Entity<InputState>,
    rom_glob: Entity<InputState>,
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
    [Some(e.name.as_str()), p.map(|p| p.name.as_str())].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")
}

impl RomFolderDialog {
    /// `scanner` = `Some` edits that folder; otherwise a new one for
    /// `emulator_id` (or the first emulator).
    pub fn new(
        paths: DataPaths,
        emulators: Vec<UserEmulator>,
        emulator_id: Option<String>,
        scanner: Option<RomScanner>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let wanted = scanner.as_ref().map(|s| s.emulator_id.clone()).or(emulator_id);
        let emulator = wanted.as_ref().and_then(|id| emulators.iter().find(|e| &e.id == id)).or_else(|| emulators.first());
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
            directory: new_input(
                window,
                cx,
                scanner.as_ref().map(|s| s.directory.as_str()).unwrap_or(""),
                if cfg!(windows) { r"D:\ROMs\ps2" } else { "/path/to/roms" },
            ),
            rom_glob: new_input(
                window,
                cx,
                scanner.as_ref().and_then(|s| s.rom_glob.as_deref()).unwrap_or(""),
                "${title}/@(*[BASE]*.nsp|*.xci)",
            ),
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
            self.error = tr!("Add an emulator first.").into();
            cx.notify();
            return;
        };
        if directory.is_empty() {
            self.error = tr!("Choose the folder with the ROMs.").into();
            cx.notify();
            return;
        }
        let rom_glob = text_of(&self.rom_glob, cx);
        if !rom_glob.is_empty() {
            if let Err(err) = heronite_emulation::glob::validate(&rom_glob) {
                self.error = tr!("Invalid ROM glob pattern: {error}", error = err.as_str());
                cx.notify();
                return;
            }
        }
        // A folder named after its old target follows the new one.
        let name = self.scanner.as_ref().and_then(|s| {
            let old = self.emulators.iter().find(|e| e.id == s.emulator_id).map(|e| default_name(e, e.profile(&s.profile_id)));
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
            rom_glob: Some(rom_glob),
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
        let emu_name = self.emulator().map(|e| e.name.clone()).unwrap_or_else(|| tr!("No emulators yet").into());
        let emu_opts: Vec<(SharedString, bool)> =
            self.emulators.iter().map(|e| (e.name.clone().into(), e.id == self.emulator_id)).collect();
        let profile_name = self.profile().map(profile_label).unwrap_or_default();
        let profile_opts: Vec<(SharedString, bool)> = self
            .emulator()
            .map(|e| e.profiles.iter().map(|p| (profile_label(p).into(), p.id == self.profile_id)).collect())
            .unwrap_or_default();

        let emu_select = select_field(
            "romf-emulator",
            Some(tr!("Emulator")),
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
            Some(tr!("Platform / profile")),
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
            Some(tr!("ROM folder")),
            &self.directory,
            has_dir,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_dir {
                    this.directory.update(cx, |s, cx| s.set_value("", window, cx));
                    cx.notify();
                } else {
                    choose_path(window, cx, true, tr!("ROM folder"), |this: &mut Self, p, window, cx| {
                        this.directory.update(cx, |s, cx| s.set_value(p, window, cx));
                        cx.notify();
                    });
                }
            }),
        );
        let glob_value = text_of(&self.rom_glob, cx);
        let has_effective_glob =
            !glob_value.is_empty() || self.profile().and_then(|p| p.rom_glob.as_deref()).is_some_and(|g| !g.trim().is_empty());
        let toggles = panel()
            .child(div().when(has_effective_glob, |d| d.opacity(0.55)).child(
                checkbox("romf-subfolders", self.scan_subfolders, tr!("Include subfolders")).on_click(cx.listener(|this, _, _, cx| {
                    this.scan_subfolders = !this.scan_subfolders;
                    cx.notify();
                })),
            ))
            .children(has_effective_glob.then(|| meta_text(tr!("Subfolder depth is controlled by the glob pattern."))))
            .child(checkbox("romf-merge", self.merge_related, tr!("Merge discs into one game")).on_click(cx.listener(|this, _, _, cx| {
                this.merge_related = !this.merge_related;
                cx.notify();
            })))
            .child(checkbox("romf-auto", self.auto_scan, tr!("Scan this folder when Collection refreshes")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.auto_scan = !this.auto_scan;
                    cx.notify();
                },
            )))
            .child(meta_text(tr!("With this on, saving also scans the folder right away and adds the games it finds.")));

        let glob_err = (!glob_value.is_empty()).then(|| heronite_emulation::glob::validate(&glob_value).err()).flatten();
        let presets = self
            .profile()
            .map(|p| heronite_emulation::glob::presets_for_profile(p.platform_id.as_deref(), &p.extensions))
            .unwrap_or_default();
        let mut preset_row = div().flex().flex_wrap().items_center().gap(px(6.));
        for preset in presets {
            let pat = preset.pattern.clone();
            let active = glob_value == pat;
            preset_row = preset_row.child(
                button_sm(
                    SharedString::from(format!("romf-preset-{}", preset.id)),
                    if active { ButtonKind::Primary } else { ButtonKind::Glass },
                    tr(preset.label),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let pat = pat.clone();
                    this.rom_glob.update(cx, |s, cx| s.set_value(pat, window, cx));
                    cx.notify();
                })),
            );
        }
        if !glob_value.is_empty() {
            preset_row = preset_row.child(button_sm("romf-preset-clear", ButtonKind::Ghost, tr!("Clear")).on_click(cx.listener(
                |this, _, window, cx| {
                    this.rom_glob.update(cx, |s, cx| s.set_value("", window, cx));
                    cx.notify();
                },
            )));
        }
        let glob_section = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(text_field(tr!("ROM glob pattern (optional)"), &self.rom_glob))
            .child(meta_text(tr!(
                "Steam ROM Manager syntax (for example {example}). Leave empty to scan by file extensions.",
                example = "${title}/@(*[BASE]*.nsp|*.xci)"
            )))
            .child(preset_row)
            .children(glob_err.map(|e| div().text_color(theme::danger_text()).text_size(px(12.5)).child(e)));

        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(tr!("Games found in this folder are added to the Collection and launched with the chosen emulator profile.")))
            .child(emu_select)
            .child(profile_select)
            .child(folder)
            .child(glob_section)
            .child(toggles)
            .children(
                (!self.error.is_empty()).then(|| div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())),
            );
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("romf-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "romf-save",
                ButtonKind::Primary,
                if self.saving {
                    if self.auto_scan {
                        tr!("Scanning\u{2026}")
                    } else {
                        tr!("Saving\u{2026}")
                    }
                } else if edit {
                    tr!("Save")
                } else {
                    tr!("Add ROM folder")
                },
                !self.saving && !self.emulators.is_empty(),
                cx.listener(|this, _, _, cx| this.save(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "rom-folder".into(),
                title: if edit { tr!("Edit ROM folder") } else { tr!("Add ROM folder") }.into(),
                width: 640f32.min(f32::from(crate::ui::content_size(window).width) * 0.9) + 64.,
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
