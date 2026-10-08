//! Background jobs of the Wine Manager (installs with progress and cancel,
//! removals, release refreshes). The state lives here, not in a view, so
//! downloads continue while the page is closed. Threads are plain
//! `std::thread`s; the UI polls [`WineManager::job`] / [`WineManager::take_events`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use heronite_data::HeroicPaths;
use parking_lot::Mutex;

use crate::install::{self, InstallError};
use crate::repos::Repository;
use crate::store;
use crate::types::{Progress, WineConfig, WineVersionInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Install,
    Update,
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobPhase {
    /// Queued/starting (fetching the checksum, no bytes yet).
    Starting,
    Downloading { percentage: f64, downloaded: u64, total: u64, speed: f64, eta_secs: Option<u64> },
    Unzipping,
    Removing,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub kind: JobKind,
    pub phase: JobPhase,
}

/// Something the UI should react to (toast, reload the list).
#[derive(Clone, Debug, PartialEq)]
pub enum JobEvent {
    Installed { version: String, update: bool },
    Cancelled { version: String },
    Removed { version: String },
    Failed { version: String, message: String },
    /// The release list was refreshed; `errors` lists repositories that failed.
    Refreshed { errors: Vec<String> },
    RefreshFailed { message: String },
}

struct Active {
    job: Job,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    jobs: HashMap<String, Active>,
    events: Vec<JobEvent>,
    refreshing: bool,
}

struct Inner {
    heroic: HeroicPaths,
    state: Mutex<State>,
    revision: AtomicU64,
}

/// Cheap to clone (shared state).
#[derive(Clone)]
pub struct WineManager {
    inner: Arc<Inner>,
}

impl WineManager {
    pub fn new(heroic: HeroicPaths) -> Self {
        Self { inner: Arc::new(Inner { heroic, state: Mutex::new(State::default()), revision: AtomicU64::new(0) }) }
    }

    pub fn heroic(&self) -> &HeroicPaths {
        &self.inner.heroic
    }

    /// Bumped on every state change (progress included).
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.inner.revision.fetch_add(1, Ordering::Relaxed);
    }

    pub fn job(&self, version: &str) -> Option<Job> {
        self.inner.state.lock().jobs.get(version).map(|a| a.job.clone())
    }

    pub fn has_active_jobs(&self) -> bool {
        let s = self.inner.state.lock();
        !s.jobs.is_empty() || s.refreshing
    }

    pub fn refreshing(&self) -> bool {
        self.inner.state.lock().refreshing
    }

    /// Events since the last call.
    pub fn take_events(&self) -> Vec<JobEvent> {
        std::mem::take(&mut self.inner.state.lock().events)
    }

    /// Asks a running install to stop (cleanup happens on its thread).
    pub fn cancel(&self, version: &str) {
        if let Some(a) = self.inner.state.lock().jobs.get(version) {
            a.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Registers a job; `false` when one for that version is already running.
    fn begin(&self, version: &str, kind: JobKind, phase: JobPhase) -> Option<Arc<AtomicBool>> {
        let mut s = self.inner.state.lock();
        if s.jobs.contains_key(version) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        s.jobs.insert(version.to_owned(), Active { job: Job { kind, phase }, cancel: cancel.clone() });
        drop(s);
        self.bump();
        Some(cancel)
    }

    fn set_phase(&self, version: &str, phase: JobPhase) {
        if let Some(a) = self.inner.state.lock().jobs.get_mut(version) {
            a.job.phase = phase;
        }
        self.bump();
    }

    fn finish(&self, version: &str, event: JobEvent) {
        let mut s = self.inner.state.lock();
        s.jobs.remove(version);
        s.events.push(event);
        drop(s);
        self.bump();
    }

    /// Downloads and installs (or updates, when `info.has_update`) a build.
    pub fn start_install(&self, info: WineVersionInfo) -> bool {
        let update = info.has_update();
        let kind = if update { JobKind::Update } else { JobKind::Install };
        let Some(cancel) = self.begin(&info.version, kind, JobPhase::Starting) else { return false };
        let this = self.clone();
        std::thread::Builder::new()
            .name(format!("wine-install-{}", info.version))
            .spawn(move || {
                let cfg = WineConfig::load(&this.inner.heroic);
                let version = info.version.clone();
                let result = install::install_version(&this.inner.heroic, &cfg, &info, &cancel, &mut |p| {
                    let phase = match p {
                        Progress::Downloading { percentage, downloaded, total, speed, eta_secs } => {
                            JobPhase::Downloading { percentage, downloaded, total, speed, eta_secs }
                        }
                        Progress::Unzipping => JobPhase::Unzipping,
                    };
                    this.set_phase(&version, phase);
                });
                let event = match result {
                    Ok(_) => JobEvent::Installed { version: version.clone(), update },
                    Err(InstallError::Aborted) => JobEvent::Cancelled { version: version.clone() },
                    Err(InstallError::Failed(message)) => {
                        log::error!("wine manager: install of {version} failed: {message}");
                        JobEvent::Failed { version: version.clone(), message }
                    }
                };
                this.finish(&version, event);
            })
            .is_ok()
    }

    /// Deletes an installed build.
    pub fn start_remove(&self, info: WineVersionInfo) -> bool {
        if self.begin(&info.version, JobKind::Remove, JobPhase::Removing).is_none() {
            return false;
        }
        let this = self.clone();
        std::thread::Builder::new()
            .name(format!("wine-remove-{}", info.version))
            .spawn(move || {
                let version = info.version.clone();
                let event = match install::remove_version(&this.inner.heroic, &info) {
                    Ok(()) => JobEvent::Removed { version: version.clone() },
                    Err(e) => JobEvent::Failed { version: version.clone(), message: format!("{e:#}") },
                };
                this.finish(&version, event);
            })
            .is_ok()
    }

    /// Re-fetches the release lists from GitHub (one request per repository)
    /// and merges them into `wine-downloader-info.json`.
    pub fn start_refresh(&self, repos: Vec<Repository>) -> bool {
        {
            let mut s = self.inner.state.lock();
            if s.refreshing {
                return false;
            }
            s.refreshing = true;
        }
        self.bump();
        let this = self.clone();
        std::thread::Builder::new()
            .name("wine-refresh".into())
            .spawn(move || {
                let event = match store::refresh(&this.inner.heroic, &repos, 50) {
                    Ok(o) => JobEvent::Refreshed { errors: o.errors },
                    Err(e) => JobEvent::RefreshFailed { message: format!("{e:#}") },
                };
                let mut s = this.inner.state.lock();
                s.refreshing = false;
                s.events.push(event);
                drop(s);
                this.bump();
            })
            .is_ok()
    }
}
