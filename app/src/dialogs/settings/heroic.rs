//! Settings tabs backed by Heroic's own files: "General" (global settings)
//! and "Game defaults" (the Wine / Other / Advanced / Gamescope defaults new
//! games inherit). Both host a [`HeroicSettingsPanel`], created the first
//! time the tab is shown.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::SettingsDialog;
use crate::dialogs::heroic_settings::logic::DEFAULT_SECTIONS;
use crate::dialogs::heroic_settings::{HeroicSettingsPanel, Section, Target};
use crate::ui::controls::tabs;

#[derive(Default)]
pub struct State {
    general: Option<Entity<HeroicSettingsPanel>>,
    defaults: Option<Entity<HeroicSettingsPanel>>,
    defaults_section: Option<Section>,
}

fn make(d: &SettingsDialog, target: Target, section: Section, window: &mut Window, cx: &mut Context<SettingsDialog>) -> Entity<HeroicSettingsPanel> {
    let heroic = d.ctx.heroic.clone();
    let panel = cx.new(|cx| HeroicSettingsPanel::new(heroic, target, section, window, cx));
    cx.observe(&panel, |_, _, cx| cx.notify()).detach();
    panel
}

pub fn render_general(d: &mut SettingsDialog, window: &mut Window, cx: &mut Context<SettingsDialog>) -> AnyElement {
    if d.heroic.general.is_none() {
        d.heroic.general = Some(make(d, Target::Defaults, Section::General, window, cx));
    }
    let panel = d.heroic.general.clone().expect("created above");
    div().child(panel).into_any_element()
}

pub fn render_defaults(d: &mut SettingsDialog, window: &mut Window, cx: &mut Context<SettingsDialog>) -> AnyElement {
    let section = d.heroic.defaults_section.unwrap_or(DEFAULT_SECTIONS[0]);
    if d.heroic.defaults.is_none() {
        d.heroic.defaults = Some(make(d, Target::Defaults, section, window, cx));
    }
    let panel = d.heroic.defaults.clone().expect("created above");
    let items: Vec<(Section, &'static str)> = DEFAULT_SECTIONS.iter().map(|s| (*s, s.label())).collect();
    let entity = cx.entity().downgrade();
    let sub_panel = panel.clone();
    let bar = tabs("gdef", &items, section, move |s, window, cx| {
        sub_panel.update(cx, |p, cx| p.set_section(s, window, cx));
        entity
            .update(cx, |d, cx| {
                d.heroic.defaults_section = Some(s);
                cx.notify();
            })
            .ok();
    });
    div().flex().flex_col().gap(px(16.)).child(bar).child(panel).into_any_element()
}

/// `HERONITE_DEBUG_DIALOG=settings:general|settings:defaults[:<section>]`.
pub fn debug_open(d: &mut SettingsDialog, which: &str) {
    if which == "settings:general" {
        d.tab = super::Tab::General;
        return;
    }
    d.tab = super::Tab::GameDefaults;
    d.heroic.defaults_section = match which.rsplit(':').next() {
        Some("other") => Some(Section::Other),
        Some("advanced") => Some(Section::Advanced),
        Some("gamescope") => Some(Section::Gamescope),
        _ => None,
    };
}
