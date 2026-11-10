//! What a metadata record knows besides its fields, kept in additive keys of
//! its `extra` (the file stays Heronite's format, and a build without them
//! keeps them):
//!
//! - `sourceMatches`: per source, which game of that source the record is
//!   ([`SourceMatch`]). For IGDB, Steam and Lutris the id lives in
//!   `igdbId`, `steamAppId` and `lutrisSlug`, which stay the truth: an entry
//!   whose id differs from them (another build changed them) is dropped.
//! - `lockedFields`: fields no update changes until they are unlocked.
//! - `noMatch`: the user said no source has this game (under this title):
//!   automatic updates stop asking until it is fetched by hand or renamed.

use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::MatchedBy;
use crate::util::{normalize_title, now_iso};

pub const SOURCE_MATCHES: &str = "sourceMatches";
pub const LOCKED_FIELDS: &str = "lockedFields";
pub const NO_MATCH: &str = "noMatch";

const IGDB: &str = "igdb";
const STEAM: &str = "steam";
const LUTRIS: &str = "lutris";

/// Which game of a source a record is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMatch {
    pub id: String,
    /// The game's name and release year in the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// How it was first found; `None`: not known (older records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<MatchedBy>,
    /// The user chose it: every lookup of the source uses it, and it is sure.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

impl SourceMatch {
    /// Whether it says who the game is with certainty.
    pub fn is_sure(&self) -> bool {
        self.pinned || self.matched_by.is_some_and(MatchedBy::is_sure)
    }
}

/// The id a legacy field holds for `source` (`None` for other sources).
fn legacy_id(md: &CollectionGameMetadata, source: &str) -> Option<Option<String>> {
    let non_empty = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
    Some(match source {
        IGDB => md.igdb_id.filter(|n| n.get() != 0.0).map(|n| (n.get() as i64).to_string()),
        STEAM => non_empty(&md.steam_app_id),
        LUTRIS => non_empty(&md.lutris_slug),
        _ => return None,
    })
}

