//! One bulk job, game by game: what it asks for ([`Plan`], from the
//! arguments and the settings), the Steam tags pass, each game's fetch,
//! merge and write, and its result.
//!
//! The modes map onto the merge's options ([`UpdateMode::merge_flags`]):
//! Fill = no overwrite (empty fields only) and keep manual; Refresh =
//! overwrite, keep manual; Replace = overwrite everything. The fields not
//! chosen go to the merge as left out (`MergeArgs::fields`) and the sources
//! not chosen are not asked.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use heronite_data::cover_family::{CoverFamilies, CoverFamily};
use heronite_data::focus::steam_language;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, MetadataSettings};
use heronite_data::settings::{ALL_FIELDS, METADATA_FIELDS};
use heronite_data::steam_tags::SteamTagsFile;
use indexmap::IndexMap;

use super::bulk::initial;
use super::counts::{missing_details, tag_steam_id, waits_for_details};
use super::fetch::{asks_details, decide_in, found, mark_not_found, review_candidates, SourceUnreachable};
use super::gather::Gathered;
use super::undo::{clear_backups, Pictures, UndoLog};
use super::{tag_fill, MetadataService, Offline, NO_DATA};
use crate::job::{FieldChange, GameOutcome, GameResult, JobKind, NoChangeReason, SkipReason, UpdateMode};
use crate::merge::{has_field_value, read_field};
use crate::record;
use crate::review::ReviewEntry;
use crate::sources::{extra_fields, ids, is_on};
use crate::steam_tags;
use crate::store::{as_runner, get_all_game_metadata, get_game_metadata, upsert_game_metadata};
use crate::types::{ArtKind, BulkArgs, BulkItem, BulkProgress, BulkState, MetadataApplyArgs};
use crate::util::{blank_metadata, now_ms};

type Records = anyhow::Result<IndexMap<String, CollectionGameMetadata>>;

/// A job's arguments resolved against the settings.
struct Plan {
    mode: Option<UpdateMode>,
    overwrite: bool,
    preserve_manual: bool,
    /// The older "Skip games that already have metadata".
    skip_fetched: bool,
    /// The chosen `ALL_FIELDS`, in that order.
    fields: Vec<&'static str>,
    /// Those some chosen source can fill.
    fillable: Vec<&'static str>,
    /// `MetadataApplyArgs::only_fields`: `None` when every field is chosen.
    only_fields: Option<Vec<String>>,
    /// `None` when every source that is on may be asked.
    only_sources: Option<Vec<String>>,
    /// Steam tags are chosen and on.
    tags: bool,
    /// Every field from every source: a game no source has is marked as
    /// failed in its record (`fetchError`), as a fetch of the game does.
    full: bool,
    download: bool,
    pictures: Vec<ArtKind>,
    /// A family's download: the games (`runner:appName`) with a picture of
    /// it set by hand, which need none.
    custom: HashSet<String>,
}

/// What the Steam tags pass did, by Steam app id.
#[derive(Default)]
struct TagPass {
    asked: HashSet<String>,
    changed: HashSet<String>,
    unreachable: bool,
}

/// What the details part asks a game for.
struct Ask {
    /// `MetadataApplyArgs::only_fields`.
    fields: Option<Vec<String>>,
    /// Every source for every field: a game none has is marked as failed.
    full: bool,
}

/// What the details part did with a game.
struct Details {
    outcome: GameOutcome,
    changes: Vec<FieldChange>,
    /// The older progress' error line.
    error: Option<String>,
}

impl Details {
    fn failed(game: &BulkItem, err: impl std::fmt::Display) -> Self {
        let reason = err.to_string();
        Self { error: Some(format!("{}: {reason}", game.title)), outcome: GameOutcome::Failed(reason), changes: Vec::new() }
    }
}

/// The chosen fields whose value differs between `before` and `after`,
/// with the source of the new value. A picture copied to disk keeps its
/// URL when the job replaces it: those in `rewritten` changed too.
fn changed_fields(
    before: Option<&CollectionGameMetadata>,
    after: &CollectionGameMetadata,
    fields: &[&str],
    rewritten: &[&str],
) -> Vec<FieldChange> {
    fields
        .iter()
        .filter(|f| {
            rewritten.contains(f)
                || ((has_field_value(before, f) || has_field_value(Some(after), f)) && read_field(before, f) != read_field(Some(after), f))
        })
        .map(|f| FieldChange { field: f.to_string(), source: after.field_sources.get(*f).cloned().unwrap_or_default() })
        .collect()
}

