//! Background lossless compression of the captures folder (Settings →
//! Screenshots; the conversion itself is `heronite_media::compress`).
//!
//! Nothing is stored about the queue: the work left is "the PNGs still in the
//! folder", and each converted file carries its own marker. So a quit, a
//! crash or a game starting just stops the current file (the PNG stays) and
//! the next run picks up where this one left.
//!
//! It stays out of the way:
//! - one worker thread, at idle CPU and I/O priority, one file at a time,
//!   libwebp single-threaded (a file is ~1 s and ~170 MB at peak);
//! - never while a game launched from here runs: a launch aborts the current
//!   encode at once, and work resumes [`AFTER_GAME`] after the last game ends;
//! - the first run waits [`AFTER_START`] so startup is not slowed down.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::screenshot_prefs::ScreenshotPrefs;
use heronite_data::{AppPaths, DataPaths};
use heronite_media::compress::{self, FolderUsage, Outcome};
use heronite_media::ThumbnailCache;

/// Delay of the first run after the app starts.
const AFTER_START: Duration = Duration::from_secs(60);
/// Delay after the last running game ends (post-game backups, shader caches).
const AFTER_GAME: Duration = Duration::from_secs(60);
/// Art references are updated every this many conversions (and at the end).
const REFS_BATCH: usize = 25;

/// Where the service is, for the settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Compression is off.
    Off,
    /// No screenshots folder is set.
    NoFolder,
    /// A run is scheduled (after the start or after a game).
    Waiting,
    /// Listing the folder.
    Scanning,
    /// `done` of `total` files handled; `current` is the file being encoded.
    Working { done: usize, total: usize, current: String },
    /// A game runs; `left` files were not reached yet (`None`: not scanned).
    Paused { left: Option<usize> },
    /// The last run went through everything it found.
    Idle,
}

