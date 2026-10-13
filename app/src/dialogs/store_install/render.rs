//! Desktop layout of the install / import dialog.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::util::file_size_local;
use heronite_i18n::tr;
use serde_json::Value;

use super::{language_name, platform_name, text_of, StoreInstallDialog};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::services::downloads::Store;
use crate::theme;
use crate::ui::controls::{action_button, button, caption, card, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, path_field, select_field};
use crate::ui::icon;

impl StoreInstallDialog {
    fn size_row(&self) -> Div {
        let cell = |ic: Icon, label: &str, value: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(ic, px(16.), theme::text_primary()))
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(format!("{label}:")))
                .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value))
        };
        let mut row = div().flex().flex_wrap().gap_x(px(28.)).gap_y(px(6.));
        if self.info.is_some() {
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
                    .child(icon(Icon::FaSpinner, px(16.), theme::text_primary()))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Getting download and install size..."))),
            );
        }
        row
    }

    fn space_line(&self) -> Option<Div> {
        let space = self.space.as_ref()?;
        let after = space.free as f64 - self.disk_bytes();
        let enough = self.info.is_none() || after >= 0.;
        let mut line = div()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .child(tr!("Space available: {space}", space = space.message));
        if self.info.is_some() && enough {
            line = line.child(tr!(" - After install: {size}", size = file_size_local(after)));
        }
        if !space.writable {
            line = line.child(div().text_color(rgb(0xebcb8b)).child(tr!("Warning: path might not be writable.")));
        }
        if !enough {
            line = line.child(div().text_color(rgb(0xebcb8b)).child(tr!("(Not enough disk space)")));
        }
        Some(line)
    }

    /// Platform and language selects (GOG).
    fn choice_row(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if self.store != Store::Gog {
            return None;
        }
        let platforms: Vec<(SharedString, bool)> = self.platforms.iter().map(|p| (platform_name(p).into(), *p == self.platform)).collect();
        let platform = select_field(
            "store-platform",
            Some(tr!("Platform:")),
            platform_name(&self.platform),
            self.platforms.len() < 2,
            self.platform_open,
            platforms,
            cx,
            |this: &mut Self, _, cx| {
                this.platform_open = !this.platform_open;
                this.language_open = false;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.platform_open = false;
                this.set_platform(i, cx);
                cx.notify();
            },
        );
        let languages = self.languages().to_vec();
        let current: SharedString = if self.info.is_some() {
            language_name(&self.language).into()
        } else if self.info_error.is_some() {
            "".into()
        } else {
            tr!("Loading…").into()
        };
        let options: Vec<(SharedString, bool)> = languages.iter().map(|l| (language_name(l).into(), *l == self.language)).collect();
        let language = select_field(
            "store-language",
            Some(tr!("Language:")),
            current,
            languages.len() < 2,
            self.language_open,
            options,
            cx,
            |this: &mut Self, _, cx| {
                this.language_open = !this.language_open;
                this.platform_open = false;
                cx.notify();
            },
            move |this: &mut Self, i, _, cx| {
                if let Some(l) = languages.get(i) {
                    this.language = l.clone();
                }
                this.language_open = false;
                cx.notify();
            },
        );
        Some(div().flex().gap(px(12.)).child(platform.flex_1().min_w_0()).child(language.flex_1().min_w_0()))
    }

    fn dlc_list(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if self.dlcs.is_empty() {
            return None;
        }
        let all = self.dlcs.iter().all(|(_, on)| *on);
        let mut rows: Vec<AnyElement> = vec![checkbox("store-dlc-all", all, tr!("Install all DLCs"))
            .on_click(cx.listener(move |this, _, _, cx| {
                let set = !this.dlcs.iter().all(|(_, on)| *on);
                this.dlcs.iter_mut().for_each(|d| d.1 = set);
                cx.notify();
            }))
            .into_any_element()];
        for (i, (dlc, on)) in self.dlcs.iter().enumerate() {
            rows.push(
                checkbox(SharedString::from(format!("store-dlc-{i}")), *on, dlc.title.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(e) = this.dlcs.get_mut(i) {
                            e.1 = !e.1;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(caption(tr!("Components to install")))
                .child(div().id("store-dlc-list").max_h(px(240.)).overflow_y_scroll().child(card(rows))),
        )
    }

    fn wine_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.wine_applies() {
            return None;
        }
        let toggle = div()
            .id("store-wine-toggle")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .py(px(8.))
            .text_size(px(14.))
            .text_color(theme::text_primary())
            .on_click(cx.listener(|this, _, _, cx| {
                this.wine_details = !this.wine_details;
                cx.notify();
            }))
            .child(icon(if self.wine_details { Icon::ExpandMore } else { Icon::ChevronRight }, px(18.), theme::text_primary()))
            .child(tr!("Show Wine settings"));
        let mut col = crate::ui::controls::card_box().gap(px(10.)).pb(px(14.)).child(toggle);
        if !self.wine_details {
            return Some(col);
        }
        let shared = self.shared_prefix;
        col = col.child(checkbox("store-shared-prefix", shared, tr!("Use shared Wine prefix")).on_click(cx.listener(
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
            "store-prefix-pick",
            Some(tr!("WinePrefix")),
            &self.prefix,
            has_prefix,
            shared,
            cx.listener(move |_, _, window, cx| {
                choose_path(window, cx, true, tr!("Select WinePrefix folder"), |this: &mut Self, p, window, cx| {
                    this.prefix.update(cx, |s, cx| s.set_value(p, window, cx));
                    cx.notify();
                });
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
            "store-wine",
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
        Some(col)
    }

    fn install_body(&mut self, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap(px(14.));
        body = body.children(self.choice_row(cx));
        body = body.child(self.size_row());
        let has = !text_of(&self.path, cx).is_empty();
        body = body.child(path_field(
            "store-path-pick",
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
        body = body.children(self.space_line());
        body.children(self.dlc_list(cx))
    }

    fn import_body(&mut self, cx: &mut Context<Self>) -> Div {
        let has = !text_of(&self.import_path, cx).is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(meta_text(tr!("Choose the folder where the game is already installed (the folder that holds the game files).")))
            .child(path_field(
                "store-import-pick",
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
            ))
    }
}

impl Render for StoreInstallDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let importing = self.importing();
        let mut body = if importing { self.import_body(cx) } else { self.install_body(cx) };
        if let Some(wine) = self.wine_section(cx) {
            body = body.child(wine);
        }
        let primary = if importing {
            action_button(
                "store-do-import",
                ButtonKind::Primary,
                tr!("Import"),
                !text_of(&self.import_path, cx).is_empty(),
                cx.listener(|this, _, _, cx| this.import(cx)),
            )
        } else {
            action_button(
                "store-do-install",
                ButtonKind::Primary,
                if self.not_enough_space() { tr!("Force install") } else { tr!("Install") },
                self.can_install(cx),
                cx.listener(|this, _, _, cx| this.install(cx)),
            )
        };
        let switch = button("store-switch-mode", ButtonKind::Glass, if importing { tr!("Install") } else { tr!("Import game") })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx)));
        let footer = div().flex().gap(px(10.)).child(switch).child(primary).into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "store-install".into(), title: self.title.clone().into(), width: 600., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}
