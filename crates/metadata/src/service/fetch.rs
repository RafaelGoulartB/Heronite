//! Fetching one game: what the sources found (`gather`) that is sure
//! enough to save ([`decide`], the rule of [`crate::confidence`]), the merge a preview proposes (with the record's
//! locks and per-source matches), and the saves of a fetch and of the
//! per-game dialog.
//!
//! What an automatic fetch writes (the idle fill, every bulk job, a
//! family's download), decided with the registry the sources were asked
//! from:
//! - a details source found the game ([`found`]) and the rule is sure of
//!   it: the merge is saved, and a fetch of the details marks the game as
//!   fetched;
//! - the rule is not sure: nothing is saved, the game waits for review;
//! - no details source found it, and each one asked answered: a fetch of
//!   the details marks the game as found in no source ([`mark_not_found`]:
//!   tried again in a week; a job only when it asked every source for every
//!   field), and the sure pictures found beside (libretro by the dump's own
//!   name, SteamGridDB by the Steam id or the exact name) are saved, never
//!   as fetched details, and into a game never fetched only with the mark
//!   (a record holding only a picture would keep the idle fill away); a
//!   fetch of pictures only that nothing answers writes nothing;
//! - none found it and one that could have did not answer (offline, or it
//!   asked to wait, as a throttled service does; for a fetch of the details,
//!   only a details source counts): nothing is saved or marked
//!   ([`SourceUnreachable`], or [`Offline`] when no source answered at
//!   all), so the next pass asks again. A job reports the game as such
//!   ([`GameOutcome::Offline`](crate::job::GameOutcome::Offline)) and goes on
//!   without waiting.

use std::collections::HashSet;

use anyhow::{bail, Result};
use heronite_data::cover_family::CoverFamilies;
use heronite_data::field_names::NameRules;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, MetadataSettings};
use heronite_data::settings::{ALL_FIELDS, METADATA_FIELDS};
use heronite_i18n::tr;
use indexmap::IndexMap;

use super::gather::{Asked, Gathered};
use super::{MetadataService, Offline, NO_DATA};
use crate::confidence::{judge, same_game, Guess, Verdict};
use crate::images::materialize_metadata_image;
use crate::merge::{clear_field, copy_field, merge_metadata, with_preferred_source, MergeArgs, MergeCandidate};
use crate::record::{self, SourceMatch};
use crate::review::{ReviewEntry, ReviewReason};
use crate::sources::{is_field_on, is_on, SourceKind, SourceRegistry};
use crate::store::{as_runner, current_or_blank, get_game_metadata, upsert_game_metadata};
use crate::types::*;
use crate::util::now_iso;

/// An automatic fetch saved nothing: the sources were not sure which game
/// this is, and the game waits in the review queue. A diagnostic: the UI
/// words the [`ReviewReason`].
#[derive(Debug)]
pub struct SentToReview(pub ReviewReason);

impl std::fmt::Display for SentToReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the sources are not sure which game this is ({:?}); it waits for review", self.0)
    }
}

impl std::error::Error for SentToReview {}

/// An automatic fetch asked no source (each is off, or not for this game):
/// nothing was saved, as when offline. A diagnostic.
#[derive(Debug)]
pub(super) struct NoSourceAsked;

impl std::fmt::Display for NoSourceAsked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no metadata source was asked for this game")
    }
}

impl std::error::Error for NoSourceAsked {}

/// An automatic fetch found the game in no source while a source that
/// could have found it did not answer: nothing was saved or marked, and
/// the next pass asks again. A diagnostic.
#[derive(Debug)]
pub(super) struct SourceUnreachable;

impl std::fmt::Display for SourceUnreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a metadata source that could have this game did not answer")
    }
}

impl std::error::Error for SourceUnreachable {}

/// The reason, when `err` is [`SentToReview`].
pub fn is_sent_to_review(err: &anyhow::Error) -> Option<ReviewReason> {
    err.downcast_ref::<SentToReview>().map(|s| s.0)
}

