//! Console rows of "Frame generation" (see `settings_panel::lsfg`).

use std::rc::Rc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::lsfg::{self, Layer};
use serde_json::{json, Value};

use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;
use crate::icons::Icon;
use crate::ui::lsfg::{open_site, open_store, open_ui, Missing};

impl SettingsPanel {
    /// `steam`: a Steam game, whose rows follow the launch options' lock.
    pub(super) fn c_lsfg(&self, steam: bool) -> Vec<Item> {
        let disabled = steam && !self.steam_fields_editable();
        let s = self.lsfg_settings();
        let missing = s.enabled.then(|| self.lsfg_missing()).flatten();
        // On but without effect: the reason replaces the description.
        let desc = missing.map_or(tr!("Smoother motion from generated frames (needs lsfg-vk and Lossless Scaling)"), Missing::status);
        let mut v = vec![
            Self::c_header(tr!("Frame generation")),
            self.c_toggle(lsfg::KEY_ENABLED, false, tr!("Lossless Scaling frame generation"), Some(desc), disabled),
        ];
        if !s.enabled {
            return v;
        }
        if let Some(m) = missing {
            v.push(item(Row::info("i:lsfg-missing", m.title(), "").danger().desc(m.why()), Act::None));
            if matches!(m, Missing::Layer | Missing::OldLayer) {
                v.push(Self::c_do("lsfg-site", tr!("Open lsfg-vk.dev"), Icon::OpenInNew, true, |_, _, _| open_site()));
            }
            if m != Missing::OldLayer {
                v.push(Self::c_do("lsfg-store", tr!("Lossless Scaling on Steam"), Icon::OpenInNew, true, |_, _, _| open_store()));
            }
            v.push(item(
                Row::action("d:lsfg-look-again", tr!("Look again")).icon(Icon::Refresh).desc(m.steps().join(" ")),
                Act::Do(Rc::new(|this, _, cx| this.load_lsfg(cx))),
            ));
            return v;
        }
        let Some(install) = &self.lsfg else { return v };
        let uses_profile = install.uses_profile(&s.profile);
        let profiles = match install.layer {
            Some(Layer::V2) => install.profiles.clone(),
            _ => Vec::new(),
        };
        if !profiles.is_empty() {
            let mut opts: Vec<SharedString> = vec![tr!("Set here").into()];
            opts.extend(profiles.iter().map(|p| SharedString::from(tr!("lsfg-vk profile: {name}", name = p))));
            let index = profiles.iter().position(|p| uses_profile && *p == s.profile).map_or(0, |i| i + 1);
            let list = profiles.clone();
            v.push(item(
                Row::choice("c:lsfg-source", tr!("Settings"), opts, index)
                    .enabled(!disabled)
                    .desc(tr!("Profiles are edited in lsfg-vk-ui, even while the game runs")),
                Act::Choice(Rc::new(move |this, i, cx| {
                    let name = if i == 0 { String::new() } else { list[i - 1].clone() };
                    this.set(lsfg::KEY_PROFILE, Value::String(name), cx)
                })),
            ));
        }
        if uses_profile {
            if let Some(program) = self.lsfg_ui.clone() {
                v.push(Self::c_do("lsfg-ui", tr!("Open lsfg-vk-ui"), Icon::OpenInNew, true, move |_, _, _| open_ui(&program)));
            }
            return v;
        }
        let multipliers = lsfg::MULTIPLIERS.iter().map(|n| SharedString::from(format!("{n}×"))).collect();
        let at = lsfg::MULTIPLIERS.iter().position(|n| *n == s.multiplier).unwrap_or(0);
        v.push(item(
            Row::choice("c:lsfg-multiplier", tr!("Multiplier"), multipliers, at)
                .enabled(!disabled)
                .desc(tr!("Frames shown for each frame the game draws: 2× turns 60 FPS into 120")),
            Act::Choice(Rc::new(|this, i, cx| this.set(lsfg::KEY_MULTIPLIER, json!(lsfg::MULTIPLIERS[i]), cx))),
        ));
        let flows = lsfg::FLOW_SCALES.iter().map(|n| SharedString::from(format!("{n}%"))).collect();
        let at = lsfg::FLOW_SCALES.iter().position(|n| *n == s.flow_scale).unwrap_or(lsfg::FLOW_SCALES.len() - 1);
        v.push(item(
            Row::choice("c:lsfg-flow", tr!("Flow scale"), flows, at)
                .enabled(!disabled)
                .desc(tr!("Lower is faster: 100% at 1080p, 75% at 1440p, 50% at 4K")),
            Act::Choice(Rc::new(|this, i, cx| this.set(lsfg::KEY_FLOW_SCALE, json!(lsfg::FLOW_SCALES[i]), cx))),
        ));
        v.push(self.c_toggle(
            lsfg::KEY_PERFORMANCE,
            false,
            tr!("Performance mode"),
            Some(tr!("Lighter model, a little less quality")),
            disabled,
        ));
        v.push(self.c_toggle(lsfg::KEY_HDR, false, "HDR", Some(tr!("Turn on when the game outputs HDR")), disabled));
        v
    }
}
