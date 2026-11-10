//! The review queue's entry points: listing it, reading one game, searching
//! the sources by another name, and the user's answer ([`Resolution`]).

use std::sync::atomic::Ordering;

use anyhow::{bail, Result};
use heronite_data::library::metadata_key;
use heronite_data::models::CollectionGameMetadata;
use indexmap::IndexMap;

use super::fetch::{decide_in, review_candidates};
use super::{MetadataService, Offline};
use crate::record;
use crate::review::{self, group_hits, MatchChoice, MatchPick, ReviewEntry, ReviewItem, ReviewReason};
use crate::sources::{is_on, SourceContext};
use crate::store::{as_runner, current_or_blank, get_game_metadata, upsert_game_metadata};
use crate::types::{BulkItem, Lookup, MetadataApplyArgs, SourceAnswer, SourceHit};

/// The user's answer for a game of the review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// "Use this match": the game of each source (a [`MatchChoice`]'s
    /// picks). They are looked up, merged and saved pinned, so the game is
    /// not asked again and later updates use them.
    UseMatch(Vec<MatchPick>),
    /// "In no source": automatic updates stop asking about the game until
    /// it is fetched by hand or its title changes.
    InNoSource,
    /// It stays in the queue.
    Skip,
}

/// What [`MetadataService::resolve_review`] did.
#[derive(Debug, Clone)]
pub enum Resolved {
    Saved(Box<CollectionGameMetadata>),
    InNoSource,
    Skipped,
}

/// Some chosen games could not be read from their source (gone, or the
/// source did not answer for them): nothing was saved. Holds their sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchUnavailable(pub Vec<String>);

impl std::fmt::Display for MatchUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the chosen game could not be read from {}", self.0.join(", "))
    }
}

impl std::error::Error for MatchUnavailable {}

/// A search of the sources by name, grouped by game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceSearch {
    pub choices: Vec<MatchChoice>,
    /// The sources asked, in order.
    pub asked: Vec<String>,
    /// Those that did not answer.
    pub unreachable: Vec<String>,
    /// Those that need a key first (IGDB without one).
    pub needs_key: Vec<String>,
}

impl MetadataService {
    /// Changes whenever this service changes the queue (a job, the idle
    /// fill, a resolve, an undo): compare it to know when to read it again.
    pub fn review_revision(&self) -> u64 {
        self.review_revision.load(Ordering::SeqCst)
    }

    fn queue_changed(&self) {
        self.review_revision.fetch_add(1, Ordering::SeqCst);
    }

    /// The games waiting for review, oldest first. One read of a small file.
    pub fn review_queue(&self) -> Vec<ReviewItem> {
        review::load(&self.paths)
            .games
            .into_iter()
            .map(|(key, e)| ReviewItem {
                choices: e.choices().len(),
                key,
                app_name: e.app_name,
                runner: e.runner,
                title: e.title,
                reason: e.reason,
            })
            .collect()
    }

    /// One game of the queue (`runner:appName`), with what each source
    /// proposed ([`ReviewEntry::choices`]). No network.
    pub fn review_entry(&self, key: &str) -> Option<ReviewEntry> {
        let mut entry = review::load(&self.paths).games.shift_remove(key)?;
        // Queued while adult covers were allowed: not shown now they are not.
        if !self.metadata_settings().allow_adult_covers {
            entry.hits.iter_mut().for_each(SourceHit::drop_adult);
        }
        Some(entry)
    }

    /// What the sources propose for `game` now (asks them), as an entry of
    /// the review, for a game that is not queued ("Fix matches"). Its
    /// reason is [`ReviewReason::Requested`] when the sources are sure.
    /// Writes nothing.
    pub fn review_entry_for(&self, game: &BulkItem) -> Result<ReviewEntry> {
        let runner = as_runner(&game.runner);
        let args = MetadataApplyArgs {
            app_name: game.app_name.clone(),
            runner: runner.to_owned(),
            title: game.title.clone(),
            steam_app_id: game.steam_app_id.clone(),
            ..Default::default()
        };
        let gathered = self.gather(&args);
        if gathered.offline {
            return Err(Offline.into());
        }
        let current = get_game_metadata(&self.paths, runner, &game.app_name)?;
        let reason = decide_in(&gathered.sources, &gathered.candidates, current.as_ref()).review.unwrap_or(ReviewReason::Requested);
        Ok(ReviewEntry::new(&game.app_name, runner, &game.title, reason, &review_candidates(&gathered.sources, &gathered.candidates)))
    }

    /// Searches every source that can search, is on, is healthy and has its
    /// database when it needs one (or those of `only`) for `query`, at
    /// once, and groups the hits by game.
    pub fn search_sources(&self, query: &str, only: Option<&[String]>) -> SourceSearch {
        let ctx = SourceContext::new(&self.paths, &self.http);
        let settings = ctx.settings();
        let sources = self.sources();
        let asked: Vec<&str> = sources
            .lookups()
            .map(|s| s.descriptor())
            .filter(|d| d.searchable && is_on(settings, &d.id) && only.is_none_or(|o| o.contains(&d.id)))
            // Not asked before the user downloads its database, nor while it cannot be asked.
            .filter(|d| sources.downloaded(&self.paths, &d.id) && sources.health(&d.id).is_ok())
            .map(|d| d.id.as_str())
            .collect();
        let answers: Vec<(&str, Lookup<_>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = asked
                .iter()
                .map(|&id| {
                    let (ctx, sources) = (&ctx, &sources);
                    (id, scope.spawn(move || sources.search(ctx, id, query)))
                })
                .collect();
            handles.into_iter().map(|(id, h)| (id, h.join().unwrap_or(Lookup::NotFound))).collect()
        });
        let mut out = SourceSearch { asked: asked.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let mut hits = Vec::new();
        for (id, answer) in answers {
            match answer {
                Lookup::Found(found) => hits.extend(found),
                Lookup::Unreachable => out.unreachable.push(id.to_owned()),
                Lookup::Skipped => out.needs_key.push(id.to_owned()),
                Lookup::NotFound => {}
            }
        }
        out.choices = group_hits(hits);
        out
    }

