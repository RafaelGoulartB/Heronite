//! "HDR (RenoDX)" in the installed ReShade's card: RenoDX's HDR mod for the
//! game and the ReShade version with add-on support it needs
//! (`heronite_launch::gfxmods`). It is one closed line by default, so the
//! card reads as before; opened, it shows the two steps in order (the
//! version, then the mod) and what HDR needs before playing, since an HDR
//! mod cannot fix a system whose HDR is off.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::gfxmods::anticheat::Policy;
use heronite_launch::gfxmods::catalog::{Engine, Found, How};
use heronite_launch::gfxmods::renodx;
use heronite_launch::reshade::detect::Arch;
use heronite_launch::reshade::manifest::Manifest;
use heronite_launch::reshade::Overview;

use super::render::{buttons, warn_line};
use super::render_setup::link;
use super::SettingsPanel;
use crate::dialogs::settings_panel::parts::help;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, row, ButtonKind};
use crate::ui::icon;

/// RenoDX's list of mods, for what the catalog does not say.
const RENODX_LIST: &str = "https://github.com/clshortfuse/renodx/wiki/mods";

/// Why RenoDX cannot be installed here, as the panel says it.
fn renodx_refusal(o: &Overview, m: &Manifest, found: &Found) -> Option<String> {
    if o.risk.blocks(Policy::Block) {
        return Some(tr!("Not available for games with anti-cheat.").to_string());
    }
    if !m.addon {
        return Some(tr!("Needs the add-on version of ReShade first (above).").to_string());
    }
    let arch = if m.arch == "x86" { Arch::X86 } else { Arch::X64 };
    match renodx::url_for(found, arch) {
        None => Some(tr!("This mod has no {bits}-bit version.", bits = arch.bits()).to_string()),
        Some(u) if !renodx::allowed_url(u) => Some(tr!("This mod is not on GitHub, so Heronite does not download it.").to_string()),
        Some(_) => None,
    }
}

fn how_line(found: &Found) -> String {
    match found.how {
        How::Steam | How::Exe => tr!("Made for this game").to_string(),
        How::Title => tr!("Made for this game (found by its name)").to_string(),
        How::Engine(Engine::Unreal) => tr!("Generic mod for Unreal Engine games: it may not suit this one").to_string(),
        How::Engine(Engine::Unity) => tr!("Generic mod for Unity games: it may not suit this one").to_string(),
    }
}

/// The link to RenoDX's list of mods.
fn renodx_list(cx: &mut Context<SettingsPanel>) -> Div {
    div().flex().child(link("renodx-page", tr!("RenoDX's mod list")).on_click(cx.listener(|_, _, _, cx| cx.open_url(RENODX_LIST))))
}

fn small_button(
    id: &'static str,
    kind: ButtonKind,
    label: &str,
    enabled: bool,
    on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    action_button(id, kind, label.to_string(), enabled, on).h(px(30.)).text_size(px(12.))
}

