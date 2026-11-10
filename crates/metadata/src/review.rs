//! The review queue: games an automatic update did not save because the
//! sources were not sure who they are ([`crate::confidence`]), with what
//! each source proposed, so the review opens without the network.
//!
//! `local_library/metadata-review.json` (camelCase, tab-indented like the
//! other files; safe to delete, nothing else points at it):
//!
//! ```json
//! {"games": {"sideload:plain": {
//!   "appName": "plain", "runner": "sideload", "title": "Plain Game",
//!   "reason": "loneGuess", "queuedAt": "2026-11-10T12:00:00.000Z",
//!   "hits": [{"source": "igdb", "id": "72", "title": "Portal 2", "year": 2011,
//!             "platform": "PC, Mac", "thumbUrl": "https://…", "matchedBy": "fuzzyTitle"}]}}}
//! ```
//!
//! The service reads and changes it (`MetadataService::review_queue`,
//! `resolve_review`); a job's Undo takes back the entries it added.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::models::Extra;
use heronite_data::paths::DataPaths;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::confidence::same_game;
use crate::types::{MetadataCandidate, SourceHit};
use crate::util::{normalize_title, now_iso};

/// Why a game waits for the user. The UI words each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewReason {
    /// One source found a game whose name is only close to the title.
    LoneGuess,
    /// Sources found games that are not the same.
    SourcesDisagree,
    /// The user asked to check the match ("Fix matches"); never queued.
    Requested,
    /// A reason a newer build wrote.
    #[serde(other)]
    Other,
}

/// `local_library/metadata-review.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    /// Keyed like `metadata.json` (`runner:appName`), oldest first.
    #[serde(default)]
    pub games: IndexMap<String, ReviewEntry>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A game waiting for review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEntry {
    pub app_name: String,
    pub runner: String,
    /// The title the sources were asked with.
    pub title: String,
    pub reason: ReviewReason,
    /// When it was queued (ISO 8601).
    pub queued_at: String,
    /// What each source proposed, in source order.
    #[serde(default)]
    pub hits: Vec<SourceHit>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl ReviewEntry {
    pub fn new(app_name: &str, runner: &str, title: &str, reason: ReviewReason, candidates: &[MetadataCandidate]) -> Self {
        Self {
            app_name: app_name.to_owned(),
            runner: runner.to_owned(),
            title: title.to_owned(),
            reason,
            queued_at: now_iso(),
            hits: candidates.iter().map(MetadataCandidate::hit).collect(),
            extra: Extra::new(),
        }
    }

    /// The proposals grouped by game, most sources first.
    pub fn choices(&self) -> Vec<MatchChoice> {
        let mut choices = group_hits(self.hits.clone());
        choices.sort_by_key(|c| std::cmp::Reverse(c.hits.len()));
        choices
    }
}

/// The queue in a few words, for lists and counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem {
    pub key: String,
    pub app_name: String,
    pub runner: String,
    pub title: String,
    pub reason: ReviewReason,
    /// How many games the sources proposed.
    pub choices: usize,
}

/// One game of a source, chosen: the user's "this is it".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchPick {
    pub source: String,
    pub id: String,
}

/// One game as several sources have it: what a choice of the review or a
/// search shows (the first hit that has each detail gives it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchChoice {
    pub title: Option<String>,
    pub year: Option<i32>,
    pub platform: Option<String>,
    /// Its name in another script ([`SourceHit::alt_title`]).
    pub alt_title: Option<String>,
    pub thumb_url: Option<String>,
    /// One per source, in the order they came.
    pub hits: Vec<SourceHit>,
}

impl MatchChoice {
    /// The picks "Use this match" saves: the hits that have an id.
    pub fn picks(&self) -> Vec<MatchPick> {
        self.hits.iter().filter_map(|h| Some(MatchPick { source: h.source.clone(), id: h.id.clone()? })).collect()
    }

    fn fill_from(&mut self, hit: &SourceHit) {
        self.title = self.title.take().or_else(|| hit.title.clone());
        self.year = self.year.or(hit.year);
        self.platform = self.platform.take().or_else(|| hit.platform.clone());
        self.alt_title = self.alt_title.take().or_else(|| hit.alt_title.clone());
        self.thumb_url = self.thumb_url.take().or_else(|| hit.thumb_url.clone());
    }
}

