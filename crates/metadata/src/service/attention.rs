//! Which games of the Collection need attention, and why ([`Attention`]):
//! from what is on disk (`metadata.json`, the review queue, `art.json`),
//! one read of each file, no network.

use anyhow::Result;
use heronite_data::cover_family::{CoverFamilies, CoverFamily};
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
use crate::types::{ArtKind, BulkItem};

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
    /// Per cover family a view uses beside the tall one (each family in use
    /// has an entry, maybe empty): details but no picture of that family,
    /// fetched or set by hand, so the view shows another one cropped. Apart
    /// from `no_cover`, and not a reason to need attention.
    pub cropped: IndexMap<CoverFamily, IndexSet<String>>,
}

impl Attention {
    /// The games without a match: waiting for review, no details at all or
    /// failed last time.
    pub fn without_match(&self) -> IndexSet<String> {
        self.to_review.iter().chain(&self.no_details).chain(&self.failed).cloned().collect()
    }

    /// Every game that needs attention, for any reason. A cropped cover is
    /// not one: it is a choice of the aspect, listed apart in `cropped`.
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

/// The classification of `games` (see [`Attention`]); `in_use`: the cover
/// families some view draws.
pub(super) fn classify(
    games: &[BulkItem],
    records: &IndexMap<String, CollectionGameMetadata>,
    queue: &ReviewFile,
    art: &ArtFile,
    in_use: &[CoverFamily],
) -> Attention {
    let families: Vec<ArtKind> = in_use.iter().filter(|f| **f != CoverFamily::Tall).map(|f| ArtKind::cover(*f)).collect();
    let cropped = families.iter().filter_map(|k| Some((k.family()?, IndexSet::new()))).collect();
    let mut out = Attention { games: games.len(), cropped, ..Default::default() };
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
            let custom = |kind: ArtKind| art.games.get(&key).is_some_and(|a| kind.of_art(a).is_some_and(|u| !u.is_empty()));
            for kind in &families {
                if !has_field_value(md, kind.as_str()) && !custom(*kind) {
                    let family = kind.family().expect("a cover");
                    out.cropped.entry(family).or_default().insert(key.clone());
                }
            }
            if !has_field_value(md, "cover") && !custom(ArtKind::Cover) {
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
        Ok(classify(games, &records, &review::load(&self.paths), &art, &CoverFamilies::load(&self.paths).in_use()))
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

        let a = classify(&games, &records, &queue, &art, &[CoverFamily::Tall]);
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
        assert!(a.cropped.is_empty(), "only the tall family in use: nothing is cropped");
    }

    #[test]
    fn a_family_in_use_without_its_picture_is_cropped() {
        use CoverFamily::*;
        let games: Vec<BulkItem> = ["bare", "square", "custom", "nodetails", "wide"].map(item).into();
        let mut records = IndexMap::new();
        for app in ["bare", "square", "custom", "wide"] {
            let mut md = blank_metadata(app, "sideload");
            md.description = Some("d".into());
            md.cover_url = Some("https://c".into());
            match app {
                "square" => md.cover_square_url = Some("https://sq".into()),
                "wide" => md.cover_wide_url = Some("https://w".into()),
                _ => {}
            }
            records.insert(format!("sideload:{app}"), md);
        }
        let art: ArtFile =
            serde_json::from_value(serde_json::json!({"games": {"sideload:custom": {"coverSquareUrl": "localart://art/x"}}})).unwrap();
        let a = classify(&games, &records, &ReviewFile::default(), &art, &[Tall, Square, Wide]);
        let keys = |f| a.cropped[&f].iter().map(|k| k.trim_start_matches("sideload:").to_string()).collect::<Vec<_>>();
        assert_eq!(keys(Square), ["bare", "wide"]);
        assert_eq!(keys(Wide), ["bare", "square", "custom"]);
        assert!(!a.cropped.contains_key(&Tall));
        assert!(a.no_cover.is_empty(), "a cropped cover is not a missing one");
        assert!(!a.contains("sideload:bare"), "a cropped cover does not need attention");
        assert_eq!(a.all().into_iter().collect::<Vec<_>>(), ["sideload:nodetails"], "only the game without details");
        // A family in use that every game has: an empty entry.
        let a = classify(&games[1..2], &records, &ReviewFile::default(), &art, &[Tall, Square]);
        assert!(a.cropped[&Square].is_empty() && a.complete() == 1);
    }

    #[test]
    fn a_game_with_only_its_tall_cover_is_complete_when_square_is_in_use() {
        let games = [item("tall")];
        let mut md = blank_metadata("tall", "sideload");
        md.description = Some("d".into());
        md.cover_url = Some("https://c".into());
        let records = IndexMap::from([("sideload:tall".to_string(), md)]);
        let a = classify(&games, &records, &ReviewFile::default(), &ArtFile::default(), &[CoverFamily::Tall, CoverFamily::Square]);
        assert_eq!(a.cropped[&CoverFamily::Square].iter().collect::<Vec<_>>(), ["sideload:tall"]);
        assert!(a.all().is_empty() && !a.contains("sideload:tall"));
        assert_eq!(a.complete(), 1);
    }

    #[test]
    fn classifying_two_thousand_games_is_fast() {
        use CoverFamily::*;
        let games: Vec<BulkItem> = (0..2000).map(|i| item(&format!("g{i}"))).collect();
        let records: IndexMap<String, CollectionGameMetadata> = (0..2000)
            .map(|i| {
                let mut md = blank_metadata(&format!("g{i}"), "sideload");
                md.description = Some("d".into());
                md.cover_url = Some("https://c".into());
                if i % 2 == 0 {
                    md.cover_square_url = Some("https://sq".into());
                }
                (format!("sideload:g{i}"), md)
            })
            .collect();
        let start = std::time::Instant::now();
        let a = classify(&games, &records, &ReviewFile::default(), &ArtFile::default(), &[Tall, Square, Wide]);
        let took = start.elapsed();
        assert_eq!((a.cropped[&Square].len(), a.cropped[&Wide].len()), (1000, 2000));
        assert!(took < std::time::Duration::from_millis(500), "{took:?}");
    }
}
