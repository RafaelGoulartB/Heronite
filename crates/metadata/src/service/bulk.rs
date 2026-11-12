//! Bulk metadata jobs: one runs at a time, the others wait in a queue
//! (FIFO); and the idle auto-fill.
//!
//! The idle fills (this one, the age ratings and the Steam tags) never run
//! with a job: a fill does not start while a job runs or waits, and stops
//! before its next game when one is started; the job waits for it (at most
//! [`IDLE_FILL_WAIT`]: one game's lookup in practice) before its first game.
//! Both would otherwise fetch the same games and write the same records.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::LocalMetaFile;
use heronite_data::status::STATUS_ONLY_KEY;

use super::fetch::{NoSourceAsked, SourceUnreachable};
use super::job_run::result_of;
use super::undo::UndoLog;
use super::{is_offline, is_sent_to_review, tag_fill, MetadataService};
use crate::job::{GameOutcome, JobKind, QueuedJob, SkipReason};
use crate::sources::is_on;
use crate::store::get_all_game_metadata;
use crate::types::*;
use crate::util::now_ms;
use crate::{record, review};

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Games filled per idle pass.
pub const AUTO_FILL_BATCH: usize = 40;
/// The longest a job waits for an idle fill to stop. Past it the job runs
/// anyway: each write is a locked read-modify-write, so nothing is lost.
const IDLE_FILL_WAIT: Duration = Duration::from_secs(30);

pub(super) type OnProgress = Arc<dyn Fn(&BulkProgress) + Send + Sync>;

pub(super) struct Queued {
    id: u64,
    args: BulkArgs,
    on_progress: OnProgress,
}

/// The jobs' shared state, behind `MetadataService::jobs`.
#[derive(Default)]
pub(super) struct Jobs {
    /// What was last published.
    pub(super) progress: BulkProgress,
    queue: VecDeque<Queued>,
    next_id: u64,
    /// A thread owns the queue: it runs a job or is about to.
    pub(super) running: bool,
    /// The last finished or stopped job's undo, until the next job starts.
    pub(super) undo: Option<UndoLog>,
}

impl Jobs {
    fn queued(&self) -> Vec<QueuedJob> {
        self.queue.iter().map(|q| QueuedJob { id: q.id, kind: q.args.kind, mode: q.args.mode, games: q.args.games.len() }).collect()
    }

    fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// The progress a job starts with.
pub(super) fn initial(id: u64, args: &BulkArgs) -> BulkProgress {
    BulkProgress {
        state: BulkState::Running,
        total: args.games.len(),
        percent: if args.games.is_empty() { 100 } else { 0 },
        job: id,
        kind: args.kind,
        mode: args.mode,
        started_at_ms: now_ms(),
        ..Default::default()
    }
}

impl MetadataService {
    /// `getBulkMetadataProgress`: the running job's, else the last one's.
    pub fn bulk_progress(&self) -> BulkProgress {
        self.jobs.lock().progress.clone()
    }

    /// Stores `next` (with the queue and the undo as they are now) and
    /// hands it to `on_progress`, outside the lock.
    pub(super) fn publish(&self, next: &mut BulkProgress, on_progress: &dyn Fn(&BulkProgress)) {
        {
            let mut jobs = self.jobs.lock();
            next.queued = jobs.queued();
            next.can_undo = jobs.undo.is_some() && !next.is_running();
            jobs.progress = next.clone();
        }
        on_progress(next);
    }

    /// Whether a job runs or waits (the idle fills give way to it).
    pub(super) fn job_pending(&self) -> bool {
        self.jobs.lock().running
    }

    /// Whether a job of `kind` runs or waits.
    pub(super) fn job_of_kind(&self, kind: JobKind) -> bool {
        let jobs = self.jobs.lock();
        (jobs.running && jobs.progress.is_running() && jobs.progress.kind == kind) || jobs.queue.iter().any(|q| q.args.kind == kind)
    }

    /// Whether job `id` was asked to stop.
    pub(super) fn stopping(&self, id: u64) -> bool {
        self.stop_job.load(Ordering::SeqCst) == id
    }

    /// `cancelBulkMetadata`: the running job stops before its next game; the
    /// jobs waiting run after it ([`clear_bulk_queue`](Self::clear_bulk_queue) drops them).
    pub fn cancel_bulk_metadata(&self, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        let stopping = {
            let mut jobs = self.jobs.lock();
            (jobs.progress.state == BulkState::Running && jobs.progress.job != 0).then(|| {
                self.stop_job.store(jobs.progress.job, Ordering::SeqCst);
                jobs.progress.state = BulkState::Cancelling;
                jobs.progress.clone()
            })
        };
        if let Some(p) = stopping {
            on_progress(&p);
        }
        self.bulk_progress()
    }

    /// Drops the jobs waiting; the running one goes on.
    pub fn clear_bulk_queue(&self, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        let p = {
            let mut jobs = self.jobs.lock();
            jobs.queue.clear();
            jobs.progress.queued.clear();
            jobs.progress.clone()
        };
        on_progress(&p);
        p
    }

