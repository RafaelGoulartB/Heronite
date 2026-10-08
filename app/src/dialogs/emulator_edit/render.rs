//! Rendering of the emulator form pieces.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::catalog::emulator_definition;
use heronite_emulation::detect::profiles_for_definition;
use heronite_emulation::{emulator_definitions, SystemEnv};

use super::{text_of, EmulatorEditDialog, Menu, KINDS, WORKING_DIRS};
use crate::dialogs::scan_roms::profile_label;
use crate::dialogs::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, caption, ButtonKind};
use crate::ui::fields::{choose_path, meta_text, panel, path_field, select_field, text_field};

impl EmulatorEditDialog {
    pub(super) fn render_fields(&mut self, mut body: Div, cx: &mut Context<Self>) -> Div {
        let flatpak = self.launch_kind == "flatpak";
        let exe_value = text_of(&self.executable, cx);
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
                body = body.child(div().text_size(px(13.33)).text_color(theme::danger_text()).child("This file does not exist."));
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
        body
    }

    pub(super) fn render_profiles(&mut self, mut body: Div, cx: &mut Context<Self>) -> Div {
        body = body.child(caption("Profiles")).child(meta_text(
            "One per platform. Arguments may use {ImagePath} (the ROM), {ImageName}, {ImageNameNoExt}, {ImageDir}, {EmulatorDir}, {CorePath}, {Name} and {InstallDir}.",
        ));
        for i in 0..self.profiles.len() {
            body = body.child(self.render_profile_card(i, cx));
        }
        let missing = self.missing_catalog_profiles();
        let mut adders = div().flex().gap(px(8.)).child(
            button("prof-add", ButtonKind::Glass, "Add profile").on_click(cx.listener(|this, _, window, cx| this.add_profile(window, cx))),
        );
        if !missing.is_empty() {
            adders = adders.child(
                button("prof-add-catalog", ButtonKind::Glass, format!("Add {} catalog profile(s)", missing.len()))
                    .on_click(cx.listener(|this, _, _, cx| this.add_catalog_profiles(cx))),
            );
        }
        body.child(adders)
    }

    fn render_profile_card(&self, i: usize, cx: &mut Context<Self>) -> Div {
        let can_remove = self.profiles.len() > 1;
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
                            ButtonKind::Glass,
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
                        ButtonKind::Glass,
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
        card
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .gap(px(10.))
            .child(button("emu-edit-cancel", ButtonKind::Ghost, "Cancel").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "emu-edit-save",
                ButtonKind::Primary,
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
            .into_any_element()
    }
}
