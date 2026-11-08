//! PC facts from PCGamingWiki (`store_cache/pcgamingwiki.json`): the cache
//! `heronite_metadata::pcgw` writes and the Collection reads.
//!
//! PCGamingWiki's content is CC BY-NC-SA 3.0. Only bare facts are kept
//! (supported or not, engine and anti-cheat names, save folders) with the
//! page name and revision, never its notes or prose; the UI names and links
//! the page and the licence wherever the facts are shown.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::paths::DataPaths;
use crate::text::remove_special_characters;

pub const SITE: &str = "https://www.pcgamingwiki.com";
pub const LICENCE: &str = "CC BY-NC-SA 3.0";
pub const LICENCE_URL: &str = "https://creativecommons.org/licenses/by-nc-sa/3.0/";

/// A support value of the wiki's tables (`true`, `limited`, `hackable`,
/// `false`; `unknown` and `n/a` are kept as not known).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    Yes,
    Limited,
    Hackable,
    No,
}

impl Support {
    /// The wiki's value; `None` for empty, `unknown`, `n/a` and anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "true" => Some(Self::Yes),
            "limited" => Some(Self::Limited),
            "hackable" => Some(Self::Hackable),
            "false" => Some(Self::No),
            _ => None,
        }
    }
}

/// Where a game keeps its saves on one platform (`Windows`, `Steam`...),
/// with the wiki's path tokens spelled out (`%APPDATA%`, `<game folder>`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveLocation {
    pub platform: String,
    pub paths: Vec<String>,
}

/// The facts of one PCGamingWiki page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PcgwFacts {
    /// Page title (`Half-Life 2`).
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultrawide: Option<Support>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Support>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps_120: Option<Support>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uhd_4k: Option<Support>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<Support>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_controller: Option<Support>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anticheat: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saves: Vec<SaveLocation>,
    /// Issues the page has a fix for.
    #[serde(default)]
    pub fixes: u32,
    /// The page's HowLongToBeat id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hltb_id: Option<String>,
    /// Steam app ids and GOG ids the page lists (to check a title match).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steam_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gog_ids: Vec<String>,
}

impl PcgwFacts {
    pub fn url(&self) -> String {
        page_url(&self.page)
    }

    /// The Windows save folders, else the first platform's.
    pub fn save_paths(&self) -> Option<&SaveLocation> {
        self.saves.iter().find(|s| s.platform.eq_ignore_ascii_case("windows")).or_else(|| self.saves.first())
    }
}

/// `https://www.pcgamingwiki.com/wiki/Half-Life_2`.
pub fn page_url(page: &str) -> String {
    let mut out = String::with_capacity(page.len() + 40);
    out.push_str(SITE);
    out.push_str("/wiki/");
    for b in page.trim().replace(' ', "_").bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'(' | b')' | b'!' | b'\'' | b',' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One cached lookup: the facts, or `None` when the wiki has no page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PcgwEntry {
    #[serde(default)]
    pub facts: Option<PcgwFacts>,
    /// Unix ms.
    pub fetched_at: i64,
}

/// `store_cache/pcgamingwiki.json`: entries by `steam:<id>`, `gog:<id>` or
/// `title:<key>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PcgwFile {
    #[serde(default)]
    pub entries: IndexMap<String, PcgwEntry>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PcgwFile {
    pub fn load(paths: &DataPaths) -> Self {
        json_store::read(&paths.pcgw()).unwrap_or_default()
    }
}

/// How a game is looked up: its Steam app id, GOG id, then exact title.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PcgwQuery {
    pub steam_id: Option<String>,
    pub gog_id: Option<String>,
    pub title: String,
}

impl PcgwQuery {
    /// A Collection game: its Steam app id (its own, else its metadata's),
    /// its GOG id (GOG games) and its Collection title.
    pub fn for_game(game: &crate::library::CollectionGame) -> Self {
        let digits = |v: &str| -> Option<String> {
            let v = v.trim();
            (!v.is_empty() && v.chars().all(|c| c.is_ascii_digit())).then(|| v.to_owned())
        };
        let steam_id = game
            .steam_app_id
            .as_deref()
            .and_then(digits)
            .or_else(|| game.metadata.as_ref().and_then(|m| m.steam_app_id.as_deref()).and_then(digits));
        let gog_id = (game.runner == "gog").then(|| digits(&game.app_name)).flatten();
        Self { steam_id, gog_id, title: game.title.trim().to_owned() }
    }

