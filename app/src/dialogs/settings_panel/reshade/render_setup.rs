//! Before ReShade is installed: a short card that says what it is with one
//! button, and, once opened, the whole install as one card in three steps
//! (the game, a preset, the shaders) ending with the install button. The
//! drawer scrolls to the card when it opens or grows, so it is never opened
//! out of sight.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_launch::reshade::detect::Api;
use heronite_launch::reshade::Overview;

use super::actions::file_name;
use super::render::{api_label, buttons, warn_line};
use super::{Confirm, SettingsPanel};
use crate::dialogs::settings_panel::parts::help;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, card_box, icon_button_sm, row, toggle, ButtonKind};
use crate::ui::icon;

/// The section's title row: its badge, a title, a line under it and a
/// control on the right.
pub(super) fn title_row(title: impl Into<SharedString>, about: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(12.))
        .child(badge(34.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title.into()))
                .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(about.into())),
        )
        .child(div().flex_none().child(control))
}

/// The badge of the section: its icon on an accent tile.
fn badge(size: f32) -> Div {
    div().flex_none().size(px(size)).rounded(px(size * 0.3)).bg(theme::accent_tint()).flex().items_center().justify_center().child(icon(
        Icon::AutoAwesome,
        px(size * 0.5),
        theme::accent(),
    ))
}

/// A numbered step title with an optional note after it.
fn step(n: u8, title: &str, note: Option<String>) -> Div {
    let number = div()
        .flex_none()
        .size(px(20.))
        .rounded_full()
        .bg(theme::accent_tint())
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::accent())
        .child(n.to_string());
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(number)
        .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title.to_string()))
        .children(note.map(|n| div().text_size(px(12.)).text_color(theme::text_dim()).child(n)))
}

fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// A part of the setup card: its step title and what it holds, padded.
fn part(title: Div, body: Vec<AnyElement>) -> Div {
    div().flex().flex_col().gap(px(10.)).py(px(14.)).child(title).children(body)
}

/// A clickable text in the accent colour.
fn link(id: &'static str, label: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .text_size(px(12.))
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.text_color(theme::accent_overlay()))
        .child(label.to_string())
}

impl SettingsPanel {
    /// "DirectX 11 · 64-bit" of the target, or nothing while unknown.
    fn reshade_target_line(o: &Overview) -> String {
        let t = o.target.as_ref();
        format!("{} · {}", o.exe.as_deref().map(file_name).unwrap_or_default(), api_label(t, t.and_then(|t| t.api)))
    }

