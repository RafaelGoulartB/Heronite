//! The ReShade section before it is installed, and its states that are not
//! an install: unsupported, banned, in the way, found installed by hand.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::reshade::detect::{Api, Target};
use heronite_launch::reshade::{Overview, Source, State};

use super::actions::file_name;
use super::{Confirm, SettingsPanel};
use crate::dialogs::settings_panel::parts::{heading, help, rows, Rows};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, checkbox, row, toggle, ButtonKind};
use crate::ui::icon;

/// A line in the accent (warning) colour with its icon.
pub(super) fn warn_line(text: &str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(6.))
        .child(icon(Icon::Warning, px(14.), theme::accent()).mt(px(1.)))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(16.)).text_color(theme::accent()).child(text.to_string()))
}

pub(super) fn buttons() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.))
}

/// "DirectX 11 · 64-bit".
pub(super) fn api_label(t: Option<&Target>, api: Option<Api>) -> String {
    let api = match (api, t.and_then(|t| t.directx)) {
        (Some(Api::Dxgi), Some(v)) | (Some(Api::D3d9), Some(v)) => format!("DirectX {v}"),
        (Some(Api::Dxgi), None) => "DirectX 10–12".into(),
        (Some(Api::D3d9), None) => "DirectX 9".into(),
        (Some(Api::D3d8), _) => "DirectX 8".into(),
        (Some(Api::OpenGl), _) => "OpenGL".into(),
        (Some(Api::Vulkan), _) => "Vulkan".into(),
        (Some(Api::DDraw), _) => "DirectDraw".into(),
        (None, _) => tr!("No graphics API").into(),
    };
    match t {
        Some(t) => format!("{api} · {}", tr!("{bits}-bit", bits = t.arch.bits())),
        None => api,
    }
}

