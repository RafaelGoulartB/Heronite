//! The uploads to the cloud account besides the library sync
//! (`heronite_sync::cloud`): the backups as they are made, Ludusavi's
//! backups after each game, the screenshots once compressed. One at a time,
//! in a queue:
//! - never while a game launched from here runs: a launch stops the current
//!   upload, which goes back first in the queue (rclone sends only what is
//!   not there yet, so nothing is sent twice);
//! - the screenshots wait for the compression to have nothing left, so a PNG
//!   about to become a WebP is never sent.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::proc::Cancel;
use heronite_data::screenshot_prefs::ScreenshotPrefs;
use heronite_data::settings::{self, DEFAULT_BACKUP_KEEP};
use heronite_sync::cloud::{self, Upload};
use heronite_sync::SyncError;

use super::LibrarySync;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// The backup folder just made.
    Backup(PathBuf),
    Saves,
    Captures,
}

impl Job {
    pub fn upload(&self) -> Upload {
        match self {
            Job::Backup(_) => Upload::Backups,
            Job::Saves => Upload::Saves,
            Job::Captures => Upload::Captures,
        }
    }
}

/// The upload running now.
pub struct Uploading {
    pub job: Job,
    /// How far rclone says it is.
    pub percent: Option<u8>,
    cancel: Arc<Cancel>,
}

#[derive(Default)]
pub(super) struct Queue {
    jobs: VecDeque<Job>,
    current: Option<Uploading>,
    /// Bringing the backups from the account (Settings → Cloud).
    fetching: bool,
    /// How the last fetch went: backups brought, or why it failed.
    fetched: Option<Result<usize, String>>,
}

impl LibrarySync {
    pub fn uploading(&self) -> Option<&Uploading> {
        self.queue.current.as_ref()
    }

    /// Uploads waiting for their turn.
    pub fn queued(&self) -> usize {
        self.queue.jobs.len()
    }

    pub fn fetching(&self) -> bool {
        self.queue.fetching
    }

    pub fn fetched(&self) -> Option<&Result<usize, String>> {
        self.queue.fetched.as_ref()
    }

    /// Adds `job` to the queue (once; a newer backup replaces one waiting)
    /// and starts it when nothing is in the way.
    pub fn upload(&mut self, job: Job, cx: &mut Context<Self>) {
        let jobs = &mut self.queue.jobs;
        if matches!(job, Job::Backup(_)) {
            jobs.retain(|j| !matches!(j, Job::Backup(_)));
        }
        if !jobs.contains(&job) && self.queue.current.as_ref().is_none_or(|c| c.job != job) {
            jobs.push_back(job);
        }
        self.pump(cx);
    }

    /// Everything that is on, now: the saves and the screenshots (the
    /// backups go as they are made).
    pub fn upload_all(&mut self, cx: &mut Context<Self>) {
        self.upload(Job::Saves, cx);
        self.upload(Job::Captures, cx);
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
            let job = c.job.clone();
            if !self.queue.jobs.contains(&job) {
                self.queue.jobs.push_front(job);
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
        let Some(at) = self.queue.jobs.iter().position(|j| !(compressing && *j == Job::Captures)) else { return };
        let Some(job) = self.queue.jobs.remove(at) else { return };
        let cancel = Cancel::new();
        self.queue.current = Some(Uploading { job: job.clone(), percent: None, cancel: cancel.clone() });
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
            let result = run_job(&data, &app, &job, &cancel, &mut on_percent);
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
                            if let Some(c) = s.queue.current.take() {
                                match result {
                                    Ok(()) | Err(SyncError::Off | SyncError::Cancelled) => {}
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

    /// Brings the newest backup of each PC from the account into the backup
    /// folder (Settings → Backups then lists it).
    pub fn fetch_backups(&mut self, cx: &mut Context<Self>) {
        if self.queue.fetching {
            return;
        }
        self.queue.fetching = true;
        self.queue.fetched = None;
        cx.notify();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let folder = settings::load(&data).map(|s| s.backup.folder).unwrap_or_default();
                    if folder.trim().is_empty() {
                        return Err(SyncError::FolderMissing(String::new()));
                    }
                    cloud::fetch_backups(&data, &app, Path::new(folder.trim()), &Cancel::new())
                })
                .await;
            this.update(cx, |s, cx| {
                s.queue.fetching = false;
                s.queue.fetched = Some(result.map_err(|e| match e {
                    SyncError::FolderMissing(f) if f.is_empty() => {
                        heronite_i18n::tr!("Choose a backup folder in Settings → Backups first.").to_string()
                    }
                    e => e.localized(),
                }));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Runs one upload (blocking): finds what it needs in the settings.
fn run_job(
    data: &heronite_data::DataPaths,
    app: &heronite_data::AppPaths,
    job: &Job,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(u8),
) -> Result<(), SyncError> {
    let s = settings::load(data).map_err(|e| SyncError::Data(format!("{e:#}")))?;
    match job {
        Job::Backup(path) => cloud::upload_backup(data, app, path, s.backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP), cancel),
        Job::Saves => {
            // Where Ludusavi last put a game's backup: its backup folder,
            // whether set here or in Ludusavi.
            let root = s.ludusavi.last_backup_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
            let Some(root) = root else { return Err(SyncError::Off) };
            cloud::upload_saves(data, app, &root, cancel, on_percent)
        }
        Job::Captures => {
            let Some(folder) = s.screenshots_folder().map(Path::to_path_buf).or_else(|| data.default_screenshots()) else {
                return Err(SyncError::Off);
            };
            let pending_png = ScreenshotPrefs::load(app).compress_lossless;
            cloud::upload_captures(data, app, &folder, pending_png, cancel, on_percent)
        }
    }
}
