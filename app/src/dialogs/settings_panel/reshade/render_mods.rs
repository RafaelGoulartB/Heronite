//! The installed ReShade's graphics mods: the build with add-on support and
//! RenoDX's HDR mod for the game (`heronite_launch::gfxmods`). Each row says
//! plainly what the mod needs, since an HDR mod cannot fix a system whose
//! HDR is off.

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
use crate::ui::controls::{action_button, row, ButtonKind};

/// RenoDX's list of mods, for what the catalog does not say.
const RENODX_LIST: &str = "https://github.com/clshortfuse/renodx/wiki/mods";

/// Why RenoDX cannot be installed here, as the panel says it.
fn renodx_refusal(o: &Overview, m: &Manifest, found: &Found) -> Option<String> {
    if o.risk.blocks(Policy::Block) {
        return Some(tr!("Not offered for games with anti-cheat.").to_string());
    }
    if !m.addon {
        return Some(tr!("Switch ReShade to the build with add-on support first.").to_string());
    }
    let arch = if m.arch == "x86" { Arch::X86 } else { Arch::X64 };
    match renodx::url_for(found, arch) {
        None => Some(tr!("This mod has no build for {bits}-bit games.", bits = arch.bits()).to_string()),
        Some(u) if !renodx::allowed_url(u) => {
            Some(tr!("This mod is not published on GitHub, so Heronite does not download it.").to_string())
        }
        Some(_) => None,
    }
}

fn how_line(found: &Found) -> String {
    match found.how {
        How::Steam | How::Exe => tr!("Made for this game").to_string(),
        How::Title => tr!("Made for this game (matched by its title)").to_string(),
        How::Engine(Engine::Unreal) => tr!("The generic mod for Unreal Engine games: it may not suit this one").to_string(),
        How::Engine(Engine::Unity) => tr!("The generic mod for Unity games: it may not suit this one").to_string(),
    }
}

impl SettingsPanel {
    /// The add-on build's row and the HDR mod's row.
    pub(super) fn reshade_mod_rows(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Vec<Div> {
        vec![self.reshade_build_row(o, m, cx), self.reshade_renodx_row(o, m, cx)]
    }

    fn reshade_build_row(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let blocked = o.risk.blocks(Policy::Block);
        let has_addon = o.renodx_installed.is_some();
        let (about, label, enabled) = match m.addon {
            true => (tr!("With add-on support: loads add-ons such as RenoDX."), tr!("Use the standard build"), !busy && !has_addon),
            false => (
                tr!("Standard. Add-ons such as RenoDX need the build with add-on support."),
                tr!("Use the add-on build"),
                !busy && !blocked,
            ),
        };
        let on = !m.addon;
        let button = action_button(
            "reshade-build",
            ButtonKind::Glass,
            label.to_string(),
            enabled,
            cx.listener(move |this, _, _, cx| this.set_reshade_addon(on, cx)),
        )
        .h(px(30.))
        .text_size(px(12.));
        let mut notes: Vec<Div> = Vec::new();
        if m.addon && has_addon {
            notes.push(help(tr!("Remove the HDR mod before going back to the standard build.")));
        }
        if blocked && !m.addon {
            notes.push(help(tr!("Not offered for games with anti-cheat: anti-cheats block the add-on build more often than the standard one, and can ban the account.")));
        } else {
            notes.push(help(tr!("The add-on build lets add-ons read and change everything the game draws. Anti-cheats look for that more than for the standard build, so use it in single-player games only.")));
        }
        notes
            .into_iter()
            .fold(div().flex().flex_col().gap(px(4.)).child(row(tr!("Build"), Some(about), button)), |d, n| d.child(n.pb(px(8.))))
    }

    fn reshade_renodx_row(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let busy = self.reshade.busy.is_some();
        let label = tr!("HDR (RenoDX)");
        let Some(found) = o.renodx.clone() else {
            let list = link("renodx-list", tr!("RenoDX's list")).on_click(cx.listener(|_, _, _, cx| cx.open_url(RENODX_LIST)));
            return row(label, Some(tr!("Heronite knows no RenoDX mod for this game.")), list);
        };
        let installed = o.renodx_installed.as_ref();
        let current = installed.is_some_and(|i| i.id == found.id);
        let about = match (installed, current) {
            (Some(_), true) => tr!("{name} · installed", name = found.name.clone()).to_string(),
            _ => found.name.clone(),
        };
        let refusal = renodx_refusal(o, m, &found);
        let control = match current {
            true => buttons()
                .child(
                    action_button("renodx-update", ButtonKind::Glass, tr!("Update").to_string(), !busy && refusal.is_none(), {
                        let f = found.clone();
                        cx.listener(move |this, _, _, cx| this.install_renodx(f.clone(), true, cx))
                    })
                    .h(px(30.))
                    .text_size(px(12.)),
                )
                .child(
                    action_button(
                        "renodx-remove",
                        ButtonKind::Glass,
                        tr!("Remove").to_string(),
                        !busy,
                        cx.listener(|this, _, _, cx| this.remove_renodx(cx)),
                    )
                    .h(px(30.))
                    .text_size(px(12.)),
                ),
            false => buttons().child(
                action_button("renodx-install", ButtonKind::Primary, tr!("Install").to_string(), !busy && refusal.is_none(), {
                    let f = found.clone();
                    cx.listener(move |this, _, _, cx| this.install_renodx(f.clone(), false, cx))
                })
                .h(px(30.))
                .text_size(px(12.)),
            ),
        };
        let mut notes: Vec<Div> = vec![help(&how_line(&found))];
        if !found.mod_.working() {
            notes.push(warn_line(tr!("RenoDX marks this mod as in progress: it may not work yet.")));
        }
        if let Some(note) = found.mod_.note.as_deref().filter(|n| !n.is_empty()) {
            notes.push(help(&tr!("RenoDX's note: {note}", note)));
        }
        if let Some(i) = installed.filter(|_| !current) {
            notes.push(help(&tr!("Another RenoDX mod is installed ({file}); installing replaces it.", file = i.file.clone())));
        }
        if let Some(why) = &refusal {
            notes.push(warn_line(why));
        }
        notes.push(help(tr!(
            "Needs HDR turned on in the system and on the display, and the game's own HDR setting as RenoDX's page says. Under Proton, HDR also needs a compositor that supports it (gamescope, or KDE Plasma 6) and Proton's HDR option. Turn off Windows Auto HDR and RTX HDR for this game: they would tone-map twice. Its settings are in ReShade's menu (Home)."
        )));
        notes.push(
            div().flex().child(link("renodx-page", tr!("RenoDX's list")).on_click(cx.listener(|_, _, _, cx| cx.open_url(RENODX_LIST)))),
        );
        notes.into_iter().fold(div().flex().flex_col().gap(px(4.)).pb(px(12.)).child(row(label, Some(&about), control)), |d, n| d.child(n))
    }
}
