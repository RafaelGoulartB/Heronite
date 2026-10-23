//! Rendering of the install / import dialog.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::util::file_size_local;
use heronite_i18n::tr;
use serde_json::Value;

use super::{text_of, EpicInstallDialog, Mode};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, path_field, select_field};
use crate::ui::icon;

impl EpicInstallDialog {
    fn size_row(&self) -> Div {
        let have = self.info.is_some();
        let cell = |ic: Icon, label: &str, value: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(ic, px(16.), crate::theme::text_primary()))
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(format!("{label}:")))
                .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value))
        };
        let mut row = div().flex().flex_wrap().gap_x(px(28.)).gap_y(px(6.));
        if have {
            row = row.child(cell(Icon::Download, tr!("Download size"), file_size_local(self.download_bytes()))).child(cell(
                Icon::FaHardDrive,
                tr!("Install size"),
                file_size_local(self.disk_bytes()),
            ));
        } else if let Some(err) = &self.info_error {
            row = row.child(
                div().w_full().text_size(px(14.)).text_color(theme::danger_text()).child(err.lines().next().unwrap_or("").to_string()),
            );
        } else {
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(Icon::FaSpinner, px(16.), crate::theme::text_primary()))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Getting download and install size..."))),
            );
        }
        row
    }

    fn space_line(&self, cx: &App) -> Option<Div> {
        let space = self.space.as_ref()?;
        let after = self.free_after_queue()? - self.disk_bytes();
        let enough = self.info.is_none() || after >= 0.;
        let mut line = div()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .child(tr!("Space available: {space}", space = space.message));
        if self.reserved > 0 {
            line = line.child(tr!("({size} for the download queue)", size = file_size_local(self.reserved as f64)));
        }
        if self.info.is_some() && enough {
            line = line.child(tr!(" - After install: {size}", size = file_size_local(after)));
        }
        if !space.writable {
            line = line.child(div().text_color(theme::caution_text()).child(tr!("Warning: path might not be writable.")));
        }
        if !enough {
            line = line.child(div().text_color(theme::caution_text()).child(tr!("(Not enough disk space)")));
        }
        let _ = cx;
        Some(line)
    }

    fn wine_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.linux_windows_game() {
            return None;
        }
        let toggle = div()
            .id("epic-wine-toggle")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .py(px(8.))
            .text_size(px(14.))
            .text_color(crate::theme::text_primary())
            .on_click(cx.listener(|this, _, _, cx| {
                this.wine_details = !this.wine_details;
                cx.notify();
            }))
            .child(icon(if self.wine_details { Icon::ExpandMore } else { Icon::ChevronRight }, px(18.), crate::theme::text_primary()))
            .child(tr!("Show Wine settings"));
        let mut col = crate::ui::controls::card_box().gap(px(10.)).pb(px(14.)).child(toggle);
        if !self.wine_details {
            return Some(col);
        }
        let shared = self.shared_prefix;
        col = col.child(checkbox("epic-shared-prefix", shared, tr!("Use shared Wine prefix")).on_click(cx.listener(
            |this, _, window, cx| {
                this.shared_prefix = !this.shared_prefix;
                let value = if this.shared_prefix { this.shared_prefix_path.clone() } else { this.suggested_prefix.clone() };
                this.prefix.update(cx, |s, cx| s.set_value(value, window, cx));
                cx.notify();
            },
        )));
        if shared {
            col = col.child(meta_text(tr!("Only use this option if you know what you are doing. Sharing the same prefix for multiple games can create problems if their dependencies are incompatible.")));
        }
        let has_prefix = !text_of(&self.prefix, cx).is_empty();
        col = col.child(path_field(
            "epic-prefix-pick",
            Some(tr!("WinePrefix")),
            &self.prefix,
            has_prefix,
            shared,
            cx.listener(move |this, _, window, cx| {
                choose_path(window, cx, true, tr!("Select WinePrefix folder"), |this: &mut Self, p, window, cx| {
                    this.prefix.update(cx, |s, cx| s.set_value(p, window, cx));
                    cx.notify();
                });
                let _ = this;
            }),
        ));
        let current = if shared {
            self.default_wine.as_ref().and_then(|w| w.get("name")).and_then(Value::as_str).unwrap_or("").to_string()
        } else {
            self.wine_list.get(self.wine_pick).map(|w| w.name.clone()).unwrap_or_else(|| {
                if self.wine_loaded {
                    tr!("No Wine or Proton found").into()
                } else {
                    tr!("Looking for Wine and Proton...").into()
                }
            })
        };
        let options: Vec<(SharedString, bool)> =
            self.wine_list.iter().enumerate().map(|(i, w)| (w.name.clone().into(), i == self.wine_pick)).collect();
        col = col.child(select_field(
            "epic-wine",
            Some(tr!("Wine version:")),
            current,
            shared || self.wine_list.is_empty(),
            self.wine_open,
            options,
            cx,
            |this: &mut Self, _, cx| {
                this.wine_open = !this.wine_open;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.wine_pick = i;
                this.wine_open = false;
                cx.notify();
            },
        ));
        if self.wine_loaded && self.wine_list.is_empty() {
            col = col.child(crate::dialogs::no_proton_note());
        }
        Some(col)
    }
}