impl SettingsPanel {
    pub(in crate::dialogs::settings_panel) fn reshade_section(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(6.)).child(heading("ReShade"));
        if !self.reshade_applies() {
            let text = tr!("ReShade needs a Windows game. On Linux, vkBasalt applies ReShade effects to Vulkan and OpenGL games.");
            return col.child(rows().child(row("ReShade", Some(text), div())).card());
        }
        let Some(o) = self.reshade.overview.clone() else {
            return col.child(rows().child(row("ReShade", Some(tr!("Looking at the game…")), div())).card());
        };
        col = match &o.state {
            State::Installed => col.child(self.reshade_installed(&o, cx)),
            State::NotInstalled => col.child(self.reshade_install_form(&o, cx)),
            _ => col.child(self.reshade_other_state(&o, cx).card()),
        };
        if let Some(busy) = &self.reshade.busy {
            let line = if busy.line.is_empty() { tr!("Working…").to_string() } else { busy.line.clone() };
            let cancel = self.outline_button("reshade-cancel", tr!("Cancel"), true, cx, |this, _, cx| this.cancel_reshade(cx));
            col = col.child(rows().child(row(line, None, cancel)).card());
        }
        if let Some((text, error)) = &self.reshade.message {
            let color = if *error { theme::danger_text() } else { theme::success() };
            col = col.child(div().px(px(6.)).text_size(px(12.)).line_height(px(16.)).text_color(color).child(text.clone()));
        }
        if o.offline {
            col = col.child(div().px(px(6.)).child(help(tr!("Offline: the package list and versions are the ones saved earlier."))));
        }
        col
    }

    fn reshade_other_state(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Rows {
        let exe = o.exe.as_deref().map(file_name).unwrap_or_default();
        let mut r = rows();
        match &o.state {
            State::NoExecutable => {
                r = r.child(row(
                    "ReShade",
                    Some(tr!("Choose the game's executable in the Launch tab first: ReShade goes next to it.")),
                    div(),
                ));
            }
            State::Launcher => {
                let text = tr!("{exe} does not draw the game (it looks like a launcher). Pick the executable that does.", exe);
                r = r.child(row("ReShade", Some(&text), div()));
                r = self.reshade_exe_field(r, o, cx);
            }
            State::Unsupported(Api::Vulkan) => {
                let text = match cfg!(windows) {
                    true => tr!("{exe} uses Vulkan. ReShade adds itself to Vulkan games as a layer, through its own installer.", exe),
                    false => tr!(
                        "{exe} uses Vulkan. Heronite cannot add ReShade to Vulkan games; vkBasalt applies ReShade effects to them.",
                        exe
                    ),
                };
                r = r.child(row("ReShade", Some(&text), div()));
                r = self.reshade_exe_field(r, o, cx);
            }
            State::Unsupported(_) => {
                let text =
                    tr!("{exe} uses Direct3D 8, which needs the d3d8to9 wrapper before ReShade. Heronite does not install it yet.", exe);
                r = r.child(row("ReShade", Some(&text), div()));
            }
            State::Banned => {
                let text = tr!("ReShade's own compatibility list says {exe} bans or blocks ReShade, so Heronite does not install it.", exe);
                r = r.child(row("ReShade", None, div())).child(div().py(px(12.)).child(warn_line(&text)));
            }
            State::Conflict { path, product } => {
                let owner = product.clone().unwrap_or_else(|| tr!("another program").into());
                let text = tr!(
                    "{file} in the game's folder belongs to {owner}. Heronite never replaces another program's DLL.",
                    file = file_name(path),
                    owner
                );
                r = r.child(row("ReShade", None, div())).child(div().py(px(12.)).child(warn_line(&text)));
                let dxgi = o.target.as_ref().and_then(|t| t.api) == Some(Api::Dxgi) && o.proxy.as_deref() != Some("d3d11.dll");
                if dxgi {
                    let b = self.outline_button("reshade-as-d3d11", tr!("Install as d3d11.dll instead"), true, cx, |this, _, cx| {
                        this.pick_reshade_api("d3d11", cx)
                    });
                    r = r.field(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(buttons().child(b))
                            .child(help(tr!("DirectX 11 games also load d3d11.dll from their folder."))),
                    );
                }
            }
            State::Manual { proxy, version, off } => {
                let version = version.clone().unwrap_or_default();
                let title = match off {
                    false => tr!("ReShade {version} installed by hand ({proxy})", version, proxy),
                    true => tr!("ReShade {version} installed by hand ({proxy}, switched off)", version, proxy),
                };
                let about = tr!("Adopt it to switch, update and remove it here: nothing is downloaded or changed. Replace installs Heronite's copy and keeps ReShade.ini and the preset.");
                r = r.child(row(title, Some(about), div()));
                let adopt = action_button(
                    "reshade-adopt",
                    ButtonKind::Primary,
                    tr!("Adopt"),
                    self.reshade.busy.is_none(),
                    cx.listener(|this, _, _, cx| this.adopt_reshade(cx)),
                );
                let replace = self.outline_button("reshade-replace", tr!("Replace"), self.reshade.busy.is_none(), cx, |this, _, cx| {
                    this.start_reshade_install(true, true, cx)
                });
                r = r.field(buttons().child(adopt).child(replace));
                if matches!(self.reshade.confirm, Some(Confirm::AntiCheat { .. })) {
                    r = r.child(self.reshade_anticheat_card(o, cx));
                }
            }
            State::NotInstalled | State::Installed => {}
        }
        r
    }

    /// "Executable": the detected one, or one of the programs next to it.
    fn reshade_exe_field(&self, r: Rows, o: &Arc<Overview>, cx: &mut Context<Self>) -> Rows {
        let current = o.exe.as_deref().map(file_name).unwrap_or_default();
        let chosen = !self.reshade_settings().target.is_empty();
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
        // Nothing to pick: the only program is the one shown.
        if !chosen && o.candidates.iter().all(|c| o.exe.as_deref() == Some(c.as_path())) {
            return r;
        }
        let list = o.candidates.clone();
        r.field(self.select_row(
            "reshade-exe",
            Some(tr!("Executable")),
            current,
            items,
            self.reshade.busy.is_some(),
            cx,
            move |this, i, _, cx| this.pick_reshade_exe(if i == 0 { None } else { list.get(i - 1).cloned() }, cx),
        ))
    }

    fn reshade_install_form(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let t = o.target.as_ref();
        let mut card = rows();
        let exe = o.exe.as_deref().map(file_name).unwrap_or_default();
        let mut about = format!(
            "{} · {}",
            api_label(t, t.and_then(|t| t.api)),
            tr!("ReShade goes in its folder as {dll}", dll = o.proxy.clone().unwrap_or_default())
        );
        if t.is_some_and(|t| t.api_from_list) {
            about.push_str(&format!(" · {}", tr!("API from ReShade's list")));
        }
        card = card.child(row(exe, Some(&about), div()));
        if let Some(l) = &o.launcher {
            card = card.child(div().py(px(12.)).child(help(&tr!(
                "{launcher} is a launcher: ReShade goes with the program that draws the game.",
                launcher = file_name(l)
            ))));
        }
        card = self.reshade_exe_field(card, o, cx);
        if t.is_some_and(|t| t.small_stack) {
            card = card.child(div().py(px(12.)).child(warn_line(tr!("This program reserves little stack memory: ReShade may crash it."))));
        }
        if t.and_then(|t| t.api) == Some(Api::Dxgi) {
            let current = o.proxy.clone().unwrap_or_default();
            let items = ["dxgi.dll", "d3d11.dll"].iter().map(|n| (SharedString::from(*n), current == *n)).collect();
            card = card.field_help(
                self.select_row("reshade-dll", Some(tr!("Installed as")), current, items, busy, cx, |this, i, _, cx| {
                    this.pick_reshade_api(["dxgi", "d3d11"][i], cx)
                }),
                vec![help(tr!("dxgi.dll works for DirectX 10 to 12. Pick d3d11.dll when another mod already uses dxgi.dll."))],
            );
        }
        card = self.reshade_preset_row(card, cx);
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            card =
                card.child(div().py(px(12.)).child(warn_line(&tr!(
                    "Anti-cheat: {names}. You will be asked to confirm before installing.",
                    names = names.join(", ")
                ))));
        }
        let mut col = div().flex().flex_col().gap(px(6.)).child(card.card());
        col = col.child(heading(tr!("Shader packages"))).child(self.reshade_packages(o, cx).card());
        let version = o.latest.clone();
        if matches!(self.reshade.confirm, Some(Confirm::AntiCheat { .. })) {
            return col.child(rows().child(self.reshade_anticheat_card(o, cx)).card());
        }
        let label = match &version {
            Some(v) => tr!("Install ReShade {version}", version = v),
            None => tr!("Install ReShade").into(),
        };
        let install = action_button(
            "reshade-install",
            ButtonKind::Primary,
            label,
            !busy && version.is_some(),
            cx.listener(|this, _, _, cx| this.start_reshade_install(false, true, cx)),
        );
        col.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .pt(px(4.))
                .child(buttons().child(install))
                .child(help(tr!("Downloads ReShade and the shaders from their official sources into the game's folder."))),
        )
    }

    /// The preset to import with the install.
    fn reshade_preset_row(&mut self, card: Rows, cx: &mut Context<Self>) -> Rows {
        let busy = self.reshade.busy.is_some();
        let (about, chosen) = match &self.reshade.preset {
            Some((path, effects)) => {
                (format!("{} · {}", file_name(path), heronite_i18n::trn!(effects.len(), "{n} effect", "{n} effects")), true)
            }
            None => (tr!("Optional: a preset (.ini) made for this game. It is copied; the original is not changed.").to_string(), false),
        };
        let mut control = buttons();
        let pick = self.outline_button(
            "reshade-preset",
            if chosen { tr!("Change…") } else { tr!("Import preset…") },
            !busy,
            cx,
            |this, window, cx| this.pick_reshade_preset(window, cx),
        );
        control = control.child(pick);
        if chosen {
            control = control.child(self.outline_button("reshade-preset-clear", tr!("Clear"), !busy, cx, |this, _, cx| {
                this.reshade.preset = None;
                this.reshade.picked = None;
                cx.notify();
            }));
        }
        let mut card = card.child(row(tr!("Preset"), Some(&about), control));
        let missing = self.reshade_preset_needs().missing;
        if !missing.is_empty() {
            let text = tr!(
                "Not in ReShade's packages: {files}. After installing, add them with Add folder or Add repository.",
                files = missing.join(", ")
            );
            card = card.child(div().py(px(12.)).child(warn_line(&text)));
        }
        card
    }

    fn reshade_packages(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Rows {
        let picked = self.reshade_picked();
        let needed = self.reshade_preset_needs().packages;
        let mut r = rows();
        let shown: Vec<_> =
            o.packages.iter().filter(|p| self.reshade.all_packages || p.enabled || p.required || picked.contains(&p.name)).collect();
        for p in &shown {
            let on = p.required || picked.contains(&p.name);
            let mut desc = p.description.clone();
            if p.required {
                desc = format!("{desc} · {}", tr!("Always installed"));
            } else if needed.contains(&p.name) {
                desc = format!("{desc} · {}", tr!("Used by the preset"));
            }
            let id = SharedString::from(format!("reshade-pkg-{}", p.name));
            let sw = toggle(SharedString::from(format!("reshade-pkg-sw-{}", p.name)), on, p.required);
            let mut el = row(p.name.clone(), Some(&desc), sw).id(id);
            if !p.required && self.reshade.busy.is_none() {
                let name = p.name.clone();
                el = el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle_reshade_package(&name, cx)));
            }
            r = r.child(el);
        }
        if o.packages.is_empty() {
            r = r.child(row(tr!("Shader packages"), Some(tr!("ReShade's package list could not be read. Check the connection.")), div()));
        } else if !self.reshade.all_packages && shown.len() < o.packages.len() {
            let more = self.outline_button(
                "reshade-all-packages",
                &tr!("Show all {n} packages", n = o.packages.len()),
                true,
                cx,
                |this, _, cx| {
                    this.reshade.all_packages = true;
                    cx.notify();
                },
            );
            r = r.field(buttons().child(more));
        }
        r
    }

    /// The anti-cheat question: who says what, the risk, a box to tick, and
    /// Cancel as the default.
    pub(super) fn reshade_anticheat_card(&self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let Some(Confirm::AntiCheat { accepted, .. }) = self.reshade.confirm.clone() else { return div() };
        let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
        let title = div().flex().items_center().gap(px(8.)).child(icon(Icon::Warning, px(18.), theme::accent())).child(
            div()
                .text_size(px(15.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(tr!("This game uses {anticheats}", anticheats = names.join(", "))),
        );
        let risk = tr!(
            "This game uses {anticheats}. Anti-cheat software can block ReShade, stop the game from starting, or ban your account, especially in online modes. Heronite cannot tell whether it is safe for this game.",
            anticheats = names.join(", ")
        );
        let mut sources = div().flex().flex_col().gap(px(2.));
        for f in &o.risk.findings {
            let from = match &f.source {
                Source::List => tr!("{name}: listed on areweanticheatyet.com", name = heronite_data::anticheat::short_name(&f.name)),
                Source::File(p) => {
                    tr!("{name}: {file} is in the game's folder", name = heronite_data::anticheat::short_name(&f.name), file = file_name(p))
                }
            };
            sources = sources.child(help(&from));
        }
        let tick = checkbox("reshade-ac-accept", accepted, tr!("I understand the risk and want to install anyway")).on_click(cx.listener(
            |this, _, _, cx| {
                if let Some(Confirm::AntiCheat { accepted, .. }) = &mut this.reshade.confirm {
                    *accepted = !*accepted;
                }
                cx.notify();
            },
        ));
        let cancel = action_button(
            "reshade-ac-cancel",
            ButtonKind::Primary,
            tr!("Cancel"),
            true,
            cx.listener(|this, _, _, cx| {
                this.reshade.confirm = None;
                cx.notify();
            }),
        );
        let go = action_button(
            "reshade-ac-install",
            ButtonKind::Danger,
            tr!("Install anyway"),
            accepted,
            cx.listener(|this, _, _, cx| this.accept_anticheat(cx)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(12.))
            .child(title)
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(risk))
            .child(sources)
            .child(tick)
            .child(buttons().child(cancel).child(go))
    }
}
