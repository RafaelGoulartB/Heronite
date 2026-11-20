//! [`MetadataHub`]: owns the `MetadataService`, the bulk jobs' progress, their
//! undo, the idle auto-fill and the answers to the review queue; lives as
//! long as the window. The "Update metadata" dialog, the dock's Metadata
//! menu, the header pill and Settings → Metadata read it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_metadata::service::{Attention, Resolution, Resolved};
use heronite_metadata::{BulkArgs, BulkItem, BulkProgress, BulkState, MetadataService, ReviewItem, UndoReport};

use super::fake::Fake;
use super::family_notice::FamilyNotice;
use super::source_downloads::{FakeDownload, SourceDownloads};

pub enum HubEvent {
    /// The metadata job or idle fill changed data on disk: reload the library.
    DataChanged,
    /// The settings page asks the Collection to open the "Update metadata" dialog.
    OpenUpdate,
    /// A job finished or stopped (its last progress).
    JobDone(Box<BulkProgress>),
    /// The last job was undone.
    Undone(UndoReport),
    /// The review queue changed (a job or the idle fill queued games, a
    /// game was answered, an undo): [`MetadataHub::queue`] holds it again.
    ReviewChanged,
    /// Someone asks the Collection to open "Review matches".
    OpenReview(ReviewRequest),
    /// Someone asks for "Update metadata" on the games that need attention.
    OpenAttention,
    /// Someone asks for Settings → Metadata (a key is missing).
    OpenSettings,
    /// The database of a source was downloaded, refreshed or removed (or
    /// that failed): its row reads its state again.
    SourceDownloaded,
    /// Someone asks the Collection for "Get more sources" or for the
    /// installer of a file (`dialogs/extensions`).
    OpenSources(crate::dialogs::extensions::SourcesRequest),
    /// Extensions were loaded, installed or removed, or their lists read
    /// again ([`MetadataHub::extensions`]): Settings → Metadata reads its
    /// rows again.
    ExtensionsChanged,
}

/// The games "Review matches" goes through.
#[derive(Clone, Debug, Default)]
pub struct ReviewRequest {
    /// `None`: every game of the queue.
    pub games: Option<Vec<BulkItem>>,
    /// The game shown first (an index of `games`).
    pub start: usize,
}

pub struct MetadataHub {
    pub service: Arc<MetadataService>,
    pub progress: BulkProgress,
    /// The job whose end the header pill no longer announces: its popover
    /// or the dialog's results showed it.
    pub seen: u64,
    /// An undo runs off the UI thread.
    pub undoing: bool,
    /// What the last undo did, until the next job starts.
    pub undone: Option<UndoReport>,
    /// The service's review revision last announced.
    review_revision: u64,
    /// The review queue, oldest first, read off the UI thread at start and
    /// whenever the service changed it.
    pub queue: Vec<ReviewItem>,
    /// The idle fill queued games nobody looked at yet: the header pill
    /// says so until the review or its popover opens.
    pub idle_review: bool,
    /// The idle fill ended: the next read of the queue tells whether it
    /// queued games.
    idle_check: Option<usize>,
    /// The notice under the Cover aspect pickers (`family_notice.rs`).
    pub notice: Option<FamilyNotice>,
    /// `HERONITE_DEBUG_METADATA_JOB`: jobs are made up (see `fake.rs`).
    fake: Option<Fake>,
    /// The sources' database downloads (`source_downloads.rs`).
    pub(super) source_downloads: SourceDownloads,
    /// A loop follows the running downloads.
    pub(super) following_downloads: bool,
    /// `HERONITE_DEBUG_SOURCE_DOWNLOAD`.
    pub(super) fake_download: Option<FakeDownload>,
    /// The installed extensions and their lists, as last read
    /// (`dialogs/extensions/state.rs`).
    pub extensions: crate::dialogs::extensions::ExtensionsState,
    tx: Arc<Mutex<UnboundedSender<BulkProgress>>>,
}

