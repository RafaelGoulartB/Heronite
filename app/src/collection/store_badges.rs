//! The stores a game comes from on its cover (Settings → Appearance →
//! Covers → Store icon): their logos in one small dark chip at the
//! bottom-left, the look of the "Hidden" and copies badges.

use gpui_kit::prelude::*;
use gpui_kit::*;

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

/// The logos of `stores` (Store facet values, the game's own first), each
/// once and in order.
pub fn store_icons<'a>(stores: impl IntoIterator<Item = &'a str>) -> Vec<Icon> {
    let mut icons = Vec::new();
    for i in stores.into_iter().filter_map(store_icon) {
        if !icons.contains(&i) {
            icons.push(i);
        }
    }
    icons
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
            .gap(px(5.5))
            .h(px(20.))
            .px(px(5.5))
            .rounded(px(4.5))
            .bg(theme::black_a(0.62))
            .children(logos.iter().map(|i| icon(*i, px(13.2), theme::white_a(0.92))))
            .children(more.map(|n| {
                div()
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::white_a(0.7))
                    .child(format!("+{n}"))
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::{shown, store_icon, store_icons};
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
        let icons = store_icons(["GOG", "Steam", "Lutris", "Steam", "Epic Games"]);
        assert_eq!(icons, vec![Icon::BrandGog, Icon::BrandSteam, Icon::BrandEpicGames]);
    }

    #[test]
    fn past_three_stores_the_rest_is_counted() {
        let three = [Icon::BrandSteam, Icon::BrandGog, Icon::BrandEa];
        assert_eq!(shown(&three), (&three[..], None));
        let four = [Icon::BrandSteam, Icon::BrandGog, Icon::BrandEa, Icon::BrandAmazon];
        assert_eq!(shown(&four), (&four[..2], Some(2)));
    }
}
