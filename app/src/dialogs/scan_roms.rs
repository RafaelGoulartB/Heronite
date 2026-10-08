//! `ScanRomsDialog` ("Add emulated games"): pick an emulator + profile and a
//! ROM folder, preview what a scan finds and import it as Collection games.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::{
    add_detected_emulators, get_emulation_state, import_scanned_roms, preview_rom_scan, upsert_user_emulator,
    EmulationState, RomScanArgs, RomScanImportArgs, RomScanPreview, SystemEnv, UpsertUserEmulatorArgs,
};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{
    choose_path, meta_text, new_input, on_commit, panel, paragraph, path_field, select_field,
    text_field,
};
use crate::ui::icon;

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

fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}

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

    fn detect(&mut self, cx: &mut Context<Self>) {
        self.detecting = true;
        self.error.clear();
        let paths = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { add_detected_emulators(&paths, &SystemEnv) }).await;
            this.update(cx, |this, cx| {
                this.detecting = false;
                match res {
                    Ok(list) => {
                        this.reload(true, false, cx);
                        if list.is_empty() {
                            this.error = "No emulators were found on PATH or Flatpak. Add one manually.".into();
                        }
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

    fn add_custom(&mut self, cx: &mut Context<Self>) {
        let exe = text_of(&self.custom_exe, cx);
        if exe.is_empty() {
            return;
        }
        let name = text_of(&self.custom_name, cx);
        let extensions: Vec<String> = text_of(&self.custom_ext, cx)
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(|e| e.trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        let args = text_of(&self.custom_args, cx);
        let paths = self.ctx.heroic.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let display = if name.is_empty() { "Custom emulator".to_string() } else { name.clone() };
                    upsert_user_emulator(
                        &paths,
                        UpsertUserEmulatorArgs {
                            definition_id: "custom".into(),
                            name: display,
                            launch_kind: if exe.to_lowercase().ends_with(".exe") { "wine" } else { "native" }.into(),
                            executable: exe,
                            profiles: Some(vec![UserEmulatorProfile {
                                id: "custom".into(),
                                definition_profile_id: Some("custom".into()),
                                name: if name.is_empty() { "Custom".into() } else { name },
                                platform_name: Some("Other".into()),
                                arguments: if args.is_empty() { "{ImagePath}".into() } else { args },
                                extensions,
                                ..Default::default()
                            }]),
                            ..Default::default()
                        },
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(e) => {
                        this.custom_open = false;
                        // Select it as soon as the state is reloaded.
                        this.initial = ScanInitial {
                            emulator_id: Some(e.id.clone()),
                            profile_id: e.profiles.first().map(|p| p.id.clone()),
                            directory: None,
                        };
                        this.reload_then_initial(cx);
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

    fn reload_then_initial(&self, cx: &mut Context<Self>) {
        self.reload(false, true, cx);
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let directory = text_of(&self.directory, cx);
        let Some(profile) = self.profile().map(|p| p.id.clone()) else { return };
        if self.emulator_id.is_empty() || directory.is_empty() {
            return;
        }
        let args = RomScanArgs {
            emulator_id: self.emulator_id.clone(),
            profile_id: profile,
            directory,
            scan_subfolders: Some(self.scan_subfolders),
            merge_related_files: Some(self.merge_related),
        };
        let paths = self.ctx.heroic.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { preview_rom_scan(&paths, &args) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(p) => {
                        if !p.errors.is_empty() {
                            this.error = p.errors.join("\n");
                        }
                        this.preview = Some(p);
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

    fn import(&mut self, cx: &mut Context<Self>) {
        let (Some(preview), Some(profile)) = (self.preview.clone(), self.profile().map(|p| p.id.clone())) else {
            return;
        };
        let args = RomScanImportArgs {
            emulator_id: self.emulator_id.clone(),
            profile_id: profile,
            directory: text_of(&self.directory, cx),
            scanner_id: self.scanner_id.clone(),
            scan_subfolders: Some(self.scan_subfolders),
            merge_related_files: Some(self.merge_related),
            auto_scan_on_refresh: Some(self.auto_scan),
            games: preview.games,
        };
        let paths = self.ctx.heroic.clone();
        self.loading = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { import_scanned_roms(&paths, &args) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(r) if r.errors.is_empty() => cx.emit(DialogEvent::ChangedAndClose),
                    Ok(r) => {
                        this.error = r.errors.join("\n");
                        cx.emit(DialogEvent::Changed);
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

    fn toggle_all(&mut self, on: bool) {
        if let Some(p) = self.preview.as_mut() {
            for g in p.games.iter_mut().filter(|g| !g.already_imported) {
                g.import = on;
            }
        }
    }
}

/// What the profile select shows: the profile name, followed by its platform
/// when that adds information (a custom profile's platform is just "Other").
pub(crate) fn profile_label(p: &UserEmulatorProfile) -> String {
    let platform = p.platform_name.as_deref().map(str::trim).unwrap_or("");
    let name = p.name.trim();
    if name.is_empty() {
        return if platform.is_empty() { "Profile".into() } else { platform.into() };
    }
    if platform.is_empty() || platform.eq_ignore_ascii_case("other") || platform.eq_ignore_ascii_case(name) {
        name.into()
    } else {
        format!("{name} \u{b7} {platform}")
    }
}

pub(crate) fn mini_checkbox(id: impl Into<ElementId>, checked: bool, disabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(18.))
        .flex_none()
        .mt(px(2.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border(px(2.))
        .border_color(theme::accent())
        .when(checked, |d| d.bg(theme::accent()).child(icon(Icon::Check, px(14.), theme::background())))
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| d.cursor_pointer())
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
        let emulators = &self.state.emulators;
        let emu_name = self.emulator().map(|e| e.name.clone()).unwrap_or_else(|| "No emulators yet".into());
        let emu_opts: Vec<(SharedString, bool)> =
            emulators.iter().map(|e| (e.name.clone().into(), e.id == self.emulator_id)).collect();
        let profile_name = self.profile().map(profile_label).unwrap_or_default();
        let profile_opts: Vec<(SharedString, bool)> = self
            .emulator()
            .map(|e| {
                e.profiles
                    .iter()
                    .map(|p| (profile_label(p).into(), p.id == self.profile_id))
                    .collect()
            })
            .unwrap_or_default();
        let missing = self.state.detected.iter().filter(|d| !d.already_added).count();
        let selected_count = self.preview.as_ref().map(|p| p.games.iter().filter(|g| g.import).count()).unwrap_or(0);

        let emu_select = select_field(
            "scan-emulator",
            Some("Emulator"),
            emu_name,
            emulators.is_empty(),
            self.emu_open,
            emu_opts,
            cx,
            |this: &mut Self, _, cx| {
                this.emu_open = !this.emu_open;
                this.profile_open = false;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                if let Some(e) = this.state.emulators.get(i) {
                    let (id, p) = (e.id.clone(), e.profiles.first().map(|p| p.id.clone()).unwrap_or_default());
                    this.select_target(&id, &p, None, cx);
                }
            },
        );
        let row = div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(div().flex_1().min_w(px(220.)).child(emu_select))
            .child(action_button(
                "scan-detect",
                ButtonKind::Outline,
                if self.detecting { "Detecting…" } else { "Detect on this PC" },
                !self.detecting,
                cx.listener(|this, _, _, cx| this.detect(cx)),
            ))
            .child(button("scan-custom", ButtonKind::Outline, "Custom…").on_click(cx.listener(|this, _, _, cx| {
                this.custom_open = !this.custom_open;
                cx.notify();
            })));

        let custom = self.custom_open.then(|| {
            let has_exe = !text_of(&self.custom_exe, cx).is_empty();
            panel()
                .gap(px(8.))
                .child(text_field("Name", &self.custom_name))
                .child(path_field(
                    "custom-exe-pick",
                    Some("Executable"),
                    &self.custom_exe,
                    has_exe,
                    false,
                    cx.listener(move |this, _, window, cx| {
                        if has_exe {
                            this.custom_exe.update(cx, |s, cx| s.set_value("", window, cx));
                            cx.notify();
                        } else {
                            choose_path(window, cx, false, "Executable", |this: &mut Self, p, window, cx| {
                                this.custom_exe.update(cx, |s, cx| s.set_value(p, window, cx));
                                cx.notify();
                            });
                        }
                    }),
                ))
                .child(text_field("ROM extensions (comma-separated)", &self.custom_ext))
                .child(text_field("Arguments ({ImagePath} is the ROM)", &self.custom_args))
                .child(div().flex().child(action_button(
                    "custom-save",
                    ButtonKind::Default,
                    "Save emulator",
                    has_exe && !busy,
                    cx.listener(|this, _, _, cx| this.add_custom(cx)),
                )))
        });

        let profile_select = select_field(
            "scan-profile",
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
                    let e = this.emulator_id.clone();
                    this.select_target(&e, &p, None, cx);
                }
            },
        );

        let folder = path_field(
            "scan-folder-pick",
            Some("ROM folder"),
            &self.directory,
            !dir_value.is_empty(),
            false,
            {
                let has = !dir_value.is_empty();
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.directory.update(cx, |s, cx| s.set_value("", window, cx));
                        this.preview = None;
                        cx.notify();
                    } else {
                        choose_path(window, cx, true, "ROM folder", |this: &mut Self, p, window, cx| {
                            this.directory.update(cx, |s, cx| s.set_value(p, window, cx));
                            this.preview = None;
                            cx.notify();
                        });
                    }
                })
            },
        );

        let toggles = panel()
            .child(checkbox("scan-subfolders", self.scan_subfolders, "Include subfolders").on_click(cx.listener(
                |this, _, _, cx| {
                    this.scan_subfolders = !this.scan_subfolders;
                    cx.notify();
                },
            )))
            .child(checkbox("scan-merge", self.merge_related, "Merge discs into one game").on_click(cx.listener(
                |this, _, _, cx| {
                    this.merge_related = !this.merge_related;
                    cx.notify();
                },
            )))
            .child(
                checkbox("scan-auto", self.auto_scan, "Scan this folder when Collection refreshes").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.auto_scan = !this.auto_scan;
                        cx.notify();
                    }),
                ),
            );

        let can_scan = !self.emulator_id.is_empty() && !dir_value.is_empty() && !busy;
        let scan_button = action_button(
            "scan-run",
            ButtonKind::Default,
            if busy && self.preview.is_none() { "Scanning…" } else { "Scan folder" },
            can_scan,
            cx.listener(|this, _, _, cx| this.scan(cx)),
        );

        let preview = self.preview.as_ref().map(|p| {
            let mut list = div().id("scan-games").flex().flex_col().gap(px(4.)).max_h(px(240.)).overflow_y_scroll();
            for (i, g) in p.games.iter().enumerate() {
                let sub = if g.already_imported {
                    "Already in Collection".to_string()
                } else if g.roms.len() > 1 {
                    format!("{} discs", g.roms.len())
                } else {
                    g.roms.first().map(|r| r.path.clone()).unwrap_or_default()
                };
                let imported = g.already_imported;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("scan-game-{i}")))
                        .flex()
                        .gap(px(12.))
                        .items_start()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(4.))
                        .when(imported, |d| d.opacity(0.6))
                        .when(!imported, |d| {
                            d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.06))).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(g) = this.preview.as_mut().and_then(|p| p.games.get_mut(i)) {
                                        g.import = !g.import;
                                    }
                                    cx.notify();
                                },
                            ))
                        })
                        .child(mini_checkbox(SharedString::from(format!("scan-cb-{i}")), g.import, imported))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().font_weight(theme::W_BOLD).text_size(px(16.)).child(g.title.clone()))
                                .child(div().text_size(px(13.33)).text_color(white()).child(sub)),
                        ),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(div().font_weight(theme::W_BOLD).child(format!("{} game(s) found", p.games.len())))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(button("scan-all", ButtonKind::Outline, "Select all").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.toggle_all(true);
                                        cx.notify();
                                    },
                                )))
                                .child(button("scan-none", ButtonKind::Outline, "Select none").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.toggle_all(false);
                                        cx.notify();
                                    },
                                ))),
                        ),
                )
                .child(list)
        });

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
                div().text_color(theme::danger()).text_size(px(13.33)).child(self.error.clone())
            }))
            ;

        let footer = div()
            .flex()
            .gap(px(16.))
            .child(button("scan-close", ButtonKind::Outline, "Close").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "scan-import",
                ButtonKind::Default,
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