/// What of a game's candidates may be saved.
pub(super) struct Decision {
    pub(super) used: Vec<MetadataCandidate>,
    /// Set when nothing may: the game goes to review.
    pub(super) review: Option<ReviewReason>,
}

/// [`judge`] over the candidates of details sources, anchored by the
/// matches `current` has of sources that did not answer this time: the
/// sure ones, and those of records from before matches were recorded (an
/// IGDB id or a Lutris slug alone), named and dated as the record is.
///
/// A pictures or extras source (`is_picture`) never sends a game to review: its
/// candidate is used when it is sure, or the same game as a candidate used
/// or a match the record has, and left out otherwise.
pub(super) fn decide(
    candidates: &[MetadataCandidate],
    current: Option<&CollectionGameMetadata>,
    is_picture: &dyn Fn(&str) -> bool,
) -> Decision {
    let (pictures, details): (Vec<&MetadataCandidate>, Vec<&MetadataCandidate>) = candidates.iter().partition(|c| is_picture(&c.source));
    let guesses: Vec<Guess<'_>> =
        details.iter().map(|c| Guess { matched_by: c.matched_by, title: c.match_name.as_deref(), year: c.match_year }).collect();
    let known: Vec<SourceMatch> = current
        .map(|md| {
            record::source_matches(md)
                .into_iter()
                .filter(|(source, m)| (m.is_sure() || m.matched_by.is_none()) && !candidates.iter().any(|c| &c.source == source))
                .map(|(_, m)| SourceMatch {
                    title: m.title.clone().or_else(|| md.title.clone()),
                    year: m.year.or_else(|| release_year(md.release_date.as_deref())),
                    ..m
                })
                .collect()
        })
        .unwrap_or_default();
    let anchors: Vec<Guess<'_>> =
        known.iter().map(|m| Guess { matched_by: Some(MatchedBy::SourceId), title: m.title.as_deref(), year: m.year }).collect();
    match judge(&guesses, &anchors) {
        Verdict::Save(used) => {
            let used: Vec<&MetadataCandidate> = used.into_iter().map(|i| details[i]).collect();
            let known: Vec<(&str, Option<i32>)> = used
                .iter()
                .filter_map(|c| Some((c.match_name.as_deref()?, c.match_year)))
                .chain(anchors.iter().filter_map(|a| Some((a.title?, a.year))))
                .collect();
            let agrees = |c: &MetadataCandidate| {
                c.matched_by.is_none_or(MatchedBy::is_sure)
                    || c.match_name.as_deref().is_some_and(|name| known.iter().any(|(t, y)| same_game(name, c.match_year, t, *y)))
            };
            let pictures = pictures.into_iter().filter(|c| agrees(c));
            Decision { used: used.into_iter().chain(pictures).cloned().collect(), review: None }
        }
        Verdict::Review(reason) => Decision { used: Vec::new(), review: Some(reason) },
    }
}

/// Whether a fetch of `fields` (`None`: every field) asks for the game's
/// details: some `METADATA_FIELDS` entry. One that asks only for the other
/// cover families' pictures (a family's download) is no fetch of the
/// details: it neither marks the game as fetched nor as failed.
pub(super) fn asks_details(fields: Option<&[String]>) -> bool {
    fields.is_none_or(|fields| fields.iter().any(|f| METADATA_FIELDS.contains(&f.as_str())))
}

/// Marks `md` as found in no source (`fetchError`, so the idle fill tries
/// again after a week). A "in no source" said under another title goes:
/// it no longer holds, and kept it would have the idle fill ask again at
/// every start.
pub(super) fn mark_not_found(md: &mut CollectionGameMetadata, title: &str) {
    md.fetch_error = Some(NO_DATA.into());
    md.fetched_at = Some(now_iso());
    if record::no_match(md).is_some() && !record::marked_no_match(md, title) {
        record::clear_no_match(md);
    }
}

