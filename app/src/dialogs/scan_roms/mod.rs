//! `ScanRomsDialog` ("Add emulated games"): pick an emulator + profile and a
//! ROM folder, preview what a scan finds and import it as Collection games.

mod actions;
mod render;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::{get_emulation_state, EmulationState, RomScanPreview, SystemEnv};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::{text_of, DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{meta_text, new_input, on_commit, paragraph};

/// Where the dialog starts (from a saved scanner / the emulator manager).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanInitial {
    pub emulator_id: Option<String>,
    pub profile_id: Option<String>,
    pub directory: Option<String>,
}

pub struct ScanRomsDialog {
    ctx: DialogCtx,
    initial: ScanInitial,
    state: EmulationState,
    emulator_id: String,
    profile_id: String,
    scanner_id: Option<String>,
    directory: Entity<InputState>,
    scan_subfolders: bool,
    merge_related: bool,
    auto_scan: bool,
    preview: Option<RomScanPreview>,
    loading: bool,
    detecting: bool,
    error: String,
    pending_dir: Option<String>,
    emu_open: bool,
    profile_open: bool,
    custom_open: bool,
    custom_name: Entity<InputState>,
    custom_exe: Entity<InputState>,
    custom_ext: Entity<InputState>,
    custom_args: Entity<InputState>,
}

impl EventEmitter<DialogEvent> for ScanRomsDialog {}

impl ScanRomsDialog {
    pub fn new(ctx: DialogCtx, initial: ScanInitial, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let directory = new_input(window, cx, initial.directory.as_deref().unwrap_or(""), "");
        on_commit(&directory, window, cx, |this: &mut Self, _, _, cx| {
            this.preview = None;
            cx.notify();
        });
        let this = Self {
            ctx,
            initial,
            state: EmulationState { catalog: vec![], emulators: vec![], scanners: vec![], detected: vec![] },
            emulator_id: String::new(),
            profile_id: String::new(),
            scanner_id: None,
            directory,
            scan_subfolders: true,
            merge_related: true,
            auto_scan: true,
            preview: None,
            loading: false,
            detecting: false,
            error: String::new(),
            pending_dir: None,
            emu_open: false,
            profile_open: false,
            custom_open: false,
            custom_name: new_input(window, cx, "", ""),
            custom_exe: new_input(window, cx, "", ""),
            custom_ext: new_input(window, cx, "iso,cue,chd,zip", ""),
            custom_args: new_input(window, cx, "{ImagePath}", ""),
        };
        let mut this = this;
        // Screenshots: the hidden window does not repaint late results.
        if crate::env::flag("HERONITE_DEBUG_SYNC") {
            if let Ok(state) = get_emulation_state(&this.ctx.heroic, &SystemEnv, false) {
                this.state = state;
                this.apply_initial(cx);
            }
            return this;
        }
        this.reload(true, true, cx);
        this
    }

