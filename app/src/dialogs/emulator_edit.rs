//! Add / edit one emulator: name, executable and launch kind, and its
//! profiles (platform, arguments, extensions, RetroArch core). Used by the
//! Emulators page. Writes `local_library/emulators.json` through
//! `heronite_emulation::store`.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_emulation::detect::profiles_for_definition;
use heronite_emulation::store::{catalog_profile, insert_user_emulator, new_id, replace_user_emulator};
use heronite_emulation::catalog::emulator_definition;
use heronite_emulation::{emulator_definitions, list_rom_scanners, DetectedEmulator, RomScanner, SystemEnv};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::scan_roms::profile_label;
use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, section_title, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{
    choose_path, meta_text, new_input, panel, path_field, select_field, text_field,
};

const KINDS: [(&str, &str); 3] = [("native", "Native (Linux)"), ("flatpak", "Flatpak"), ("wine", "Windows program (Wine)")];
const WORKING_DIRS: [(&str, &str); 3] = [("", "Default"), ("emulator", "Emulator folder"), ("rom", "ROM folder")];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Definition,
    Kind,
    WorkingDir,
}

/// Inputs of the profile being edited.
struct ProfileForm {
    name: Entity<InputState>,
    platform: Entity<InputState>,
    arguments: Entity<InputState>,
    extensions: Entity<InputState>,
    core: Entity<InputState>,
    working_dir: Option<String>,
}

pub struct EmulatorEditDialog {
    paths: HeroicPaths,
    create: bool,
    base: UserEmulator,
    definition_id: String,
    launch_kind: String,
    profiles: Vec<UserEmulatorProfile>,
    scanners: Vec<RomScanner>,
    detected: Vec<DetectedEmulator>,
    name: Entity<InputState>,
    executable: Entity<InputState>,
    install_dir: Entity<InputState>,
    flatpak_id: Entity<InputState>,
    open_profile: Option<usize>,
    form: Option<ProfileForm>,
    menu: Option<Menu>,
    /// Input values to apply on the next render (needs a `Window`).
    pending: Vec<(Entity<InputState>, String)>,
    saving: bool,
    error: String,
    /// The emulator that was saved (read by the page after the dialog closes).
    pub saved: Option<UserEmulator>,
}

impl EventEmitter<DialogEvent> for EmulatorEditDialog {}

fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}

fn split_extensions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let e = e.trim().trim_start_matches('.').to_lowercase();
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