    /// The cache keys, most precise first.
    pub fn keys(&self) -> Vec<String> {
        fn id(v: &Option<String>) -> Option<&str> {
            v.as_deref().map(str::trim).filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
        }
        let title = remove_special_characters(self.title.trim());
        let mut keys = Vec::new();
        keys.extend(id(&self.steam_id).map(|v| format!("steam:{v}")));
        keys.extend(id(&self.gog_id).map(|v| format!("gog:{v}")));
        if !title.is_empty() {
            keys.push(format!("title:{}", title.to_lowercase()));
        }
        keys
    }
}

/// The cache, loaded once for the Collection.
#[derive(Debug, Clone, Default)]
pub struct PcgwCache {
    file: PcgwFile,
}

impl PcgwCache {
    pub fn load(paths: &DataPaths) -> Self {
        Self { file: PcgwFile::load(paths) }
    }

    pub fn from_file(file: PcgwFile) -> Self {
        Self { file }
    }

    /// The first cached entry of `query`'s keys.
    pub fn entry(&self, query: &PcgwQuery) -> Option<&PcgwEntry> {
        query.keys().iter().find_map(|k| self.file.entries.get(k))
    }

    /// The facts of `query`, when a page was found.
    pub fn facts(&self, query: &PcgwQuery) -> Option<&PcgwFacts> {
        self.entry(query)?.facts.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_values_and_urls() {
        assert_eq!(Support::parse(" true "), Some(Support::Yes));
        assert_eq!(Support::parse("Limited"), Some(Support::Limited));
        assert_eq!(Support::parse("hackable"), Some(Support::Hackable));
        assert_eq!(Support::parse("false"), Some(Support::No));
        assert_eq!(Support::parse("unknown"), None);
        assert_eq!(Support::parse("n/a"), None);
        assert_eq!(Support::parse(""), None);
        assert_eq!(page_url("Half-Life 2"), "https://www.pcgamingwiki.com/wiki/Half-Life_2");
        assert_eq!(page_url("Mörk Borg & Co?"), "https://www.pcgamingwiki.com/wiki/M%C3%B6rk_Borg_%26_Co%3F");
    }

    #[test]
    fn keys_and_lookup() {
        let q = PcgwQuery { steam_id: Some("220".into()), gog_id: Some("".into()), title: "Half-Life: 2".into() };
        assert_eq!(q.keys(), ["steam:220", "title:half-life 2"]);
        let q2 = PcgwQuery { steam_id: Some("abc".into()), gog_id: Some("1207658924".into()), title: String::new() };
        assert_eq!(q2.keys(), ["gog:1207658924"]);
        let mut file = PcgwFile::default();
        let facts = PcgwFacts { page: "Half-Life 2".into(), fixes: 3, ..Default::default() };
        file.entries.insert("title:half-life 2".into(), PcgwEntry { facts: Some(facts), fetched_at: 1 });
        file.entries.insert("gog:1207658924".into(), PcgwEntry { facts: None, fetched_at: 1 });
        let c = PcgwCache::from_file(file);
        assert_eq!(c.facts(&q).unwrap().fixes, 3, "falls back to the title key");
        assert!(c.entry(&q2).is_some() && c.facts(&q2).is_none(), "no page");
    }

    #[test]
    fn save_paths_prefer_windows() {
        let f = PcgwFacts {
            saves: vec![
                SaveLocation { platform: "Steam".into(), paths: vec!["a".into()] },
                SaveLocation { platform: "Windows".into(), paths: vec!["b".into()] },
            ],
            ..Default::default()
        };
        assert_eq!(f.save_paths().unwrap().paths, ["b"]);
    }
}
