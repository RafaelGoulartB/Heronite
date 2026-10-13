//! Background jobs of the Wine Manager (installs with progress and cancel,
//! removals, release checks). The state lives here, not in a view, so
//! downloads continue while the page is closed. Threads are plain
//! `std::thread`s; the UI polls [`WineManager::job`] / [`WineManager::take_events`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use heronite_data::DataPaths;
use parking_lot::Mutex;

use crate::checks::{self, CheckMode, CheckOutcome};
use crate::install::{self, InstallError};
use crate::repos::Repository;
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
    Downloading {
        percentage: f64,
        downloaded: u64,
        total: u64,
        speed: f64,
        eta_secs: Option<u64>,
    },
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
    Installed {
        version: String,
        update: bool,
    },
    Cancelled {
        version: String,
    },
    Removed {
        version: String,
    },
    Failed {
        version: String,
        message: String,
    },
    /// The releases of `repo` were checked (see [`checks`]).
    Checked {
        repo: Repository,
        mode: CheckMode,
        outcome: CheckOutcome,
    },
}

struct Active {
    job: Job,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    jobs: HashMap<String, Active>,
    events: Vec<JobEvent>,
    /// Sources waiting to be checked, in order; the first one is being checked
    /// while `checking` is set.
    checks: Vec<(Repository, CheckMode)>,
    checking: bool,
}

struct Inner {
    data_paths: DataPaths,
    remote: Arc<dyn checks::Remote>,
    state: Mutex<State>,
    revision: AtomicU64,
}

/// Cheap to clone (shared state).
#[derive(Clone)]
pub struct WineManager {
    inner: Arc<Inner>,
}

impl WineManager {
    pub fn new(data_paths: DataPaths) -> Self {
        Self::with_remote(data_paths, Arc::new(checks::GitHub))
    }

    /// Checks releases through `remote` instead of GitHub (tests).
    pub fn with_remote(data_paths: DataPaths, remote: Arc<dyn checks::Remote>) -> Self {
        Self { inner: Arc::new(Inner { data_paths, remote, state: Mutex::new(State::default()), revision: AtomicU64::new(0) }) }
    }

    pub fn data_paths(&self) -> &DataPaths {
        &self.inner.data_paths
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
        !s.jobs.is_empty() || s.checking
    }

    /// `repo` is being checked or waits for its turn.
    pub fn checking(&self, repo: Repository) -> bool {
        self.inner.state.lock().checks.iter().any(|(r, _)| *r == repo)
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
                let cfg = WineConfig::load(&this.inner.data_paths);
                let version = info.version.clone();
                let result = install::install_version(&this.inner.data_paths, &cfg, &info, &cancel, &mut |p| {
                    let phase = match p {
                        Progress::Downloading { percentage, downloaded, total, speed, eta_secs } => {
                            JobPhase::Downloading { percentage, downloaded, total, speed, eta_secs }
                        }
                        Progress::Unzipping => JobPhase::Unzipping,
                    };
                    this.set_phase(&version, phase);
                });
                let event = match result {
                    Ok(_) => {
                        // The first build of a new setup becomes the default.
                        crate::default::ensure_default(&this.inner.data_paths);
                        JobEvent::Installed { version: version.clone(), update }
                    }
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
                let event = match install::remove_version(&this.inner.data_paths, &info) {
                    Ok(()) => JobEvent::Removed { version: version.clone() },
                    Err(e) => JobEvent::Failed { version: version.clone(), message: format!("{e:#}") },
                };
                this.finish(&version, event);
            })
            .is_ok()
    }

    /// Checks the releases of `repo` in the background (one at a time, in
    /// the order asked). Asking for a source already waiting does nothing,
    /// except that a manual request replaces an automatic one.
    pub fn start_check(&self, repo: Repository, mode: CheckMode) {
        let mut s = self.inner.state.lock();
        if let Some(i) = s.checks.iter().position(|(r, _)| *r == repo) {
            // The first one is already running as it is.
            if i > 0 && mode == CheckMode::Manual {
                s.checks[i].1 = mode;
            }
            return;
        }
        s.checks.push((repo, mode));
        if s.checking {
            return;
        }
        s.checking = true;
        drop(s);
        self.bump();
        let this = self.clone();
        let spawned = std::thread::Builder::new().name("wine-check".into()).spawn(move || loop {
            let Some((repo, mode)) = this.inner.state.lock().checks.first().copied() else { break };
            let outcome = checks::check_source(&this.inner.data_paths, &*this.inner.remote, repo, mode, now_secs());
            let mut s = this.inner.state.lock();
            s.checks.remove(0);
            s.events.push(JobEvent::Checked { repo, mode, outcome });
            if s.checks.is_empty() {
                s.checking = false;
                drop(s);
                this.bump();
                break;
            }
            drop(s);
            this.bump();
        });
        if let Err(e) = spawned {
            log::warn!("wine manager: could not start the release check: {e}");
            let mut s = self.inner.state.lock();
            s.checks.clear();
            s.checking = false;
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::tests::Fake;
    use std::time::{Duration, Instant};

    fn wait_events(m: &WineManager, n: usize) -> Vec<JobEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while events.len() < n && Instant::now() < deadline {
            events.extend(m.take_events());
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    #[test]
    fn checks_run_one_at_a_time_in_order_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::new("<feed></feed>"));
        // Holds the first check until every request is queued.
        let gate = fake.feed.lock().unwrap();
        let m = WineManager::with_remote(DataPaths::at(dir.path()), fake.clone());
        m.start_check(Repository::DwProton, CheckMode::Auto);
        m.start_check(Repository::ProtonEm, CheckMode::Auto);
        m.start_check(Repository::DwProton, CheckMode::Auto);
        m.start_check(Repository::ProtonEm, CheckMode::Manual);
        assert!(m.checking(Repository::ProtonEm) && !m.checking(Repository::ProtonGe));
        drop(gate);
        let events = wait_events(&m, 2);
        let done: Vec<(Repository, CheckMode)> = events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Checked { repo, mode, .. } => Some((*repo, *mode)),
                _ => None,
            })
            .collect();
        assert_eq!(done, [(Repository::DwProton, CheckMode::Auto), (Repository::ProtonEm, CheckMode::Manual)]);
        assert_eq!(*fake.asked.lock().unwrap(), [Repository::DwProton, Repository::ProtonEm]);
        assert!(!m.has_active_jobs());
    }
}