impl Render for EpicInstallDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let importing = self.mode == Mode::Import;
        let mut body = div().flex().flex_col().gap(px(14.));

        if let Some((header, detail)) = self.third_party_notice() {
            let mut notice = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(tr!("This game is managed by a third-party application")),
                )
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(header));
            if !detail.is_empty() {
                notice = notice.child(meta_text(detail));
            }
            body = body.child(
                crate::ui::controls::card_box()
                    .flex_row()
                    .gap(px(12.))
                    .p(px(14.))
                    .child(icon(
                        if self.third_party.as_ref().is_some_and(Result::is_ok) { Icon::Check } else { Icon::Warning },
                        px(20.),
                        theme::text_primary(),
                    ))
                    .child(notice.flex_1().min_w_0()),
            );
        } else if importing {
            body =
                body.child(meta_text(tr!("Choose the folder where the game is already installed (the folder that holds the game files).")));
            let has = !text_of(&self.import_path, cx).is_empty();
            body = body.child(path_field(
                "epic-import-pick",
                Some(tr!("Select game folder")),
                &self.import_path,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.import_path.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    } else {
                        choose_path(window, cx, true, tr!("Choose the game folder to import"), |this: &mut Self, p, window, cx| {
                            this.import_path.update(cx, |s, cx| s.set_value(p, window, cx));
                            cx.notify();
                        });
                    }
                }),
            ));
        } else {
            body = body.child(self.size_row());
            let has = !text_of(&self.path, cx).is_empty();
            body = body.child(path_field(
                "epic-path-pick",
                Some(tr!("Select install path")),
                &self.path,
                has,
                false,
                cx.listener(move |_, _, window, cx| {
                    choose_path(window, cx, true, tr!("Select install path"), |this: &mut Self, p, window, cx| {
                        this.path.update(cx, |s, cx| s.set_value(p, window, cx));
                        this.check_space(cx);
                        cx.notify();
                    });
                }),
            ));
            body = body.children(self.space_line(cx));

            if !self.sdl.is_empty() || !self.dlcs.is_empty() {
                body = body.child(crate::ui::controls::caption(tr!("Components to install")));
            }
            if !self.sdl.is_empty() {
                let mut rows: Vec<AnyElement> = Vec::new();
                for (i, (sdl, on)) in self.sdl.iter().enumerate() {
                    let checked = sdl.required || *on;
                    let row = checkbox(SharedString::from(format!("epic-sdl-{i}")), checked, sdl.name.clone());
                    rows.push(
                        if sdl.required {
                            row.opacity(0.7).cursor_not_allowed()
                        } else {
                            row.on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(e) = this.sdl.get_mut(i) {
                                    e.1 = !e.1;
                                }
                                cx.notify();
                            }))
                        }
                        .into_any_element(),
                    );
                }
                body = body.child(crate::ui::controls::card(rows));
            }
            if !self.dlcs.is_empty() {
                let all = self.dlcs.iter().all(|(_, on)| *on);
                let mut rows: Vec<AnyElement> = vec![checkbox("epic-dlc-all", all, tr!("Install all DLCs"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let set = !this.dlcs.iter().all(|(_, on)| *on);
                        for d in this.dlcs.iter_mut() {
                            d.1 = set;
                        }
                        cx.notify();
                    }))
                    .into_any_element()];
                for (i, (dlc, on)) in self.dlcs.iter().enumerate() {
                    rows.push(
                        checkbox(SharedString::from(format!("epic-dlc-{i}")), *on, dlc.title.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(e) = this.dlcs.get_mut(i) {
                                    e.1 = !e.1;
                                }
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
                body = body.child(div().id("epic-dlc-list").max_h(px(240.)).overflow_y_scroll().child(crate::ui::controls::card(rows)));
            }
        }
        if let Some(wine) = self.wine_section(cx) {
            body = body.child(wine);
        }

        let not_enough = self.not_enough();
        let ready = self.can_install(cx);
        let primary = if importing {
            action_button(
                "epic-do-import",
                ButtonKind::Primary,
                tr!("Import"),
                !text_of(&self.import_path, cx).is_empty(),
                cx.listener(|this, _, _, cx| this.import(cx)),
            )
        } else {
            action_button(
                "epic-do-install",
                ButtonKind::Primary,
                if not_enough { tr!("Force install") } else { tr!("Install") },
                ready,
                cx.listener(|this, _, _, cx| this.install(cx)),
            )
        };
        let switch = button("epic-switch-mode", ButtonKind::Glass, if importing { tr!("Install") } else { tr!("Import game") }).on_click(
            cx.listener(|this, _, _, cx| {
                this.mode = if this.mode == Mode::Install { Mode::Import } else { Mode::Install };
                cx.notify();
            }),
        );
        let footer = div().flex().gap(px(10.)).when(self.third_party.is_none(), |d| d.child(switch)).child(primary).into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "epic-install".into(), title: self.title.clone().into(), width: 600., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}