/// Whether `source` only follows the details sources in `sources` (a
/// pictures source such as SteamGridDB, or an extras one): see [`decide`].
/// A source `sources` lacks counts as a details source.
pub(super) fn is_picture_source(sources: &SourceRegistry, source: &str) -> bool {
    sources.get(source).is_some_and(|d| d.kind != SourceKind::Details)
}

/// Whether `candidates` answer a fetch of `fields` (`None`: every field).
/// One that [asks for details](asks_details) is answered only by a details
/// source: a pictures or extras source alone does not say which game it
/// is, so the game counts as found by no source (`fetchError`, tried again
/// by the idle fill, "failed last time"). One that asks only for pictures,
/// by any source.
pub(super) fn found(sources: &SourceRegistry, fields: Option<&[String]>, candidates: &[MetadataCandidate]) -> bool {
    if asks_details(fields) {
        candidates.iter().any(|c| !is_picture_source(sources, &c.source))
    } else {
        !candidates.is_empty()
    }
}

/// [`decide`] with the kinds `sources` gives.
pub(super) fn decide_in(sources: &SourceRegistry, candidates: &[MetadataCandidate], current: Option<&CollectionGameMetadata>) -> Decision {
    decide(candidates, current, &|s| is_picture_source(sources, s))
}

/// The candidates a review entry lists: those of details sources (a
/// pictures or extras source does not decide which game it is).
pub(super) fn review_candidates(sources: &SourceRegistry, candidates: &[MetadataCandidate]) -> Vec<MetadataCandidate> {
    candidates.iter().filter(|c| !is_picture_source(sources, &c.source)).cloned().collect()
}

impl MetadataService {
    /// The fields a fetch with `args` merges: its `only_fields`, else every
    /// `METADATA_FIELDS` entry and the covers of the families this
    /// installation keeps ([`CoverFamilies`]).
    pub(super) fn fetch_fields(&self, args: &MetadataApplyArgs) -> Vec<String> {
        match &args.only_fields {
            Some(fields) => fields.clone(),
            None => {
                METADATA_FIELDS.iter().chain(CoverFamilies::load(&self.paths).kept_extra_fields().iter()).map(|f| f.to_string()).collect()
            }
        }
    }

    /// The candidates the merge may use: those of sources that are on, each
    /// without the fields switched off for its source.
    pub(super) fn usable(settings: &MetadataSettings, candidates: &[MetadataCandidate]) -> Vec<(String, CollectionGameMetadata)> {
        candidates
            .iter()
            .filter(|c| is_on(settings, &c.source))
            .map(|c| {
                let mut md = c.metadata.clone();
                for field in ALL_FIELDS.iter().filter(|f| !is_field_on(settings, &c.source, f)) {
                    clear_field(&mut md, field);
                }
                (c.source.clone(), md)
            })
            .collect()
    }

    /// The order of each field for this game: the settings' orders, with a
    /// source that leads for this game's audience and found it surely
    /// ahead of the sources that found it only by title (now, or the first
    /// time, for a match found again by its id)
    /// ([`SourceRegistry::field_orders_led`](crate::sources::SourceRegistry::field_orders_led)).
    fn game_orders(
        &self,
        sources: &SourceRegistry,
        args: &MetadataApplyArgs,
        current: Option<&CollectionGameMetadata>,
        candidates: &[MetadataCandidate],
        settings: &MetadataSettings,
    ) -> IndexMap<String, Vec<String>> {
        let leads = |c: &&MetadataCandidate| {
            c.matched_by.is_some_and(MatchedBy::is_sure) && sources.get(&c.source).is_some_and(|d| d.leads_for.is_some())
        };
        if !candidates.iter().any(|c| leads(&c)) {
            return sources.field_orders(settings);
        }
        let game = self.source_game(sources, args, current);
        let leaders: Vec<String> = candidates
            .iter()
            .filter(leads)
            .filter(|c| sources.get(&c.source).and_then(|d| d.leads_for.as_ref()).is_some_and(|a| a.matches(&game)))
            .map(|c| c.source.clone())
            .collect();
        // Found by title now, or again by the id a match by title stored
        // (not one the user pinned).
        let title = |by: Option<MatchedBy>| matches!(by, Some(MatchedBy::ExactTitle | MatchedBy::FuzzyTitle));
        let by_title = |c: &&MetadataCandidate| {
            title(c.matched_by)
                || (c.matched_by == Some(MatchedBy::SourceId)
                    && current.and_then(|md| record::source_match(md, &c.source)).is_some_and(|m| !m.pinned && title(m.matched_by)))
        };
        let title_found: Vec<String> =
            candidates.iter().filter(by_title).filter(|c| !leaders.contains(&c.source)).map(|c| c.source.clone()).collect();
        sources.field_orders_led(settings, &leaders, &title_found)
    }

