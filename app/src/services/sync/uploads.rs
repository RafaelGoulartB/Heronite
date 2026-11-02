//! The uploads to the cloud account besides the library sync
//! (`heronite_sync::cloud`): the backups as they are made, Ludusavi's
//! backups after each game, the screenshots once compressed (each only when
//! set to automatic), and "Send now" (asked by the user, `manual`: manual
//! ones go too). One at a time, in a queue:
//! - never while a game launched from here runs: a launch stops the current
//!   upload, which goes back first in the queue (rclone sends only what is
//!   not there yet, so nothing is sent twice);
//! - the screenshots wait for the compression to have nothing left, so a PNG
//!   about to become a WebP is never sent;
//! - an account that takes no more requests for now (Google Drive's quota a
//!   minute) gets the upload again later, waiting longer each time
//!   ([`RETRY_AFTER`]);
//! - closing the app stops the upload (rclone leaves nothing half made in the
//!   account); what it had not sent shows as not sent and goes at the next
//!   start (`LatestBackup`, the saves and the screenshots are queued then).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::proc::Cancel;
use heronite_data::screenshot_prefs::ScreenshotPrefs;
use heronite_data::settings::{self, DEFAULT_BACKUP_KEEP};
use heronite_sync::cloud::{self, Upload};
use heronite_sync::SyncError;

use super::LibrarySync;

/// How long an upload waits after the account refused requests: longer
/// each time it happens again (the last one then stays).
const RETRY_AFTER: [Duration; 4] =
    [Duration::from_secs(2 * 60), Duration::from_secs(5 * 60), Duration::from_secs(15 * 60), Duration::from_secs(30 * 60)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// The backup folder just made.
    Backup(PathBuf),
    /// The newest backup of the backup folder, unless it went up already
    /// (at start: one a closed app did not finish).
    LatestBackup,
    Saves,
    Captures,
}

impl Job {
    pub fn upload(&self) -> Upload {
        match self {
            Job::Backup(_) | Job::LatestBackup => Upload::Backups,
            Job::Saves => Upload::Saves,
            Job::Captures => Upload::Captures,
        }
    }
}

/// The upload running now.
pub struct Uploading {
    pub job: Job,
    manual: bool,
    /// How far rclone says it is.
    pub percent: Option<u8>,
    cancel: Arc<Cancel>,
}

#[derive(Default)]
pub(super) struct Queue {
    /// Each job, and whether the user asked for it.
    jobs: VecDeque<(Job, bool)>,
    current: Option<Uploading>,
    /// Times in a row the account refused each kind, and when it goes again.
    refused: HashMap<Upload, (usize, SystemTime)>,
}

impl LibrarySync {
    pub fn uploading(&self) -> Option<&Uploading> {
        self.queue.current.as_ref()
    }

    /// Uploads waiting for their turn.
    pub fn queued(&self) -> usize {
        self.queue.jobs.len()
    }

    /// When `upload` goes again after the account refused requests.
    pub fn retry_at(&self, upload: Upload) -> Option<SystemTime> {
        self.queue.refused.get(&upload).map(|(_, at)| *at)
    }

    /// Adds `job` to the queue (once; a newer backup replaces one waiting)
    /// and starts it when nothing is in the way. `manual`: the user asked
    /// for it, so it goes even when set to manual.
    pub fn upload(&mut self, job: Job, manual: bool, cx: &mut Context<Self>) {
        let jobs = &mut self.queue.jobs;
        if job.upload() == Upload::Backups {
            jobs.retain(|(j, _)| j.upload() != Upload::Backups);
        }
        if let Some(queued) = jobs.iter_mut().find(|(j, _)| *j == job) {
            queued.1 |= manual;
        } else if self.queue.current.as_ref().is_none_or(|c| c.job != job || (manual && !c.manual)) {
            jobs.push_back((job, manual));
        }
        self.pump(cx);
    }

    /// The saves and the screenshots now (the backups go as they are made,
    /// or with their own "Send now").
    pub fn upload_all(&mut self, manual: bool, cx: &mut Context<Self>) {
        self.upload(Job::Saves, manual, cx);
        self.upload(Job::Captures, manual, cx);
    }

    /// Stops the upload running and empties the queue; the next game or
    /// timer starts them again.
    pub fn stop_uploads(&mut self, cx: &mut Context<Self>) {
        self.queue.jobs.clear();
        if let Some(c) = &self.queue.current {
            c.cancel.request();
        }
        cx.notify();
    }