/// Groups hits that are the same game, at most one per source, in the order
/// the hits came (a hit joins the group with the same name before one it
/// only resembles). The same hit twice (source and id) counts once.
pub fn group_hits(hits: Vec<SourceHit>) -> Vec<MatchChoice> {
    let mut groups: Vec<MatchChoice> = Vec::new();
    for hit in hits {
        let dup = |c: &MatchChoice| c.hits.iter().any(|h| h.source == hit.source && h.id.is_some() && h.id == hit.id);
        if groups.iter().any(dup) {
            continue;
        }
        let fits = |c: &MatchChoice| {
            !c.hits.iter().any(|h| h.source == hit.source)
                && c.hits.iter().any(|h| match (&h.title, &hit.title) {
                    (Some(a), Some(b)) => same_game(a, h.year, b, hit.year),
                    _ => false,
                })
        };
        let key = hit.title.as_deref().map(normalize_title);
        let exact = groups.iter().position(|c| fits(c) && c.title.as_deref().map(normalize_title) == key);
        match exact.or_else(|| groups.iter().position(fits)) {
            Some(i) => {
                groups[i].fill_from(&hit);
                groups[i].hits.push(hit);
            }
            None => {
                let mut choice =
                    MatchChoice { title: None, year: None, platform: None, alt_title: None, thumb_url: None, hits: Vec::new() };
                choice.fill_from(&hit);
                choice.hits.push(hit);
                groups.push(choice);
            }
        }
    }
    groups
}

/// The queue as on disk; a missing or unreadable file is an empty queue
/// (it only holds what the next update would find again).
pub fn load(paths: &DataPaths) -> ReviewFile {
    json_store::read(&paths.metadata_review()).unwrap_or_else(|e| {
        log::warn!("review queue: {e:#}");
        ReviewFile::default()
    })
}

/// A read-modify-write of the queue. An unreadable file starts over empty.
pub fn update<R>(paths: &DataPaths, f: impl FnOnce(&mut ReviewFile) -> Result<R>) -> Result<R> {
    let path = paths.metadata_review();
    if json_store::read::<ReviewFile>(&path).is_err() {
        log::warn!("review queue unreadable, starting over: {}", path.display());
        json_store::write(&path, &ReviewFile::default())?;
    }
    json_store::update_if_changed::<ReviewFile, _>(&path, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MatchedBy;

    fn hit(source: &str, id: &str, title: &str, year: Option<i32>) -> SourceHit {
        SourceHit {
            source: source.into(),
            id: Some(id.into()),
            title: Some(title.into()),
            year,
            platform: None,
            thumb_url: None,
            matched_by: Some(MatchedBy::FuzzyTitle),
            alt_title: None,
            thumb_adult: false,
        }
    }

    #[test]
    fn hits_of_the_same_game_make_one_choice_one_per_source() {
        let groups = group_hits(vec![
            hit("igdb", "72", "Portal 2", Some(2011)),
            hit("igdb", "99969", "Portal 2: In Motion", Some(2011)),
            hit("steam", "620", "Portal 2", None),
            hit("lutris", "portal", "Portal", Some(2007)),
            hit("lutris", "portal", "Portal", Some(2007)),
        ]);
        let shape: Vec<Vec<&str>> = groups.iter().map(|g| g.hits.iter().map(|h| h.source.as_str()).collect()).collect();
        assert_eq!(shape, [vec!["igdb", "steam"], vec!["igdb"], vec!["lutris"]]);
        assert_eq!((groups[0].title.as_deref(), groups[0].year), (Some("Portal 2"), Some(2011)));
        assert_eq!(
            groups[0].picks(),
            [MatchPick { source: "igdb".into(), id: "72".into() }, MatchPick { source: "steam".into(), id: "620".into() }]
        );
    }

    #[test]
    fn the_file_round_trips_and_keeps_unknown_keys() {
        let text = r#"{"games": {"sideload:a": {"appName": "a", "runner": "sideload", "title": "A",
            "reason": "somethingNew", "queuedAt": "t", "future": 1}}, "version": 2}"#;
        let file: ReviewFile = serde_json::from_str(text).unwrap();
        let entry = &file.games["sideload:a"];
        assert_eq!((entry.reason, entry.hits.len()), (ReviewReason::Other, 0));
        let back = serde_json::to_value(&file).unwrap();
        assert_eq!((back["version"].as_i64(), back["games"]["sideload:a"]["future"].as_i64()), (Some(2), Some(1)));
    }

    #[test]
    fn a_broken_file_is_an_empty_queue_and_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.metadata_review(), "{ nope").unwrap();
        assert!(load(&paths).games.is_empty());
        update(&paths, |f| {
            let entry = ReviewEntry::new("a", "sideload", "A", ReviewReason::LoneGuess, &[]);
            f.games.insert("sideload:a".into(), entry);
            Ok(())
        })
        .unwrap();
        assert_eq!(load(&paths).games.len(), 1);
    }
}