    /// The merge of `candidates` (those to save) into `current`, with the
    /// locked fields kept, the locks of the arguments set, and the matches
    /// of the candidates recorded. It marks the game as fetched only when it
    /// asks for details and a details source of `sources` is among
    /// `candidates` ([`found`]); otherwise the record keeps its
    /// `fetchedAt` and `fetchError`.
    pub(super) fn propose(
        &self,
        sources: &SourceRegistry,
        args: &MetadataApplyArgs,
        current: Option<&CollectionGameMetadata>,
        candidates: &[MetadataCandidate],
    ) -> CollectionGameMetadata {
        let has_picks = args.field_picks.as_ref().is_some_and(|p| !p.is_empty());
        let overwrite = args.overwrite_existing != Some(false);
        let preserve_manual = if has_picks { false } else { args.preserve_manual.unwrap_or(args.overwrite_existing == Some(false)) };
        let settings = self.metadata_settings();
        let priority = with_preferred_source(&self.game_orders(sources, args, current, candidates, &settings), args.source.as_deref());
        let usable = Self::usable(&settings, candidates);
        let merge_candidates: Vec<MergeCandidate<'_>> = usable.iter().map(|(source, md)| MergeCandidate { source, metadata: md }).collect();
        // A field the user picked a source for is merged, kept family or not.
        let mut fields = self.fetch_fields(args);
        for picked in args.field_picks.iter().flat_map(|p| p.keys()) {
            if ALL_FIELDS.contains(&picked.as_str()) && !fields.contains(picked) {
                fields.push(picked.clone());
            }
        }
        let mut proposed = merge_metadata(MergeArgs {
            app_name: &args.app_name,
            runner: &args.runner,
            current,
            candidates: &merge_candidates,
            priority: &priority,
            overwrite_existing: overwrite,
            preserve_manual,
            field_picks: args.field_picks.as_ref(),
            fields: Some(&fields),
        });
        // Names the user renamed, merged or removed in the fields manager.
        NameRules::load(&self.paths).apply_metadata(&mut proposed);
        // The rules above touch every field: those not chosen stay as stored.
        if args.only_fields.is_some() {
            for field in METADATA_FIELDS.iter().filter(|f| !fields.iter().any(|c| c == *f)) {
                copy_field(current, &mut proposed, field);
            }
        }
        // A locked field stays as it is, unless the arguments set its lock
        // (the per-game dialog saves the user's pick, then the lock).
        let named = |f: &str| args.locks.as_ref().is_some_and(|l| l.contains_key(f));
        for field in current.map(record::locked_fields).unwrap_or_default() {
            if ALL_FIELDS.contains(&field.as_str()) && !named(&field) {
                copy_field(current, &mut proposed, &field);
            }
        }
        for (field, locked) in args.locks.iter().flatten() {
            record::set_locked(&mut proposed, field, *locked);
        }
        if current.is_some_and(|c| record::no_match(c).is_some() && !record::marked_no_match(c, &args.title)) {
            record::clear_no_match(&mut proposed);
        }
        record_matches(args, current, candidates, &mut proposed);
        let asked = args.only_fields.as_deref();
        if !(asks_details(asked) && found(sources, asked, candidates)) {
            proposed.fetched_at = current.and_then(|c| c.fetched_at.clone());
            proposed.fetch_error = current.and_then(|c| c.fetch_error.clone());
        }
        proposed
    }

