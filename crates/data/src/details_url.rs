//! Where the focus panel's "open details" button goes. Heronite opens
//! Heroic's game page (`/gamepage/<runner>/<appName>`), which this app does
//! not have; the closest thing is the game's store page.

use crate::library::CollectionGame;

fn is_web(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The store/details page of `game`, in order of preference:
/// 1. `GameInfo.store_url` (Epic and GOG entries carry it);
/// 2. the Steam store page of its Steam app id;
/// 3. the first web site in its Collection metadata.
pub fn details_url(game: &CollectionGame) -> Option<String> {
    let store = game.game.rest.get("store_url").and_then(|v| v.as_str()).map(str::trim).filter(|u| is_web(u));
    if let Some(url) = store {
        return Some(url.to_string());
    }
    if let Some(id) = game.steam_app_id.as_deref().filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        return Some(format!("https://store.steampowered.com/app/{id}"));
    }
    game.metadata.as_ref().and_then(|m| m.websites.iter().map(|w| w.trim()).find(|w| is_web(w))).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game_with;
    use serde_json::json;

    #[test]
    fn prefers_store_url_then_steam_then_websites() {
        let none = game_with("a", "sideload", |_, _, _, _, _| {});
        assert_eq!(details_url(&none), None);

        let site = game_with("a", "sideload", |_, _, md, _, _| {
            md.websites = vec!["not a url".into(), "https://example.com/game".into()];
        });
        assert_eq!(details_url(&site).as_deref(), Some("https://example.com/game"));

        let steam = game_with("a", "sideload", |_, _, md, meta, _| {
            md.websites = vec!["https://example.com/game".into()];
            meta.steam_app_id = Some("620".into());
        });
        assert_eq!(details_url(&steam).as_deref(), Some("https://store.steampowered.com/app/620"));

        let epic = game_with("a", "legendary", |info, _, _, meta, _| {
            info.rest.insert("store_url".into(), json!("https://www.epicgames.com/store/product/control"));
            meta.steam_app_id = Some("620".into());
        });
        assert_eq!(details_url(&epic).as_deref(), Some("https://www.epicgames.com/store/product/control"));
    }

    #[test]
    fn rejects_non_web_and_non_numeric_values() {
        let g = game_with("a", "sideload", |info, _, md, meta, _| {
            info.rest.insert("store_url".into(), json!("file:///etc/passwd"));
            meta.steam_app_id = Some("../x".into());
            md.websites = vec!["javascript:alert(1)".into()];
        });
        assert_eq!(details_url(&g), None);
    }
}
