//! The focus panel's "Links": a game's store pages and the sites about it,
//! each told by its address. They come from the game itself (the store page
//! an Epic or GOG entry carries, its Steam app id) and from the websites its
//! metadata lists, for every copy of the game; the Steam app id also gives
//! the community sites keyed by it (SteamDB, PCGamingWiki, ProtonDB).

use crate::library::CollectionGame;

/// What a link is about, which gives its icon and name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LinkKind {
    // Stores, first.
    Steam,
    Epic,
    Gog,
    Amazon,
    Itch,
    Humble,
    Ea,
    Ubisoft,
    BattleNet,
    Rockstar,
    Microsoft,
    PlayStation,
    Nintendo,
    GameJolt,
    /// A site with no icon of its own (the game's official site, usually).
    Website,
    // About the game.
    PcGamingWiki,
    SteamDb,
    ProtonDb,
    Igdb,
    Metacritic,
    Wikipedia,
    Fandom,
    // Its communities.
    YouTube,
    Twitch,
    Discord,
    Reddit,
    X,
}

impl LinkKind {
    /// The kind of `host` (lower case, no port), `None` for one left out.
    fn of_host(host: &str) -> Option<LinkKind> {
        let is = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
        Some(match () {
            _ if is("steampowered.com") || is("steamcommunity.com") => LinkKind::Steam,
            _ if is("epicgames.com") => LinkKind::Epic,
            _ if is("gog.com") => LinkKind::Gog,
            _ if is("amazon.com") || is("primegaming.com") => LinkKind::Amazon,
            _ if is("itch.io") => LinkKind::Itch,
            _ if is("humblebundle.com") => LinkKind::Humble,
            _ if is("ea.com") || is("origin.com") => LinkKind::Ea,
            _ if is("ubisoft.com") || is("ubi.com") => LinkKind::Ubisoft,
            _ if is("battle.net") || is("blizzard.com") => LinkKind::BattleNet,
            _ if is("rockstargames.com") => LinkKind::Rockstar,
            _ if is("microsoft.com") || is("xbox.com") => LinkKind::Microsoft,
            _ if is("playstation.com") => LinkKind::PlayStation,
            _ if is("nintendo.com") || is("nintendo.co.jp") || is("nintendo.co.uk") || is("nintendo.de") => LinkKind::Nintendo,
            _ if is("gamejolt.com") => LinkKind::GameJolt,
            _ if is("pcgamingwiki.com") => LinkKind::PcGamingWiki,
            _ if is("steamdb.info") => LinkKind::SteamDb,
            _ if is("protondb.com") => LinkKind::ProtonDb,
            _ if is("igdb.com") => LinkKind::Igdb,
            _ if is("metacritic.com") => LinkKind::Metacritic,
            _ if is("wikipedia.org") => LinkKind::Wikipedia,
            _ if is("fandom.com") || is("wikia.com") => LinkKind::Fandom,
            _ if is("youtube.com") || is("youtu.be") => LinkKind::YouTube,
            _ if is("twitch.tv") => LinkKind::Twitch,
            _ if is("discord.gg") || is("discord.com") || is("discordapp.com") => LinkKind::Discord,
            _ if is("reddit.com") => LinkKind::Reddit,
            _ if is("twitter.com") || is("x.com") => LinkKind::X,
            // Social networks with nothing about the game a player looks for.
            _ if ["facebook.com", "instagram.com", "tiktok.com", "vk.com", "weibo.com", "bilibili.com", "t.me"].iter().any(|d| is(d)) => {
                return None
            }
            _ => LinkKind::Website,
        })
    }
}

/// One link of the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameLink {
    pub kind: LinkKind,
    pub url: String,
    /// The address's host without `www.` (what a [`LinkKind::Website`]
    /// shows as its name).
    pub host: String,
}

/// At most this many sites with no icon of their own.
const MAX_WEBSITES: usize = 3;

/// The host of a web address, lower case, without `www.`; `None` for
/// anything but `http(s)`.
fn host_of(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty() && host.contains('.')).then(|| host.to_owned())
}

/// A web address the user typed, made whole (`https://` when it has no
/// scheme) and told by its host; `None` for anything that is not a web
/// address with a host.
pub fn classify(typed: &str) -> Option<GameLink> {
    let typed = typed.trim();
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    let lower = typed.to_ascii_lowercase();
    let url = if lower.starts_with("http://") || lower.starts_with("https://") {
        typed.to_owned()
    } else if typed.contains("://") {
        return None;
    } else {
        format!("https://{typed}")
    };
    let host = host_of(&url)?;
    let kind = LinkKind::of_host(&host).unwrap_or(LinkKind::Website);
    Some(GameLink { kind, url, host })
}

/// Whether Links shows `link`: social networks are left out, even typed
/// by hand.
pub fn is_shown(link: &GameLink) -> bool {
    LinkKind::of_host(&link.host).is_some()
}

