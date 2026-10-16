//! "Presets" in the Advanced tab: save the game's environment variables,
//! wrappers and arguments under a name, apply them to another game in one
//! pick (`heronite_data::launch_presets`). Shared by every game.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::launch_presets::{self, Preset};
use heronite_i18n::{tr, trn};

use super::parts::{heading, help, rows};
use super::{Notice, SettingsPanel};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::fields::input_box;
use crate::ui::icon;

/// "2 variables · 1 wrapper · arguments".
pub(super) fn summary(p: &Preset) -> String {
    let mut parts = Vec::new();
    if !p.env.is_empty() {
        parts.push(trn!(p.env.len(), "{n} variable", "{n} variables"));
    }
    if !p.wrappers.is_empty() {
        parts.push(trn!(p.wrappers.len(), "{n} wrapper", "{n} wrappers"));
    }
    if !p.args.trim().is_empty() {
        parts.push(tr!("arguments").to_string());
    }
    parts.join(" \u{b7} ")
}

impl SettingsPanel {
    pub(super) fn load_presets(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { launch_presets::load(&paths) }).await;
            this.update(cx, |this, cx| {
                this.presets = list;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Adds preset `i` to the game's settings.
    pub(super) fn apply_preset(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(p) = self.presets.get(i).cloned() else { return };
        let changes = launch_presets::apply(&p, &self.settings);
        let none = changes.is_empty();
        for (key, value) in changes {
            self.set(&key, value, cx);
        }
        let text =
            if none { tr!("The game already has everything in {name}.", name = p.name) } else { tr!("Applied {name}.", name = p.name) };
        self.notice = Some(Notice { text: text.into(), error: false });
        cx.notify();
    }

    /// Saves the game's variables, wrappers and arguments under `name`.
    pub(super) fn save_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            self.notice = Some(Notice { text: tr!("Type a name for the preset.").into(), error: true });
            return cx.notify();
        }
        let preset = launch_presets::capture(name, &self.settings);
        if preset.is_empty() {
            self.notice = Some(Notice {
                text: tr!("Nothing to save: add environment variables, wrappers or game arguments first.").into(),
                error: true,
            });
            return cx.notify();
        }
        let paths = self.paths.clone();
        let saved = preset.name.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { launch_presets::put(&paths, preset) }).await;
            this.update(cx, |this, cx| {
                this.notice = Some(match res {
                    Ok(()) => Notice { text: tr!("Saved the preset {name}.", name = saved).into(), error: false },
                    Err(e) => Notice { text: tr!("Could not save the preset: {err}", err = format!("{e:#}")).into(), error: true },
                });
                this.load_presets(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn delete_preset(&mut self, name: String, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { launch_presets::remove(&paths, &name) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    this.notice =
                        Some(Notice { text: tr!("Could not delete the preset: {err}", err = format!("{e:#}")).into(), error: true });
                }
                this.load_presets(cx);
            })
            .ok();
        })
        .detach();
    }

    /// `disabled`: a Steam game whose launch options cannot change now.
    pub(super) fn presets_section(&mut self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let mut card = rows();
        if !self.presets.is_empty() {
            let items = self.presets.iter().map(|p| (SharedString::from(format!("{} \u{b7} {}", p.name, summary(p))), false)).collect();
            card = card.field_help(
                self.select_row(
                    "preset-apply",
                    Some(tr!("Apply a preset")),
                    tr!("Choose a preset\u{2026}").into(),
                    items,
                    disabled,
                    cx,
                    |this, i, _, cx| this.apply_preset(i, cx),
                ),
                vec![help(tr!("Adds its variables, wrappers and arguments to this game, without duplicates."))],
            );
        }
        let save = div().flex().items_center().gap(px(8.)).child(div().flex_1().child(input_box(&self.input("preset.name"), false))).child(
            action_button(
                "preset-save",
                ButtonKind::Glass,
                tr!("Save as preset"),
                !disabled,
                cx.listener(|this, _, window, cx| {
                    let name = this.input_text("preset.name", cx);
                    this.save_preset(&name, cx);
                    this.set_input("preset.name", "", window, cx);
                }),
            ),
        );
        card = card.field_help(
            save,
            vec![help(tr!("Saves this game's variables, wrappers and arguments under a name, to reuse in other games."))],
        );
        for (i, p) in self.presets.iter().enumerate() {
            let name = p.name.clone();
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(220.))
                            .truncate()
                            .text_size(px(14.))
                            .text_color(theme::text_primary())
                            .child(p.name.clone()),
                    )
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_muted()).child(summary(p)))
                    .child(
                        div()
                            .id(SharedString::from(format!("preset-del-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.10)))
                            .child(icon(Icon::DeleteOutline, px(16.), theme::text_muted()))
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_preset(name.clone(), cx))),
                    ),
            );
        }
        div().flex().flex_col().gap(px(6.)).child(heading(tr!("Presets"))).child(card.card())
    }
}
