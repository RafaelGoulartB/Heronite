//! The stores a game comes from on its cover (Settings → Appearance →
//! Covers → Store icon): their logos in one small dark chip at the
//! bottom-left, the look of the "Hidden" and copies badges. A ROM shows its
//! emulator's icon instead.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::FacetKind;
use heronite_data::CollectionGame;
use heronite_emulation::catalog::guess_definition_id;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Logos past this many give way to "+N".
const MAX_SHOWN: usize = 3;

/// The logo of a store, by its name in the Store facet
/// (`heronite_data::collection::store_facet`). `None` for the sources
/// without one (launchers, emulators, games added by hand).
pub fn store_icon(store: &str) -> Option<Icon> {
    Some(match store {
        "Steam" => Icon::BrandSteam,
        "Epic Games" => Icon::BrandEpicGames,
        "GOG" => Icon::BrandGog,
        "Amazon Games" => Icon::BrandAmazon,
        "Battle.net" => Icon::BrandBattleNet,
        "EA app" => Icon::BrandEa,
        "Ubisoft Connect" => Icon::BrandUbisoft,
        "Rockstar Games" => Icon::BrandRockstar,
        _ => return None,
    })
}

/// The icons of `games` (one game's copies, the card's own first): each
/// ROM's emulator, each other copy's stores; each icon once, in order.
pub fn game_icons(games: &[impl std::ops::Deref<Target = CollectionGame>]) -> Vec<Icon> {
    let store = FacetKind::Store.index();
    once_each(games.iter().flat_map(|g| match emulator_icon(g) {
        Some(i) => vec![i],
        None => g.facets[store].iter().filter_map(|f| store_icon(&f.value)).collect(),
    }))
}

/// The icon of the emulator a ROM plays in (by its name, else its catalog
/// id; a gamepad when neither is known), `None` for a game that is not one.
fn emulator_icon(g: &CollectionGame) -> Option<Icon> {
    let meta = g.meta.as_deref().filter(|m| m.launch_kind == "emulator" || m.source == "emulator")?;
    Some(crate::pages::emulators::icon::of_definition(&emulator_definition(meta.emulator_name.as_deref(), meta.emulator_id.as_deref())))
}

/// The catalog emulator a ROM's `name` points at ("PCSX2 Nightly" is
/// PCSX2), else its `id` (a catalog id, or one that matches nothing).
fn emulator_definition(name: Option<&str>, id: Option<&str>) -> String {
    let by_name = guess_definition_id(name);
    match id {
        Some(id) if by_name == "custom" => id.to_string(),
        _ => by_name,
    }
}

/// `icons` without the repeats, in order.
fn once_each(icons: impl Iterator<Item = Icon>) -> Vec<Icon> {
    let mut out = Vec::new();
    for i in icons {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// The logos and the "+N" a chip shows for `stores`: all of them up to
/// [`MAX_SHOWN`], else one less and the count of the rest.
fn shown(stores: &[Icon]) -> (&[Icon], Option<usize>) {
    if stores.len() <= MAX_SHOWN {
        (stores, None)
    } else {
        (&stores[..MAX_SHOWN - 1], Some(stores.len() - (MAX_SHOWN - 1)))
    }
}

/// The chip with the logos of `stores`, not placed (the tags of the
/// categories take it in their row); `None` without any.
pub fn chip(stores: &[Icon]) -> Option<Div> {
    if stores.is_empty() {
        return None;
    }
    let (logos, more) = shown(stores);
    Some(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.8))
            .h(px(21.))
            .px(px(5.8))
            .rounded(px(4.7))
            .bg(theme::black_a(0.62))
            .children(logos.iter().map(|i| icon(*i, px(13.9), theme::white_a(0.92))))
            .children(more.map(|n| {
                div()
                    .text_size(px(11.5))
                    .line_height(px(15.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::white_a(0.7))
                    .child(format!("+{n}"))
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::{emulator_definition, once_each, shown, store_icon};
    use crate::icons::Icon;

    #[test]
    fn stores_with_a_logo_get_it_and_the_others_none() {
        assert_eq!(store_icon("Steam"), Some(Icon::BrandSteam));
        assert_eq!(store_icon("Epic Games"), Some(Icon::BrandEpicGames));
        assert_eq!(store_icon("Ubisoft Connect"), Some(Icon::BrandUbisoft));
        assert_eq!(store_icon("Lutris"), None);
        assert_eq!(store_icon("Xbox"), None);
    }

    #[test]
    fn each_store_shows_once_in_order() {
        let icons = once_each(["GOG", "Steam", "Lutris", "Steam", "Epic Games"].into_iter().filter_map(store_icon));
        assert_eq!(icons, vec![Icon::BrandGog, Icon::BrandSteam, Icon::BrandEpicGames]);
    }

    #[test]
    fn a_rom_takes_its_emulator_from_the_name_then_the_id() {
        assert_eq!(emulator_definition(Some("PCSX2 Nightly"), Some("0b1c-uuid")), "pcsx2");
        assert_eq!(emulator_definition(None, Some("duckstation")), "duckstation");
        assert_eq!(emulator_definition(Some("My emulator"), None), "custom");
    }

    #[test]
    fn past_three_stores_the_rest_is_counted() {
        let three = [Icon::BrandSteam, Icon::BrandGog, Icon::BrandEa];
        assert_eq!(shown(&three), (&three[..], None));
        let four = [Icon::BrandSteam, Icon::BrandGog, Icon::BrandEa, Icon::BrandAmazon];
        assert_eq!(shown(&four), (&four[..2], Some(2)));
    }
}