/// The links of a game, from all its `copies` (the card's own first): the
/// stores, then the sites about it, then its communities. One per kind
/// (the first found), but up to [`MAX_WEBSITES`] sites without an icon.
/// `linux`: ProtonDB is only offered where Proton runs games.
pub fn links(copies: &[&CollectionGame], linux: bool) -> Vec<GameLink> {
    let mut found: Vec<String> = Vec::new();
    let mut steam_id: Option<String> = None;
    let mut igdb_slug: Option<String> = None;
    for g in copies {
        if let Some(url) = g.game.rest.get("store_url").and_then(|v| v.as_str()) {
            found.push(url.trim().to_owned());
        }
        let id = g.steam_app_id.as_deref().or(g.metadata.as_ref().and_then(|m| m.steam_app_id.as_deref()));
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
            steam_id.get_or_insert_with(|| id.to_owned());
        }
        if let Some(m) = &g.metadata {
            found.extend(m.websites.iter().map(|w| w.trim().to_owned()));
            if let Some(slug) = m.igdb_slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                igdb_slug.get_or_insert_with(|| slug.to_owned());
            }
        }
    }
    if let Some(id) = &steam_id {
        found.insert(0, format!("https://store.steampowered.com/app/{id}"));
        found.push(format!("https://steamdb.info/app/{id}/"));
        found.push(format!("https://www.pcgamingwiki.com/api/appid.php?appid={id}"));
        if linux {
            found.push(format!("https://www.protondb.com/app/{id}"));
        }
    }
    if let Some(slug) = igdb_slug {
        found.push(format!("https://www.igdb.com/games/{slug}"));
    }
    let mut out: Vec<GameLink> = Vec::new();
    for url in found {
        let Some(host) = host_of(&url) else { continue };
        let Some(kind) = LinkKind::of_host(&host) else { continue };
        let taken = match kind {
            LinkKind::Website => out.iter().filter(|l| l.kind == kind).count() >= MAX_WEBSITES || out.iter().any(|l| l.host == host),
            _ => out.iter().any(|l| l.kind == kind),
        };
        if !taken {
            out.push(GameLink { kind, url, host });
        }
    }
    out.sort_by_key(|l| l.kind);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game_with;
    use serde_json::json;

    #[test]
    fn hosts_are_read_from_web_addresses_only() {
        assert_eq!(host_of("https://www.GOG.com/en/game/x").as_deref(), Some("gog.com"));
        assert_eq!(host_of("http://user@store.steampowered.com:443/app/1").as_deref(), Some("store.steampowered.com"));
        assert_eq!(host_of("file:///etc/passwd"), None);
        assert_eq!(host_of("https://localhost/"), None);
    }

    #[test]
    fn typed_addresses_are_completed() {
        let l = classify(" steamdb.info/app/620 ").unwrap();
        assert_eq!((l.kind, l.url.as_str()), (LinkKind::SteamDb, "https://steamdb.info/app/620"));
        assert_eq!(classify("http://Example.org/x").unwrap().host, "example.org");
        let social = classify("https://facebook.com/x").unwrap();
        assert_eq!(social.kind, LinkKind::Website);
        assert!(!is_shown(&social));
        assert!(is_shown(&l));
        assert!(classify("ftp://x.org").is_none());
        assert!(classify("not a link").is_none());
        assert!(classify("localhost").is_none());
    }

    #[test]
    fn known_sites_get_their_kind() {
        assert_eq!(LinkKind::of_host("store.epicgames.com"), Some(LinkKind::Epic));
        assert_eq!(LinkKind::of_host("hades.fandom.com"), Some(LinkKind::Fandom));
        assert_eq!(LinkKind::of_host("en.wikipedia.org"), Some(LinkKind::Wikipedia));
        assert_eq!(LinkKind::of_host("supergiantgames.com"), Some(LinkKind::Website));
        assert_eq!(LinkKind::of_host("notepic.com"), Some(LinkKind::Website), "a domain ending alike is not the store");
        assert_eq!(LinkKind::of_host("facebook.com"), None);
    }

    #[test]
    fn a_steam_game_gets_its_store_and_community_sites() {
        let g = game_with("hades", "sideload", |info, _, md, _, _| {
            info.rest.insert("store_url".into(), json!("https://store.epicgames.com/p/hades"));
            md.steam_app_id = Some("1145360".into());
            md.igdb_slug = Some("hades--1".into());
            md.websites = vec![
                "https://www.supergiantgames.com/games/hades".into(),
                "https://store.steampowered.com/app/1145360/Hades/".into(),
                "https://twitter.com/SupergiantGames".into(),
                "https://www.facebook.com/SupergiantGames".into(),
                "https://en.wikipedia.org/wiki/Hades_(video_game)".into(),
                "https://discord.gg/supergiant".into(),
            ];
        });
        let l = links(&[&g], true);
        let kinds: Vec<LinkKind> = l.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                LinkKind::Steam,
                LinkKind::Epic,
                LinkKind::Website,
                LinkKind::PcGamingWiki,
                LinkKind::SteamDb,
                LinkKind::ProtonDb,
                LinkKind::Igdb,
                LinkKind::Wikipedia,
                LinkKind::Discord,
                LinkKind::X
            ]
        );
        assert_eq!(l[0].url, "https://store.steampowered.com/app/1145360");
        assert_eq!(l[2].host, "supergiantgames.com");
        assert!(!links(&[&g], false).iter().any(|l| l.kind == LinkKind::ProtonDb));
    }

    #[test]
    fn copies_add_their_stores_once() {
        let a = game_with("a", "gog", |info, _, _, _, _| {
            info.rest.insert("store_url".into(), json!("https://www.gog.com/game/a"));
        });
        let b = game_with("b", "legendary", |info, _, md, _, _| {
            info.rest.insert("store_url".into(), json!("https://store.epicgames.com/p/a"));
            md.websites = vec!["https://www.gog.com/en/game/a".into(), "https://a.example".into(), "https://a.example/press".into()];
        });
        let l = links(&[&a, &b], false);
        let kinds: Vec<LinkKind> = l.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [LinkKind::Epic, LinkKind::Gog, LinkKind::Website]);
        assert_eq!(l[1].url, "https://www.gog.com/game/a", "the copy's own store page first");
        assert!(links(&[], true).is_empty());
    }
}