    /// Reloads the emulation state in the background. `first` applies the
    /// initial selection.
    fn reload(&self, detect: bool, first: bool, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let state = cx.background_spawn(async move { get_emulation_state(&paths, &SystemEnv, detect) }).await;
            this.update(cx, |this, cx| {
                match state {
                    Ok(s) => {
                        this.state = s;
                        if first {
                            this.apply_initial(cx);
                        } else if this.emulator_id.is_empty() {
                            if let Some(e) = this.state.emulators.first() {
                                let (e, p) = (e.id.clone(), e.profiles.first().map(|p| p.id.clone()).unwrap_or_default());
                                this.select_target(&e, &p, None, cx);
                            }
                        }
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_initial(&mut self, cx: &mut Context<Self>) {
        let init = self.initial.clone();
        let first = init
            .emulator_id
            .as_ref()
            .and_then(|id| self.state.emulators.iter().find(|e| &e.id == id))
            .or_else(|| self.state.emulators.first());
        if let Some(e) = first {
            let profile = init
                .profile_id
                .filter(|p| e.profiles.iter().any(|x| &x.id == p))
                .or_else(|| e.profiles.first().map(|p| p.id.clone()))
                .unwrap_or_default();
            let id = e.id.clone();
            self.select_target(&id, &profile, init.directory.as_deref(), cx);
        }
    }

    fn select_target(&mut self, emulator: &str, profile: &str, keep_dir: Option<&str>, cx: &mut Context<Self>) {
        self.emulator_id = emulator.to_string();
        self.profile_id = profile.to_string();
        self.preview = None;
        self.emu_open = false;
        self.profile_open = false;
        let saved = self.state.scanners.iter().find(|s| s.emulator_id == emulator && s.profile_id == profile).cloned();
        self.scanner_id = saved.as_ref().map(|s| s.id.clone());
        let dir = keep_dir.filter(|d| !d.is_empty()).map(str::to_string).or_else(|| saved.as_ref().map(|s| s.directory.clone())).unwrap_or_default();
        self.pending_dir = Some(dir);
        self.scan_subfolders = saved.as_ref().map(|s| s.scan_subfolders).unwrap_or(true);
        self.merge_related = saved.as_ref().map(|s| s.merge_related_files).unwrap_or(true);
        self.auto_scan = saved.as_ref().map(|s| s.auto_scan_on_refresh).unwrap_or(true);
        cx.notify();
    }

    fn emulator(&self) -> Option<&UserEmulator> {
        self.state.emulators.iter().find(|e| e.id == self.emulator_id)
    }

    fn profile(&self) -> Option<&UserEmulatorProfile> {
        let e = self.emulator()?;
        e.profiles.iter().find(|p| p.id == self.profile_id).or_else(|| e.profiles.first())
    }
}

/// What the profile select shows: the profile name, followed by its platform
/// when that adds information (a custom profile's platform is just "Other").
pub(crate) fn profile_label(p: &UserEmulatorProfile) -> String {
    let platform = p.platform_name.as_deref().map(str::trim).unwrap_or("");
    let name = p.name.trim();
    if name.is_empty() {
        return if platform.is_empty() || platform.eq_ignore_ascii_case("other") { "Profile".into() } else { platform.into() };
    }
    if platform.is_empty() || platform.eq_ignore_ascii_case("other") || platform.eq_ignore_ascii_case(name) {
        name.into()
    } else {
        format!("{name} \u{b7} {platform}")
    }
}

impl Render for ScanRomsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The directory input is created before the state arrives; apply a
        // pending value here where a `Window` is at hand.
        if let Some(dir) = self.pending_dir.take() {
            self.directory.update(cx, |s, cx| s.set_value(dir, window, cx));
        }
        let dir_value = text_of(&self.directory, cx);
        let busy = self.loading;
        let missing = self.state.detected.iter().filter(|d| !d.already_added).count();
        let selected_count = self.preview.as_ref().map(|p| p.games.iter().filter(|g| g.import).count()).unwrap_or(0);

        let row = self.render_emulator_row(cx);
        let custom = self.render_custom(busy, cx);
        let profile_select = self.render_profile_select(cx);
        let folder = self.render_folder(&dir_value, cx);
        let toggles = self.render_toggles(cx);

        let can_scan = !self.emulator_id.is_empty() && !dir_value.is_empty() && !busy;
        let scan_button = action_button(
            "scan-run",
            ButtonKind::Glass,
            if busy && self.preview.is_none() { "Scanning…" } else { "Scan folder" },
            can_scan,
            cx.listener(|this, _, _, cx| this.scan(cx)),
        );
        let preview = self.render_preview(cx);

        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(
                "Pick an emulator, its platform profile, and the folder with ROMs. Collection scans that folder and adds the games.",
            ))
            .child(row)
            .children((missing > 0).then(|| meta_text(format!("{missing} emulator(s) found on this PC."))))
            .children(custom)
            .child(profile_select)
            .child(folder)
            .child(toggles)
            .child(div().flex().child(scan_button))
            .children(preview)
            .children((!self.error.is_empty()).then(|| {
                div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())
            }))
            ;

        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("scan-close", ButtonKind::Ghost, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "scan-import",
                ButtonKind::Primary,
                format!("Add {selected_count} to Collection"),
                self.preview.is_some() && selected_count > 0 && !busy,
                cx.listener(|this, _, _, cx| this.import(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "scan-roms".into(),
                title: "Add emulated games".into(),
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
    use super::profile_label;
    use heronite_launch::emulators::UserEmulatorProfile;

    fn profile(name: &str, platform: Option<&str>) -> UserEmulatorProfile {
        UserEmulatorProfile { name: name.into(), platform_name: platform.map(Into::into), ..Default::default() }
    }

    #[test]
    fn profile_label_shows_the_name_when_the_platform_is_other() {
        assert_eq!(profile_label(&profile("PCSX2", Some("Other"))), "PCSX2");
        assert_eq!(profile_label(&profile("PCSX2", None)), "PCSX2");
        assert_eq!(profile_label(&profile("PlayStation 2", Some("PlayStation 2"))), "PlayStation 2");
        assert_eq!(profile_label(&profile("SNES", Some("Super Nintendo"))), "SNES \u{b7} Super Nintendo");
        assert_eq!(profile_label(&profile("", Some("Other"))), "Profile");
        assert_eq!(profile_label(&profile("", Some("Arcade"))), "Arcade");
    }
}