    /// `startBulkMetadata`: runs the job on its own thread, or queues it
    /// behind the running one, and returns the progress right away (the new
    /// job's when it started, else the running one's with this job last in
    /// `queued`). `on_progress` is called after every step of this job (and
    /// when it is queued) from the job's thread; it must not wait for the
    /// service.
    pub fn start_bulk_metadata(
        self: &Arc<Self>,
        args: BulkArgs,
        on_progress: impl Fn(&BulkProgress) + Send + Sync + 'static,
    ) -> BulkProgress {
        self.start_bulk_job(args, on_progress).1
    }

    /// [`start_bulk_metadata`](Self::start_bulk_metadata), with the new
    /// job's id. The progress alone cannot tell it: while another job runs
    /// it is that job's.
    pub fn start_bulk_job(
        self: &Arc<Self>,
        args: BulkArgs,
        on_progress: impl Fn(&BulkProgress) + Send + Sync + 'static,
    ) -> (u64, BulkProgress) {
        let on_progress: OnProgress = Arc::new(on_progress);
        let (id, first, mut now) = {
            let mut jobs = self.jobs.lock();
            let id = jobs.take_id();
            if jobs.running {
                jobs.queue.push_back(Queued { id, args, on_progress: on_progress.clone() });
                (id, None, jobs.progress.clone())
            } else {
                jobs.running = true;
                let p = initial(id, &args);
                jobs.progress = p.clone();
                (id, Some(Queued { id, args, on_progress: on_progress.clone() }), p)
            }
        };
        self.publish(&mut now, &*on_progress);
        if let Some(first) = first {
            let this = Arc::clone(self);
            std::thread::Builder::new().name("metadata-bulk".into()).spawn(move || this.work(first)).expect("spawn bulk thread");
        }
        (id, now)
    }

    /// Blocking variant of [`start_bulk_metadata`](Self::start_bulk_metadata)
    /// for callers that already have a worker thread (and for tests). While
    /// another job runs it does nothing and returns that job's progress.
    /// Jobs queued while it runs are run after it, before it returns.
    pub fn run_bulk_metadata(&self, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        let id = {
            let mut jobs = self.jobs.lock();
            if jobs.running {
                return jobs.progress.clone();
            }
            jobs.running = true;
            jobs.take_id()
        };
        // The caller sees a panic of its own job.
        let mut release = Release { service: self, current: None };
        let (done, mut next) = self.finish(self.run_job(id, args, on_progress), on_progress);
        while let Some(job) = next {
            release.current = Some((job.id, job.on_progress.clone()));
            next = self.finish(self.run_job(job.id, &job.args, &*job.on_progress), &*job.on_progress).1;
        }
        done
    }

    /// The job thread: runs `first`, then the queue until it is empty.
    fn work(&self, first: Queued) {
        let mut release = Release { service: self, current: None };
        let mut next = Some(first);
        while let Some(job) = next {
            release.current = Some((job.id, job.on_progress.clone()));
            next = self.finish(self.run_job(job.id, &job.args, &*job.on_progress), &*job.on_progress).1;
        }
    }

    /// Publishes a job's final progress and takes the next job waiting; with
    /// none, lets the queue go first, so a caller that reacts to `Done` can
    /// start the next job at once.
    ///
    /// With a next job, the stored progress is already that job's start: it
    /// is taken off the queue here, and its own first progress comes only
    /// after the undo is cleared, so storing the `Done` would show no job
    /// running and none waiting in between.
    fn finish(&self, mut done: BulkProgress, on_progress: &dyn Fn(&BulkProgress)) -> (BulkProgress, Option<Queued>) {
        let next = {
            let mut jobs = self.jobs.lock();
            let next = jobs.queue.pop_front();
            jobs.running = next.is_some();
            done.queued = jobs.queued();
            // The next job drops this undo as it starts.
            done.can_undo = jobs.undo.is_some() && next.is_none();
            jobs.progress = match &next {
                Some(job) => BulkProgress { queued: done.queued.clone(), ..initial(job.id, &job.args) },
                None => done.clone(),
            };
            next
        };
        on_progress(&done);
        (done, next)
    }

