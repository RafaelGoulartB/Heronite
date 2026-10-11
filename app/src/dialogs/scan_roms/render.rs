//! Rendering of the dialog's sections.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{profile_label, text_of, ScanRomsDialog};
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, mini_checkbox, ButtonKind};
use crate::ui::fields::{choose_path, panel, path_field, select_field, text_field};

impl ScanRomsDialog {
    pub(super) fn render_emulator_row(&self, cx: &mut Context<Self>) -> Div {
        let emulators = &self.state.emulators;
        let emu_name = self.emulator().map(|e| e.name.clone()).unwrap_or_else(|| "No emulators yet".into());
        let emu_opts: Vec<(SharedString, bool)> = emulators.iter().map(|e| (e.name.clone().into(), e.id == self.emulator_id)).collect();

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
        div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(div().flex_1().min_w(px(220.)).child(emu_select))
            .child(action_button(
                "scan-detect",
                ButtonKind::Glass,
                if self.detecting { "Detecting…" } else { "Detect on this PC" },
                !self.detecting,
                cx.listener(|this, _, _, cx| this.detect(cx)),
            ))
            .child(button("scan-custom", ButtonKind::Glass, "Custom…").on_click(cx.listener(|this, _, _, cx| {
                this.custom_open = !this.custom_open;
                cx.notify();
            })))
    }

    pub(super) fn render_custom(&self, busy: bool, cx: &mut Context<Self>) -> Option<Div> {
        self.custom_open.then(|| {
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
                    ButtonKind::Glass,
                    "Save emulator",
                    has_exe && !busy,
                    cx.listener(|this, _, _, cx| this.add_custom(cx)),
                )))
        })
    }

    pub(super) fn render_profile_select(&self, cx: &mut Context<Self>) -> Div {
        let profile_name = self.profile().map(profile_label).unwrap_or_default();
        let profile_opts: Vec<(SharedString, bool)> = self
            .emulator()
            .map(|e| e.profiles.iter().map(|p| (profile_label(p).into(), p.id == self.profile_id)).collect())
            .unwrap_or_default();

        select_field(
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
        )
    }

    pub(super) fn render_folder(&self, dir_value: &str, cx: &mut Context<Self>) -> Div {
        path_field("scan-folder-pick", Some("ROM folder"), &self.directory, !dir_value.is_empty(), false, {
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
        })
    }

    pub(super) fn render_toggles(&self, cx: &mut Context<Self>) -> Div {
        panel()
            .child(checkbox("scan-subfolders", self.scan_subfolders, "Include subfolders").on_click(cx.listener(|this, _, _, cx| {
                this.scan_subfolders = !this.scan_subfolders;
                cx.notify();
            })))
            .child(checkbox("scan-merge", self.merge_related, "Merge discs into one game").on_click(cx.listener(|this, _, _, cx| {
                this.merge_related = !this.merge_related;
                cx.notify();
            })))
            .child(checkbox("scan-auto", self.auto_scan, "Scan this folder when Collection refreshes").on_click(cx.listener(
                |this, _, _, cx| {
                    this.auto_scan = !this.auto_scan;
                    cx.notify();
                },
            )))
    }

    pub(super) fn render_preview(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.preview.as_ref().map(|p| {
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
                        .py(px(8.))
                        .rounded(px(theme::R_ITEM))
                        .when(imported, |d| d.opacity(0.6))
                        .when(!imported, |d| {
                            d.cursor_pointer().hover(|s| s.bg(theme::menu_item_hover())).on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(g) = this.preview.as_mut().and_then(|p| p.games.get_mut(i)) {
                                    g.import = !g.import;
                                }
                                cx.notify();
                            }))
                        })
                        .child(mini_checkbox(SharedString::from(format!("scan-cb-{i}")), g.import, imported))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(theme::W_BOLD)
                                        .text_size(px(14.))
                                        .text_color(theme::text_primary())
                                        .child(g.title.clone()),
                                )
                                .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(sub)),
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
                        .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(format!("{} game(s) found", p.games.len())))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(button("scan-all", ButtonKind::Glass, "Select all").on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_all(true);
                                    cx.notify();
                                })))
                                .child(button("scan-none", ButtonKind::Glass, "Select none").on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_all(false);
                                    cx.notify();
                                }))),
                        ),
                )
                .child(crate::ui::controls::card_box().py(px(6.)).child(list))
        })
    }
}
