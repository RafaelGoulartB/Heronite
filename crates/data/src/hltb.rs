//! Read-only lookup of the cached wiki info (HowLongToBeat, PCGamingWiki) that
//! Heronite keeps in `store_cache/wikigameinfo.json`.
//!
//! Heronite's `getWikiGameInfo(title)` caches under `removeSpecialcharacters(title)`
//! where `title` is the Collection title (`collectionGameTitle`); when that key
//! is missing it also tries the store title. Entries older than 30 days are
//! refetched by Heronite; here they are used as they are (no network).

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::json_store;
use crate::paths::DataPaths;
use crate::text::remove_special_characters;

/// `HeroicHowLongToBeatEntry`, hours.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HltbEntry {
    #[serde(default)]
    pub main_story: f64,
    #[serde(default)]
    pub main_extra: f64,
    #[serde(default)]
    pub completionist: f64,
    #[serde(default)]
    pub game_id: Option<u64>,
    #[serde(default)]
    pub game_name: Option<String>,
    #[serde(default)]
    pub game_image_url: Option<String>,
    #[serde(default)]
    pub game_web_link: Option<String>,
}

impl HltbEntry {
    /// The Focus shows the card only when this holds.
    pub fn is_displayable(&self) -> bool {
        self.main_story > 0.0 || self.main_extra > 0.0
    }

    /// Position (0-100) of the "you are here" marker on the bar.
    pub fn marker_pct(&self, played_minutes: f64) -> Option<f64> {
        let max = self.completionist.max(self.main_extra).max(self.main_story);
        let played_hours = played_minutes / 60.0;
        if max > 0.0 && played_hours > 0.0 {
            Some((played_hours / max * 100.0).min(100.0))
        } else {
            None
        }
    }
}