impl MetadataService {
    fn plan(&self, args: &BulkArgs, settings: &MetadataSettings) -> Plan {
        let (overwrite, preserve_manual) = match args.mode {
            Some(mode) => mode.merge_flags(),
            None => (args.overwrite_existing == Some(true), args.preserve_manual != Some(false)),
        };
        // Every field by default: `METADATA_FIELDS` and the kept families'
        // covers. A family no view uses and the user does not keep is never
        // filled, unless the job is the download of that family.
        let families = CoverFamilies::load(&self.paths);
        let every: Vec<&'static str> = METADATA_FIELDS.iter().copied().chain(families.kept_extra_fields()).collect();
        let allowed = |f: &str| match CoverFamily::from_field(f) {
            Some(family) if family != CoverFamily::Tall => families.is_kept(family) || args.kind == JobKind::Covers(family),
            _ => true,
        };
        let chosen = |f: &str| match &args.fields {
            Some(list) => list.iter().any(|c| c == f),
            None => every.contains(&f),
        };
        let fields: Vec<&'static str> = ALL_FIELDS.into_iter().filter(|f| chosen(f) && allowed(f)).collect();
        let only_fields = (fields != every).then(|| fields.iter().map(|f| f.to_string()).collect());
        // Naming every details source offered is asking every source.
        let offered: Vec<String> =
            self.bulk_sources(settings).into_iter().filter(|d| self.sources().has_lookup(&d.id)).map(|d| d.id).collect();
        let only_sources = args.sources.clone().filter(|list| !offered.iter().all(|id| list.contains(id)));
        let tags = args.fields.as_ref().is_some_and(|list| list.iter().any(|f| f == extra_fields::STEAM_TAGS))
            && is_on(settings, ids::STEAM_TAGS)
            && args.sources.as_ref().is_none_or(|list| list.iter().any(|s| s == ids::STEAM_TAGS));
        let pictures = ArtKind::ALL.into_iter().filter(|k| fields.contains(&k.as_str())).collect();
        Plan {
            mode: args.mode,
            overwrite,
            preserve_manual,
            skip_fetched: args.mode.is_none() && !overwrite,
            fillable: self.fillable_fields(settings, &fields, only_sources.as_deref()),
            fields,
            full: only_fields.is_none() && only_sources.is_none(),
            only_fields,
            only_sources,
            tags,
            download: args.download_images.unwrap_or(settings.download_images),
            pictures,
            custom: match args.kind {
                JobKind::Covers(family) => self.custom_art_of(ArtKind::cover(family)),
                _ => HashSet::new(),
            },
        }
    }

    /// The games (`runner:appName`) with a picture of `kind` set by hand.
    fn custom_art_of(&self, kind: ArtKind) -> HashSet<String> {
        let art: heronite_data::models::ArtFile = heronite_data::json_store::read(&self.paths.art()).unwrap_or_default();
        art.games.into_iter().filter(|(_, a)| kind.of_art(a).is_some_and(|u| !u.is_empty())).map(|(k, _)| k).collect()
    }

