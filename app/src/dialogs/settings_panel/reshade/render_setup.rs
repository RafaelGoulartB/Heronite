//! Before ReShade is installed: a switch like the other settings of the
//! tab. Turning it on opens, in the same card, what the install needs (an
//! optional preset, the shader packages as chips) and the install button;
//! the install shows its progress there. The drawer scrolls to the card
//! when it opens or grows, so nothing opens out of sight.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_launch::reshade::detect::Api;
use heronite_launch::reshade::Overview;

use super::actions::file_name;
use super::render::{api_label, warn_line};
use super::{Confirm, SettingsPanel};
use crate::dialogs::settings_panel::parts::help;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, card_box, row, toggle, ButtonKind};
use crate::ui::icon;

/// The section's title row: its badge, a title, what is under it (a line of
/// text, or text with a link) and a control on the right.
pub(super) fn title_row(title: impl Into<SharedString>, about: impl IntoElement, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(12.))
        .child(crate::ui::controls::icon_badge(Icon::Palette))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(title.into()))
                .child(about),
        )
        .child(div().flex_none().child(control))
}

/// The small text under a title.
pub(super) fn about(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text.into())
}

pub(super) fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// A part of the card under a separator: a small label, then its content.
fn part(label: Option<Div>, body: Vec<AnyElement>) -> Div {
    div().flex().flex_col().gap(px(8.)).py(px(12.)).children(label).children(body)
}

fn label(text: &str, note: Option<&str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(text.to_string()))
        .children(note.map(|n| div().text_size(px(12.)).text_color(theme::text_dim()).child(format!("({n})"))))
}

/// A clickable text in the accent colour.
pub(super) fn link(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .text_size(px(12.))
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.text_color(theme::accent_overlay()))
        .child(text.to_string())
}

fn chip(text: String) -> Div {
    div()
        .flex_none()
        .px(px(9.))
        .py(px(3.))
        .rounded_full()
        .bg(theme::ink_a(0.07))
        .text_size(px(12.))
        .text_color(theme::text_primary())
        .child(text)
}

/// A bar of `done` out of `total` steps.
pub(super) fn progress(done: usize, total: usize) -> Div {
    let frac = if total == 0 { 0. } else { (done as f32 / total as f32).clamp(0.05, 1.) };
    div().h(px(6.)).w_full().rounded_full().bg(theme::ink_a(0.1)).child(div().h_full().w(relative(frac)).rounded_full().bg(theme::accent()))
}

