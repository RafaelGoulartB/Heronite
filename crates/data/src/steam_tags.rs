//! Steam store tags: the tags Steam players voted for on a game's store page
//! ("popular user-defined tags"), cached by `heronite-metadata` in
//! `store_cache/steam_tags.json` and read here into the Tag facet.
//!
//! The file keeps tag ids with their weights per app, and the id -> name list
//! per Steam language, so the names follow the interface language without
//! asking Steam again. A game gets the heaviest tags only: tags far below its
//! top one are noise, and the count is capped by the user's setting.

use std::collections::{BTreeMap, HashMap};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::collection::{items, FacetKind};
use crate::json_store;
use crate::library::CollectionGame;
use crate::models::Extra;
use crate::paths::DataPaths;

/// Default and bounds of "Steam tags per game".
pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 20;
/// A tag under this share of the game's heaviest tag is left out.
const MIN_SHARE: f64 = 0.25;

/// One tag of an app with Steam's weight (higher = more votes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagWeight {
    pub tagid: u32,
    pub weight: u32,
}

/// The tags of one app as fetched.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTags {
    /// Heaviest first. Empty when Steam knows the app but has no tags, or
    /// does not list it (delisted, not a store app).
    #[serde(default)]
    pub tags: Vec<TagWeight>,
    /// Milliseconds since the epoch.
    #[serde(default)]
    pub fetched_at: i64,
}

/// Steam's tag id -> name list in one language.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagNames {
    #[serde(default)]
    pub version_hash: String,
    #[serde(default)]
    pub fetched_at: i64,
    #[serde(default)]
    pub names: BTreeMap<u32, String>,
}

/// `store_cache/steam_tags.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamTagsFile {
    /// Steam app id -> its tags.
    #[serde(default)]
    pub apps: IndexMap<String, AppTags>,
    /// Steam language name (`english`, `brazilian`) -> tag names.
    #[serde(default)]
    pub names: IndexMap<String, TagNames>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl SteamTagsFile {
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.steam_tags()).unwrap_or_else(|err| {
            log::warn!("Steam tags unreadable, ignored: {err:#}");
            Self::default()
        })
    }
}

/// The tags to show, per Steam app id, already named.
#[derive(Debug, Clone, Default)]
pub struct SteamTags {
    by_app: HashMap<String, Vec<String>>,
}

impl SteamTags {
    /// The cached tags named in `steam_language` (English for ids that
    /// language lacks), at most `limit` per game. Nothing when `enabled` is
    /// off.
    pub fn load(paths: &DataPaths, enabled: bool, limit: u32, steam_language: &str) -> Self {
        if !enabled {
            return Self::default();
        }
        Self::from_file(&SteamTagsFile::load(paths), limit, steam_language)
    }

    pub fn from_file(file: &SteamTagsFile, limit: u32, steam_language: &str) -> Self {
        let local = file.names.get(steam_language).map(|n| &n.names);
        let english = file.names.get("english").map(|n| &n.names);
        let name = |id: u32| local.and_then(|n| n.get(&id)).or_else(|| english.and_then(|n| n.get(&id))).cloned();
        let by_app = file
            .apps
            .iter()
            .filter(|(_, app)| !app.tags.is_empty())
            .map(|(id, app)| (id.clone(), shown(&app.tags, limit).into_iter().filter_map(|t| name(t.tagid)).collect::<Vec<_>>()))
            .filter(|(_, names)| !names.is_empty())
            .collect();
        Self { by_app }
    }

    pub fn is_empty(&self) -> bool {
        self.by_app.is_empty()
    }

    /// The tags of the game with this Steam app id.
    pub fn of(&self, steam_app_id: &str) -> &[String] {
        self.by_app.get(steam_app_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Fills the game's Tag facet.
    pub fn apply(&self, game: &mut CollectionGame) {
        if self.by_app.is_empty() {
            return;
        }
        let Some(id) = game.tag_steam_id() else { return };
        let names = self.of(id).to_vec();
        game.facets[FacetKind::Tag.index()] = items(names);
    }
}

/// The tags a game shows: heaviest first, none far below the top one, at most
/// `limit` (clamped to 1..=[`MAX_LIMIT`]).
pub fn shown(tags: &[TagWeight], limit: u32) -> Vec<TagWeight> {
    let mut sorted = tags.to_vec();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.weight));
    let top = sorted.first().map(|t| t.weight).unwrap_or(0) as f64;
    let limit = limit.clamp(1, MAX_LIMIT) as usize;
    sorted.into_iter().filter(|t| t.weight as f64 >= top * MIN_SHARE).take(limit).collect()
}

