//! "Links" in the focus panel's side column and the grid's drawer: one
//! icon per store page or site of the focused game
//! (`heronite_data::game_links`), its name on hover, opened in the browser.
//! Off until turned on in Settings → Appearance → Game sections.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::Section;
use heronite_data::game_links::{links, GameLink, LinkKind};
use heronite_i18n::tr;

use super::focus::h3;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

fn link_icon(kind: LinkKind) -> Icon {
    match kind {
        LinkKind::Steam => Icon::BrandSteam,
        LinkKind::Epic => Icon::BrandEpicGames,
        LinkKind::Gog => Icon::BrandGog,
        LinkKind::Amazon => Icon::BrandAmazon,
        LinkKind::Itch => Icon::BrandItch,
        LinkKind::Humble => Icon::BrandHumble,
        LinkKind::Ea => Icon::BrandEa,
        LinkKind::Ubisoft => Icon::BrandUbisoft,
        LinkKind::BattleNet => Icon::BrandBattleNet,
        LinkKind::Rockstar => Icon::BrandRockstar,
        LinkKind::Microsoft | LinkKind::Nintendo => Icon::Storefront,
        LinkKind::PlayStation => Icon::BrandPlayStation,
        LinkKind::GameJolt => Icon::BrandGameJolt,
        LinkKind::Website => Icon::Language,
        LinkKind::PcGamingWiki => Icon::BrandPcGamingWiki,
        LinkKind::SteamDb => Icon::BrandSteamDb,
        LinkKind::ProtonDb => Icon::BrandProtonDb,
        LinkKind::Igdb => Icon::BrandIgdb,
        LinkKind::Metacritic => Icon::BrandMetacritic,
        LinkKind::Wikipedia => Icon::BrandWikipedia,
        LinkKind::Fandom => Icon::BrandFandom,
        LinkKind::YouTube => Icon::BrandYouTube,
        LinkKind::Twitch => Icon::BrandTwitch,
        LinkKind::Discord => Icon::BrandDiscord,
        LinkKind::Reddit => Icon::BrandReddit,
        LinkKind::X => Icon::BrandX,
    }
}

/// What the link's hover says: the store or site, by its own name.
fn link_name(link: &GameLink) -> String {
    let name = match link.kind {
        LinkKind::Steam => "Steam",
        LinkKind::Epic => "Epic Games Store",
        LinkKind::Gog => "GOG",
        LinkKind::Amazon => "Amazon",
        LinkKind::Itch => "itch.io",
        LinkKind::Humble => "Humble Store",
        LinkKind::Ea => "EA",
        LinkKind::Ubisoft => "Ubisoft Store",
        LinkKind::BattleNet => "Battle.net",
        LinkKind::Rockstar => "Rockstar Games",
        LinkKind::Microsoft => "Microsoft Store",
        LinkKind::PlayStation => "PlayStation Store",
        LinkKind::Nintendo => "Nintendo eShop",
        LinkKind::GameJolt => "Game Jolt",
        LinkKind::PcGamingWiki => "PCGamingWiki",
        LinkKind::SteamDb => "SteamDB",
        LinkKind::ProtonDb => "ProtonDB",
        LinkKind::Igdb => "IGDB",
        LinkKind::Metacritic => "Metacritic",
        LinkKind::Wikipedia => "Wikipedia",
        LinkKind::Fandom => "Fandom",
        LinkKind::YouTube => "YouTube",
        LinkKind::Twitch => "Twitch",
        LinkKind::Discord => "Discord",
        LinkKind::Reddit => "Reddit",
        LinkKind::X => "X",
        LinkKind::Website => return link.host.clone(),
    };
    name.to_string()
}

impl CollectionView {
    /// The section for the game `key`; `None` when it is off or the game
    /// has no link.
    pub(super) fn links_section(&self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !theme::appearance().sections.shown(Section::Links) {
            return None;
        }
        let copies = self.with_copies(key);
        // The card's own copy first: its store page leads.
        let mut games: Vec<&heronite_data::CollectionGame> = copies.iter().map(|g| &**g).collect();
        if let Some(i) = games.iter().position(|g| g.key == key) {
            games[..=i].rotate_right(1);
        }
        let found = links(&games, cfg!(target_os = "linux"));
        if found.is_empty() {
            return None;
        }
        let row = found.into_iter().enumerate().fold(div().flex().flex_wrap().gap(px(8.)), |row, (i, link)| {
            let url = link.url.clone();
            row.child(
                div()
                    .id(("game-link", i))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(38.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .bg(theme::focus_card_fill(true))
                    .border_1()
                    .border_color(theme::ink_a(0.1))
                    .hover(|s| s.bg(theme::ink_a(0.12)).border_color(theme::ink_a(0.2)))
                    .tooltip(tooltip(link_name(&link)))
                    .child(icon(link_icon(link.kind), px(19.), theme::text_primary()))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_url(url.clone(), cx))),
            )
        });
        Some(div().flex().flex_col().child(h3(tr!("Links"))).child(row).into_any_element())
    }
}