    /// Before a job's first game: lets a running idle fill stop.
    pub(super) fn wait_for_idle_fill(&self) {
        let deadline = Instant::now() + IDLE_FILL_WAIT;
        while (self.filling.load(Ordering::SeqCst) || self.rating_fill.load(Ordering::SeqCst) || tag_fill::running())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// `refreshMissingCollectionMetadata`: when `autoFillMissing` is on,
    /// fetches metadata for up to 40 overlay games that have none (or whose
    /// last attempt failed more than a week ago) and returns how many got
    /// data. Games waiting for review, or that the user said are in no
    /// source (under their title), are left alone, as are those no source
    /// that is on is for; a game the sources are not sure about goes to
    /// review. Stops at the first game when offline
    /// (nothing is saved, so the next call tries again); a game some source
    /// could not answer for is left as it was. Call it when the app is idle.
    pub fn refresh_missing_collection_metadata(&self) -> Result<usize> {
        self.refresh_missing_while(&|| false)
    }

    /// Same, checking `should_stop` before each game.
    pub fn refresh_missing_while(&self, should_stop: &dyn Fn() -> bool) -> Result<usize> {
        if self.filling.swap(true, Ordering::SeqCst) {
            return Ok(0);
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&self.filling);
        if self.job_pending() {
            return Ok(0);
        }

        let settings = self.metadata_settings();
        // With every source off there is nothing to ask.
        if !settings.auto_fill_missing || !self.sources().lookups().any(|s| is_on(&settings, &s.descriptor().id)) {
            return Ok(0);
        }
        let overlay: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        // Nothing writes metadata.json while this scan runs.
        let known = get_all_game_metadata(&self.paths)?;
        let queued = review::load(&self.paths).games;
        let mut missing = Vec::new();
        for meta in overlay.games.values() {
            // An entry that only holds a status was never a game to search for.
            if meta.extra.get(STATUS_ONLY_KEY) == Some(&serde_json::Value::Bool(true)) {
                continue;
            }
            let key = metadata_key(&meta.runner, &meta.app_name);
            if queued.contains_key(&key) {
                continue;
            }
            let needs = match known.get(&key) {
                // Asked again only once renamed.
                Some(c) if record::no_match(c).is_some() => !record::marked_no_match(c, &meta.title),
                None => true,
                Some(c) if c.fetch_error.is_some() => match c.fetched_at.as_deref() {
                    None | Some("") => true,
                    Some(at) => {
                        chrono::DateTime::parse_from_rfc3339(at).map(|t| now_ms() - t.timestamp_millis() > WEEK_MS).unwrap_or(false)
                    }
                },
                Some(_) => false,
            };
            if needs {
                missing.push(meta);
            }
        }
        if missing.is_empty() {
            return Ok(0);
        }
        log::info!("Filling Collection metadata for {} game(s)", missing.len());
        let (mut filled, mut tried) = (0, 0);
        for meta in missing {
            if tried == AUTO_FILL_BATCH || should_stop() || self.job_pending() {
                break;
            }
            let fetched = self.apply_game_metadata(&MetadataApplyArgs {
                app_name: meta.app_name.clone(),
                runner: meta.runner.clone(),
                title: meta.title.clone(),
                steam_app_id: meta.steam_app_id.clone(),
                source: Some("auto".into()),
                overwrite_existing: Some(false),
                preserve_manual: Some(true),
                download_images: Some(settings.download_images),
                ..Default::default()
            });
            // No source is for this game: nothing was asked or saved, and it
            // does not take a place in the batch.
            if fetched.as_ref().is_err_and(|e| e.is::<NoSourceAsked>()) {
                continue;
            }
            tried += 1;
            let saved = match fetched {
                Ok(saved) => saved,
                // Offline: the other games would fail the same way. They stay
                // unfetched, so the next start tries again.
                Err(e) if is_offline(&e) => {
                    log::info!("Collection metadata fill stopped: {e}");
                    break;
                }
                Err(e) if is_sent_to_review(&e).is_some() => continue,
                // Left as it was: asked again at the next pass.
                Err(e) if e.is::<SourceUnreachable>() => continue,
                Err(e) => return Err(e),
            };
            if saved.fetch_error.is_none() {
                filled += 1;
            }
        }
        Ok(filled)
    }
}

/// When the job thread panics outside a job's games (those end the job as a
/// stop does, see `run_job`): lets the queue go, so later jobs can start,
/// and hands a `Done` to the running job's callback and to each waiting
/// job's, so nothing goes on showing them as running. Every normal exit
/// lets the queue go in `finish`. Only builds that unwind get here (debug,
/// tests): the release profile aborts on a panic.
struct Release<'a> {
    service: &'a MetadataService,
    /// The running job and its callback.
    current: Option<(u64, OnProgress)>,
}

impl Drop for Release<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let (last, dropped) = {
            let mut jobs = self.service.jobs.lock();
            jobs.running = false;
            let dropped: Vec<Queued> = jobs.queue.drain(..).collect();
            let p = &mut jobs.progress;
            if p.state != BulkState::Done {
                p.finished_at_ms = now_ms();
            }
            p.state = BulkState::Done;
            p.current_title = None;
            p.queued.clear();
            jobs.progress.can_undo = jobs.undo.is_some();
            (jobs.progress.clone(), dropped)
        };
        if let Some((id, on_progress)) = &self.current {
            // Before its first progress the last one stored is the job before.
            let done = if last.job == *id {
                last
            } else {
                BulkProgress { state: BulkState::Done, job: *id, finished_at_ms: now_ms(), ..Default::default() }
            };
            on_progress(&done);
        }
        for q in dropped {
            let stopped = q.args.games.iter().map(|g| Arc::new(result_of(g, GameOutcome::Skipped(SkipReason::Stopped), Vec::new())));
            let done =
                BulkProgress { state: BulkState::Done, results: stopped.collect(), finished_at_ms: now_ms(), ..initial(q.id, &q.args) };
            (q.on_progress)(&done);
        }
    }
}
