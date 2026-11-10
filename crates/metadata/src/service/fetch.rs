//! Fetching one game: asking the sources ([`MetadataService::gather`]),
//! keeping what is sure enough to save ([`decide`], the rule of
//! [`crate::confidence`]), the merge a preview proposes (with the record's
//! locks and per-source matches), and the saves of a fetch and of the
//! per-game dialog.

use std::collections::HashSet;

use anyhow::{bail, Result};
use heronite_data::field_names::NameRules;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::settings::METADATA_FIELDS;
use heronite_i18n::tr;
use indexmap::IndexMap;

use super::{MetadataService, Offline, NO_DATA};
use crate::confidence::{judge, Guess, Verdict};
use crate::images::materialize_metadata_image;
use crate::merge::{clear_field, copy_field, merge_metadata, with_preferred_source, MergeArgs, MergeCandidate};
use crate::record::{self, SourceMatch};
use crate::review::{ReviewEntry, ReviewReason};
use crate::sources::{is_field_on, is_on, Needs, SourceContext, SourceDescriptor, SourceGame};
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

/// The reason, when `err` is [`SentToReview`].
pub fn is_sent_to_review(err: &anyhow::Error) -> Option<ReviewReason> {
    err.downcast_ref::<SentToReview>().map(|s| s.0)
}

/// What the providers returned and whether the online ones answered at all.
pub(super) struct Gathered {
    pub(super) candidates: Vec<MetadataCandidate>,
    pub(super) offline: bool,
    /// Every source with a lookup, in registry order (hidden ones only when
    /// they found the game).
    pub(super) states: Vec<SourceState>,
}

/// What of a game's candidates may be saved.
pub(super) struct Decision {
    pub(super) used: Vec<MetadataCandidate>,
    /// Set when nothing may: the game goes to review.
    pub(super) review: Option<ReviewReason>,
}

/// [`judge`] over `candidates`, anchored by the matches `current` has of
/// sources that did not answer this time: the sure ones, and those of
/// records from before matches were recorded (an IGDB id or a Lutris slug
/// alone), named and dated as the record is.
pub(super) fn decide(candidates: &[MetadataCandidate], current: Option<&CollectionGameMetadata>) -> Decision {
    let guesses: Vec<Guess<'_>> =
        candidates.iter().map(|c| Guess { matched_by: c.matched_by, title: c.match_name.as_deref(), year: c.match_year }).collect();
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
        Verdict::Save(used) => Decision { used: used.into_iter().map(|i| candidates[i].clone()).collect(), review: None },
        Verdict::Review(reason) => Decision { used: Vec::new(), review: Some(reason) },
    }
}

/// The games of each source the lookups must use: the record's pinned
/// ones, changed by the arguments' `pins`.
fn pins(args: &MetadataApplyArgs, current: Option<&CollectionGameMetadata>) -> IndexMap<String, String> {
    let mut pins = current.map(record::pinned_ids).unwrap_or_default();
    for (source, pin) in args.pins.iter().flatten() {
        match pin.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => pins.insert(source.clone(), id.to_owned()),
            None => pins.shift_remove(source),
        };
    }
    pins
}

impl MetadataService {
    /// `game` as the sources see it. The overlay is read only when needed:
    /// for the Steam id the arguments lack, or for a source's audience.
    pub(super) fn source_game(&self, args: &MetadataApplyArgs) -> SourceGame {
        let has_steam_id = args.steam_app_id.as_deref().is_some_and(|s| !s.is_empty());
        let overlay = (!has_steam_id || self.sources.needs_overlay())
            .then(|| json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok())
            .flatten()
            .and_then(|mut file| file.games.shift_remove(&args.app_name));
        SourceGame::new(args, overlay.as_ref())
    }

    /// `gatherCandidates`: asks every provider that is on and applies to the
    /// game at once (IGDB calls queue among themselves) and keeps those that
    /// answered, in provider order.
    pub fn gather_candidates(&self, game: &MetadataApplyArgs) -> Vec<MetadataCandidate> {
        self.gather(game).candidates
    }