/// `formatHours` of `CollectionHowLongToBeat.tsx` (`—` for zero).
pub fn format_hours(value: f64) -> String {
    if value <= 0.0 || value.is_nan() {
        return "—".into();
    }
    let total_minutes = (value * 60.0).round() as u64;
    let (hours, mins) = (total_minutes / 60, total_minutes % 60);
    match (hours, mins) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// The parts of a cached `WikiInfo` the Collection reads.
#[derive(Debug, Clone, Default)]
pub struct WikiEntry {
    pub hltb: Option<HltbEntry>,
    /// `pcgamingwiki.genres`.
    pub pcgw_genres: Vec<String>,
    /// `pcgamingwiki.releaseDate[0]` without its `Platform:` prefix.
    pub pcgw_release_date: Option<String>,
}

impl WikiEntry {
    fn from_value(v: &Value) -> Self {
        let hltb = v
            .get("howlongtobeat")
            .filter(|h| h.is_object())
            .and_then(|h| serde_json::from_value(h.clone()).ok());
        let pcgw = v.get("pcgamingwiki").filter(|p| p.is_object());
        let pcgw_genres = pcgw
            .and_then(|p| p.get("genres"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let pcgw_release_date = pcgw
            .and_then(|p| p.get("releaseDate"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(strip_platform_prefix);
        Self { hltb, pcgw_genres, pcgw_release_date }
    }
}

/// `.replace(/^[^:]+:\s*/, '')`
fn strip_platform_prefix(value: &str) -> String {
    match value.split_once(':') {
        Some((head, tail)) if !head.is_empty() => tail.trim_start().to_owned(),
        _ => value.to_owned(),
    }
}

/// `store_cache/wikigameinfo.json`, loaded once (about 30 KB).
#[derive(Debug, Clone, Default)]
pub struct WikiCache {
    entries: Map<String, Value>,
}

impl WikiCache {
    pub fn load(paths: &DataPaths) -> Self {
        let entries = match json_store::read_value(&paths.wiki_game_info()) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        Self { entries }
    }

    fn get(&self, title: &str) -> Option<&Value> {
        if title.is_empty() || title == "__timestamp" {
            return None;
        }
        let key = remove_special_characters(title);
        if let Some(v) = self.entries.get(&key).filter(|v| v.is_object()) {
            return Some(v);
        }
        // electron-store (dot-prop) stores `a.b` as nested objects.
        if key.contains('.') {
            let mut parts = key.split('.');
            let mut cur = self.entries.get(parts.next()?)?;
            for part in parts {
                cur = cur.get(part)?;
            }
            return Some(cur).filter(|v| v.is_object());
        }
        None
    }

    /// Cached wiki info for `title` (the Collection title); `store_title` is
    /// the second key Heroic tries (the game's own title) when different.
    pub fn lookup(&self, title: &str, store_title: &str) -> Option<WikiEntry> {
        let entry = self.get(title).or_else(|| {
            let fallback = remove_special_characters(store_title);
            if !fallback.is_empty() && fallback != remove_special_characters(title) {
                self.get(store_title)
            } else {
                None
            }
        })?;
        Some(WikiEntry::from_value(entry))
    }

    /// Just the HowLongToBeat part, shown only when it has times.
    pub fn hltb(&self, title: &str, store_title: &str) -> Option<HltbEntry> {
        self.lookup(title, store_title)?.hltb.filter(HltbEntry::is_displayable)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dotted_titles_are_nested_like_electron_store() {
        let v = serde_json::json!({"Ori and the Blind Forest": {}, "S": {"T": {"R": {"A": {"howlongtobeat": {"mainStory": 3.5}}}}}});
        let cache = super::WikiCache { entries: v.as_object().unwrap().clone() };
        assert!(cache.lookup("S.T.R.A", "").and_then(|e| e.hltb).is_some());
    }

    use super::*;
    use serde_json::json;

    fn cache() -> WikiCache {
        let entries = json!({
            "Onimusha Way of the Sword": {
                "pcgamingwiki": {"genres": ["Action"], "releaseDate": ["Windows: Apr 10, 2024"]},
                "howlongtobeat": {"mainStory": 20.56138888888889, "mainExtra": 29.620277777777776,
                    "completionist": 47.873333333333335, "gameId": 160598,
                    "gameWebLink": "https://howlongtobeat.com/game/160598"}
            },
            "Empty Game": {"howlongtobeat": null, "pcgamingwiki": null},
            "__timestamp": {"Empty Game": "x"}
        });
        WikiCache { entries: entries.as_object().unwrap().clone() }
    }

    #[test]
    fn key_is_title_without_special_characters() {
        let c = cache();
        let hit = c.hltb("Onimusha: Way of the Sword", "").unwrap();
        assert_eq!(hit.game_id, Some(160598));
        assert_eq!(format_hours(hit.main_story), "20h 34m");
        assert_eq!(format_hours(hit.main_extra), "29h 37m");
        assert_eq!(format_hours(0.0), "—");
        assert_eq!(format_hours(0.5), "30m");
        assert_eq!(format_hours(3.0), "3h");
        let e = c.lookup("Onimusha: Way of the Sword", "").unwrap();
        assert_eq!(e.pcgw_genres, ["Action"]);
        assert_eq!(e.pcgw_release_date.as_deref(), Some("Apr 10, 2024"));
    }

    #[test]
    fn falls_back_to_store_title_only_when_title_is_missing() {
        let c = cache();
        assert!(c.hltb("Custom Name", "Onimusha: Way of the Sword").is_some());
        // Present but without HLTB: no fallback, like `??` in the TS.
        assert!(c.hltb("Empty Game", "Onimusha: Way of the Sword").is_none());
        assert!(c.lookup("Nope", "Nope too").is_none());
        assert!(c.lookup("__timestamp", "").is_none());
    }

    #[test]
    fn marker_position() {
        let e = HltbEntry { main_story: 10.0, main_extra: 20.0, completionist: 40.0, ..Default::default() };
        assert_eq!(e.marker_pct(0.0), None);
        assert_eq!(e.marker_pct(1200.0), Some(50.0));
        assert_eq!(e.marker_pct(100_000.0), Some(100.0));
        assert!(e.is_displayable());
        assert!(!HltbEntry::default().is_displayable());
    }

    #[test]
    fn real_cache_loads() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE") else { return };
        let c = WikiCache::load(&DataPaths::at(root));
        eprintln!("wiki entries: {}", c.len());
        assert!(c.hltb("Control", "Control").is_some() || !c.is_empty());
    }
}
