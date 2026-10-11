//! Add / edit one emulator: name, executable and launch kind, and its
//! profiles (platform, arguments, extensions, RetroArch core). Used by the
//! Emulators page. Writes `local_library/emulators.json` through
//! `heronite_emulation::store`.

mod console;
mod profiles;
mod render;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_emulation::catalog::emulator_definition;
use heronite_emulation::detect::profiles_for_definition;
use heronite_emulation::store::{insert_user_emulator, replace_user_emulator};
use heronite_emulation::{list_rom_scanners, DetectedEmulator, RomScanner, SystemEnv};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::{text_of, DialogEvent};
use crate::theme;
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::new_input;

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
    paths: DataPaths,
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
    console_list: crate::console::list::ListNav,
    console_profile_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for EmulatorEditDialog {}

impl EmulatorEditDialog {
    /// Adds a new emulator (`existing` = `None`) or edits one.
    pub fn new(paths: DataPaths, existing: Option<UserEmulator>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let create = existing.is_none();
        let base =
            existing.unwrap_or_else(|| UserEmulator { definition_id: "custom".into(), launch_kind: "native".into(), ..Default::default() });
        let profiles = if create { heronite_emulation::store::default_profiles("custom", None) } else { base.profiles.clone() };
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
            console_list: Default::default(),
            console_profile_list: Default::default(),
            layer: Default::default(),
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
        if crate::env::flag("HERONITE_DEBUG_SYNC") {
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

    /// Debug helper: fills the form as a user would and saves.
    pub fn debug_fill_and_save(&mut self, definition: &str, executable: &str, name: Option<&str>, cx: &mut Context<Self>) {
        if self.create {
            self.select_definition(definition, cx);
        }
        let input = self.executable.clone();
        self.set_later(&input, executable);
        if let Some(n) = name {
            let input = self.name.clone();
            self.set_later(&input, n);
        }
        // Inputs only take values on render; save from the next frame.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            this.update(cx, |this, cx| this.save(cx)).ok();
        })
        .detach();
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
        let pick = self.detected.iter().filter(|d| d.definition_id == self.definition_id).min_by_key(|d| d.already_added).cloned();
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
        if crate::console::active() {
            return self.render_console(cx);
        }
        let mut body = div().flex().flex_col().gap(px(12.));
        body = self.render_fields(body, cx);
        body = self.render_profiles(body, cx);
        if !self.error.is_empty() {
            body = body.child(div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone()));
        }

        let footer = self.render_footer(cx);
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