impl EmulatorEditDialog {
    /// Adds a new emulator (`existing` = `None`) or edits one.
    pub fn new(paths: HeroicPaths, existing: Option<UserEmulator>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let create = existing.is_none();
        let base = existing.unwrap_or_else(|| UserEmulator {
            definition_id: "custom".into(),
            launch_kind: "native".into(),
            ..Default::default()
        });
        let profiles = if create {
            heronite_emulation::store::default_profiles("custom", None)
        } else {
            base.profiles.clone()
        };
        let mut this = Self {
            paths: paths.clone(),
            create,
            definition_id: base.definition_id.clone(),
            launch_kind: if base.launch_kind.is_empty() { "native".into() } else { base.launch_kind.clone() },
            profiles,
            scanners: vec![],
            detected: vec![],
            name: new_input(window, cx, &base.name, "Name"),
            executable: new_input(window, cx, if base.launch_kind == "flatpak" { "" } else { &base.executable }, ""),
            install_dir: new_input(window, cx, base.install_dir.as_deref().unwrap_or(""), ""),
            flatpak_id: new_input(window, cx, base.flatpak_id.as_deref().unwrap_or(""), "org.example.Emulator"),
            base,
            open_profile: None,
            form: None,
            menu: None,
            pending: vec![],
            saving: false,
            error: String::new(),
            saved: None,
        };
        // Folders that point at this emulator (profiles in use) and, for new
        // emulators, what is installed on this PC (prefills the executable).
        let (p, want_detect) = (paths, create);
        let load = move || {
            let scanners = list_rom_scanners(&p).unwrap_or_default();
            let detected = if want_detect {
                let existing = heronite_emulation::list_user_emulators(&p).unwrap_or_default();
                heronite_emulation::discover::discover_emulators(&SystemEnv, &existing)
            } else {
                vec![]
            };
            (scanners, detected)
        };
        // Screenshots: the hidden window does not repaint late results.
        if std::env::var_os("HERONITE_DEBUG_SYNC").is_some() {
            let (scanners, detected) = load();
            this.scanners = scanners;
            this.detected = detected;
            this.prefill_executable(cx);
            return this;
        }
        cx.spawn(async move |this, cx| {
            let (scanners, detected) = cx.background_spawn(async move { load() }).await;
            this.update(cx, |this, cx| {
                this.scanners = scanners;
                this.detected = detected;
                this.prefill_executable(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        this
    }

    /// Debug helper: opens the first profile for editing.
    pub fn open_first_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_profile(0, window, cx);
    }

    /// Debug helper: picks a catalog definition as if chosen in the select.
    pub fn debug_select_definition(&mut self, id: &str, cx: &mut Context<Self>) {
        self.select_definition(id, cx);
    }

    fn set_later(&mut self, input: &Entity<InputState>, value: impl Into<String>) {
        self.pending.push((input.clone(), value.into()));
    }

    /// Fills the executable from what was detected for the chosen definition
    /// (only while the executable is empty).
    fn prefill_executable(&mut self, cx: &mut Context<Self>) {
        if !self.create || self.definition_id == "custom" || !text_of(&self.executable, cx).is_empty() {
            return;
        }
        let pick = self
            .detected
            .iter()
            .filter(|d| d.definition_id == self.definition_id)
            .min_by_key(|d| d.already_added)
            .cloned();
        if let Some(d) = pick {
            self.launch_kind = d.launch_kind.clone();
            if d.launch_kind == "flatpak" {
                let fid = d.flatpak_id.clone().unwrap_or_default();
                let input = self.flatpak_id.clone();
                self.set_later(&input, fid);
            } else {
                let input = self.executable.clone();
                self.set_later(&input, d.executable.clone());
            }
            self.profiles = profiles_for_definition(&SystemEnv, &self.definition_id, &d.executable, &d.launch_kind);
        }
    }

    fn select_definition(&mut self, id: &str, cx: &mut Context<Self>) {
        let previous = emulator_definition(&self.definition_id).map(|d| d.name.clone());
        self.commit_profile(cx);
        self.definition_id = id.to_string();
        self.menu = None;
        self.open_profile = None;
        self.form = None;
        let def = emulator_definition(id);
        let name_now = text_of(&self.name, cx);
        if name_now.is_empty() || Some(&name_now) == previous.as_ref() {
            let name = if id == "custom" { String::new() } else { def.map(|d| d.name.clone()).unwrap_or_default() };
            let input = self.name.clone();
            self.set_later(&input, name);
        }
        for input in [self.executable.clone(), self.flatpak_id.clone()] {
            self.set_later(&input, "");
        }
        self.launch_kind = "native".into();
        if let Some(fid) = def.and_then(|d| d.flatpak_id.clone()) {
            let input = self.flatpak_id.clone();
            self.set_later(&input, fid);
        }
        self.profiles = profiles_for_definition(&SystemEnv, id, "", "native");
        self.prefill_executable(cx);
        cx.notify();
    }

    fn core_relevant(&self, p: &UserEmulatorProfile) -> bool {
        p.core_path.is_some()
            || p.arguments.contains("{CorePath}")
            || emulator_definition(&self.definition_id)
                .and_then(|d| d.profiles.iter().find(|x| Some(&x.id) == p.definition_profile_id.as_ref()))
                .is_some_and(|d| d.core.is_some())
    }

    fn edit_profile(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        let Some(p) = self.profiles.get(i).cloned() else { return };
        self.open_profile = Some(i);
        self.menu = None;
        self.form = Some(ProfileForm {
            name: new_input(window, cx, &p.name, "Profile name"),
            platform: new_input(window, cx, p.platform_name.as_deref().unwrap_or(""), "Platform (e.g. PlayStation 2)"),
            arguments: new_input(window, cx, &p.arguments, "{ImagePath}"),
            extensions: new_input(window, cx, &p.extensions.join(", "), "iso, cue, chd"),
            core: new_input(window, cx, p.core_path.as_deref().unwrap_or(""), "/path/to/core_libretro.so"),
            working_dir: p.working_dir.clone(),
        });
        cx.notify();
    }

    /// Writes the open profile's inputs back into `self.profiles`.
    fn commit_profile(&mut self, cx: &App) {
        let (Some(i), Some(f)) = (self.open_profile, self.form.as_ref()) else { return };
        let Some(p) = self.profiles.get_mut(i) else { return };
        p.name = text_of(&f.name, cx);
        let platform = text_of(&f.platform, cx);
        p.platform_name = (!platform.is_empty()).then_some(platform);
        p.arguments = text_of(&f.arguments, cx);
        p.extensions = split_extensions(&text_of(&f.extensions, cx));
        let core = text_of(&f.core, cx);
        p.core_path = (!core.is_empty()).then_some(core);
        p.working_dir = f.working_dir.clone();
    }

    fn close_form(&mut self, cx: &App) {
        self.commit_profile(cx);
        self.open_profile = None;
        self.form = None;
    }

    fn add_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        self.profiles.push(UserEmulatorProfile {
            id: new_id(),
            name: "New profile".into(),
            platform_name: Some("Other".into()),
            platform_id: Some("other".into()),
            arguments: "{ImagePath}".into(),
            ..Default::default()
        });
        let i = self.profiles.len() - 1;
        self.edit_profile(i, window, cx);
    }

    /// Catalog profiles of this definition the emulator does not have yet.
    fn missing_catalog_profiles(&self) -> Vec<String> {
        let Some(def) = emulator_definition(&self.definition_id) else { return vec![] };
        if def.id == "custom" {
            return vec![];
        }
        def.profiles
            .iter()
            .filter(|d| !self.profiles.iter().any(|p| p.definition_profile_id.as_deref() == Some(d.id.as_str())))
            .map(|d| d.id.clone())
            .collect()
    }

    fn add_catalog_profiles(&mut self, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        let exe = text_of(&self.executable, cx);
        let kind = self.launch_kind.clone();
        let fresh = profiles_for_definition(&SystemEnv, &self.definition_id, &exe, &kind);
        for id in self.missing_catalog_profiles() {
            // Prefer the version with its core resolved.
            let p = fresh
                .iter()
                .find(|p| p.definition_profile_id.as_deref() == Some(id.as_str()))
                .cloned()
                .or_else(|| catalog_profile(&self.definition_id, &id));
            if let Some(p) = p {
                self.profiles.push(p);
            }
        }
        cx.notify();
    }

    fn remove_profile(&mut self, i: usize, cx: &mut Context<Self>) {
        self.commit_profile(cx);
        if self.profiles.len() <= 1 || i >= self.profiles.len() {
            return;
        }
        self.profiles.remove(i);
        self.open_profile = None;
        self.form = None;
        cx.notify();
    }

    fn profile_users(&self, p: &UserEmulatorProfile) -> usize {
        if self.create {
            return 0;
        }
        heronite_emulation::manager::scanners_using_profile(&self.scanners, &self.base.id, &p.id)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.commit_profile(cx);
        let mut emulator = self.base.clone();
        emulator.definition_id = self.definition_id.clone();
        emulator.name = text_of(&self.name, cx);
        emulator.launch_kind = self.launch_kind.clone();
        if self.launch_kind == "flatpak" {
            let id = text_of(&self.flatpak_id, cx);
            if id.is_empty() {
                self.error = "Enter the Flatpak application id (for example org.DolphinEmu.dolphin-emu).".into();
                cx.notify();
                return;
            }
            emulator.flatpak_id = Some(id);
            emulator.executable = "flatpak".into();
        } else {
            let exe = text_of(&self.executable, cx);
            if exe.is_empty() {
                self.error = "Choose the emulator's executable.".into();
                cx.notify();
                return;
            }
            emulator.flatpak_id = None;
            emulator.executable = exe;
        }
        let dir = text_of(&self.install_dir, cx);
        emulator.install_dir = (!dir.is_empty()).then_some(dir);
        if let Some(bad) = self.profiles.iter().find(|p| p.name.trim().is_empty()) {
            self.error = format!("Every profile needs a name ({}).", bad.platform_name.clone().unwrap_or_default());
            cx.notify();
            return;
        }
        emulator.profiles = self.profiles.clone();
        let (paths, create) = (self.paths.clone(), self.create);
        self.saving = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if create {
                        insert_user_emulator(&paths, emulator)
                    } else {
                        replace_user_emulator(&paths, emulator)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(e) => {
                        this.saved = Some(e);
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for EmulatorEditDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for (input, value) in std::mem::take(&mut self.pending) {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        let flatpak = self.launch_kind == "flatpak";
        let exe_value = text_of(&self.executable, cx);

        let mut body = div().flex().flex_col().gap(px(12.));
        if self.create {
            let mut options: Vec<(SharedString, bool)> = vec![("Custom".into(), self.definition_id == "custom")];
            let defs: Vec<_> = emulator_definitions().iter().filter(|d| d.id != "custom").collect();
            options.extend(defs.iter().map(|d| (SharedString::from(d.name.clone()), d.id == self.definition_id)));
            let current = emulator_definition(&self.definition_id).map(|d| d.name.clone()).unwrap_or_default();
            let ids: Vec<String> =
                std::iter::once("custom".to_string()).chain(defs.iter().map(|d| d.id.clone())).collect();
            body = body
                .child(meta_text(
                    "Pick an emulator from the catalog to start with its platforms and arguments, or Custom to describe any program.",
                ))
                .child(select_field(
                    "emu-def",
                    Some("Emulator"),
                    if self.definition_id == "custom" { "Custom".to_string() } else { current },
                    false,
                    self.menu == Some(Menu::Definition),
                    options,
                    cx,
                    |this: &mut Self, _, cx| {
                        this.menu = if this.menu == Some(Menu::Definition) { None } else { Some(Menu::Definition) };
                        cx.notify();
                    },
                    move |this: &mut Self, i, _, cx| {
                        if let Some(id) = ids.get(i) {
                            this.select_definition(&id.clone(), cx);
                        }
                    },
                ));
        }
        body = body.child(text_field("Name", &self.name));
        let kind_label = KINDS.iter().find(|k| k.0 == self.launch_kind).map(|k| k.1).unwrap_or("Native (Linux)");
        body = body.child(select_field(
            "emu-kind",
            Some("Run as"),
            kind_label,
            false,
            self.menu == Some(Menu::Kind),
            KINDS.iter().map(|k| (SharedString::from(k.1), k.0 == self.launch_kind)).collect(),
            cx,
            |this: &mut Self, _, cx| {
                this.menu = if this.menu == Some(Menu::Kind) { None } else { Some(Menu::Kind) };
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.launch_kind = KINDS[i].0.to_string();
                this.menu = None;
                if this.launch_kind == "flatpak" && text_of(&this.flatpak_id, cx).is_empty() {
                    if let Some(fid) = emulator_definition(&this.definition_id).and_then(|d| d.flatpak_id.clone()) {
                        let input = this.flatpak_id.clone();
                        this.set_later(&input, fid);
                    }
                }
                cx.notify();
            },
        ));
        if flatpak {
            body = body.child(text_field("Flatpak application id", &self.flatpak_id));
        } else {
            let has = !exe_value.is_empty();
            body = body.child(path_field(
                "emu-exe-pick",
                Some("Executable"),
                &self.executable,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.executable.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    } else {
                        choose_path(window, cx, false, "Emulator executable", |this: &mut Self, p, window, cx| {
                            this.executable.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                            if p.to_lowercase().ends_with(".exe") {
                                this.launch_kind = "wine".into();
                            }
                            if this.create {
                                let (def, kind) = (this.definition_id.clone(), this.launch_kind.clone());
                                if emulator_definition(&def).is_some_and(|d| d.profiles.iter().any(|p| p.core.is_some())) {
                                    this.profiles = profiles_for_definition(&SystemEnv, &def, &p, &kind);
                                    this.open_profile = None;
                                    this.form = None;
                                }
                            }
                            cx.notify();
                        });
                    }
                }),
            ));
            if has && !std::path::Path::new(&exe_value).exists() && exe_value.contains('/') {
                body = body.child(div().text_size(px(13.33)).text_color(theme::danger()).child("This file does not exist."));
            }
        }
        let dir_value = text_of(&self.install_dir, cx);
        let has_dir = !dir_value.is_empty();
        body = body
            .child(path_field(
                "emu-dir-pick",
                Some("Install folder (optional)"),
                &self.install_dir,
                has_dir,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has_dir {
                        this.install_dir.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    } else {
                        choose_path(window, cx, true, "Install folder", |this: &mut Self, p, window, cx| {
                            this.install_dir.update(cx, |s, cx| s.set_value(p, window, cx));
                            cx.notify();
                        });
                    }
                }),
            ))
            .child(meta_text("{EmulatorDir} in arguments. Leave empty to use the executable's folder."));

        // Profiles.
        body = body.child(section_title("Profiles")).child(meta_text(
            "One per platform. Arguments may use {ImagePath} (the ROM), {ImageName}, {ImageNameNoExt}, {ImageDir}, {EmulatorDir}, {CorePath}, {Name} and {InstallDir}.",
        ));
        let can_remove = self.profiles.len() > 1;
        for i in 0..self.profiles.len() {
            let p = self.profiles[i].clone();
            let users = self.profile_users(&p);
            let open = self.open_profile == Some(i);
            let summary = format!(
                "{}  \u{b7}  {}",
                if p.arguments.is_empty() { "(catalog arguments)".to_string() } else { p.arguments.clone() },
                if p.extensions.is_empty() { "any extension".to_string() } else { p.extensions.join(", ") }
            );
            let remove_block = if !can_remove {
                Some("An emulator needs at least one profile.")
            } else if users > 0 {
                Some("A ROM folder uses this profile.")
            } else {
                None
            };
            let header = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().font_weight(theme::W_BOLD).text_size(px(16.)).truncate().child(profile_label(&p)))
                        .child(div().text_size(px(13.33)).text_color(white()).truncate().child(summary)),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(8.))
                        .child(
                            button(
                                SharedString::from(format!("prof-edit-{i}")),
                                ButtonKind::Outline,
                                if open { "Done" } else { "Edit" },
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.open_profile == Some(i) {
                                    this.close_form(cx);
                                    cx.notify();
                                } else {
                                    this.edit_profile(i, window, cx);
                                }
                            })),
                        )
                        .child(action_button(
                            SharedString::from(format!("prof-rm-{i}")),
                            ButtonKind::Outline,
                            "Remove",
                            remove_block.is_none(),
                            cx.listener(move |this, _, _, cx| this.remove_profile(i, cx)),
                        )),
                );
            let mut card = panel().child(header);
            if let Some(why) = remove_block.filter(|_| open && users > 0) {
                card = card.child(meta_text(why));
            }
            if open {
                if let Some(f) = self.form.as_ref() {
                    let (name, platform, args, ext, core) =
                        (f.name.clone(), f.platform.clone(), f.arguments.clone(), f.extensions.clone(), f.core.clone());
                    let wd = f.working_dir.clone().unwrap_or_default();
                    let wd_label = WORKING_DIRS.iter().find(|w| w.0 == wd).map(|w| w.1).unwrap_or("Default");
                    card = card
                        .child(text_field("Profile name", &name))
                        .child(text_field("Platform", &platform))
                        .child(text_field("Arguments", &args))
                        .child(text_field("ROM extensions (comma-separated)", &ext));
                    if self.core_relevant(&p) {
                        let has_core = !text_of(&core, cx).is_empty();
                        card = card.child(path_field(
                            SharedString::from(format!("prof-core-{i}")),
                            Some("RetroArch core (.so)"),
                            &core,
                            has_core,
                            false,
                            cx.listener(move |this, _, window, cx| {
                                let core = this.form.as_ref().map(|f| f.core.clone());
                                let Some(core) = core else { return };
                                if has_core {
                                    core.update(cx, |s, cx| s.set_value("", window, cx));
                                    cx.notify();
                                } else {
                                    choose_path(window, cx, false, "Libretro core", move |_: &mut Self, p, window, cx| {
                                        core.update(cx, |s, cx| s.set_value(p, window, cx));
                                        cx.notify();
                                    });
                                }
                            }),
                        ));
                    }
                    card = card.child(select_field(
                        "prof-wd",
                        Some("Working folder"),
                        wd_label,
                        false,
                        self.menu == Some(Menu::WorkingDir),
                        WORKING_DIRS.iter().map(|w| (SharedString::from(w.1), w.0 == wd)).collect(),
                        cx,
                        |this: &mut Self, _, cx| {
                            this.menu = if this.menu == Some(Menu::WorkingDir) { None } else { Some(Menu::WorkingDir) };
                            cx.notify();
                        },
                        |this: &mut Self, i, _, cx| {
                            if let Some(f) = this.form.as_mut() {
                                f.working_dir = (!WORKING_DIRS[i].0.is_empty()).then(|| WORKING_DIRS[i].0.to_string());
                            }
                            this.menu = None;
                            cx.notify();
                        },
                    ));
                }
            }
            body = body.child(card);
        }
        let missing = self.missing_catalog_profiles();
        let mut adders = div().flex().gap(px(8.)).child(
            button("prof-add", ButtonKind::Outline, "Add profile").on_click(cx.listener(|this, _, window, cx| this.add_profile(window, cx))),
        );
        if !missing.is_empty() {
            adders = adders.child(
                button("prof-add-catalog", ButtonKind::Outline, format!("Add {} catalog profile(s)", missing.len()))
                    .on_click(cx.listener(|this, _, _, cx| this.add_catalog_profiles(cx))),
            );
        }
        body = body.child(adders);
        if !self.error.is_empty() {
            body = body.child(div().text_color(theme::danger()).text_size(px(13.33)).child(self.error.clone()));
        }

        let footer = div()
            .flex()
            .gap(px(16.))
            .child(button("emu-edit-cancel", ButtonKind::Outline, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "emu-edit-save",
                ButtonKind::Default,
                if self.saving {
                    "Saving\u{2026}"
                } else if self.create {
                    "Add emulator"
                } else {
                    "Save"
                },
                !self.saving,
                cx.listener(|this, _, _, cx| this.save(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "emulator-edit".into(),
                title: if self.create { "Add emulator" } else { "Edit emulator" }.into(),
                width: 720f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
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

#[cfg(test)]
mod tests {
    use super::split_extensions;

    #[test]
    fn extensions_are_normalised() {
        assert_eq!(split_extensions(".ISO, cue  chd,,iso"), ["iso", "cue", "chd"]);
        assert!(split_extensions("  ").is_empty());
    }
}
