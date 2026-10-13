//! The Steam library as the Steam client last showed it
//! (`store_cache/steam_library.json`, written by `heronite_steam::library`),
//! and how its games join the Collection's other entries.
//!
//! A game owned on Steam that the Collection already has from another source
//! (an Epic, GOG or Amazon game, a local game) is that entry's Steam *twin*:
//! no Steam entry is added for it, and the entry gets Steam's install (and,
//! when only the Steam copy is installed, plays through Steam). Twins are
//! found by Steam app id (the entry's own or its metadata's), else by a title
//! that only one owned game and one entry share.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

/// Set (`true`) in the overlay meta of the Steam entries this app adds by
/// itself (library import, games installed outside the Collection). Such an
/// entry, while untouched, gives way to a twin from another source.
pub const AUTO_KEY: &str = "heroniteSteamAuto";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamOwnedFile {
    /// Only games (no tools, soundtracks, demos or games hidden in Steam).
    #[serde(default)]
    pub games: Vec<SteamOwnedGame>,
    /// When the client was last read (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamOwnedGame {
    pub app_id: String,
    pub name: String,
    /// Kept current from the `appmanifest_*.acf` files between reads.
    #[serde(default)]
    pub installed: bool,
    /// Steam's total for the account.
    #[serde(default)]
    pub playtime_minutes: u64,
    /// Unix seconds; `None` = never.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_played: Option<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl SteamOwnedFile {
    pub fn read(paths: &DataPaths) -> Self {
        json_store::read(&paths.steam_owned()).unwrap_or_else(|e| {
            log::warn!("{}: {e:#}", paths.steam_owned().display());
            Self::default()
        })
    }

    /// Whether the client was ever read (until then nothing counts as owned).
    pub fn known(&self) -> bool {
        self.updated_at.is_some()
    }

    pub fn get(&self, app_id: &str) -> Option<&SteamOwnedGame> {
        self.games.iter().find(|g| g.app_id == app_id)
    }
}

/// The title as twins are matched: lower case, letters and digits only,
/// without trademark signs (`DOOM™ Eternal` = `Doom Eternal`).
pub fn title_key(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// One Collection entry, as the twin search sees it.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub key: &'a str,
    pub title: &'a str,
    /// Steam app ids the entry already names (its meta, its metadata).
    pub steam_ids: Vec<&'a str>,
    /// The Steam app id of a Steam entry (`steam-uri`); those are not twins.
    pub steam_entry: Option<&'a str>,
    /// Emulated and browser games never join Steam games.
    pub joinable: bool,
}

/// Steam app id → key of its twin entry. An id goes to the first entry that
/// names it; a title match only counts when the title is unique on both
/// sides and no entry names that id.
pub fn twins(candidates: &[Candidate<'_>], owned: &SteamOwnedFile) -> HashMap<String, String> {
    let owned_ids: HashSet<&str> = owned.games.iter().map(|g| g.app_id.as_str()).collect();
    let mut out: HashMap<String, String> = HashMap::new();
    let mut linked_keys: HashSet<&str> = HashSet::new();
    for c in candidates.iter().filter(|c| c.joinable && c.steam_entry.is_none()) {
        if let Some(id) = c.steam_ids.iter().find(|id| owned_ids.contains(**id) && !out.contains_key(**id)) {
            out.insert(id.to_string(), c.key.to_string());
            linked_keys.insert(c.key);
        }
    }
    let mut owned_by_title: HashMap<String, Option<&str>> = HashMap::new();
    for g in &owned.games {
        owned_by_title.entry(title_key(&g.name)).and_modify(|v| *v = None).or_insert(Some(g.app_id.as_str()));
    }
    let mut entries_by_title: HashMap<String, Option<&Candidate<'_>>> = HashMap::new();
    for c in candidates.iter().filter(|c| c.joinable && c.steam_entry.is_none()) {
        entries_by_title.entry(title_key(c.title)).and_modify(|v| *v = None).or_insert(Some(c));
    }
    for (title, entry) in entries_by_title {
        let (Some(c), Some(Some(id))) = (entry, owned_by_title.get(&title)) else { continue };
        if title.is_empty() || linked_keys.contains(c.key) || out.contains_key(*id) {
            continue;
        }
        out.insert(id.to_string(), c.key.to_string());
    }
    out
}

/// Unix seconds as the timestamps file writes dates.
pub fn unix_to_iso(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(games: &[(&str, &str)]) -> SteamOwnedFile {
        SteamOwnedFile {
            games: games
                .iter()
                .map(|(id, name)| SteamOwnedGame { app_id: id.to_string(), name: name.to_string(), ..Default::default() })
                .collect(),
            updated_at: Some("x".into()),
            ..Default::default()
        }
    }

    fn cand<'a>(key: &'a str, title: &'a str, ids: &[&'a str]) -> Candidate<'a> {
        Candidate { key, title, steam_ids: ids.to_vec(), steam_entry: None, joinable: true }
    }

    #[test]
    fn titles_match_without_case_marks_or_punctuation() {
        assert_eq!(title_key("DOOM™ Eternal"), title_key("Doom Eternal"));
        assert_eq!(title_key("Tom Clancy's Rainbow Six® Siege"), "tomclancysrainbowsixsiege");
        assert_ne!(title_key("Portal"), title_key("Portal 2"));
    }

    #[test]
    fn ids_win_over_titles_and_titles_must_be_unique() {
        let o = owned(&[("1", "Control"), ("2", "Portal"), ("3", "Hades"), ("4", "Hades")]);
        let c = [
            cand("legendary_a", "Control Ultimate Edition", &["1"]),
            cand("gog_b", "Portal", &[]),
            cand("sideload_c", "Hades", &[]),
            cand("sideload_d", "Portal", &["9"]),
            Candidate { key: "sideload_steam_2", title: "Portal", steam_ids: vec!["2"], steam_entry: Some("2"), joinable: true },
            Candidate { key: "sideload_emu", title: "Control", steam_ids: vec![], steam_entry: None, joinable: false },
        ];
        let t = twins(&c, &o);
        assert_eq!(t.get("1").map(String::as_str), Some("legendary_a"));
        // "Portal" is the title of two entries: no title match.
        assert_eq!(t.get("2"), None);
        // "Hades" is owned twice: no title match either.
        assert_eq!(t.get("3"), None);
        assert_eq!(t.len(), 1);
        // Nothing owned (never read): no twins.
        assert!(twins(&c, &SteamOwnedFile::default()).is_empty());
    }

    #[test]
    fn a_title_match_needs_an_entry_without_its_own_link() {
        let o = owned(&[("1", "Control"), ("5", "Celeste")]);
        let c = [cand("legendary_a", "Celeste", &["1"])];
        let t = twins(&c, &o);
        assert_eq!(t.get("1").map(String::as_str), Some("legendary_a"));
        assert_eq!(t.get("5"), None);
    }

    #[test]
    fn dates_are_iso() {
        assert_eq!(unix_to_iso(1_499_373_907).as_deref(), Some("2017-07-06T20:45:07.000Z"));
    }
}