/// The `limit` setting as stored (0 or missing = the default).
pub fn normalize_limit(value: u32) -> u32 {
    if value == 0 {
        DEFAULT_LIMIT
    } else {
        value.min(MAX_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tw(tagid: u32, weight: u32) -> TagWeight {
        TagWeight { tagid, weight }
    }

    fn file() -> SteamTagsFile {
        let mut f = SteamTagsFile::default();
        f.apps.insert("620".into(), AppTags { tags: vec![tw(1664, 586), tw(4182, 604), tw(1625, 588), tw(9, 100)], fetched_at: 1 });
        f.apps.insert("10".into(), AppTags { tags: vec![], fetched_at: 1 });
        let names = |pairs: &[(u32, &str)]| TagNames {
            version_hash: "1".into(),
            fetched_at: 1,
            names: pairs.iter().map(|(i, n)| (*i, n.to_string())).collect(),
        };
        f.names.insert("english".into(), names(&[(4182, "Singleplayer"), (1625, "Platformer"), (1664, "Puzzle"), (9, "Strategy")]));
        f.names.insert("brazilian".into(), names(&[(4182, "Um Jogador"), (1664, "Quebra-Cabeça")]));
        f
    }

    #[test]
    fn heaviest_first_without_the_far_lighter_ones_and_capped() {
        let tags = [tw(1, 100), tw(2, 1000), tw(3, 250), tw(4, 249), tw(5, 600)];
        let ids = |l| shown(&tags, l).iter().map(|t| t.tagid).collect::<Vec<_>>();
        assert_eq!(ids(10), [2, 5, 3], "under a quarter of the top tag is noise");
        assert_eq!(ids(2), [2, 5]);
        assert_eq!(ids(0), [2], "at least one");
        assert!(shown(&[], 10).is_empty());
    }

    #[test]
    fn names_follow_the_language_and_fall_back_to_english() {
        let f = file();
        assert_eq!(SteamTags::from_file(&f, 10, "english").of("620"), ["Singleplayer", "Platformer", "Puzzle"]);
        assert_eq!(SteamTags::from_file(&f, 10, "brazilian").of("620"), ["Um Jogador", "Platformer", "Quebra-Cabeça"]);
        assert_eq!(SteamTags::from_file(&f, 2, "german").of("620"), ["Singleplayer", "Platformer"]);
        assert!(SteamTags::from_file(&f, 10, "english").of("10").is_empty());
        assert!(SteamTags::from_file(&f, 10, "english").of("404").is_empty());
    }

    #[test]
    fn a_tag_without_a_name_is_left_out() {
        let mut f = file();
        f.names.clear();
        assert!(SteamTags::from_file(&f, 10, "english").is_empty());
    }

    #[test]
    fn limit_setting_defaults_and_caps() {
        assert_eq!(normalize_limit(0), DEFAULT_LIMIT);
        assert_eq!(normalize_limit(5), 5);
        assert_eq!(normalize_limit(99), MAX_LIMIT);
    }

    #[test]
    fn file_round_trips_and_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let mut f = file();
        f.extra.insert("future".into(), serde_json::json!(1));
        json_store::write(&paths.steam_tags(), &f).unwrap();
        let back = SteamTagsFile::load(&paths);
        assert_eq!(back.apps["620"].tags.len(), 4);
        assert_eq!(back.names["brazilian"].names[&4182], "Um Jogador");
        assert_eq!(back.extra["future"], 1);
        assert!(SteamTags::load(&paths, false, 10, "english").is_empty(), "off: nothing");
        assert_eq!(SteamTags::load(&paths, true, 10, "english").of("620").len(), 3);
    }
}