/// Totals of the current or last run.
#[derive(Clone, Debug, Default)]
pub struct RunTotals {
    pub converted: usize,
    pub freed: i64,
    pub failed: usize,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum CompressionEvent {
    /// Captures replaced by their WebP (`from`, `to`).
    Converted(Vec<(PathBuf, PathBuf)>),
    /// `art.json` now points at converted files.
    ArtChanged,
}

/// What the worker tells the service. `run` is the id of the [`Run`].
enum Report {
    Started { run: u64, total: usize },
    Progress { run: u64, done: usize, total: usize, current: String },
    Converted { from: PathBuf, to: PathBuf, freed: i64 },
    Failed { error: String },
    ArtChanged,
    Finished { run: u64, left: usize, cancelled: bool },
}

struct Run {
    id: u64,
    folder: PathBuf,
    /// Raised to stop this run (a game started, quit, folder changed).
    cancel: Arc<AtomicBool>,
}

pub struct Compression {
    data_paths: DataPaths,
    app: AppPaths,
    prefs: ScreenshotPrefs,
    /// Games the launcher runs (`app_name`).
    games: HashSet<String>,
    /// To the worker thread, once a run started it.
    runs: Option<Sender<Run>>,
    reports_tx: UnboundedSender<Report>,
    /// The run in flight: its id and cancel flag.
    current: Option<(u64, Arc<AtomicBool>)>,
    next_id: u64,
    scheduled: Option<Task<()>>,
    pub phase: Phase,
    pub totals: RunTotals,
    /// Last measure of the folder (`None` while measuring / not measured).
    pub usage: Option<FolderUsage>,
    usage_task: Option<Task<()>>,
}

impl EventEmitter<CompressionEvent> for Compression {}

struct CompressionGlobal(Entity<Compression>);
impl Global for CompressionGlobal {}

/// Creates the service (once) and schedules the first run when it is on.
pub fn init(cx: &mut App, data_paths: DataPaths) -> Entity<Compression> {
    if let Some(g) = cx.try_global::<CompressionGlobal>() {
        return g.0.clone();
    }
    let app = AppPaths::for_data(&data_paths);
    let prefs = ScreenshotPrefs::load(&app);
    let entity = cx.new(|cx| Compression::new(data_paths, app, prefs, cx));
    cx.set_global(CompressionGlobal(entity.clone()));
    if prefs.compress_lossless {
        entity.update(cx, |c, cx| c.schedule(AFTER_START, cx));
    }
    entity
}

/// The service, once the hub created it.
pub fn get(cx: &App) -> Option<Entity<Compression>> {
    cx.try_global::<CompressionGlobal>().map(|g| g.0.clone())
}

/// Quit: stops the current encode (the PNG stays; the next start resumes).
pub fn shutdown(cx: &App) {
    if let Some(c) = get(cx) {
        c.read(cx).stop_run();
    }
}

impl Compression {
    fn new(data_paths: DataPaths, app: AppPaths, prefs: ScreenshotPrefs, cx: &mut Context<Self>) -> Self {
        let (reports_tx, mut reports) = unbounded::<Report>();
        cx.spawn(async move |this, cx| {
            while let Some(first) = reports.next().await {
                let mut batch = vec![first];
                while let Ok(more) = reports.try_recv() {
                    batch.push(more);
                }
                if this.update(cx, |c, cx| c.on_reports(batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let phase = if prefs.compress_lossless { Phase::Waiting } else { Phase::Off };
        Self {
            data_paths,
            app,
            prefs,
            games: HashSet::new(),
            runs: None,
            reports_tx,
            current: None,
            next_id: 0,
            scheduled: None,
            phase,
            totals: RunTotals::default(),
            usage: None,
            usage_task: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.prefs.compress_lossless
    }

    pub fn game_running(&self) -> bool {
        !self.games.is_empty()
    }

    /// A run is in flight (listing or converting).
    pub fn busy(&self) -> bool {
        self.current.is_some()
    }

    /// Stops the run in flight; the current file is left as it was.
    fn stop_run(&self) {
        if let Some((_, cancel)) = &self.current {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Every launch state change, from the hub.
    pub fn set_game_running(&mut self, app_name: &str, running: bool, cx: &mut Context<Self>) {
        let was = self.game_running();
        if running {
            self.games.insert(app_name.to_owned());
        } else {
            self.games.remove(app_name);
        }
        match (was, self.game_running()) {
            (false, true) => {
                self.scheduled = None;
                if self.busy() {
                    log::info!("screenshot compression paused: a game started");
                    self.stop_run();
                }
                if self.enabled() {
                    let left = match &self.phase {
                        Phase::Working { done, total, .. } => Some(total - done),
                        Phase::Paused { left } => *left,
                        _ => None,
                    };
                    self.phase = Phase::Paused { left };
                }
            }
            (true, false) if self.enabled() => self.schedule(AFTER_GAME, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The settings toggle.
    pub fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.prefs.compress_lossless == on {
            return;
        }
        self.prefs.compress_lossless = on;
        let (prefs, app) = (self.prefs, self.app.clone());
        cx.background_spawn(async move {
            if let Err(e) = prefs.save(&app) {
                log::error!("saving the screenshot options: {e:#}");
            }
        })
        .detach();
        if on {
            self.phase = if self.game_running() { Phase::Paused { left: None } } else { Phase::Waiting };
            self.start(cx);
        } else {
            self.scheduled = None;
            self.stop_run();
            self.phase = Phase::Off;
        }
        cx.notify();
    }

    /// The screenshots folder changed in the settings.
    pub fn folder_changed(&mut self, cx: &mut Context<Self>) {
        self.usage = None;
        self.refresh_usage(cx);
        if self.enabled() {
            self.stop_run();
            self.start(cx);
        }
    }

    /// "Compress now" (also used right after turning it on).
    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.scheduled = None;
        if !self.enabled() || self.game_running() {
            return;
        }
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let settings = cx.background_spawn(async move { heronite_data::settings::load(&data_paths) }).await;
            let folder = settings.ok().and_then(|s| s.screenshots_folder().map(Path::to_path_buf));
            this.update(cx, |c, cx| c.send_run(folder, cx)).ok();
        })
        .detach();
    }

    fn send_run(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        // The state may have changed while the settings were read.
        if !self.enabled() || self.game_running() {
            return;
        }
        let Some(folder) = folder.filter(|f| f.is_dir()) else {
            self.phase = Phase::NoFolder;
            cx.notify();
            return;
        };
        self.stop_run();
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.current = Some((self.next_id, cancel.clone()));
        self.totals = RunTotals::default();
        self.phase = Phase::Scanning;
        let run = Run { id: self.next_id, folder, cancel };
        if let Some(runs) = self.runs() {
            let _ = runs.send(run);
        }
        cx.notify();
    }

    /// The worker thread, started by the first run (compression is off by
    /// default: most sessions never need it).
    fn runs(&mut self) -> Option<&Sender<Run>> {
        if self.runs.is_none() {
            let (runs, rx) = channel::<Run>();
            let (tx, data_paths) = (self.reports_tx.clone(), self.data_paths.clone());
            match std::thread::Builder::new().name("screenshot-compress".into()).spawn(move || worker(rx, tx, data_paths)) {
                Ok(_) => self.runs = Some(runs),
                Err(e) => log::error!("could not start the screenshot compression thread: {e}"),
            }
        }
        self.runs.as_ref()
    }

    fn schedule(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if !self.enabled() {
            return;
        }
        if !self.busy() {
            self.phase = Phase::Waiting;
        }
        self.scheduled = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |c, cx| c.start(cx)).ok();
        }));
    }

    /// Measures the folder in the background (settings page, end of a run).
    pub fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        self.usage_task = Some(cx.spawn(async move |this, cx| {
            let usage = cx
                .background_spawn(async move {
                    let settings = heronite_data::settings::load(&data_paths).ok()?;
                    let folder = settings.screenshots_folder()?.to_path_buf();
                    folder.is_dir().then(|| compress::usage(&folder))
                })
                .await;
            this.update(cx, |c, cx| {
                c.usage = Some(usage.unwrap_or_default());
                cx.notify();
            })
            .ok();
        }));
    }

    fn on_reports(&mut self, batch: Vec<Report>, cx: &mut Context<Self>) {
        let mut converted = Vec::new();
        let current = self.current.as_ref().map(|(id, _)| *id);
        for report in batch {
            match report {
                // Reports of a run that was replaced only count conversions.
                Report::Started { run, .. } | Report::Progress { run, .. } | Report::Finished { run, .. } if Some(run) != current => {}
                Report::Started { total, .. } => {
                    if !self.game_running() {
                        self.phase = Phase::Working { done: 0, total, current: String::new() };
                    }
                }
                Report::Progress { done, total, current, .. } => {
                    if self.enabled() && !self.game_running() {
                        self.phase = Phase::Working { done, total, current };
                    }
                }
                Report::Converted { from, to, freed } => {
                    self.totals.converted += 1;
                    self.totals.freed += freed;
                    if let Some(u) = &mut self.usage {
                        u.total_bytes = u.total_bytes.saturating_sub(freed.max(0) as u64);
                        u.png_files = u.png_files.saturating_sub(1);
                        u.compressed_files += 1;
                        u.saved_bytes += freed.max(0) as u64;
                    }
                    converted.push((from, to));
                }
                Report::Failed { error } => {
                    self.totals.failed += 1;
                    self.totals.last_error = Some(error);
                }
                Report::ArtChanged => cx.emit(CompressionEvent::ArtChanged),
                Report::Finished { left, cancelled, .. } => {
                    self.current = None;
                    self.phase = if !self.enabled() {
                        Phase::Off
                    } else if self.game_running() {
                        Phase::Paused { left: Some(left) }
                    } else if cancelled {
                        // Stopped by a quit: nothing more happens.
                        Phase::Waiting
                    } else {
                        Phase::Idle
                    };
                    self.refresh_usage(cx);
                }
            }
        }
        if !converted.is_empty() {
            cx.emit(CompressionEvent::Converted(converted));
        }
        cx.notify();
    }
}

/// The worker thread: one run at a time, each over a fresh listing.
fn worker(runs: Receiver<Run>, reports: UnboundedSender<Report>, data_paths: DataPaths) {
    heronite_data::proc::idle_priority();
    let cache = ThumbnailCache::new(data_paths.screenshot_thumbnails_dir(), 0.0);
    // Files that failed this session are not retried until the next start.
    let mut failed: HashSet<PathBuf> = HashSet::new();
    while let Ok(mut run) = runs.recv() {
        // Requests queued while the last run went are one run now.
        while let Ok(newer) = runs.try_recv() {
            run = newer;
        }
        let (id, cancel) = (run.id, &*run.cancel);
        let send = |r: Report| reports.unbounded_send(r).is_ok();
        if cancel.load(Ordering::SeqCst) {
            send(Report::Finished { run: id, left: 0, cancelled: true });
            continue;
        }
        let files: Vec<PathBuf> = compress::pending(&run.folder).into_iter().filter(|p| !failed.contains(p)).collect();
        let total = files.len();
        log::info!("screenshot compression: {total} capture(s) to compress in {}", run.folder.display());
        if !send(Report::Started { run: id, total }) {
            return;
        }
        let mut renamed: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut done = 0;
        let mut cancelled = false;
        for path in files {
            if cancel.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }
            let current = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            send(Report::Progress { run: id, done, total, current });
            match compress::compress_file(&path, cancel) {
                Ok(Outcome::Converted { from, to, saved }) => {
                    log::info!("compressed {} -> {} ({} KB freed)", from.display(), to.display(), saved / 1024);
                    cache.rename_source(&from, &to);
                    renamed.push((from.clone(), to.clone()));
                    send(Report::Converted { from, to, freed: saved });
                }
                Ok(Outcome::Kept { reason }) => log::info!("kept {} as PNG: {reason}", path.display()),
                Ok(Outcome::Cancelled) => {
                    cancelled = true;
                    break;
                }
                Err(error) => {
                    log::warn!("compressing {}: {error}", path.display());
                    failed.insert(path);
                    send(Report::Failed { error });
                }
            }
            done += 1;
            if renamed.len() >= REFS_BATCH {
                update_refs(&data_paths, &run.folder, &mut renamed, &send);
            }
        }
        update_refs(&data_paths, &run.folder, &mut renamed, &send);
        log::info!("screenshot compression {}: {done} of {total} handled", if cancelled { "stopped" } else { "finished" });
        if !send(Report::Finished { run: id, left: total - done, cancelled }) {
            return;
        }
        crate::trim_heap_later();
    }
}

fn update_refs(data_paths: &DataPaths, root: &Path, renamed: &mut Vec<(PathBuf, PathBuf)>, send: &impl Fn(Report) -> bool) {
    match compress::rewrite_art_refs(data_paths, root, renamed) {
        Ok(0) => {}
        Ok(n) => {
            log::info!("{n} cover/background reference(s) follow converted captures");
            send(Report::ArtChanged);
        }
        Err(e) => log::warn!("updating art references: {e}"),
    }
    renamed.clear();
}