    /// `runBulkMetadata`: runs job `id` (the caller owns the queue) and
    /// returns its final progress, for the caller to publish with `finish`.
    pub(super) fn run_job(&self, id: u64, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        {
            let _gate = self.undo_gate.lock();
            self.jobs.lock().undo = None;
            clear_backups(&self.paths);
        }
        let mut p = initial(id, args);
        self.publish(&mut p, on_progress);
        let total = args.games.len();
        if total == 0 {
            return BulkProgress { state: BulkState::Done, percent: 100, finished_at_ms: now_ms(), ..p };
        }
        let mut undo = UndoLog::new(id);
        let mut errors: Vec<String> = Vec::new();
        // A panic (in builds that unwind) ends the job as a stop does: the
        // games done keep their results and their undo, the others are
        // listed as stopped.
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            self.wait_for_idle_fill();
            let settings = self.metadata_settings();
            let plan = self.plan(args, &settings);
            // One read for the decisions that need no fetch (skip, nothing
            // missing); writes are mirrored into it.
            let mut known = get_all_game_metadata(&self.paths);
            let tags = if plan.tags { self.tag_pass(id, &plan, &args.games, &known) } else { TagPass::default() };

            for (index, game) in args.games.iter().enumerate() {
                if self.stopping(id) {
                    break;
                }
                p.processed = index;
                p.percent = (index as f64 / total as f64 * 100.0).round() as u32;
                p.current_title = Some(game.title.clone());
                p.errors = errors.iter().take(8).cloned().collect();
                self.publish(&mut p, on_progress);
                let (result, error) = self.job_game(&plan, game, &mut known, &mut undo, &tags);
                match &result.outcome {
                    GameOutcome::Updated | GameOutcome::NoChange(NoChangeReason::NothingNew) => p.updated += 1,
                    GameOutcome::NoChange(_) | GameOutcome::Skipped(_) | GameOutcome::ToReview(_) => p.skipped += 1,
                    GameOutcome::NotFound | GameOutcome::Offline | GameOutcome::Failed(_) => p.failed += 1,
                }
                errors.extend(error);
                p.results.push(Arc::new(result));
            }
        }));
        if run.is_err() {
            log::error!("Metadata job {id} stopped: it panicked");
        }
        for game in &args.games[p.results.len()..] {
            p.results.push(Arc::new(result_of(game, GameOutcome::Skipped(SkipReason::Stopped), Vec::new())));
        }

        if !undo.is_empty() {
            self.jobs.lock().undo = Some(undo);
        }
        let processed = p.updated + p.skipped + p.failed;
        BulkProgress {
            state: BulkState::Done,
            processed,
            percent: (processed as f64 / total as f64 * 100.0).round() as u32,
            current_title: None,
            errors: errors.into_iter().take(8).collect(),
            finished_at_ms: now_ms(),
            ..p
        }
    }

    /// Fetches the Steam tags the job's games need, all at once (50 per
    /// request): Fill asks the games with none, the other modes every game.
    fn tag_pass(&self, id: u64, plan: &Plan, games: &[BulkItem], known: &Records) -> TagPass {
        let records = known.as_ref().ok();
        let before = SteamTagsFile::load(&self.paths);
        let mut seen = HashSet::new();
        let todo: Vec<String> = games
            .iter()
            .filter_map(|g| tag_steam_id(g, records.and_then(|r| r.get(&metadata_key(as_runner(&g.runner), &g.app_name)))))
            .filter(|steam_id| plan.overwrite || !before.apps.contains_key(steam_id))
            .filter(|steam_id| seen.insert(steam_id.clone()))
            .collect();
        if todo.is_empty() {
            return TagPass::default();
        }
        let Some(_running) = tag_fill::Running::take() else {
            log::info!("Steam tags of the job skipped: another refresh holds them");
            return TagPass::default();
        };
        let language = steam_language(heronite_i18n::current().code());
        let refresh = if plan.overwrite { steam_tags::refresh_all } else { steam_tags::refresh };
        let unreachable = match refresh(&self.paths, &self.http, &todo, language, steam_tags::GAP, &|| self.stopping(id)) {
            Ok(r) => r.unreachable,
            Err(e) => {
                log::warn!("Steam tags of the job: {e:#}");
                false
            }
        };
        let after = SteamTagsFile::load(&self.paths);
        let changed = todo
            .iter()
            .filter(|i| after.apps.get(*i).is_some_and(|a| !a.tags.is_empty() && Some(&a.tags) != before.apps.get(*i).map(|b| &b.tags)))
            .cloned()
            .collect();
        TagPass { asked: todo.into_iter().collect(), changed, unreachable }
    }

    /// One game of the job: its result and the older progress' error line.
    fn job_game(
        &self,
        plan: &Plan,
        game: &BulkItem,
        known: &mut Records,
        undo: &mut UndoLog,
        tags: &TagPass,
    ) -> (GameResult, Option<String>) {
        let key = metadata_key(as_runner(&game.runner), &game.app_name);
        let stored = match known.as_ref() {
            Ok(records) => records.get(&key),
            Err(e) => {
                let d = Details::failed(game, e);
                return (result_of(game, d.outcome, Vec::new()), d.error);
            }
        };
        if plan.skip_fetched && stored.is_some_and(|c| c.fetched_at.as_deref().is_some_and(|f| !f.is_empty()) && c.fetch_error.is_none()) {
            return (result_of(game, GameOutcome::Skipped(SkipReason::AlreadyFetched), Vec::new()), None);
        }
        if plan.custom.contains(&key) {
            return (result_of(game, GameOutcome::NoChange(NoChangeReason::NothingMissing), Vec::new()), None);
        }
        // The user said no source has it: its details are not asked again.
        let in_no_source = stored.is_some_and(|c| record::marked_no_match(c, &game.title));
        // Every field the job could change is locked.
        let all_locked = stored.is_some_and(|c| !plan.fillable.is_empty() && plan.fillable.iter().all(|f| record::is_locked(c, f)));
        let steam_id = tag_steam_id(game, stored);
        let tag_asked = steam_id.as_ref().is_some_and(|i| tags.asked.contains(i));
        let tag_change = steam_id
            .as_ref()
            .filter(|i| tags.changed.contains(*i))
            .map(|_| FieldChange { field: extra_fields::STEAM_TAGS.to_owned(), source: ids::STEAM_TAGS.to_owned() });
        // The older dialog always fetches. Fill asks only for the fields
        // the game lacks (and so only the sources that give them), and only
        // when it lacks some.
        let ask = match plan.mode {
            None => Some(Ask { fields: plan.only_fields.clone(), full: plan.full }),
            Some(UpdateMode::Fill) => {
                let missing: Vec<&str> = missing_details(stored, &plan.fillable).collect();
                let all = plan.only_fields.is_none() && missing.len() == plan.fields.len();
                (!missing.is_empty()).then(|| Ask {
                    fields: (!all).then(|| missing.iter().map(|f| f.to_string()).collect()),
                    full: plan.full && missing.len() == plan.fillable.len(),
                })
            }
            Some(_) => (!plan.fillable.is_empty()).then(|| Ask { fields: plan.only_fields.clone(), full: plan.full }),
        };
        // A game whose details were never fetched gets its pictures with
        // them, from the idle fill: a job asking only for pictures leaves it
        // (a record holding only a picture would keep the idle fill away).
        // One found in no source is asked: its mark stays.
        let no_details = ask.as_ref().is_some_and(|a| !asks_details(a.fields.as_deref())) && waits_for_details(stored);
        let ask = ask.filter(|_| !in_no_source && !all_locked && !no_details);
        let details = ask.map(|ask| self.job_details(plan, &ask, game, &key, known, undo));

        let (outcome, mut changes, error) = match details {
            Some(d) => (d.outcome, d.changes, d.error),
            None if in_no_source && !tag_asked => (GameOutcome::Skipped(SkipReason::InNoSource), Vec::new(), None),
            None if all_locked && !tag_asked => (GameOutcome::NoChange(NoChangeReason::Locked), Vec::new(), None),
            None if no_details && !tag_asked => (GameOutcome::Skipped(SkipReason::NoDetails), Vec::new(), None),
            None if tag_asked && tags.unreachable => (GameOutcome::Offline, Vec::new(), None),
            None if tag_asked => (GameOutcome::NoChange(NoChangeReason::NothingNew), Vec::new(), None),
            None => {
                let applies = !plan.fillable.is_empty() || (plan.tags && steam_id.is_some());
                let outcome = if applies && plan.mode == Some(UpdateMode::Fill) {
                    GameOutcome::NoChange(NoChangeReason::NothingMissing)
                } else {
                    GameOutcome::Skipped(SkipReason::NoSource)
                };
                (outcome, Vec::new(), None)
            }
        };
        changes.extend(tag_change);
        if changes.is_empty() {
            (result_of(game, outcome, changes), error)
        } else {
            (result_of(game, GameOutcome::Updated, changes), None)
        }
    }

    /// Fetches, merges and writes the details of a game, keeping its record
    /// as it was for the undo.
    fn job_details(&self, plan: &Plan, ask: &Ask, game: &BulkItem, key: &str, known: &mut Records, undo: &mut UndoLog) -> Details {
        let runner = as_runner(&game.runner);
        let pictures: Vec<ArtKind> =
            plan.pictures.iter().copied().filter(|k| ask.fields.as_ref().is_none_or(|f| f.iter().any(|f| f == k.as_str()))).collect();
        let args = MetadataApplyArgs {
            app_name: game.app_name.clone(),
            runner: runner.to_owned(),
            title: game.title.clone(),
            steam_app_id: game.steam_app_id.clone(),
            source: Some("auto".into()),
            overwrite_existing: Some(plan.overwrite),
            preserve_manual: Some(plan.preserve_manual),
            download_images: Some(plan.download),
            only_fields: ask.fields.clone(),
            only_sources: plan.only_sources.clone(),
            ..Default::default()
        };
        let before = match get_game_metadata(&self.paths, runner, &game.app_name) {
            Ok(before) => before,
            Err(e) => return Details::failed(game, format!("{e:#}")),
        };
        let gathered = self.gather(&args);
        let Gathered { candidates, offline, asked, sources, .. } = &gathered;
        let mut write = |md: CollectionGameMetadata, pictures, undo: &mut UndoLog| -> anyhow::Result<CollectionGameMetadata> {
            let saved = upsert_game_metadata(&self.paths, md)?;
            undo.record(key.to_owned(), before.clone(), saved.clone(), pictures);
            if let Ok(records) = known {
                records.insert(key.to_owned(), saved.clone());
            }
            Ok(saved)
        };
        if *offline {
            return Details { outcome: GameOutcome::Offline, changes: Vec::new(), error: Some(format!("{}: {Offline}", game.title)) };
        }
        // Every source is off or not for this game: nothing is written.
        if !asked {
            return Details { outcome: GameOutcome::Skipped(SkipReason::NoSource), changes: Vec::new(), error: None };
        }
        let decision = decide_in(sources, candidates, before.as_ref());
        if !found(sources, ask.fields.as_deref(), candidates) {
            // Not found while a source that could have it did not answer:
            // nothing is marked, the next pass asks again.
            if gathered.missed(ask.fields.as_deref()) {
                let error = Some(format!("{}: {SourceUnreachable}", game.title));
                return Details { outcome: GameOutcome::Offline, changes: Vec::new(), error };
            }
            // The sure pictures found beside are saved with the mark, or
            // into a record the idle fill no longer needs to fetch (one
            // with details, or marked already): never into a record that
            // would stop it from fetching the details.
            let looked_up = !waits_for_details(before.as_ref());
            let pictures_too = !decision.used.is_empty() && (ask.full || looked_up);
            let mut changes = Vec::new();
            if ask.full || pictures_too {
                let (mut md, kept) = if pictures_too {
                    let kept = (!pictures.is_empty()).then(|| undo.pictures_before(&self.paths, runner, &game.app_name));
                    let proposed = self.propose(sources, &args, before.as_ref(), &decision.used);
                    (self.persist_chosen_images(proposed, plan.download, &pictures), kept)
                } else {
                    (before.clone().unwrap_or_else(|| blank_metadata(&game.app_name, runner)), None)
                };
                if ask.full {
                    mark_not_found(&mut md, &game.title);
                }
                let kept = kept.map(|k| k.after(&self.paths));
                let rewritten = rewritten_pictures(kept.as_ref(), &md);
                match write(md, kept, undo) {
                    Ok(saved) => changes = changed_fields(before.as_ref(), &saved, &plan.fields, &rewritten),
                    Err(e) => return Details::failed(game, format!("{e:#}")),
                }
            }
            return Details { outcome: GameOutcome::NotFound, changes, error: Some(format!("{}: {NO_DATA}", game.title)) };
        }
        if let Some(reason) = decision.review {
            let entry = ReviewEntry::new(&game.app_name, runner, &game.title, reason, &review_candidates(sources, candidates));
            return match self.queue_review(entry.clone()) {
                Ok(was) => {
                    undo.record_review(key.to_owned(), was, Some(entry));
                    Details { outcome: GameOutcome::ToReview(reason), changes: Vec::new(), error: None }
                }
                Err(e) => Details::failed(game, format!("{e:#}")),
            };
        }
        let proposed = self.propose(sources, &args, before.as_ref(), &decision.used);
        let kept = (!pictures.is_empty()).then(|| undo.pictures_before(&self.paths, runner, &game.app_name));
        let saved = self.persist_chosen_images(proposed, plan.download, &pictures);
        let pictures = kept.map(|p| p.after(&self.paths));
        let rewritten = rewritten_pictures(pictures.as_ref(), &saved);
        match write(saved, pictures, undo) {
            Ok(saved) => {
                // Saved with a sure match: it no longer waits for review.
                match self.unqueue(key) {
                    Ok(Some(was)) => undo.record_review(key.to_owned(), Some(was), None),
                    Ok(None) => {}
                    Err(e) => log::warn!("review queue: {e:#}"),
                }
                let changes = changed_fields(before.as_ref(), &saved, &plan.fields, &rewritten);
                let outcome = if changes.is_empty() { GameOutcome::NoChange(NoChangeReason::NothingNew) } else { GameOutcome::Updated };
                Details { outcome, changes, error: None }
            }
            Err(e) => Details::failed(game, format!("{e:#}")),
        }
    }
}

/// The pictures of `saved` the job wrote over files `pictures` kept.
fn rewritten_pictures(pictures: Option<&Pictures>, saved: &CollectionGameMetadata) -> Vec<&'static str> {
    ArtKind::ALL.into_iter().filter(|k| pictures.is_some_and(|p| p.rewrote(k.of_record(saved)))).map(ArtKind::as_str).collect()
}

pub(super) fn result_of(game: &BulkItem, outcome: GameOutcome, changes: Vec<FieldChange>) -> GameResult {
    let runner = as_runner(&game.runner);
    GameResult {
        key: metadata_key(runner, &game.app_name),
        app_name: game.app_name.clone(),
        runner: runner.to_owned(),
        title: game.title.clone(),
        outcome,
        changes,
    }
}
