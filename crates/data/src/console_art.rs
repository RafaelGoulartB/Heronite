//! Where console mode finds a game's logo (themes can show it instead of the
//! title). Pure: it lists the places to try, best first, and touches no file;
//! the image loader skips what is missing.

use std::path::{Path, PathBuf};

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

/// The logo URL set by hand in the game's `art.json` (`logoUrl`, like
/// `coverUrl`) or by its metadata.
pub fn custom_logo(game: &CollectionGame) -> Option<&str> {
    fn from(extra: Option<&crate::models::Extra>) -> Option<&str> {
        extra?.get("logoUrl")?.as_str().filter(|v| !v.is_empty())
    }
    from(game.art.as_deref().map(|a| &a.extra)).or_else(|| from(game.metadata.as_deref().map(|m| &m.extra)))
}

/// The store's own logo (`art_logo`), when it is a real image reference.
pub fn store_logo(game: &CollectionGame) -> Option<&str> {
    game.game.art_logo.as_deref().filter(|v| !v.is_empty() && *v != "fallback" && !v.starts_with("localart:"))
}

/// Steam's CDN logos of an app, newest scheme first.
pub fn steam_logo_urls(steam_app_id: &str) -> [String; 2] {
    [
        format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{steam_app_id}/logo.png"),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{steam_app_id}/logo.png"),
    ]
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
    } else if value.starts_with('/') {
        Some(LogoSource::File(PathBuf::from(value)))
    } else {
        Some(LogoSource::Url(value.to_owned()))
    }
}

/// Every place to look for `game`'s logo, best first: the logo the user set,
/// the store's, Steam's local cache and the Steam CDN. Empty when the game
/// has none of them (callers show the title).
pub fn logo_sources(paths: &DataPaths, screenshots: Option<&Path>, home: Option<&Path>, game: &CollectionGame) -> Vec<LogoSource> {
    let mut out: Vec<LogoSource> = Vec::new();
    let mut push = |s: LogoSource| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for value in [custom_logo(game), store_logo(game)].into_iter().flatten() {
        if let Some(s) = source_of(paths, screenshots, value) {
            push(s);
        }
    }
    if let Some(id) = game.steam_app_id.as_deref().filter(|id| !id.is_empty()) {
        if let Some(home) = home {
            steam_cache_dirs(home, id).for_each(|d| push(LogoSource::SteamCache(d)));
        }
        steam_logo_urls(id).into_iter().for_each(|u| push(LogoSource::Url(u)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game_with;
    use serde_json::json;
    use std::sync::Arc;

    const HOME: &str = "/home/u";

    /// A game with the given custom logo, store logo and Steam app.
    fn game(custom: Option<&str>, store: Option<&str>, steam: Option<&str>) -> Arc<CollectionGame> {
        game_with("Abc", "epic", |info, art, _, meta, _| {
            if let Some(c) = custom {
                art.extra.insert("logoUrl".into(), json!(c));
            }
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
}