    /// `previewGameMetadata`: what each provider returned and what the merge
    /// would store (what an automatic update saves: only the sure part, see
    /// [`crate::confidence`]). Nothing is written.
    pub fn preview_game_metadata(&self, args: &MetadataApplyArgs) -> Result<MetadataPreview> {
        self.preview(args).map(|(preview, _)| preview)
    }

    /// [`preview_game_metadata`](Self::preview_game_metadata), and what the
    /// gather was asked from and missed.
    fn preview(&self, args: &MetadataApplyArgs) -> Result<(MetadataPreview, Asked)> {
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner;
        let Gathered { candidates, offline, asked, mut states, sources, unreachable } = self.gather(&args);
        let decision = decide_in(&sources, &candidates, current.as_ref());
        for state in &mut states {
            if let SourceAnswer::Found { used, .. } = &mut state.answer {
                *used = decision.used.iter().any(|c| c.source == state.source);
            }
        }
        let proposed = self.propose(&sources, &args, current.as_ref(), &decision.used);
        let preview = MetadataPreview {
            app_name: args.app_name.clone(),
            locks: current.as_ref().map(record::locked_fields).unwrap_or_default(),
            current,
            proposed,
            candidates,
            igdb_configured: self.igdb_configured(),
            offline,
            sources: states,
            review: decision.review,
        };
        let used = !decision.used.is_empty();
        Ok((preview, Asked { any: asked, sources, unreachable, used }))
    }

    /// `persistImages`: every picture of the record (each cover family and
    /// the background).
    pub(super) fn persist_images(&self, md: CollectionGameMetadata, download: bool) -> CollectionGameMetadata {
        self.persist_chosen_images(md, download, &ArtKind::ALL)
    }

    /// `persistImages` of the pictures in `kinds` only.
    pub(super) fn persist_chosen_images(
        &self,
        mut md: CollectionGameMetadata,
        download: bool,
        kinds: &[ArtKind],
    ) -> CollectionGameMetadata {
        for kind in kinds {
            let url = kind.of_record(&md).map(str::to_owned);
            *kind.of_record_mut(&mut md) =
                materialize_metadata_image(&self.paths, &self.http, &md.runner, &md.app_name, *kind, url.as_deref(), download);
        }
        md
    }

    /// `applyGameMetadata`: fetch, merge and store, by the rule of the
    /// module docs. When no provider answers ([`found`]: a pictures source
    /// alone does not count) the record is stored with `fetchError` so the
    /// idle fill retries after a week, with the sure pictures found beside;
    /// a fetch of pictures only stores nothing then. When a source that
    /// could have found it did not answer, nothing is stored
    /// ([`SourceUnreachable`]), nor when none could be reached at all
    /// ([`Offline`]): local data alone must not count as a fetch; nor when
    /// none was asked (`NoSourceAsked`: each is off or not for the game).
    /// When the sources are not sure which game it is, nothing is stored and
    /// the game waits in the review queue ([`SentToReview`]).
    pub fn apply_game_metadata(&self, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        let (preview, asked) = self.preview(args)?;
        let runner = as_runner(&args.runner);
        if preview.offline {
            return Err(Offline.into());
        }
        if !asked.any {
            return Err(NoSourceAsked.into());
        }
        let fields = args.only_fields.as_deref();
        let download = || args.download_images.unwrap_or(self.metadata_settings().download_images);
        if !found(&asked.sources, fields, &preview.candidates) {
            if asked.missed(fields) {
                return Err(SourceUnreachable.into());
            }
            if !asks_details(fields) {
                bail!(NO_DATA);
            }
            // What is used here is the sure pictures alone ([`decide`]).
            let mut failed = if asked.used {
                self.persist_images(preview.proposed, download())
            } else {
                current_or_blank(&self.paths, runner, &args.app_name)?
            };
            mark_not_found(&mut failed, &args.title);
            return upsert_game_metadata(&self.paths, failed);
        }
        if let Some(reason) = preview.review {
            let candidates = review_candidates(&asked.sources, &preview.candidates);
            self.queue_review(ReviewEntry::new(&args.app_name, runner, &args.title, reason, &candidates))?;
            return Err(SentToReview(reason).into());
        }
        let saved = upsert_game_metadata(&self.paths, self.persist_images(preview.proposed, download()))?;
        self.unqueue(&metadata_key(runner, &args.app_name))?;
        Ok(saved)
    }