impl SettingsPanel {
    /// What this adds to an installed ReShade, folded into one line ("HDR
    /// (RenoDX)" and a chevron) so the card reads as before until it is
    /// opened.
    pub(super) fn reshade_mod_rows(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Vec<Div> {
        vec![self.reshade_hdr_section(o, m, cx)]
    }

    fn reshade_hdr_section(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let open = self.reshade.renodx_details;
        let blocked = o.risk.blocks(Policy::Block);
        let found = o.renodx.clone();
        let installed = o.renodx_installed.clone();
        let current = installed.as_ref().is_some_and(|i| found.as_ref().is_some_and(|f| f.id == i.id));
        // Closed, the line still says where things stand.
        let status = match &found {
            None => tr!("No RenoDX mod known for this game").to_string(),
            Some(f) if current => tr!("{name} · installed", name = f.name.clone()).to_string(),
            Some(f) if blocked => tr!("{name} · not available with anti-cheat", name = f.name.clone()).to_string(),
            Some(f) => tr!("{name} · available", name = f.name.clone()).to_string(),
        };
        let chevron = div()
            .id("renodx-toggle")
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(theme::button_fill())
            .border_1()
            .border_color(theme::button_border())
            .hover(|s| s.bg(theme::button_fill_hover()))
            .child(icon(if open { Icon::FaChevronUp } else { Icon::FaChevronDown }, px(12.), theme::button_text()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.reshade.renodx_details = !this.reshade.renodx_details;
                cx.notify();
            }));
        let section = div().flex().flex_col().child(row(tr!("HDR (RenoDX)"), Some(&status), chevron));
        if !open {
            return section;
        }
        // Opened: the two steps in order, then what HDR needs.
        let mut inner = div()
            .mt(px(4.))
            .mb(px(12.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(theme::ink_a(0.04))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(self.reshade_build_row(o, m, cx))
            .child(div().h(px(1.)).my(px(8.)).bg(theme::ink_a(0.08)));
        let Some(found) = found else {
            return section.child(
                inner
                    .child(help(tr!("Heronite's catalog has no RenoDX mod for this game. RenoDX's list may have one to install by hand.")))
                    .child(renodx_list(cx)),
            );
        };
        let busy = self.reshade.busy.is_some();
        let refusal = renodx_refusal(o, m, &found);
        let can = !busy && refusal.is_none();
        let control = match current {
            true => {
                let f = found.clone();
                buttons()
                    .child(small_button(
                        "renodx-update",
                        ButtonKind::Glass,
                        tr!("Update"),
                        can,
                        cx.listener(move |this, _, _, cx| this.install_renodx(f.clone(), true, cx)),
                    ))
                    .child(small_button(
                        "renodx-remove",
                        ButtonKind::Glass,
                        tr!("Remove"),
                        !busy,
                        cx.listener(|this, _, _, cx| this.remove_renodx(cx)),
                    ))
            }
            false => {
                let f = found.clone();
                buttons().child(small_button(
                    "renodx-install",
                    ButtonKind::Primary,
                    tr!("Install"),
                    can,
                    cx.listener(move |this, _, _, cx| this.install_renodx(f.clone(), false, cx)),
                ))
            }
        };
        inner = inner.child(row(found.name.clone(), Some(&how_line(&found)), control));
        if !found.mod_.working() {
            inner = inner.child(warn_line(tr!("In progress at RenoDX: it may not work well yet.")));
        }
        if let Some(i) = installed.as_ref().filter(|_| !current) {
            inner = inner.child(help(&tr!("Replaces the RenoDX mod already installed ({file}).", file = i.file.clone())));
        }
        if let Some(why) = &refusal {
            inner = inner.child(warn_line(why));
        }
        if let Some(note) = found.mod_.note.as_deref().filter(|n| !n.is_empty()) {
            inner = inner.child(help(&tr!("RenoDX's note: {note}", note)));
        }
        // What HDR needs, unless an anti-cheat rules the mod out anyway.
        if !blocked {
            inner = inner.child(div().h(px(1.)).my(px(8.)).bg(theme::ink_a(0.08))).child(self.hdr_needs());
        }
        inner = inner.child(div().pt(px(4.)).child(renodx_list(cx)));
        section.child(inner)
    }

    /// "Before playing": what HDR needs on this system. Under Wine or
    /// Proton it also says whether the game's own HDR option is on.
    fn hdr_needs(&self) -> Div {
        let bullet = |text: &str| div().flex().gap(px(6.)).child(help("•")).child(help(text).flex_1().min_w_0());
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).pb(px(2.)).child(tr!("Before playing").to_string()))
            .child(bullet(tr!("Turn on HDR in the system, on the display and in the game's own settings.")));
        match self.reshade_wine() {
            Some(true) => {
                list = list.child(bullet(tr!("The desktop must support HDR (gamescope, KDE Plasma 6, Hyprland or GNOME 48+).")));
                list = match self.flag("enableHDR", false) {
                    true => list.child(bullet(tr!("This game's HDR option is on."))),
                    false => list
                        .child(warn_line(tr!("This game's HDR option is off: turn on “Wayland driver” and then “HDR” in its settings."))),
                };
            }
            _ => list = list.child(bullet(tr!("Turn off Windows Auto HDR and RTX HDR for this game, or the image is tone-mapped twice."))),
        }
        list.child(bullet(tr!("Adjust the mod in ReShade's menu (Home key).")))
    }

    fn reshade_build_row(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let blocked = o.risk.blocks(Policy::Block);
        let has_addon = o.renodx_installed.is_some();
        let (about, label, enabled) = match m.addon {
            true => (tr!("With add-on support (needed for RenoDX)"), tr!("Use the standard version"), !busy && !has_addon),
            false => (tr!("Standard version"), tr!("Use the add-on version"), !busy && !blocked),
        };
        let on = !m.addon;
        let button = small_button(
            "reshade-build",
            ButtonKind::Glass,
            label,
            enabled,
            cx.listener(move |this, _, _, cx| this.set_reshade_addon(on, cx)),
        );
        let note = match (m.addon, blocked) {
            (true, _) if has_addon => tr!("Remove the HDR mod to go back to the standard version."),
            (false, true) => tr!("Not available: this game has anti-cheat, which can block it or ban the account."),
            _ => tr!("The add-on version lets add-ons change the game's image. Anti-cheats look for it more often, so use it in single-player games only."),
        };
        div().flex().flex_col().gap(px(4.)).child(row(tr!("ReShade version"), Some(about), button)).child(help(note))
    }
}