    /// The card shown while nothing is installed and the setup is closed.
    pub(super) fn reshade_intro(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let mut text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(3.))
            .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child("ReShade"))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(theme::text_dim())
                    .child(tr!("Effects on the game's image: sharpness, colours, lighting, and presets shared by other players.")),
            )
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(Self::reshade_target_line(o)));
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            text = text.child(div().pt(px(4.)).child(warn_line(&tr!("Anti-cheat: {names}", names = names.join(", ")))));
        }
        let open = action_button(
            "reshade-setup",
            ButtonKind::Primary,
            tr!("Set up"),
            self.reshade.busy.is_none(),
            cx.listener(|this, _, _, cx| this.open_reshade_setup(cx)),
        );
        card_box()
            .flex_row()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .bg(theme::accent_wash())
            .border_color(theme::accent_wash_border())
            .child(badge(40.))
            .child(text)
            .child(div().flex_none().child(open))
    }

    /// The install as one card: the game, a preset, the shaders, then the
    /// install button (or its progress, or the anti-cheat question).
    pub(super) fn reshade_setup(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let close = icon_button_sm(
            "reshade-setup-close",
            ButtonKind::Ghost,
            Icon::Close,
            tr!("Close"),
            !busy,
            cx.listener(|this, _, _, cx| this.close_reshade_setup(cx)),
        );
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .pt(px(14.))
            .pb(px(12.))
            .child(badge(34.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("Set up ReShade")),
                    )
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Three steps; only the first one is needed."))),
            )
            .child(close);
        let reveal = self.reshade.reveal.clone();
        // Measures the card once laid out, to scroll the drawer to it.
        let probe = canvas(
            move |bounds, window, _| {
                if reveal.replace(false) {
                    crate::ui::dialog::reveal_in_drawer(bounds);
                    // A refresh asked while drawing is dropped: draw again next frame.
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        card_box()
            .relative()
            .border_color(theme::accent_tint_border())
            .child(header)
            .child(separator())
            .child(self.reshade_step_game(o, cx))
            .child(separator())
            .child(self.reshade_step_preset(cx))
            .child(separator())
            .child(self.reshade_step_shaders(o, cx))
            .child(separator())
            .child(self.reshade_setup_footer(o, cx))
            .child(probe)
    }

    fn reshade_step_game(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let t = o.target.as_ref();
        let exe = o.exe.as_deref().map(file_name).unwrap_or_default();
        let mut about = format!("{} · {}", api_label(t, t.and_then(|t| t.api)), tr!("as {dll}", dll = o.proxy.clone().unwrap_or_default()));
        if t.is_some_and(|t| t.api_from_list) {
            about.push_str(&format!(" · {}", tr!("API from ReShade's list")));
        }
        let advanced = self.reshade.advanced;
        let change =
            link("reshade-advanced", if advanced { tr!("Done") } else { tr!("Change") }).on_click(cx.listener(|this, _, _, cx| {
                this.reshade.advanced = !this.reshade.advanced;
                cx.notify();
            }));
        let summary = div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(exe))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(about)),
            )
            .child(change);
        let mut body: Vec<AnyElement> = vec![summary.into_any_element()];
        if let Some(l) = &o.launcher {
            body.push(
                help(&tr!("{launcher} is a launcher: ReShade goes with the program that draws the game.", launcher = file_name(l)))
                    .into_any_element(),
            );
        }
        if t.is_some_and(|t| t.small_stack) {
            body.push(warn_line(tr!("This program reserves little stack memory: ReShade may crash it.")).into_any_element());
        }
        if advanced {
            if let Some(f) = self.reshade_exe_select(o, cx) {
                body.push(f.into_any_element());
            }
            if t.and_then(|t| t.api) == Some(Api::Dxgi) {
                let current = o.proxy.clone().unwrap_or_default();
                let items = ["dxgi.dll", "d3d11.dll"].iter().map(|n| (SharedString::from(*n), current == *n)).collect();
                let select = self.select_row("reshade-dll", Some(tr!("Installed as")), current, items, busy, cx, |this, i, _, cx| {
                    this.pick_reshade_api(["dxgi", "d3d11"][i], cx)
                });
                body.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(select)
                        .child(help(tr!("dxgi.dll works for DirectX 10 to 12. Pick d3d11.dll when another mod already uses dxgi.dll.")))
                        .into_any_element(),
                );
            }
        }
        part(step(1, tr!("Game"), None), body)
    }

    fn reshade_step_preset(&mut self, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let mut body: Vec<AnyElement> = Vec::new();
        match &self.reshade.preset {
            None => {
                let pick = div()
                    .id("reshade-preset")
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
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
                    );
                let pick = match busy {
                    true => pick.opacity(0.45),
                    false => pick.on_click(cx.listener(|this, _, window, cx| this.pick_reshade_preset(window, cx))),
                };
                body.push(pick.into_any_element());
            }
            Some((path, effects)) => {
                let name = file_name(path);
                let count = trn!(effects.len(), "{n} effect", "{n} effects");
                let chip = div()
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
                            .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(name))
                            .child(help(&count)),
                    )
                    .child(link("reshade-preset-change", tr!("Change")).on_click(cx.listener(|this, _, window, cx| {
                        if this.reshade.busy.is_none() {
                            this.pick_reshade_preset(window, cx)
                        }
                    })))
                    .child(icon_button_sm(
                        "reshade-preset-clear",
                        ButtonKind::Ghost,
                        Icon::Close,
                        tr!("Remove"),
                        !busy,
                        cx.listener(|this, _, _, cx| {
                            this.reshade.preset = None;
                            this.reshade.picked = None;
                            cx.notify();
                        }),
                    ));
                body.push(chip.into_any_element());
                let missing = self.reshade_preset_needs().missing;
                if !missing.is_empty() {
                    let text = tr!(
                        "Not in ReShade's packages: {files}. After installing, add them with Add folder or Add repository.",
                        files = missing.join(", ")
                    );
                    body.push(warn_line(&text).into_any_element());
                }
            }
        }
        part(step(2, tr!("Preset"), Some(tr!("optional").to_string())), body)
    }

    fn reshade_step_shaders(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let picked = self.reshade_picked();
        let needed = self.reshade_preset_needs().packages;
        let note = trn!(picked.len(), "{n} package", "{n} packages");
        let mut list = div().flex().flex_col();
        let shown: Vec<_> =
            o.packages.iter().filter(|p| self.reshade.all_packages || p.enabled || p.required || picked.contains(&p.name)).collect();
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
            if !p.required && self.reshade.busy.is_none() {
                let name = p.name.clone();
                el = el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle_reshade_package(&name, cx)));
            }
            if i > 0 {
                list = list.child(separator());
            }
            list = list.child(el);
        }
        let mut body: Vec<AnyElement> = vec![list.into_any_element()];
        if o.packages.is_empty() {
            body.push(help(tr!("ReShade's package list could not be read. Check the connection.")).into_any_element());
        } else if !self.reshade.all_packages && shown.len() < o.packages.len() {
            body.push(
                link("reshade-all-packages", &tr!("Show all {n} packages", n = o.packages.len()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reshade.all_packages = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        part(step(3, tr!("Shaders"), Some(note)), body)
    }

    /// The install button with what it does, or the work in progress, or
    /// the anti-cheat question; the outcome of a failed try under it.
    fn reshade_setup_footer(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let mut footer = div().flex().flex_col().gap(px(10.)).py(px(14.));
        if matches!(self.reshade.confirm, Some(Confirm::AntiCheat { .. })) {
            return footer.child(self.reshade_anticheat_card(o, cx));
        }
        if let Some(b) = &self.reshade.busy {
            let line = if b.line.is_empty() { tr!("Working…").to_string() } else { b.line.clone() };
            let cancel = self.outline_button("reshade-cancel", tr!("Cancel"), true, cx, |this, _, cx| this.cancel_reshade(cx));
            return footer.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon(Icon::Downloading, px(18.), theme::accent()))
                    .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(theme::text_primary()).child(line))
                    .child(cancel),
            );
        }
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            footer = footer
                .child(warn_line(&tr!("Anti-cheat: {names}. You will be asked to confirm before installing.", names = names.join(", "))));
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
        footer = footer
            .child(buttons().child(install))
            .child(help(tr!("Downloads ReShade and the shaders from their official sources into the game's folder.")));
        if let Some((text, true)) = &self.reshade.message {
            footer = footer.child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::danger_text()).child(text.clone()));
        }
        footer
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

    fn close_reshade_setup(&mut self, cx: &mut Context<Self>) {
        self.reshade.setup = false;
        self.reshade.advanced = false;
        self.reshade.preset = None;
        self.reshade.picked = None;
        self.reshade.all_packages = false;
        self.reshade.confirm = None;
        self.reshade.message = None;
        cx.notify();
    }
}
