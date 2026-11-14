//! "Links" in the focus panel's side column and the grid's drawer: one
//! icon per store page or site of the focused game
//! (`heronite_data::game_links`), its name on hover, opened in the browser.
//! Its title is a menu like Details' ("Add links…" opens the game's links
//! in `dialogs/game_links.rs`), so a game without a link still shows the
//! section.
//! Off until turned on in Settings → Appearance → Game sections.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::Section;
use heronite_data::game_links::{links, GameLink, LinkKind};
use heronite_i18n::tr;

use std::rc::Rc;

use super::focus::{hover_section, section_title, MenuEntry, TitleMenu};
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tooltip};

pub(crate) fn link_icon(kind: LinkKind) -> Icon {
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
pub(crate) fn link_name(link: &GameLink) -> String {
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
    /// "Links of …": the game's links, to add and remove its own.
    pub(super) fn open_game_links(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let copies = self.with_copies(key);
        let mut games: Vec<&heronite_data::CollectionGame> = copies.iter().map(|g| &**g).collect();
        if let Some(i) = games.iter().position(|g| g.key == key) {
            games[..=i].rotate_right(1);
        }
        let found = links(&games, cfg!(target_os = "linux"));
        let service = crate::dialogs::metadata_wizard::hub(cx).read(cx).service.clone();
        let view = cx.new(|cx| crate::dialogs::game_links::GameLinksDialog::new(&game, found, service, window, cx));
        self.open_dialog(view, cx);
    }

    /// The section for the game `key` (`None` when it is off); its title
    /// opens "Add links…".
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
        let empty = found.is_empty();
        // Bare icons, dimmed until pointed at: the row reads as a footnote of
        // the page, not as a set of buttons.
        let row = found.into_iter().enumerate().fold(div().flex().flex_wrap().gap(px(4.)).ml(px(-6.)), |row, (i, link)| {
            let url = link.url.clone();
            let group = SharedString::from(format!("game-link-{i}"));
            row.child(
                div()
                    .id(("game-link", i))
                    .group(group.clone())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::ink_a(0.08)))
                    .tooltip(tooltip(link_name(&link)))
                    .child(
                        icon(link_icon(link.kind), px(18.), theme::ink_a(0.55)).group_hover(group, |s| s.text_color(theme::text_bright())),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_url(url.clone(), cx))),
            )
        });
        let weak = cx.entity().downgrade();
        let entries = vec![MenuEntry {
            id: "add",
            icon: Icon::Add,
            label: tr!("Add links…").into(),
            run: Rc::new({
                let key = key.to_string();
                move |window: &mut Window, cx: &mut App| {
                    weak.update(cx, |this, cx| this.open_game_links(&key, window, cx)).ok();
                }
            }),
            separated: false,
        }];
        let on_title = self.on_title(cx);
        let title = section_title(TitleMenu::Links, tr!("Links"), self.facts.titles, entries, &on_title);
        let body = if empty {
            div()
                .text_size(px(13.))
                .text_color(theme::text_muted())
                .child(tr!("No links yet. Add the game's sites from its title."))
                .into_any_element()
        } else {
            row.into_any_element()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .child(hover_section("focus-links", TitleMenu::Links, &on_title).child(title).child(body))
                .into_any_element(),
        )
    }
}
