//! Where console mode finds a game's logo (themes can show it instead of the
//! title). Pure: it lists the places to try, best first, and touches no file;
//! the image loader skips what is missing.

use std::path::{Path, PathBuf};

use crate::art::steam_logo_urls;
use crate::library::CollectionGame;
use crate::paths::{percent_decode, DataPaths};

/// One place a logo can come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogoSource {
    Url(String),
    File(PathBuf),
    /// Steam's local library cache of one app: `logo.png` sits in this
    /// folder or in a hashed sub-folder of it.
    SteamCache(PathBuf),
}

/// Steam installs that keep `appcache/librarycache` (native, its `~/.steam`
/// link, Flatpak and Snap), relative to the home folder.
const STEAM_ROOTS: [&str; 4] =
    [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam", "snap/steam/common/.local/share/Steam"];

/// The logo the user set (`logoUrl` of the game's `art.json`: picked in
/// Game settings → Images, or written by hand).
pub fn custom_logo(game: &CollectionGame) -> Option<&str> {
    game.art.as_deref()?.logo_url.as_deref().filter(|v| !v.is_empty())
}

/// The logo the metadata sources found (`logoUrl` of the record).
fn fetched_logo(game: &CollectionGame) -> Option<&str> {
    game.metadata.as_deref()?.logo_url.as_deref().filter(|v| !v.is_empty())
}

/// The store's own logo (`art_logo`), when it is a real image reference.
pub fn store_logo(game: &CollectionGame) -> Option<&str> {
    game.game.art_logo.as_deref().filter(|v| !v.is_empty() && *v != "fallback" && !v.starts_with("localart:"))
}

/// The library cache folders of a Steam app under `home`.
pub fn steam_cache_dirs<'a>(home: &'a Path, steam_app_id: &'a str) -> impl Iterator<Item = PathBuf> + 'a {
    STEAM_ROOTS.iter().map(move |root| home.join(root).join("appcache/librarycache").join(steam_app_id))
}

/// A local reference (`file:`, `localart:` or an absolute path) or a URL.
fn source_of(paths: &DataPaths, screenshots: Option<&Path>, value: &str) -> Option<LogoSource> {
    if value.starts_with("localart:") {
        paths.resolve_localart(value, screenshots).map(LogoSource::File)
    } else if let Some(rest) = value.strip_prefix("file://").or_else(|| value.strip_prefix("file:")) {
        Some(LogoSource::File(PathBuf::from(percent_decode(rest))))
    } else if value.starts_with('/') || Path::new(value).is_absolute() {
        Some(LogoSource::File(PathBuf::from(value)))
    } else {
        Some(LogoSource::Url(value.to_owned()))
    }
}

/// Every place to look for `game`'s logo, best first: the logo the user set,
/// then [`default_logo_sources`]. Empty when the game has none of them
/// (callers show the title).
pub fn logo_sources(paths: &DataPaths, screenshots: Option<&Path>, home: Option<&Path>, game: &CollectionGame) -> Vec<LogoSource> {
    let mut out: Vec<LogoSource> = custom_logo(game).and_then(|v| source_of(paths, screenshots, v)).into_iter().collect();
    for s in default_logo_sources(paths, screenshots, home, game) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Where `game`'s logo comes from when the user set none, best first: the
/// official ones (the store's, then Steam's local cache and its CDN), then
/// the one the metadata sources found. Steam's are asked for every game
/// with a Steam app id, its own or the one its metadata matched, so a game
/// of another store that is also on Steam gets Steam's logo; the image
/// loader skips the ones that do not exist.
pub fn default_logo_sources(paths: &DataPaths, screenshots: Option<&Path>, home: Option<&Path>, game: &CollectionGame) -> Vec<LogoSource> {
    let mut out: Vec<LogoSource> = Vec::new();
    let mut push = |s: LogoSource| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    if let Some(s) = store_logo(game).and_then(|v| source_of(paths, screenshots, v)) {
        push(s);
    }
    if let Some(id) = game.tag_steam_id() {
        if let Some(home) = home {
            steam_cache_dirs(home, id).for_each(|d| push(LogoSource::SteamCache(d)));
        }
        steam_logo_urls(id).into_iter().for_each(|u| push(LogoSource::Url(u)));
    }
    if let Some(s) = fetched_logo(game).and_then(|v| source_of(paths, screenshots, v)) {
        push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game_with;
    use std::sync::Arc;

    const HOME: &str = "/home/u";

    /// A game with the given custom logo, store logo and Steam app.
    fn game(custom: Option<&str>, store: Option<&str>, steam: Option<&str>) -> Arc<CollectionGame> {
        game_with("Abc", "epic", |info, art, _, meta, _| {
            art.logo_url = custom.map(String::from);
            info.art_logo = store.map(String::from);
            meta.steam_app_id = steam.map(String::from);
        })
    }

    fn sources(g: &CollectionGame, home: Option<&str>) -> Vec<LogoSource> {
        logo_sources(&DataPaths::at("/data"), None, home.map(Path::new), g)
    }

    #[test]
    fn a_game_without_any_logo_has_no_sources() {
        assert!(sources(&game(None, None, None), Some(HOME)).is_empty());
        assert!(sources(&game(Some(""), Some(""), Some("")), Some(HOME)).is_empty());
    }

    #[test]
    fn order_is_custom_then_store_then_steam_cache_then_cdn() {
        let g = game(Some("https://x/mine.png"), Some("https://x/store.png"), Some("570"));
        let s = sources(&g, Some(HOME));
        assert_eq!(s[0], LogoSource::Url("https://x/mine.png".into()));
        assert_eq!(s[1], LogoSource::Url("https://x/store.png".into()));
        assert_eq!(s[2], LogoSource::SteamCache(PathBuf::from("/home/u/.local/share/Steam/appcache/librarycache/570")));
        assert_eq!(s.iter().filter(|s| matches!(s, LogoSource::SteamCache(_))).count(), 4);
        assert!(matches!(&s[s.len() - 2], LogoSource::Url(u) if u.starts_with("https://shared.cloudflare.steamstatic.com/")));
        assert!(matches!(&s[s.len() - 1], LogoSource::Url(u) if u.ends_with("/570/logo.png")));
    }

    #[test]
    fn local_references_and_placeholders() {
        let paths = DataPaths::at("/data");
        let g = game(Some("file:///tmp/my%20logo.png"), Some("fallback"), None);
        assert_eq!(sources(&g, None), vec![LogoSource::File(PathBuf::from("/tmp/my logo.png"))]);

        let g = game(Some("localart://art/x.png"), Some("localart://art/y.png"), None);
        assert_eq!(sources(&g, None), vec![LogoSource::File(paths.art_dir().join("x.png"))]);

        let g = game(Some("/srv/logo.png"), None, None);
        assert_eq!(sources(&g, None), vec![LogoSource::File(PathBuf::from("/srv/logo.png"))]);
    }

    #[test]
    fn duplicates_collapse_and_no_home_leaves_the_cdn() {
        let cdn = steam_logo_urls("9");
        let g = game(Some(&cdn[1]), None, Some("9"));
        let s = sources(&g, None);
        assert_eq!(s, vec![LogoSource::Url(cdn[1].clone()), LogoSource::Url(cdn[0].clone())]);
    }

    #[test]
    fn a_game_of_another_store_gets_the_logo_of_the_steam_app_its_metadata_matched() {
        let g = game_with("Abc", "gog", |_, _, md, _, _| md.steam_app_id = Some("220".into()));
        let s = sources(&g, None);
        assert_eq!(s, steam_logo_urls("220").map(LogoSource::Url).to_vec());
    }

    #[test]
    fn the_fetched_logo_comes_after_the_official_ones() {
        let g = game_with("Abc", "epic", |info, _, md, meta, _| {
            md.logo_url = Some("https://sgdb/logo.png".into());
            info.art_logo = Some("https://x/store.png".into());
            meta.steam_app_id = Some("570".into());
        });
        let s = sources(&g, None);
        assert_eq!(s.first(), Some(&LogoSource::Url("https://x/store.png".into())));
        assert_eq!(s.last(), Some(&LogoSource::Url("https://sgdb/logo.png".into())));

        let only_fetched = game_with("Abc", "sideload", |_, _, md, _, _| md.logo_url = Some("localart://metadata-art/a/logo.png".into()));
        assert_eq!(sources(&only_fetched, None), vec![LogoSource::File(DataPaths::at("/data").metadata_art_dir().join("a/logo.png"))]);
    }

    #[test]
    fn the_defaults_leave_out_the_logo_the_user_set() {
        let g = game(Some("https://x/mine.png"), Some("https://x/store.png"), None);
        let defaults = default_logo_sources(&DataPaths::at("/data"), None, None, &g);
        assert_eq!(defaults, vec![LogoSource::Url("https://x/store.png".into())]);
    }
}