    /// Applies the user's answer for `game` (queued or not); see
    /// [`Resolution`]. Blocking (a match is looked up). Offline, or a
    /// chosen game that cannot be read ([`MatchUnavailable`]), is an error
    /// and changes nothing.
    pub fn resolve_review(&self, game: &BulkItem, resolution: &Resolution) -> Result<Resolved> {
        let runner = as_runner(&game.runner);
        let key = metadata_key(runner, &game.app_name);
        match resolution {
            Resolution::Skip => Ok(Resolved::Skipped),
            Resolution::InNoSource => {
                let mut md = current_or_blank(&self.paths, runner, &game.app_name)?;
                record::mark_no_match(&mut md, &game.title);
                upsert_game_metadata(&self.paths, md)?;
                self.unqueue(&key)?;
                Ok(Resolved::InNoSource)
            }
            Resolution::UseMatch(picks) => {
                if picks.is_empty() {
                    bail!("no match was chosen");
                }
                // The other sources find the game again (an id the record
                // has may be the wrong game being fixed): what is not the
                // chosen game is left out, and forgotten.
                let mut pins: IndexMap<String, Option<String>> = picks.iter().map(|p| (p.source.clone(), Some(p.id.clone()))).collect();
                let current = get_game_metadata(&self.paths, runner, &game.app_name)?;
                for (source, m) in current.as_ref().map(record::source_matches).unwrap_or_default() {
                    if !m.pinned {
                        pins.entry(source).or_insert(None);
                    }
                }
                let args = MetadataApplyArgs {
                    app_name: game.app_name.clone(),
                    runner: runner.to_owned(),
                    title: game.title.clone(),
                    steam_app_id: game.steam_app_id.clone(),
                    source: Some("auto".into()),
                    overwrite_existing: Some(true),
                    preserve_manual: Some(true),
                    pins: Some(pins.clone()),
                    ..Default::default()
                };
                let preview = self.preview_game_metadata(&args)?;
                if preview.offline {
                    return Err(Offline.into());
                }
                let missing: Vec<String> = picks
                    .iter()
                    .filter(|p| !preview.candidates.iter().any(|c| c.source == p.source && c.match_id.as_deref() == Some(p.id.as_str())))
                    .map(|p| p.source.clone())
                    .collect();
                if !missing.is_empty() {
                    return Err(MatchUnavailable(missing).into());
                }
                let mut md = preview.proposed;
                let used = |source: &str| {
                    preview.sources.iter().any(|s| s.source == source && matches!(s.answer, SourceAnswer::Found { used: true, .. }))
                };
                for (source, _) in pins.iter().filter(|(s, pin)| pin.is_none() && !used(s)) {
                    record::clear_source_match(&mut md, source);
                }
                record::clear_no_match(&mut md);
                let md = self.persist_images(md, self.metadata_settings().download_images);
                let saved = upsert_game_metadata(&self.paths, md)?;
                self.unqueue(&key)?;
                Ok(Resolved::Saved(Box::new(saved)))
            }
        }
    }

    /// Queues (or queues again) a game; returns the entry it replaced.
    pub(super) fn queue_review(&self, entry: ReviewEntry) -> Result<Option<ReviewEntry>> {
        let key = metadata_key(&entry.runner, &entry.app_name);
        let before = review::update(&self.paths, |file| Ok(file.games.insert(key, entry)))?;
        self.queue_changed();
        Ok(before)
    }

    /// Takes `key` out of the queue; returns the entry it had. Writes only
    /// when it was there.
    pub(super) fn unqueue(&self, key: &str) -> Result<Option<ReviewEntry>> {
        if !review::load(&self.paths).games.contains_key(key) {
            return Ok(None);
        }
        let removed = review::update(&self.paths, |file| Ok(file.games.shift_remove(key)))?;
        self.queue_changed();
        Ok(removed)
    }

    /// Puts entries back as they were (`None`: none) where they still hold
    /// what was written after (`after`). Returns how many changed.
    pub(super) fn restore_queue(&self, entries: &[(String, Option<ReviewEntry>, Option<ReviewEntry>)]) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let changed = review::update(&self.paths, |file| {
            let mut changed = 0;
            for (key, before, after) in entries {
                if file.games.get(key) != after.as_ref() {
                    continue;
                }
                match before {
                    Some(entry) => file.games.insert(key.clone(), entry.clone()),
                    None => file.games.shift_remove(key),
                };
                changed += 1;
            }
            Ok(changed)
        })?;
        if changed > 0 {
            self.queue_changed();
        }
        Ok(changed)
    }
}