    /// Stores what the user saw in `preview` without asking the providers
    /// again: the preview's candidates are merged with `args` (`field_picks`,
    /// `pins`, `locks` and the merge options) over the saved record. The
    /// candidates an automatic update would leave out are used only for the
    /// fields picked from them: their match is not recorded (an unsure guess
    /// would be looked up as sure from then on), unless the user pinned it.
    /// A fetch by hand: the game leaves the review queue and loses its "in
    /// no source" mark. Nothing is written, and an error returned, when the
    /// preview holds no online data.
    pub fn apply_preview(&self, preview: &MetadataPreview, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        if preview.offline {
            return Err(Offline.into());
        }
        if preview.candidates.is_empty() {
            bail!(tr!("No metadata sources returned data"));
        }
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner;
        let sources = self.sources();
        let decision = decide_in(&sources, &preview.candidates, current.as_ref());
        let pinned: HashSet<&str> =
            args.pins.iter().flat_map(|p| p.iter().filter(|(_, id)| id.is_some()).map(|(s, _)| s.as_str())).collect();
        let picked: HashSet<&str> = args.field_picks.iter().flat_map(|p| p.values()).map(String::as_str).collect();
        let matched = |c: &MetadataCandidate| pinned.contains(c.source.as_str()) || decision.used.iter().any(|u| u.source == c.source);
        let used: Vec<MetadataCandidate> = preview
            .candidates
            .iter()
            .filter(|c| matched(c) || picked.contains(c.source.as_str()))
            .map(|c| if matched(c) { c.clone() } else { picked_only(c) })
            .collect();
        let mut proposed = self.propose(&sources, &args, current.as_ref(), &used);
        record::clear_no_match(&mut proposed);
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let saved = upsert_game_metadata(&self.paths, self.persist_images(proposed, download))?;
        self.unqueue(&metadata_key(&args.runner, &args.app_name))?;
        Ok(saved)
    }

    /// Locks or unlocks fields of a stored record (`field -> locked`),
    /// without fetching. Creates no record.
    pub fn set_field_locks(&self, runner: &str, app_name: &str, locks: &IndexMap<String, bool>) -> Result<Option<CollectionGameMetadata>> {
        let key = metadata_key(as_runner(runner), app_name);
        json_store::update::<heronite_data::models::MetadataFile, _>(&self.paths.metadata(), |file| {
            let Some(md) = file.games.get_mut(&key) else { return Ok(None) };
            for (field, locked) in locks {
                record::set_locked(md, field, *locked);
            }
            Ok(Some(md.clone()))
        })
    }
}

/// `c` for the fields picked from it alone: without its match and the ids
/// the merge would copy into the record (IGDB, Lutris, Steam).
fn picked_only(c: &MetadataCandidate) -> MetadataCandidate {
    let mut c = c.clone();
    c.match_id = None;
    c.metadata.igdb_id = None;
    c.metadata.igdb_slug = None;
    c.metadata.lutris_slug = None;
    c.metadata.steam_app_id = None;
    c
}