    /// A game started: the upload stops and goes back first in the queue.
    pub(super) fn pause_for_game(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.queue.current {
            c.cancel.request();
            let queued = (c.job.clone(), c.manual);
            if !self.queue.jobs.iter().any(|(j, _)| *j == queued.0) {
                self.queue.jobs.push_front(queued);
            }
        }
        cx.notify();
    }

    /// Starts the next upload, unless one runs, a game runs, or the
    /// screenshots wait for the compression.
    pub(super) fn pump(&mut self, cx: &mut Context<Self>) {
        if self.queue.current.is_some() || crate::services::session::any_running(cx) {
            return;
        }
        let compressing = crate::services::compression::get(cx).is_some_and(|c| c.read(cx).busy());
        let Some(at) = self.queue.jobs.iter().position(|(j, _)| !(compressing && *j == Job::Captures)) else { return };
        let Some((job, manual)) = self.queue.jobs.remove(at) else { return };
        let cancel = Cancel::new();
        self.queue.current = Some(Uploading { job: job.clone(), manual, percent: None, cancel: cancel.clone() });
        cx.notify();

        enum Msg {
            Percent(u8),
            Done(Result<(), SyncError>),
        }
        let (tx, mut rx) = unbounded::<Msg>();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        // Uploads can take minutes: a thread of their own.
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let mut on_percent = |p: u8| {
                progress_tx.unbounded_send(Msg::Percent(p)).ok();
            };
            let result = run_job(&data, &app, &job, manual, &cancel, &mut on_percent);
            tx.unbounded_send(Msg::Done(result)).ok();
        });
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = matches!(msg, Msg::Done(_));
                this.update(cx, |s, cx| {
                    match msg {
                        Msg::Percent(p) => {
                            if let Some(c) = &mut s.queue.current {
                                c.percent = Some(p);
                            }
                        }
                        Msg::Done(result) => {
                            s.changed();
                            if let Some(c) = s.queue.current.take() {
                                match result {
                                    Ok(()) => {
                                        s.queue.refused.remove(&c.job.upload());
                                    }
                                    Err(SyncError::Off | SyncError::Cancelled) => {}
                                    Err(SyncError::RateLimited) => s.retry_later(c.job, c.manual, cx),
                                    Err(e) => log::warn!("cloud: {:?} upload: {}", c.job.upload(), e.localized()),
                                }
                            }
                            s.pump(cx);
                        }
                    }
                    cx.notify();
                })
                .ok();
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// The account refused requests: `job` goes again after a wait, longer
    /// each time in a row.
    fn retry_later(&mut self, job: Job, manual: bool, cx: &mut Context<Self>) {
        let upload = job.upload();
        let times = self.queue.refused.get(&upload).map_or(0, |(n, _)| *n);
        let wait = RETRY_AFTER[times.min(RETRY_AFTER.len() - 1)];
        self.queue.refused.insert(upload, (times + 1, SystemTime::now() + wait));
        log::info!("cloud: the account refused requests; {upload:?} goes again in {} min", wait.as_secs() / 60);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |s, cx| s.upload(job, manual, cx)).ok();
        })
        .detach();
    }

    /// The app is closing: the upload stops (the next start sends what it
    /// had not).
    pub(super) fn stop_for_quit(&mut self) {
        self.queue.jobs.clear();
        if let Some(c) = &self.queue.current {
            c.cancel.request();
        }
    }
}

/// Runs one upload (blocking): finds what it needs in the settings.
fn run_job(
    data: &heronite_data::DataPaths,
    app: &heronite_data::AppPaths,
    job: &Job,
    manual: bool,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let s = settings::load(data).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    match job {
        Job::Backup(path) => cloud::upload_backup(data, app, path, s.backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP), manual, cancel),
        Job::LatestBackup => {
            let Some(path) = s.backup.last_backup_path.as_deref().map(PathBuf::from).filter(|p| p.is_dir()) else {
                return Err(SyncError::Off);
            };
            cloud::upload_backup(data, app, &path, s.backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP), manual, cancel)
        }
        Job::Saves => {
            // Where Ludusavi last put a game's backup: its backup folder,
            // whether set here or in Ludusavi.
            let root = s.ludusavi.last_backup_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
            let Some(root) = root else { return Err(SyncError::Off) };
            cloud::upload_saves(data, app, &root, manual, cancel, on_percent)
        }
        Job::Captures => {
            let Some(folder) = s.screenshots_folder().map(Path::to_path_buf).or_else(|| data.default_screenshots()) else {
                return Err(SyncError::Off);
            };
            let pending_png = ScreenshotPrefs::load(app).compress_lossless;
            cloud::upload_captures(data, app, &folder, pending_png, manual, cancel, on_percent)
        }
    }
}
