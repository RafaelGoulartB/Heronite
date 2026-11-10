//! Which games of the Collection need attention, and why ([`Attention`]):
//! from what is on disk (`metadata.json`, the review queue, `art.json`),
//! one read of each file, no network.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{ArtFile, CollectionGameMetadata};
use heronite_data::settings::METADATA_FIELDS;
use indexmap::{IndexMap, IndexSet};

use super::MetadataService;
use crate::merge::has_field_value;
use crate::record;
use crate::review::{self, ReviewFile};
use crate::store::{as_runner, get_all_game_metadata};
use crate::types::BulkItem;

/// The games of a set that need attention, by reason (keys `runner:appName`,
/// in the set's order). A game may have several reasons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attention {
    /// How many games were classified.
    pub games: usize,
    /// Waiting in the review queue.
    pub to_review: IndexSet<String>,
    /// No record, or one without a value in any field.
    pub no_details: IndexSet<String>,
    /// The last fetch found the game in no source (`fetchError`).
    pub failed: IndexSet<String>,
    /// Details but no description.
    pub no_description: IndexSet<String>,
    /// Details but no cover (neither fetched nor set by hand).
    pub no_cover: IndexSet<String>,
    /// Marked "in no source" by the user (under their current title): left
    /// out of the three reasons above.
    pub in_no_source: IndexSet<String>,
}

impl Attention {
    /// The games without a match: waiting for review, no details at all or
    /// failed last time.
    pub fn without_match(&self) -> IndexSet<String> {
        self.to_review.iter().chain(&self.no_details).chain(&self.failed).cloned().collect()
    }

    /// Every game that needs attention, for any reason.
    pub fn all(&self) -> IndexSet<String> {
        let mut all = self.without_match();
        all.extend(self.no_description.iter().chain(&self.no_cover).cloned());
        all
    }

    pub fn contains(&self, key: &str) -> bool {
        [&self.to_review, &self.no_details, &self.failed, &self.no_description, &self.no_cover].iter().any(|s| s.contains(key))
    }

    /// The games that need nothing ("389 of 412 games are complete").
    pub fn complete(&self) -> usize {
        self.games.saturating_sub(self.all().len())
    }
}

/// The classification of `games` (see [`Attention`]).
pub(super) fn classify(
    games: &[BulkItem],
    records: &IndexMap<String, CollectionGameMetadata>,
    queue: &ReviewFile,
    art: &ArtFile,
) -> Attention {
    let mut out = Attention { games: games.len(), ..Default::default() };
    for game in games {
        let key = metadata_key(as_runner(&game.runner), &game.app_name);
        let md = records.get(&key);
        let details = METADATA_FIELDS.iter().any(|f| has_field_value(md, f));
        if md.is_some_and(|m| record::marked_no_match(m, &game.title)) {
            out.in_no_source.insert(key.clone());
        } else {
            if queue.games.contains_key(&key) {
                out.to_review.insert(key.clone());
            }
            if !details {
                out.no_details.insert(key.clone());
            }
            if md.is_some_and(|m| m.fetch_error.is_some()) {
                out.failed.insert(key.clone());
            }
        }
        if details {
            if !has_field_value(md, "description") {
                out.no_description.insert(key.clone());
            }
            let custom = art.games.get(&key).is_some_and(|a| a.cover_url.as_deref().is_some_and(|u| !u.is_empty()));
            if !has_field_value(md, "cover") && !custom {
                out.no_cover.insert(key);
            }
        }
    }
    out
}

impl MetadataService {
    /// Classifies `games` (see [`Attention`]). Fast enough for the whole
    /// Collection: one read of each file, nothing per game.
    pub fn attention(&self, games: &[BulkItem]) -> Result<Attention> {
        let records = get_all_game_metadata(&self.paths)?;
        let art: ArtFile = json_store::read(&self.paths.art())?;
        Ok(classify(games, &records, &review::load(&self.paths), &art))
    }

    /// The games of `games` that need attention, for any reason, in their
    /// order.
    pub fn needs_attention(&self, games: &[BulkItem]) -> Result<Vec<BulkItem>> {
        let all = self.attention(games)?.all();
        Ok(games.iter().filter(|g| all.contains(&metadata_key(as_runner(&g.runner), &g.app_name))).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{ReviewEntry, ReviewReason};
    use crate::util::blank_metadata;

    fn item(app: &str) -> BulkItem {
        BulkItem { app_name: app.into(), runner: "sideload".into(), title: format!("Title {app}"), steam_app_id: None }
    }

    #[test]
    fn each_reason_and_the_in_no_source_mark() {
        let games: Vec<BulkItem> =
            ["none", "queued", "failed", "nodesc", "nocover", "custom", "full", "nomatch", "renamed"].map(item).into();
        let mut records = IndexMap::new();
        let mut put = |app: &str, f: &dyn Fn(&mut CollectionGameMetadata)| {
            let mut md = blank_metadata(app, "sideload");
            md.description = Some("d".into());
            md.cover_url = Some("https://c".into());
            f(&mut md);
            records.insert(format!("sideload:{app}"), md);
        };
        put("failed", &|m| {
            *m = blank_metadata("failed", "sideload");
            m.fetch_error = Some("x".into());
        });
        put("nodesc", &|m| m.description = None);
        put("nocover", &|m| m.cover_url = None);
        put("custom", &|m| m.cover_url = None);
        put("full", &|_| {});
        put("nomatch", &|m| {
            *m = blank_metadata("nomatch", "sideload");
            m.fetch_error = Some("x".into());
            record::mark_no_match(m, "Title nomatch");
        });
        put("renamed", &|m| {
            *m = blank_metadata("renamed", "sideload");
            record::mark_no_match(m, "Old title");
        });
        let mut queue = ReviewFile::default();
        queue.games.insert("sideload:queued".into(), ReviewEntry::new("queued", "sideload", "Q", ReviewReason::LoneGuess, &[]));
        let art: ArtFile =
            serde_json::from_value(serde_json::json!({"games": {"sideload:custom": {"coverUrl": "localart://art/x"}}})).unwrap();

        let a = classify(&games, &records, &queue, &art);
        let keys = |s: &IndexSet<String>| s.iter().map(|k| k.trim_start_matches("sideload:").to_string()).collect::<Vec<_>>();
        assert_eq!(keys(&a.to_review), ["queued"]);
        assert_eq!(keys(&a.no_details), ["none", "queued", "failed", "renamed"]);
        assert_eq!(keys(&a.failed), ["failed"]);
        assert_eq!(keys(&a.no_description), ["nodesc"]);
        assert_eq!(keys(&a.no_cover), ["nocover"]);
        assert_eq!(keys(&a.in_no_source), ["nomatch"]);
        assert_eq!(keys(&a.without_match()), ["queued", "none", "failed", "renamed"]);
        assert_eq!(a.all().len(), 6);
        assert_eq!(a.complete(), 3, "custom, full and nomatch");
        assert!(a.contains("sideload:nodesc") && !a.contains("sideload:nomatch"));
    }
}