/// Records which game of its source each candidate with an id is. A match
/// found again by its id keeps how it was first found; the arguments' pins
/// set or clear the pin.
fn record_matches(
    args: &MetadataApplyArgs,
    current: Option<&CollectionGameMetadata>,
    candidates: &[MetadataCandidate],
    proposed: &mut CollectionGameMetadata,
) {
    for c in candidates {
        let Some(id) = c.match_id.clone() else { continue };
        let before = current.and_then(|md| record::source_match(md, &c.source)).filter(|m| m.id == id);
        let pinned = match args.pins.as_ref().and_then(|p| p.get(&c.source)) {
            Some(pin) => pin.as_deref() == Some(id.as_str()),
            None => before.as_ref().is_some_and(|m| m.pinned),
        };
        let matched_by = match &before {
            Some(m) if c.matched_by == Some(MatchedBy::SourceId) && m.matched_by.is_some() => m.matched_by,
            _ => c.matched_by,
        };
        let m = SourceMatch { id, title: c.match_name.clone(), year: c.match_year, matched_by, pinned };
        record::set_source_match(proposed, &c.source, m, Some(&c.metadata));
    }
    for (source, pin) in args.pins.iter().flatten() {
        if pin.is_none() {
            record::unpin(proposed, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blank_metadata;

    fn guess(source: &str, title: &str, year: &str) -> MetadataCandidate {
        let mut md = blank_metadata("a", "sideload");
        md.title = Some(title.into());
        md.release_date = Some(year.into());
        MetadataCandidate::new(source, md, Some(MatchedBy::FuzzyTitle))
    }

    fn decide_details(candidates: &[MetadataCandidate], current: Option<&CollectionGameMetadata>) -> Decision {
        decide(candidates, current, &|s| s == "steamgriddb")
    }

    #[test]
    fn a_pictures_source_never_sends_a_game_to_review() {
        let picture = |title: &str, by: MatchedBy| {
            let mut c = guess("steamgriddb", title, "2011");
            c.matched_by = Some(by);
            c
        };
        // Alone and unsure: left out, not asked.
        let alone = decide_details(&[picture("Portal 2", MatchedBy::FuzzyTitle)], None);
        assert_eq!((alone.used.len(), alone.review), (0, None));
        // Alone and sure (its Steam id): used.
        assert_eq!(decide_details(&[picture("Portal 2", MatchedBy::SteamId)], None).used.len(), 1);
        // A guess that is the game a details source is sure of: used; another game: left out.
        let mut steam = guess("steam", "Portal 2", "2011");
        steam.matched_by = Some(MatchedBy::SteamId);
        let d = decide_details(&[steam.clone(), picture("Portal 2", MatchedBy::FuzzyTitle)], None);
        assert_eq!(d.used.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["steam", "steamgriddb"]);
        let d = decide_details(&[steam, picture("Portal Knights", MatchedBy::FuzzyTitle)], None);
        assert_eq!(d.used.len(), 1);
        // A lone details guess still goes to review, the picture with it.
        let d = decide_details(&[guess("lutris", "Doom 64", "1997"), picture("Doom 64", MatchedBy::ExactTitle)], None);
        assert_eq!((d.used.len(), d.review), (0, Some(ReviewReason::LoneGuess)));
    }

    #[test]
    fn a_record_from_before_anchors_a_source_asked_alone() {
        let mut old = blank_metadata("a", "sideload");
        old.igdb_id = Some(heronite_data::num::JsNum(700.0));
        old.title = Some("The Witcher 3: Wild Hunt".into());
        old.release_date = Some("2015-05-19".into());
        let same = decide_details(&[guess("lutris", "The Witcher 3: Wild Hunt", "2015")], Some(&old));
        assert_eq!((same.used.len(), same.review), (1, None));
        let other = decide_details(&[guess("lutris", "Doom 64", "1997")], Some(&old));
        assert_eq!((other.used.len(), other.review), (0, None), "left out, not asked");
        // No record: a lone guess is asked.
        assert_eq!(decide_details(&[guess("lutris", "Doom 64", "1997")], None).review, Some(ReviewReason::LoneGuess));
        // A record without any match does not anchor.
        assert_eq!(
            decide_details(&[guess("lutris", "Doom 64", "1997")], Some(&blank_metadata("a", "sideload"))).review,
            Some(ReviewReason::LoneGuess)
        );
    }
}
