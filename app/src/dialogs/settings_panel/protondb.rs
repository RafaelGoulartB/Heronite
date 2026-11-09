//! The game's ProtonDB tier next to the Proton picker, and the build that
//! fits it: Steam's default for games that run well, the newest GE-Proton
//! (where the community's fixes land) for the rest and for games run
//! through Wine.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;
use heronite_metadata::{protondb, Endpoints, Http};

use super::SettingsPanel;
use crate::theme;

/// What ProtonDB says about the game.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum ProtonDb {
    /// No Steam id to ask with (or the game runs natively).
    #[default]
    Off,
    Loading,
    /// Lowercase tier: `platinum`, `gold`, `silver`, `bronze`, `borked`, `pending`.
    Tier(String),
    /// ProtonDB has no reports, or the game was not found on Steam.
    NotFound,
}

/// The build to suggest, as an index in the options of the picker.
pub(super) struct Recommendation {
    pub(super) label: String,
    /// Index in the picker's options; `None`: not installed.
    pub(super) index: Option<usize>,
    pub(super) in_use: bool,
}

/// Games that run well keep Steam's default.
pub(super) fn runs_well(tier: &str) -> bool {
    matches!(tier, "platinum" | "gold")
}

/// One line on what the tier means for the choice of build.
pub(super) fn tier_advice(tier: &str) -> &'static str {
    match tier {
        "platinum" | "gold" => tr!("Runs well on Proton."),
        "silver" | "bronze" => tr!("Runs with tweaks: GE-Proton often has the fixes."),
        "borked" => tr!("Reported broken: GE-Proton is the best chance."),
        _ => tr!("Not enough reports yet."),
    }
}

pub(super) fn not_found_text() -> &'static str {
    tr!("ProtonDB has no reports for this game, or it was not found on Steam under this name.")
}

/// `Gold`, `Silver`...
pub(super) fn tier_name(tier: &str) -> String {
    let mut name = tier.to_string();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    name
}

fn tier_color(tier: &str) -> Rgba {
    match tier {
        "platinum" => rgb(0xb4c7dc),
        "gold" => rgb(0xcfb53b),
        "silver" => rgb(0xa6a6a6),
        "bronze" => rgb(0xcd7f32),
        "borked" => rgb(0xe5534b),
        _ => rgb(0x9aa0a6),
    }
}

/// Index of the GE-Proton build to suggest among `labels`: a `latest` one
/// (it updates itself), else the highest version.
pub(super) fn best_ge<'a>(labels: impl IntoIterator<Item = &'a str>) -> Option<usize> {
    labels
        .into_iter()
        .enumerate()
        .filter(|(_, l)| {
            let l = l.to_ascii_lowercase();
            l.contains("ge-proton") || l.contains("proton-ge")
        })
        .max_by_key(|(_, l)| {
            let l = l.to_ascii_lowercase();
            let numbers: Vec<u32> = l.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
            (l.contains("latest"), numbers)
        })
        .map(|(i, _)| i)
}

impl SettingsPanel {
    /// The Steam id ProtonDB rates the game by: Steam's, the metadata's, or
    /// one found by title (cached).
    pub(super) fn load_protondb(&mut self, cx: &mut Context<Self>) {
        let Some(facts) = self.facts().cloned() else { return };
        self.protondb = ProtonDb::Loading;
        let cache = AppPaths::detect().cache.join("protondb");
        cx.spawn(async move |this, cx| {
            let tier = cx
                .background_spawn(async move {
                    let id = facts.protondb_id.clone().or_else(|| protondb::cached_steam_id_for_title(&cache, &facts.title).flatten());
                    if let Some((tier, true)) = id.as_deref().and_then(|id| protondb::cached_tier(&cache, id)) {
                        return Some(tier);
                    }
                    let http = Http::new(Endpoints::default());
                    let id = id.or_else(|| protondb::steam_id_for_title(&cache, &http, &facts.title))?;
                    protondb::tier(&cache, &http, &id)
                })
                .await;
            this.update(cx, |this, cx| {
                this.protondb = tier.map_or(ProtonDb::NotFound, ProtonDb::Tier);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "ProtonDB: Gold — runs well" and, when there is one, the suggested
    /// build with a button that picks it.
    pub(super) fn protondb_row(
        &self,
        rec: Option<Recommendation>,
        cx: &mut Context<Self>,
        pick: fn(&mut Self, usize, &mut Context<Self>),
    ) -> Option<Div> {
        let tier = match &self.protondb {
            ProtonDb::Off => return None,
            ProtonDb::Loading => {
                return Some(
                    row_shell().child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Asking ProtonDB\u{2026}"))),
                )
            }
            ProtonDb::NotFound => {
                return Some(
                    row_shell().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).child(tr!("ProtonDB: no rating")))
                            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(not_found_text())),
                    ),
                )
            }
            ProtonDb::Tier(t) => t.clone(),
        };
        let rating = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().size(px(8.)).flex_none().rounded_full().bg(tier_color(&tier)))
            .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).child(tr!("ProtonDB: {tier}", tier = tier_name(&tier))));
        let mut left = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(rating)
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(tier_advice(&tier)));
        let mut row = row_shell();
        if let Some(rec) = rec {
            let text = match (rec.in_use, rec.index) {
                (true, _) => tr!("Recommended: {build} (in use)", build = rec.label),
                (false, Some(_)) => tr!("Recommended: {build}", build = rec.label),
                (false, None) => tr!("Recommended: {build}, from the Wine Manager", build = rec.label),
            };
            left = left.child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_primary()).child(text));
            row = row.child(left.flex_1());
            if let (false, Some(i)) = (rec.in_use, rec.index) {
                row = row.child(
                    self.outline_button("protondb-use", tr!("Use"), self.open_select.is_none(), cx, move |this, _, cx| pick(this, i, cx)),
                );
            }
            return Some(row);
        }
        Some(row.child(left.flex_1()))
    }
}

fn row_shell() -> Div {
    div().flex().items_center().justify_between().gap(px(12.)).py(px(10.))
}

#[cfg(test)]
mod tests {
    use super::best_ge;

    #[test]
    fn the_newest_ge_build_is_suggested() {
        let labels = ["Proton Experimental", "GE-Proton9-20", "GE-Proton10-3", "Proton-GE11-1-LinUwUx", "dwproton"];
        assert_eq!(best_ge(labels), Some(3));
        let labels = ["GE-Proton11-6", "GE-Proton-latest (Heronite)"];
        assert_eq!(best_ge(labels), Some(1));
        assert_eq!(best_ge(["Proton 9.0", "Wine-GE"]), None);
    }
}
