//! "Frame generation" in the Other tab: Lossless Scaling frame generation
//! through lsfg-vk (`heronite_launch::lsfg`). The switch is always there;
//! what is missing to run it is only explained once it is on.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::lsfg::{self, Install, Layer, Lsfg};
use serde_json::{json, Value};

use super::parts::{heading, help, rows, Rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::toggle_inactive;
use crate::ui::icon;
use crate::ui::lsfg::{missing_card, open_ui, Missing};

impl SettingsPanel {
    /// Looks for lsfg-vk, Lossless Scaling and lsfg-vk's profiles off the UI thread.
    pub(super) fn load_lsfg(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let global = heronite_launch::settings::GlobalSettings::load(&paths);
                    (Install::detect(&global, &heronite_launch::SystemProbe), crate::ui::lsfg::ui_program())
                })
                .await;
            this.update(cx, |this, cx| {
                (this.lsfg, this.lsfg_ui) = (Some(found.0), found.1);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn lsfg_settings(&self) -> Lsfg {
        Lsfg::from_map(&self.settings)
    }

    /// What keeps frame generation from running (`None` also while the
    /// machine is being looked at).
    pub(super) fn lsfg_missing(&self) -> Option<Missing> {
        let install = self.lsfg.as_ref()?;
        if self.is_steam() && install.layer == Some(Layer::V1) {
            return Some(Missing::OldLayer);
        }
        Missing::of(install, install.uses_profile(&self.lsfg_settings().profile))
    }

    /// Steam games: the launch options name the DLL when lsfg-vk would not
    /// find it by itself (a Steam library outside the home's Steam folder).
    pub(super) fn steam_lsfg_dll(&mut self) {
        let Some(install) = &self.lsfg else { return };
        if !self.lsfg_settings().enabled {
            return;
        }
        let dll = install.dll.clone().filter(|d| !lsfg::found_by_layer(d, &self.home)).unwrap_or_default();
        self.settings.insert(lsfg::KEY_DLL.into(), Value::String(dll));
    }

    /// Caption, card and, when it cannot run, what to install. `disabled`:
    /// a Steam game whose launch options cannot change now.
    pub(super) fn lsfg_section(&mut self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let s = self.lsfg_settings();
        let label = tr!("Lossless Scaling frame generation");
        let about = tr!("Generated frames between the game's own: smoother, a bit more latency. Vulkan and DirectX games (not OpenGL); needs lsfg-vk and Lossless Scaling.");
        let missing = s.enabled.then(|| self.lsfg_missing()).flatten();
        let mut card = rows().child(match missing {
            Some(m) => inactive_row(label, about, m, disabled, cx),
            None => self.toggle_row("lsfg", lsfg::KEY_ENABLED, false, label, Some(about), disabled, cx),
        });
        if s.enabled && missing.is_none() && self.lsfg.is_some() {
            card = self.lsfg_rows(card, &s, disabled, cx);
        }
        let mut col = div().flex().flex_col().gap(px(6.)).child(heading(tr!("Frame generation"))).child(card.card());
        if let Some(m) = missing {
            col = col.child(missing_card("lsfg", m, cx.listener(|this, _, _, cx| this.load_lsfg(cx))));
        }
        col
    }

    fn lsfg_rows(&self, mut card: Rows, s: &Lsfg, disabled: bool, cx: &mut Context<Self>) -> Rows {
        let Some(install) = &self.lsfg else { return card };
        let uses_profile = install.uses_profile(&s.profile);
        let profiles = match install.layer {
            Some(Layer::V2) => install.profiles.clone(),
            _ => Vec::new(),
        };
        if !profiles.is_empty() || !s.profile.is_empty() {
            let here: SharedString = tr!("Set here").into();
            let profile_label = |name: &str| SharedString::from(tr!("lsfg-vk profile: {name}", name));
            let mut items = vec![(here.clone(), !uses_profile)];
            items.extend(profiles.iter().map(|p| (profile_label(p), uses_profile && *p == s.profile)));
            let shown = if uses_profile { profile_label(&s.profile) } else { here };
            let mut notes = vec![help(tr!("These values, or a profile from lsfg-vk-ui (its changes apply while playing)."))];
            if !s.profile.is_empty() && !uses_profile {
                notes.push(help(&tr!("The profile {name} is gone: the values below are used.", name = s.profile)));
            }
            let list = profiles.clone();
            card = card.field_help(
                self.select_row("lsfg-source", Some(tr!("Settings")), shown.to_string(), items, disabled, cx, move |this, i, _, cx| {
                    let name = if i == 0 { String::new() } else { list[i - 1].clone() };
                    this.set(lsfg::KEY_PROFILE, Value::String(name), cx);
                }),
                notes,
            );
        }
        if uses_profile {
            if let Some(program) = self.lsfg_ui.clone() {
                card = card.field(self.outline_button("lsfg-ui", tr!("Open lsfg-vk-ui"), true, cx, move |_, _, _| open_ui(&program)));
            }
            return card;
        }
        let multipliers = lsfg::MULTIPLIERS.iter().map(|n| (SharedString::from(format!("{n}×")), *n == s.multiplier)).collect();
        let flows = lsfg::FLOW_SCALES.iter().map(|n| (SharedString::from(format!("{n}%")), *n == s.flow_scale)).collect();
        card.field_help(
            self.select_row(
                "lsfg-multiplier",
                Some(tr!("Multiplier")),
                format!("{}×", s.multiplier),
                multipliers,
                disabled,
                cx,
                |this, i, _, cx| this.set(lsfg::KEY_MULTIPLIER, json!(lsfg::MULTIPLIERS[i]), cx),
            ),
            vec![help(tr!("2× turns 60 FPS into 120. Set an FPS limit for the most even frames."))],
        )
        .field_help(
            self.select_row("lsfg-flow", Some(tr!("Flow scale")), format!("{}%", s.flow_scale), flows, disabled, cx, |this, i, _, cx| {
                this.set(lsfg::KEY_FLOW_SCALE, json!(lsfg::FLOW_SCALES[i]), cx)
            }),
            vec![help(tr!("Lower is lighter and less precise: about 100% at 1080p, 75% at 1440p, 50% at 4K."))],
        )
        .child(self.toggle_row(
            "lsfg-performance",
            lsfg::KEY_PERFORMANCE,
            false,
            tr!("Performance mode"),
            Some(tr!("A much lighter model: less GPU load for a little less quality.")),
            disabled,
            cx,
        ))
        .child(self.toggle_row(
            "lsfg-hdr",
            lsfg::KEY_HDR,
            false,
            "HDR",
            Some(tr!("Turn on when the game outputs HDR.")),
            disabled,
            cx,
        ))
    }
}

/// The switch while frame generation is on but cannot run: a warning icon
/// before the label, what is missing under it (the card below says how to
/// fix it) and a grey switch that still turns it off.
fn inactive_row(label: &str, about: &str, missing: Missing, disabled: bool, cx: &mut Context<SettingsPanel>) -> AnyElement {
    let title = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(icon(Icon::Warning, px(15.), theme::accent()))
        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(label.to_string()));
    let status = div().text_size(px(12.)).line_height(px(16.)).text_color(theme::accent()).child(missing.status());
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(1.))
        .child(title)
        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(about.to_string()))
        .child(status);
    let row = div()
        .id("lsfg")
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .py(px(11.))
        .child(text)
        .child(div().flex_none().child(toggle_inactive("lsfg-sw", true)));
    match disabled {
        true => row.opacity(0.4).cursor_not_allowed().into_any_element(),
        false => row.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.toggle(lsfg::KEY_ENABLED, false, cx))).into_any_element(),
    }
}