impl SettingsPanel {
    /// The switch, off: nothing installed and the install not opened.
    pub(super) fn reshade_intro(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let mut sw_row = title_row(
            "ReShade",
            about(tr!("Filters on the game's image: sharpness, colours and lighting.")),
            toggle("reshade-enable-sw", false, busy),
        )
        .id("reshade-enable");
        if !busy {
            sw_row = sw_row.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.open_reshade_setup(cx)));
        }
        let mut card = card_box().child(sw_row);
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            card = card.child(div().pb(px(12.)).child(warn_line(&tr!("Anti-cheat: {names}", names = names.join(", ")))));
        }
        card
    }

    /// The switch, on: what the install needs and its button, or its progress.
    pub(super) fn reshade_setup(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let reveal = self.reshade.reveal.clone();
        // Measures the card once laid out, to scroll the drawer to it.
        let probe = canvas(
            move |bounds, window, _| {
                if reveal.replace(false) {
                    crate::ui::dialog::reveal_in_drawer(bounds, false);
                    // A refresh asked while drawing is dropped: draw again next frame.
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let card = card_box().relative();
        if let Some(b) = &self.reshade.busy {
            let line = if b.line.is_empty() { tr!("Working…").to_string() } else { b.line.clone() };
            let cancel = self.outline_button("reshade-cancel", tr!("Cancel"), true, cx, |this, _, cx| this.cancel_reshade(cx));
            return card
                .child(title_row("ReShade", about(line), cancel))
                .child(div().pb(px(14.)).child(progress(b.done, b.total)))
                .child(probe);
        }
        let t = o.target.as_ref();
        let target = format!("{} · {}", o.exe.as_deref().map(file_name).unwrap_or_default(), api_label(t, t.and_then(|t| t.api)));
        let advanced = self.reshade.advanced;
        let under = div().flex().flex_wrap().items_center().gap(px(6.)).child(about(target)).child(
            link("reshade-advanced", if advanced { tr!("Done") } else { tr!("Change") }).on_click(cx.listener(|this, _, _, cx| {
                this.reshade.advanced = !this.reshade.advanced;
                cx.notify();
            })),
        );
        let sw_row = title_row("ReShade", under, toggle("reshade-enable-sw", true, false))
            .id("reshade-enable")
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.close_reshade_setup(cx)));
        let mut card = card.child(sw_row);
        let mut notes: Vec<AnyElement> = Vec::new();
        if let Some(l) = &o.launcher {
            notes.push(
                help(&tr!("{launcher} is a launcher: ReShade goes with the program that draws the game.", launcher = file_name(l)))
                    .into_any_element(),
            );
        }
        if t.is_some_and(|t| t.small_stack) {
            notes.push(warn_line(tr!("This program reserves little stack memory: ReShade may crash it.")).into_any_element());
        }
        if advanced {
            notes.extend(self.reshade_target_fields(o, cx));
        }
        if !notes.is_empty() {
            card = card.child(div().flex().flex_col().gap(px(10.)).pb(px(12.)).children(notes));
        }
        card.child(separator())
            .child(self.reshade_preset_part(cx))
            .child(separator())
            .child(self.reshade_shaders_part(o, cx))
            .child(separator())
            .child(self.reshade_install_part(o, cx))
            .child(probe)
    }

    /// "Change": the executable and the DLL name.
    fn reshade_target_fields(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        if let Some(f) = self.reshade_exe_select(o, cx) {
            out.push(f.into_any_element());
        }
        if o.target.as_ref().and_then(|t| t.api) == Some(Api::Dxgi) {
            let current = o.proxy.clone().unwrap_or_default();
            let items = ["dxgi.dll", "d3d11.dll"].iter().map(|n| (SharedString::from(*n), current == *n)).collect();
            let select = self.select_row("reshade-dll", Some(tr!("Installed as")), current, items, false, cx, |this, i, _, cx| {
                this.pick_reshade_api(["dxgi", "d3d11"][i], cx)
            });
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(select)
                    .child(help(tr!("dxgi.dll works for DirectX 10 to 12. Pick d3d11.dll when another mod already uses dxgi.dll.")))
                    .into_any_element(),
            );
        }
        out
    }

    fn reshade_preset_part(&mut self, cx: &mut Context<Self>) -> Div {
        let mut body: Vec<AnyElement> = Vec::new();
        match &self.reshade.preset {
            None => {
                let pick = div()
                    .id("reshade-preset")
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(11.))
                    .rounded(px(theme::R_INPUT))
                    .border_1()
                    .border_dashed()
                    .border_color(theme::ink_a(0.22))
                    .hover(|s| s.bg(theme::ink_a(0.04)).border_color(theme::accent_tint_border()))
                    .cursor_pointer()
                    .child(icon(Icon::Description, px(20.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(tr!("Choose a preset (.ini)…")))
                            .child(help(tr!("A preset made for this game. It is copied; the original is not changed."))),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.pick_reshade_preset(window, cx)));
                body.push(pick.into_any_element());
            }
            Some((path, effects)) => {
                let needs = self.reshade_preset_needs();
                let found = effects.len().saturating_sub(needs.missing.len());
                let count = format!("{} · {}", trn!(effects.len(), "{n} effect", "{n} effects"), trn!(found, "{n} found", "{n} found"));
                let file = div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(theme::R_INPUT))
                    .bg(theme::accent_wash())
                    .border_1()
                    .border_color(theme::accent_wash_border())
                    .child(icon(Icon::Description, px(20.), theme::accent()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(file_name(path)))
                            .child(help(&count)),
                    )
                    .child(
                        link("reshade-preset-change", tr!("Change"))
                            .on_click(cx.listener(|this, _, window, cx| this.pick_reshade_preset(window, cx))),
                    )
                    .child(
                        div()
                            .id("reshade-preset-clear")
                            .flex_none()
                            .cursor_pointer()
                            .child(icon(Icon::Close, px(16.), theme::text_dim()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reshade.preset = None;
                                this.reshade.picked = None;
                                cx.notify();
                            })),
                    );
                body.push(file.into_any_element());
                if !needs.missing.is_empty() {
                    let text =
                        tr!("{files} is not in ReShade's packages; it can be added after installing.", files = needs.missing.join(", "));
                    body.push(warn_line(&text).into_any_element());
                }
            }
        }
        part(Some(label(tr!("Preset"), Some(tr!("optional")))), body)
    }

    fn reshade_shaders_part(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let picked = self.reshade_picked();
        let open = self.reshade.packages_open;
        let mut chips = div().flex().flex_wrap().items_center().gap(px(6.));
        for name in o.packages.iter().filter(|p| picked.contains(&p.name)).map(|p| p.name.clone()) {
            chips = chips.child(chip(name));
        }
        chips = chips.child(link("reshade-packages", if open { tr!("Done") } else { tr!("Choose packages…") }).on_click(cx.listener(
            |this, _, _, cx| {
                this.reshade.packages_open = !this.reshade.packages_open;
                cx.notify();
            },
        )));
        let mut body: Vec<AnyElement> = vec![chips.into_any_element()];
        if o.packages.is_empty() {
            body.push(help(tr!("ReShade's package list could not be read. Check the connection.")).into_any_element());
        }
        if open {
            body.push(self.reshade_package_list(o, cx).into_any_element());
        }
        part(Some(label(tr!("Shaders"), None)), body)
    }

    /// Every package with its switch (the catalog's usual ones first).
    fn reshade_package_list(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let picked = self.reshade_picked();
        let needed = self.reshade_preset_needs().packages;
        let shown: Vec<_> =
            o.packages.iter().filter(|p| self.reshade.all_packages || p.enabled || p.required || picked.contains(&p.name)).collect();
        let mut list = div().flex().flex_col().px(px(14.)).rounded(px(theme::R_INPUT)).border_1().border_color(theme::card_border());
        for (i, p) in shown.iter().enumerate() {
            let on = p.required || picked.contains(&p.name);
            let mut desc = p.description.clone();
            if p.required {
                desc = format!("{desc} · {}", tr!("Always installed"));
            } else if needed.contains(&p.name) {
                desc = format!("{desc} · {}", tr!("Used by the preset"));
            }
            let sw = toggle(SharedString::from(format!("reshade-pkg-sw-{}", p.name)), on, p.required);
            let mut el = row(p.name.clone(), Some(&desc), sw).py(px(8.)).id(SharedString::from(format!("reshade-pkg-{}", p.name)));
            if !p.required {
                let name = p.name.clone();
                el = el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle_reshade_package(&name, cx)));
            }
            if i > 0 {
                list = list.child(separator());
            }
            list = list.child(el);
        }
        if !self.reshade.all_packages && shown.len() < o.packages.len() {
            list = list.child(separator()).child(div().py(px(10.)).child(
                link("reshade-all-packages", &tr!("Show all {n} packages", n = o.packages.len())).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.reshade.all_packages = true;
                        cx.notify();
                    },
                )),
            ));
        }
        list
    }

    /// The install button with what it does, or the anti-cheat question; a
    /// failed try's error under it.
    fn reshade_install_part(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        if matches!(self.reshade.confirm, Some(Confirm::AntiCheat { .. })) {
            return part(None, vec![self.reshade_anticheat_card(o, cx).into_any_element()]);
        }
        let mut body: Vec<AnyElement> = Vec::new();
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            body.push(
                warn_line(&tr!("Anti-cheat: {names}. You will be asked to confirm before installing.", names = names.join(", ")))
                    .into_any_element(),
            );
        }
        let label = match &o.latest {
            Some(v) => tr!("Install ReShade {version}", version = v),
            None => tr!("Install ReShade").into(),
        };
        let install = action_button(
            "reshade-install",
            ButtonKind::Primary,
            label,
            o.latest.is_some(),
            cx.listener(|this, _, _, cx| this.start_reshade_install(false, true, cx)),
        );
        body.push(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(12.))
                .child(install)
                .child(help(tr!("Downloads from the official sources into the game's folder.")))
                .into_any_element(),
        );
        if let Some((text, true)) = &self.reshade.message {
            body.push(
                div().text_size(px(12.)).line_height(px(16.)).text_color(theme::danger_text()).child(text.clone()).into_any_element(),
            );
        }
        part(None, body)
    }

    /// The executable select, when there is another program to pick.
    pub(super) fn reshade_exe_select(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Option<Div> {
        let current = o.exe.as_deref().map(file_name).unwrap_or_default();
        let chosen = !self.reshade_settings().target.is_empty();
        if !chosen && o.candidates.iter().all(|c| o.exe.as_deref() == Some(c.as_path())) {
            return None;
        }
        let mut items: Vec<(SharedString, bool)> = vec![(tr!("The game's executable").into(), !chosen)];
        let base = o.exe.as_deref().and_then(|e| e.parent()).map(|p| p.to_path_buf());
        let label = |p: &std::path::Path| -> SharedString {
            base.as_deref()
                .and_then(|b| p.strip_prefix(b).ok())
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name(p))
                .into()
        };
        items.extend(o.candidates.iter().map(|c| (label(c), chosen && o.exe.as_deref() == Some(c.as_path()))));
        let list = o.candidates.clone();
        Some(self.select_row(
            "reshade-exe",
            Some(tr!("Executable")),
            current,
            items,
            self.reshade.busy.is_some(),
            cx,
            move |this, i, _, cx| this.pick_reshade_exe(if i == 0 { None } else { list.get(i - 1).cloned() }, cx),
        ))
    }

    pub(super) fn open_reshade_setup(&mut self, cx: &mut Context<Self>) {
        self.reshade.setup = true;
        self.reshade.message = None;
        self.reshade.reveal.set(true);
        cx.notify();
    }

    /// The switch turned off before installing: back to the short card.
    fn close_reshade_setup(&mut self, cx: &mut Context<Self>) {
        self.reshade.setup = false;
        self.reshade.advanced = false;
        self.reshade.packages_open = false;
        self.reshade.preset = None;
        self.reshade.picked = None;
        self.reshade.all_packages = false;
        self.reshade.confirm = None;
        self.reshade.message = None;
        cx.notify();
    }
}