    /// Asks the sources. A source with a pinned game is asked for that game
    /// (whatever its audience); IGDB and Lutris otherwise start from the id
    /// the record has, unless the arguments unpin them.
    pub(super) fn gather(&self, game: &MetadataApplyArgs) -> Gathered {
        let current = get_game_metadata(&self.paths, as_runner(&game.runner), &game.app_name).ok().flatten();
        let pins = pins(game, current.as_ref());
        let unpinned = |source: &str| game.pins.as_ref().is_some_and(|p| p.get(source).is_some_and(Option::is_none));
        let mut with_ids = game.clone();
        with_ids.igdb_id =
            game.igdb_id.or_else(|| current.as_ref().and_then(|c| c.igdb_id).map(|n| n.get() as i64).filter(|_| !unpinned("igdb")));
        with_ids.lutris_slug =
            game.lutris_slug.clone().or_else(|| current.as_ref().and_then(|c| c.lutris_slug.clone()).filter(|_| !unpinned("lutris")));
        let game_view = self.source_game(&with_ids);
        let ctx = SourceContext::new(&self.paths, &self.http);
        let settings = ctx.settings();
        let lookups: Vec<(&SourceDescriptor, Option<NotAsked>, Lookup<MetadataCandidate>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .sources
                .lookups()
                .map(|source| {
                    let (ctx, game_view) = (&ctx, &game_view);
                    let d = source.descriptor();
                    let pin = pins.get(&d.id).filter(|_| d.searchable);
                    let why = if !is_on(settings, &d.id) {
                        Some(NotAsked::Off)
                    } else if !Self::wanted(d, game) {
                        Some(NotAsked::NotChosen)
                    } else if pin.is_none() && !d.applies_to.matches(game_view) {
                        Some(NotAsked::NotForThisGame)
                    } else {
                        None
                    };
                    let handle = why.is_none().then(|| {
                        scope.spawn(move || match pin {
                            Some(id) => source.lookup_id(ctx, game_view, id),
                            None => source.lookup(ctx, game_view),
                        })
                    });
                    (d, why, handle)
                })
                .collect();
            handles
                .into_iter()
                .map(|(source, why, h)| match h.map(|h| h.join()) {
                    None => (source, why, Lookup::Skipped),
                    Some(Ok(l)) => (source, why, l),
                    Some(Err(_)) => {
                        log::warn!("Metadata source {} failed for {}: panicked", source.id, game.title);
                        (source, why, Lookup::NotFound)
                    }
                })
                .collect()
        });
        // Offline: every online provider that was asked failed to connect.
        let mut asked = lookups.iter().filter(|(s, _, l)| s.network && !matches!(l, Lookup::Skipped)).peekable();
        let offline = asked.peek().is_some() && asked.all(|(_, _, l)| matches!(l, Lookup::Unreachable));
        let mut candidates = Vec::new();
        let mut states = Vec::new();
        for (d, why, lookup) in lookups {
            let answer = match lookup {
                Lookup::Found(c) => {
                    let answer = SourceAnswer::Found { hit: c.hit(), pinned: pins.contains_key(&d.id) && d.searchable, used: false };
                    candidates.push(c);
                    answer
                }
                Lookup::NotFound => SourceAnswer::NotFound,
                Lookup::Unreachable => SourceAnswer::Unreachable,
                Lookup::Skipped => SourceAnswer::NotAsked {
                    why: why.unwrap_or(if d.needs == Needs::Nothing { NotAsked::NotForThisGame } else { NotAsked::NeedsKey }),
                },
            };
            if !d.hidden || matches!(answer, SourceAnswer::Found { .. }) {
                states.push(SourceState { source: d.id.clone(), answer });
            }
        }
        Gathered { candidates, offline, states }
    }

    /// Whether the arguments' `only_sources` and `only_fields` leave `d` to ask.
    fn wanted(d: &SourceDescriptor, args: &MetadataApplyArgs) -> bool {
        args.only_sources.as_ref().is_none_or(|s| s.contains(&d.id))
            && args.only_fields.as_ref().is_none_or(|f| f.iter().any(|field| d.provides(field)))
    }

    /// The candidates the merge may use: those of sources that are on, each
    /// without the fields switched off for its source.
    pub(super) fn usable(settings: &MetadataSettings, candidates: &[MetadataCandidate]) -> Vec<(String, CollectionGameMetadata)> {
        candidates
            .iter()
            .filter(|c| is_on(settings, &c.source))
            .map(|c| {
                let mut md = c.metadata.clone();
                for field in METADATA_FIELDS.iter().filter(|f| !is_field_on(settings, &c.source, f)) {
                    clear_field(&mut md, field);
                }
                (c.source.clone(), md)
            })
            .collect()
    }

    /// The merge of `candidates` (those to save) into `current`, with the
    /// locked fields kept, the locks of the arguments set, and the matches
    /// of the candidates recorded.
    pub(super) fn propose(
        &self,
        args: &MetadataApplyArgs,
        current: Option<&CollectionGameMetadata>,
        candidates: &[MetadataCandidate],
    ) -> CollectionGameMetadata {
        let has_picks = args.field_picks.as_ref().is_some_and(|p| !p.is_empty());
        let overwrite = args.overwrite_existing != Some(false);
        let preserve_manual = if has_picks { false } else { args.preserve_manual.unwrap_or(args.overwrite_existing == Some(false)) };
        let settings = self.metadata_settings();
        let priority = with_preferred_source(&self.sources.field_orders(&settings), args.source.as_deref());
        let usable = Self::usable(&settings, candidates);
        let merge_candidates: Vec<MergeCandidate<'_>> = usable.iter().map(|(source, md)| MergeCandidate { source, metadata: md }).collect();
        let mut proposed = merge_metadata(MergeArgs {
            app_name: &args.app_name,
            runner: &args.runner,
            current,
            candidates: &merge_candidates,
            priority: &priority,
            overwrite_existing: overwrite,
            preserve_manual,
            field_picks: args.field_picks.as_ref(),
            fields: args.only_fields.as_deref(),
        });
        // Names the user renamed, merged or removed in the fields manager.
        NameRules::load(&self.paths).apply_metadata(&mut proposed);
        // The rules above touch every field: those not chosen stay as stored.
        if let Some(fields) = &args.only_fields {
            for field in METADATA_FIELDS.iter().filter(|f| !fields.iter().any(|c| c == *f)) {
                copy_field(current, &mut proposed, field);
            }
        }
        // A locked field stays as it is, unless the arguments set its lock
        // (the per-game dialog saves the user's pick, then the lock).
        let named = |f: &str| args.locks.as_ref().is_some_and(|l| l.contains_key(f));
        for field in current.map(record::locked_fields).unwrap_or_default() {
            if METADATA_FIELDS.contains(&field.as_str()) && !named(&field) {
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
        proposed
    }

    /// `previewGameMetadata`: what each provider returned and what the merge
    /// would store (what an automatic update saves: only the sure part, see
    /// [`crate::confidence`]). Nothing is written.
    pub fn preview_game_metadata(&self, args: &MetadataApplyArgs) -> Result<MetadataPreview> {
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner;
        let Gathered { candidates, offline, mut states } = self.gather(&args);
        let decision = decide(&candidates, current.as_ref());
        for state in &mut states {
            if let SourceAnswer::Found { used, .. } = &mut state.answer {
                *used = decision.used.iter().any(|c| c.source == state.source);
            }
        }
        let proposed = self.propose(&args, current.as_ref(), &decision.used);
        Ok(MetadataPreview {
            app_name: args.app_name.clone(),
            locks: current.as_ref().map(record::locked_fields).unwrap_or_default(),
            current,
            proposed,
            candidates,
            igdb_configured: self.igdb_configured(),
            offline,
            sources: states,
            review: decision.review,
        })
    }

    /// `persistImages`.
    pub(super) fn persist_images(&self, md: CollectionGameMetadata, download: bool) -> CollectionGameMetadata {
        self.persist_chosen_images(md, download, &[ArtKind::Cover, ArtKind::Hero])
    }

    /// `persistImages` of the pictures in `kinds` only.
    pub(super) fn persist_chosen_images(
        &self,
        mut md: CollectionGameMetadata,
        download: bool,
        kinds: &[ArtKind],
    ) -> CollectionGameMetadata {
        if kinds.contains(&ArtKind::Cover) {
            md.cover_url = materialize_metadata_image(
                &self.paths,
                &self.http,
                &md.runner,
                &md.app_name,
                ArtKind::Cover,
                md.cover_url.as_deref(),
                download,
            );
        }
        if kinds.contains(&ArtKind::Hero) {
            md.hero_url = materialize_metadata_image(
                &self.paths,
                &self.http,
                &md.runner,
                &md.app_name,
                ArtKind::Hero,
                md.hero_url.as_deref(),
                download,
            );
        }
        md
    }

    /// `applyGameMetadata`: fetch, merge and store. When no provider answers
    /// the record is stored with `fetchError` so the idle fill retries after
    /// a week. When none could be reached at all nothing is stored
    /// ([`Offline`]): local data alone must not count as a fetch. When the
    /// sources are not sure which game it is, nothing is stored and the game
    /// waits in the review queue ([`SentToReview`]).
    pub fn apply_game_metadata(&self, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        let preview = self.preview_game_metadata(args)?;
        let runner = as_runner(&args.runner);
        if preview.offline {
            return Err(Offline.into());
        }
        if preview.candidates.is_empty() {
            let mut failed = current_or_blank(&self.paths, runner, &args.app_name)?;
            failed.fetch_error = Some(NO_DATA.into());
            failed.fetched_at = Some(now_iso());
            return upsert_game_metadata(&self.paths, failed);
        }
        if let Some(reason) = preview.review {
            self.queue_review(ReviewEntry::new(&args.app_name, runner, &args.title, reason, &preview.candidates))?;
            return Err(SentToReview(reason).into());
        }
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let saved = upsert_game_metadata(&self.paths, self.persist_images(preview.proposed, download))?;
        self.unqueue(&metadata_key(runner, &args.app_name))?;
        Ok(saved)
    }

    /// Stores what the user saw in `preview` without asking the providers
    /// again: the preview's candidates are merged with `args` (`field_picks`,
    /// `pins`, `locks` and the merge options) over the saved record. The
    /// candidates an automatic update would leave out are used only for the
    /// fields picked from them. A fetch by hand: the game leaves the review
    /// queue and loses its "in no source" mark. Nothing is written, and an
    /// error returned, when the preview holds no online data.
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
        let decision = decide(&preview.candidates, current.as_ref());
        let chosen: HashSet<&str> = args
            .field_picks
            .iter()
            .flat_map(|p| p.values())
            .chain(args.pins.iter().flat_map(|p| p.iter().filter(|(_, id)| id.is_some()).map(|(s, _)| s)))
            .map(String::as_str)
            .collect();
        let used: Vec<MetadataCandidate> = preview
            .candidates
            .iter()
            .filter(|c| chosen.contains(c.source.as_str()) || decision.used.iter().any(|u| u.source == c.source))
            .cloned()
            .collect();
        let mut proposed = self.propose(&args, current.as_ref(), &used);
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

    #[test]
    fn a_record_from_before_anchors_a_source_asked_alone() {
        let mut old = blank_metadata("a", "sideload");
        old.igdb_id = Some(heronite_data::num::JsNum(700.0));
        old.title = Some("The Witcher 3: Wild Hunt".into());
        old.release_date = Some("2015-05-19".into());
        let same = decide(&[guess("lutris", "The Witcher 3: Wild Hunt", "2015")], Some(&old));
        assert_eq!((same.used.len(), same.review), (1, None));
        let other = decide(&[guess("lutris", "Doom 64", "1997")], Some(&old));
        assert_eq!((other.used.len(), other.review), (0, None), "left out, not asked");
        // No record: a lone guess is asked.
        assert_eq!(decide(&[guess("lutris", "Doom 64", "1997")], None).review, Some(ReviewReason::LoneGuess));
        // A record without any match does not anchor.
        assert_eq!(
            decide(&[guess("lutris", "Doom 64", "1997")], Some(&blank_metadata("a", "sideload"))).review,
            Some(ReviewReason::LoneGuess)
        );
    }
}