impl EventEmitter<HubEvent> for MetadataHub {}

/// Lets any dialog reach the hub without threading it through `DialogCtx`.
pub struct HubGlobal(pub Entity<MetadataHub>);
impl Global for HubGlobal {}

pub fn hub(cx: &App) -> Entity<MetadataHub> {
    cx.global::<HubGlobal>().0.clone()
}

impl MetadataHub {
    pub fn new(data_paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let service = Arc::new(MetadataService::new(data_paths));
        // The installed extensions join the sources off the UI thread (each
        // script is compiled); a fetch before that asks the built-in ones.
        let loading = service.clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                use heronite_metadata_ext::ExtensionsExt;
                loading.load_extensions();
            })
            .await;
            this.update(cx, |h, cx| h.read_extensions(false, cx)).ok();
        })
        .detach();
        let (tx, mut rx) = unbounded::<BulkProgress>();
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.next().await {
                let alive = this.update(cx, |this, cx| this.on_progress(p, cx)).is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        let progress = service.bulk_progress();
        let mut this = Self {
            review_revision: service.review_revision(),
            service,
            progress,
            seen: 0,
            undoing: false,
            undone: None,
            queue: Vec::new(),
            idle_review: false,
            idle_check: None,
            notice: None,
            fake: Fake::from_env(),
            source_downloads: SourceDownloads::new(),
            following_downloads: false,
            fake_download: FakeDownload::from_env(),
            extensions: crate::dialogs::extensions::ExtensionsState::from_env(),
            tx: Arc::new(Mutex::new(tx)),
        };
        this.read_queue(cx);
        Self::listen_families(cx);
        this
    }

    /// Reads the review queue off the UI thread into [`Self::queue`]; also
    /// after a change made without the service (a game removed).
    pub(crate) fn read_queue(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let queue = cx.background_spawn(async move { service.review_queue() }).await;
            this.update(cx, |this, cx| {
                // `idle-review` keeps its made-up queue.
                if this.fake_idle() {
                    return;
                }
                if let Some(before) = this.idle_check.take() {
                    this.idle_review |= queue.len() > before;
                }
                this.idle_review &= !queue.is_empty();
                this.queue = queue;
                cx.emit(HubEvent::ReviewChanged);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The made-up queue of `HERONITE_DEBUG_METADATA_JOB=idle-review`.
    pub(super) fn set_fake_queue(&mut self, queue: Vec<ReviewItem>, cx: &mut Context<Self>) {
        self.idle_review = !queue.is_empty();
        self.queue = queue;
        cx.notify();
    }

    /// Whether `key` (`runner:appName`) waits for review.
    pub fn queued(&self, key: &str) -> bool {
        self.queue.iter().any(|q| q.key == key)
    }

    /// The review opened: what the pill announced was seen.
    pub fn review_seen(&mut self, cx: &mut Context<Self>) {
        self.idle_review = false;
        if self.progress.state == BulkState::Done {
            self.seen = self.progress.job;
        }
        cx.notify();
    }

    /// Asks the Collection to open "Review matches" on `request`.
    pub fn request_review(&mut self, request: ReviewRequest, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenReview(request));
    }

    /// Asks the Collection for "Update metadata" on the games that need attention.
    pub fn request_attention(&mut self, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenAttention);
    }

    /// Asks the Collection for Settings → Metadata.
    pub fn request_settings(&mut self, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenSettings);
    }

    pub(super) fn on_progress(&mut self, p: BulkProgress, cx: &mut Context<Self>) {
        let ended = p.state == BulkState::Done && (self.progress.state != BulkState::Done || self.progress.job != p.job);
        if p.job != self.progress.job {
            self.undone = None;
        }
        self.progress = p;
        if ended {
            self.family_job_ended(self.progress.job);
            if self.fake.is_none() {
                cx.emit(HubEvent::DataChanged);
            }
            cx.emit(HubEvent::JobDone(Box::new(self.progress.clone())));
            self.note_review(cx);
        }
        cx.notify();
    }

    /// Reads the review queue again (then announces
    /// [`HubEvent::ReviewChanged`]) when the service changed it since the
    /// last time.
    pub fn note_review(&mut self, cx: &mut Context<Self>) {
        let now = self.service.review_revision();
        if now != self.review_revision {
            self.review_revision = now;
            self.read_queue(cx);
        }
    }

    /// Applies the user's answer for a game of the review off the UI thread;
    /// the Collection reloads and the queue is announced as changed after.
    pub fn resolve_review(&mut self, game: BulkItem, resolution: Resolution, cx: &mut Context<Self>) -> Task<anyhow::Result<Resolved>> {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { service.resolve_review(&game, &resolution) }).await;
            if matches!(done, Ok(Resolved::Saved(_) | Resolved::InNoSource)) {
                this.update(cx, |this, cx| {
                    cx.emit(HubEvent::DataChanged);
                    this.note_review(cx);
                })
                .ok();
            }
            done
        })
    }

    /// The games of `games` that need attention, by reason, read off the UI
    /// thread (no network).
    pub fn attention(&self, games: Vec<BulkItem>, cx: &mut Context<Self>) -> Task<anyhow::Result<Attention>> {
        let service = self.service.clone();
        cx.background_spawn(async move { service.attention(&games) })
    }

    pub fn running(&self) -> bool {
        self.progress.is_running()
    }

    /// Hands every progress of the service's jobs to the hub's loop.
    fn sender(&self) -> impl Fn(&BulkProgress) + Send + Sync + 'static {
        let tx = self.tx.clone();
        move |p| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.unbounded_send(p.clone());
            }
        }
    }

    /// Starts `args`, or queues it behind the running job. Returns the new
    /// job's id.
    pub fn start(&mut self, args: BulkArgs, cx: &mut Context<Self>) -> u64 {
        let kind = args.kind;
        let id = if self.fake.is_some() {
            self.fake_start(args, cx)
        } else {
            let (id, p) = self.service.start_bulk_job(args, self.sender());
            if p.job != self.progress.job {
                self.undone = None;
            }
            self.progress = p;
            cx.notify();
            id
        };
        self.note_family_job(kind, id);
        id
    }

    /// Stops the running job before its next game; the queued ones follow.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.fake.is_some() {
            return self.fake_stop(cx);
        }
        self.progress = self.service.cancel_bulk_metadata(&self.sender());
        cx.notify();
    }

    /// Drops the jobs waiting behind the running one.
    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        if self.fake.is_some() {
            self.progress.queued.clear();
        } else {
            self.progress = self.service.clear_bulk_queue(&self.sender());
        }
        cx.notify();
    }

    /// Whether "Undo" can be offered for the last job.
    pub fn can_undo(&self) -> bool {
        self.progress.can_undo && !self.running() && !self.undoing
    }

    /// Undoes the last job off the UI thread; the Collection reloads after.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if !self.can_undo() {
            return;
        }
        self.undoing = true;
        cx.notify();
        if self.fake.is_some() {
            return self.fake_undo(cx);
        }
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { service.undo_bulk_metadata() }).await;
            this.update(cx, |this, cx| {
                this.undoing = false;
                this.progress = this.service.bulk_progress();
                match done {
                    Ok(Some(report)) => {
                        log::info!("metadata job {} undone: {} game(s), {} later change(s) kept", report.job, report.games, report.kept);
                        this.undone = Some(report);
                        cx.emit(HubEvent::DataChanged);
                        cx.emit(HubEvent::Undone(report));
                        this.note_review(cx);
                    }
                    Ok(None) => {}
                    Err(err) => log::warn!("metadata undo: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The undo of the job shown, once done.
    pub fn undone_of(&self, job: u64) -> Option<UndoReport> {
        self.undone.filter(|u| u.job == job)
    }

    pub fn request_update(&mut self, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenUpdate);
    }

    /// Jobs are made up (`HERONITE_DEBUG_METADATA_JOB`).
    pub fn faking(&self) -> bool {
        self.fake.is_some()
    }

    pub(super) fn fake_ref(&self) -> Option<&Fake> {
        self.fake.as_ref()
    }

    pub(super) fn fake_mut(&mut self) -> Option<&mut Fake> {
        self.fake.as_mut()
    }

    /// Covers for the local games that have none (games just added from a
    /// folder); the Collection reloads when some were found.
    pub fn fill_local_covers(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { service.refresh_missing_local_covers() }).await;
            match found {
                Ok(n) if n > 0 => {
                    this.update(cx, |_, cx| cx.emit(HubEvent::DataChanged)).ok();
                }
                Ok(_) => {}
                Err(err) => log::warn!("cover fill: {err:#}"),
            }
        })
        .detach();
    }

    /// Steam tags for the games that lack them (when the setting is on); the
    /// Collection reloads when some came.
    pub fn refresh_steam_tags(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| match cx.background_spawn(async move { service.refresh_steam_tags_while(&|| false) }).await {
            Ok(r) if r.changed() => {
                this.update(cx, |_, cx| cx.emit(HubEvent::DataChanged)).ok();
            }
            Ok(_) => {}
            Err(err) => log::warn!("Steam tags: {err:#}"),
        })
        .detach();
    }

    /// Idle maintenance like Heronite's `scheduleIdleMaintenance` (90 s after
    /// startup): fills missing Collection metadata when `autoFillMissing` is
    /// on, and local covers; then refreshes the sources' databases that are
    /// a week old (only those the user downloaded, while the source is on).
    /// Runs off the UI thread.
    pub fn schedule_idle(&mut self, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(service) = this.update(cx, |this, _| this.service.clone()) else { return };
            let changed = cx
                .background_spawn(async move {
                    let filled = match service.refresh_missing_collection_metadata() {
                        Ok(n) => n,
                        Err(err) => {
                            log::warn!("idle metadata fill: {err:#}");
                            0
                        }
                    };
                    let rated = match service.fill_missing_age_ratings_while(&|| false) {
                        Ok(n) => n,
                        Err(err) => {
                            log::warn!("idle age rating fill: {err:#}");
                            0
                        }
                    };
                    let covers = match service.refresh_missing_local_covers() {
                        Ok(n) => n,
                        Err(err) => {
                            log::warn!("idle cover fill: {err:#}");
                            0
                        }
                    };
                    let tags = match service.refresh_steam_tags_while(&|| false) {
                        Ok(r) => usize::from(r.changed()),
                        Err(err) => {
                            log::warn!("idle Steam tags: {err:#}");
                            0
                        }
                    };
                    filled + rated + covers + tags
                })
                .await;
            if changed > 0 {
                log::info!("idle fill updated {changed} item(s)");
                this.update(cx, |_, cx| cx.emit(HubEvent::DataChanged)).ok();
            }
            this.update(cx, |this, cx| {
                // The queue read that follows tells whether the fill queued games.
                if this.service.review_revision() != this.review_revision {
                    this.idle_check = Some(this.queue.len());
                }
                this.note_review(cx);
                this.refresh_source_downloads(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The idle maintenance's refresh of the sources' databases.
    fn refresh_source_downloads(&mut self, cx: &mut Context<Self>) {
        if self.fake_download.is_some() {
            return;
        }
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let refreshed = cx.background_spawn(async move { service.refresh_due_source_downloads() }).await;
            if refreshed > 0 {
                log::info!("idle maintenance refreshed {refreshed} source database(s)");
            }
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        // The refresh shows in Settings like a download by hand.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            this.update(cx, |this, cx| {
                if !this.service.downloading_sources().is_empty() {
                    this.follow_downloads(cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