fn stored(md: &CollectionGameMetadata) -> IndexMap<String, SourceMatch> {
    md.extra.get(SOURCE_MATCHES).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// The match of `source`, if the record has one.
pub fn source_match(md: &CollectionGameMetadata, source: &str) -> Option<SourceMatch> {
    let entry = stored(md).shift_remove(source);
    match legacy_id(md, source) {
        None => entry.filter(|m| !m.id.is_empty()),
        Some(None) => None,
        Some(Some(id)) => Some(entry.filter(|m| m.id == id).unwrap_or(SourceMatch { id, ..Default::default() })),
    }
}

/// Every match the record has: the legacy ones (IGDB, Steam, Lutris), then
/// the others in the order they were saved.
pub fn source_matches(md: &CollectionGameMetadata) -> IndexMap<String, SourceMatch> {
    let mut out = IndexMap::new();
    for source in [IGDB, STEAM, LUTRIS] {
        if let Some(m) = source_match(md, source) {
            out.insert(source.to_owned(), m);
        }
    }
    for (source, m) in stored(md) {
        if legacy_id(md, &source).is_none() && !m.id.is_empty() {
            out.insert(source, m);
        }
    }
    out
}

/// Records `m` as the match of `source`. For IGDB, Steam and Lutris the id
/// also goes to its own field (with IGDB's slug from `found`, the source's
/// record of the game).
pub fn set_source_match(md: &mut CollectionGameMetadata, source: &str, m: SourceMatch, found: Option<&CollectionGameMetadata>) {
    match source {
        IGDB => {
            if let Ok(id) = m.id.parse::<i64>() {
                md.igdb_id = Some(JsNum(id as f64));
                md.igdb_slug = found.and_then(|f| f.igdb_slug.clone()).or_else(|| md.igdb_slug.clone());
            }
        }
        STEAM => md.steam_app_id = Some(m.id.clone()),
        LUTRIS => md.lutris_slug = Some(m.id.clone()),
        _ => {}
    }
    let mut all = stored(md);
    all.insert(source.to_owned(), m);
    write(md, SOURCE_MATCHES, &all);
}

/// Forgets which game of `source` the record is (for IGDB and Lutris their
/// own keys too; a Steam id is the game's own and stays).
pub fn clear_source_match(md: &mut CollectionGameMetadata, source: &str) {
    match source {
        IGDB => {
            md.igdb_id = None;
            md.igdb_slug = None;
        }
        LUTRIS => md.lutris_slug = None,
        _ => {}
    }
    let mut all = stored(md);
    if all.shift_remove(source).is_some() {
        if all.is_empty() {
            md.extra.remove(SOURCE_MATCHES);
        } else {
            write(md, SOURCE_MATCHES, &all);
        }
    }
}

/// Lets later lookups of `source` find the game again by themselves (the
/// record keeps the id it has).
pub fn unpin(md: &mut CollectionGameMetadata, source: &str) {
    let mut all = stored(md);
    if let Some(m) = all.get_mut(source) {
        m.pinned = false;
        write(md, SOURCE_MATCHES, &all);
    }
}

/// The ids of the sources the user pinned.
pub fn pinned_ids(md: &CollectionGameMetadata) -> IndexMap<String, String> {
    source_matches(md).into_iter().filter(|(_, m)| m.pinned).map(|(s, m)| (s, m.id)).collect()
}

fn write<T: Serialize>(md: &mut CollectionGameMetadata, key: &str, value: &T) {
    match serde_json::to_value(value) {
        Ok(v) => {
            md.extra.insert(key.to_owned(), v);
        }
        Err(e) => log::warn!("metadata record key {key}: {e}"),
    }
}

/// The locked fields (`METADATA_FIELDS` ids), in the order they were locked.
pub fn locked_fields(md: &CollectionGameMetadata) -> Vec<String> {
    md.extra.get(LOCKED_FIELDS).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

pub fn is_locked(md: &CollectionGameMetadata, field: &str) -> bool {
    md.extra.get(LOCKED_FIELDS).and_then(Value::as_array).is_some_and(|list| list.iter().any(|f| f.as_str() == Some(field)))
}

/// Locks or unlocks `field`. The key goes away with the last lock.
pub fn set_locked(md: &mut CollectionGameMetadata, field: &str, locked: bool) {
    let mut list = locked_fields(md);
    list.retain(|f| f != field);
    if locked {
        list.push(field.to_owned());
    }
    if list.is_empty() {
        md.extra.remove(LOCKED_FIELDS);
    } else {
        write(md, LOCKED_FIELDS, &list);
    }
}

/// The user's "In no source" for a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoMatch {
    /// The game's title when it was said; a new title asks again.
    pub title: String,
    /// When (ISO 8601).
    pub at: String,
}

pub fn no_match(md: &CollectionGameMetadata) -> Option<NoMatch> {
    md.extra.get(NO_MATCH).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Whether the user said no source has the game titled `title`.
pub fn marked_no_match(md: &CollectionGameMetadata, title: &str) -> bool {
    no_match(md).is_some_and(|m| normalize_title(&m.title) == normalize_title(title))
}

pub fn mark_no_match(md: &mut CollectionGameMetadata, title: &str) {
    write(md, NO_MATCH, &NoMatch { title: title.to_owned(), at: now_iso() });
}

pub fn clear_no_match(md: &mut CollectionGameMetadata) {
    md.extra.remove(NO_MATCH);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blank_metadata;

    #[test]
    fn legacy_fields_are_the_truth_for_their_sources() {
        let mut md = blank_metadata("a", "sideload");
        md.igdb_id = Some(JsNum(72.0));
        md.lutris_slug = Some("portal".into());
        assert_eq!(source_match(&md, "igdb"), Some(SourceMatch { id: "72".into(), ..Default::default() }));
        assert_eq!(source_matches(&md).keys().collect::<Vec<_>>(), ["igdb", "lutris"]);

        let pinned = SourceMatch { id: "4242".into(), title: Some("Other".into()), year: Some(2001), pinned: true, ..Default::default() };
        let mut found = blank_metadata("a", "sideload");
        found.igdb_slug = Some("other".into());
        set_source_match(&mut md, "igdb", pinned.clone(), Some(&found));
        assert_eq!((md.igdb_id, md.igdb_slug.as_deref()), (Some(JsNum(4242.0)), Some("other")));
        assert_eq!(source_match(&md, "igdb"), Some(pinned));
        assert_eq!(pinned_ids(&md).get("igdb").map(String::as_str), Some("4242"));

        // Another build re-matched IGDB: the stored entry no longer applies.
        md.igdb_id = Some(JsNum(9.0));
        assert_eq!(source_match(&md, "igdb"), Some(SourceMatch { id: "9".into(), ..Default::default() }));
        md.igdb_id = None;
        assert_eq!(source_match(&md, "igdb"), None);
    }

    #[test]
    fn other_sources_live_in_the_new_key() {
        let mut md = blank_metadata("a", "sideload");
        let m = SourceMatch { id: "x1".into(), matched_by: Some(MatchedBy::FuzzyTitle), ..Default::default() };
        set_source_match(&mut md, "exampledb", m.clone(), None);
        assert_eq!(source_match(&md, "exampledb"), Some(m));
        assert!(!source_match(&md, "exampledb").unwrap().is_sure());
        let json = serde_json::to_value(&md).unwrap();
        assert_eq!(json["sourceMatches"]["exampledb"], serde_json::json!({"id": "x1", "matchedBy": "fuzzyTitle"}));
        unpin(&mut md, "exampledb");
        assert!(pinned_ids(&md).is_empty());
        clear_source_match(&mut md, "exampledb");
        assert!(source_matches(&md).is_empty() && !md.extra.contains_key(SOURCE_MATCHES));
        md.lutris_slug = Some("x".into());
        clear_source_match(&mut md, "lutris");
        assert_eq!(md.lutris_slug, None);
    }

    #[test]
    fn locks_and_no_match_are_additive_keys() {
        let mut md = blank_metadata("a", "sideload");
        assert!(locked_fields(&md).is_empty() && !is_locked(&md, "genres"));
        set_locked(&mut md, "genres", true);
        set_locked(&mut md, "description", true);
        set_locked(&mut md, "genres", true);
        assert_eq!(locked_fields(&md), ["description", "genres"]);
        assert!(is_locked(&md, "genres"));
        set_locked(&mut md, "genres", false);
        set_locked(&mut md, "description", false);
        assert!(!md.extra.contains_key(LOCKED_FIELDS));

        mark_no_match(&mut md, "Ghost Game");
        assert!(marked_no_match(&md, "ghost game") && !marked_no_match(&md, "Ghost Game 2"));
        clear_no_match(&mut md);
        assert!(no_match(&md).is_none());
    }
}
