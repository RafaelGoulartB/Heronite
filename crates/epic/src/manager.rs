//! The download manager (port of `downloadmanager/downloadqueue.ts`): a queue
//! of installs and updates that runs one at a time on a worker thread,
//! persisted in `store/download-manager.json`.

use std::sync::Arc;

use heronite_data::DataPaths;
use parking_lot::Mutex;

use crate::cmd::Cancel;
use crate::ops::remove_partial_download;
use crate::progress::Progress;
use crate::queue::{self, now_ms, DmElement, DmStatus};

/// What a job returns.
#[derive(Debug, Clone)]
pub struct JobResult {
    pub status: DmStatus,
    pub error: Option<String>,
}

/// Runs one queue element. Blocking; reports progress through `on_progress`
/// and stops when `cancel` is requested.
pub trait Job: Send + Sync {
    fn run(&self, element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult;
}

/// `DownloadManagerState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmState {
    Idle,
    Running,
    Paused,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    /// State, queue or history changed.
    Changed,
    /// New progress report of the running element.
    Progress { app_name: String, progress: Progress },
    /// An element ended (done, error or abort); `Changed` follows.
    Finished { element: DmElement, error: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: DmState,
    pub queue: Vec<DmElement>,
    pub finished: Vec<DmElement>,
    /// The element that runs now (first of the queue) and its last report.
    pub current: Option<(String, Option<Progress>)>,
}

struct Current {
    app_name: String,
    cancel: Arc<Cancel>,
    progress: Option<Progress>,
    pause_requested: bool,
    /// `(path, folder)` to delete once the process stopped (cancel + delete).
    remove_on_abort: Option<(String, String)>,
}

struct Inner {
    state: DmState,
    queue: Vec<DmElement>,
    finished: Vec<DmElement>,
    current: Option<Current>,
    worker: bool,
}

struct Shared {
    data_paths: DataPaths,
    /// In-memory only (debug fakes): never reads or writes download-manager.json.
    ephemeral: bool,
    job: Arc<dyn Job>,
    notify: Arc<dyn Fn(Event) + Send + Sync>,
    inner: Mutex<Inner>,
}

#[derive(Clone)]
pub struct DownloadManager {
    shared: Arc<Shared>,
}

impl DownloadManager {
    /// Loads the persisted queue. Nothing runs until [`start`](Self::start).
    pub fn new(data_paths: DataPaths, job: Arc<dyn Job>, notify: impl Fn(Event) + Send + Sync + 'static) -> Self {
        let file = queue::load(&data_paths);
        Self::with_state(data_paths, job, notify, file.queue, file.finished, false)
    }

    /// A queue that lives only in memory (screenshots / debug fakes): it
    /// neither loads nor saves `store/download-manager.json`.
    pub fn new_ephemeral(data_paths: DataPaths, job: Arc<dyn Job>, notify: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self::with_state(data_paths, job, notify, Vec::new(), Vec::new(), true)
    }

    fn with_state(
        data_paths: DataPaths,
        job: Arc<dyn Job>,
        notify: impl Fn(Event) + Send + Sync + 'static,
        queue: Vec<DmElement>,
        finished: Vec<DmElement>,
        ephemeral: bool,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                data_paths,
                ephemeral,
                job,
                notify: Arc::new(notify),
                inner: Mutex::new(Inner {
                    state: DmState::Idle,
                    queue,
                    finished,
                    current: None,
                    worker: false,
                }),
            }),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let g = self.shared.inner.lock();
        Snapshot {
            state: g.state,
            queue: g.queue.clone(),
            finished: g.finished.clone(),
            current: g.queue.first().map(|e| {
                let progress = g.current.as_ref().filter(|c| c.app_name == e.params.app_name).and_then(|c| c.progress.clone());
                (e.params.app_name.clone(), progress)
            }),
        }
    }

    fn persist(&self, g: &Inner) {
        if self.shared.ephemeral {
            return;
        }
        if let Err(err) = queue::save(&self.shared.data_paths, &g.queue, &g.finished) {
            log::error!("saving the download queue: {err:#}");
        }
    }

    fn emit(&self, ev: Event) {
        (self.shared.notify)(ev);
    }

    /// `addToQueue`: replaces an element of the same game, otherwise appends;
    /// starts the queue when it was idle.
    pub fn add(&self, element: DmElement) {
        {
            let mut g = self.shared.inner.lock();
            let running_same = g
                .current
                .as_ref()
                .is_some_and(|c| c.app_name == element.params.app_name);
            if running_same {
                return;
            }
            match g.queue.iter().position(|e| e.same_game(&element.params.app_name, &element.params.runner)) {
                Some(i) => g.queue[i] = element,
                None => g.queue.push(element),
            }
            self.persist(&g);
        }
        self.emit(Event::Changed);
        let idle = self.shared.inner.lock().state == DmState::Idle;
        if idle {
            self.start();
        }
    }

    /// `initQueue`: runs the queue unless it is paused or already running.
    pub fn start(&self) {
        {
            let mut g = self.shared.inner.lock();
            if g.worker || g.queue.is_empty() {
                if g.state == DmState::Paused && !g.worker {
                    g.state = DmState::Idle;
                }
                return;
            }
            g.state = DmState::Running;
            g.worker = true;
        }
        self.emit(Event::Changed);
        let me = self.clone();
        std::thread::Builder::new()
            .name("epic-downloads".into())
            .spawn(move || me.worker_loop())
            .expect("spawning the download worker");
    }

    fn worker_loop(&self) {
        loop {
            let (element, cancel) = {
                let mut g = self.shared.inner.lock();
                if g.state == DmState::Paused {
                    break;
                }
                let Some(first) = g.queue.first_mut() else { break };
                first.start_time = now_ms();
                let element = first.clone();
                let cancel = Cancel::new();
                g.state = DmState::Running;
                g.current = Some(Current {
                    app_name: element.params.app_name.clone(),
                    cancel: cancel.clone(),
                    progress: None,
                    pause_requested: false,
                    remove_on_abort: None,
                });
                self.persist(&g);
                (element, cancel)
            };
            self.emit(Event::Changed);

            let app = element.params.app_name.clone();
            let mut report = |p: Progress| {
                if let Some(c) = self.shared.inner.lock().current.as_mut() {
                    c.progress = Some(p.clone());
                }
                self.emit(Event::Progress { app_name: app.clone(), progress: p });
            };
            let result = self.shared.job.run(&element, &cancel, &mut report);

            let mut ended = element.clone();
            ended.end_time = now_ms();
            let (finished_event, remove) = {
                let mut g = self.shared.inner.lock();
                let cur = g.current.take();
                let pause = cur.as_ref().is_some_and(|c| c.pause_requested);
                let remove = cur.and_then(|c| c.remove_on_abort);
                if pause {
                    if let Some(first) = g.queue.iter_mut().find(|e| e.params.app_name == app) {
                        first.start_time = ended.start_time;
                    }
                    self.persist(&g);
                    (None, remove)
                } else {
                    ended.status = Some(result.status);
                    match g.finished.iter().position(|e| e.params.app_name == app) {
                        Some(i) => g.finished[i] = ended.clone(),
                        None => g.finished.push(ended.clone()),
                    }
                    g.queue.retain(|e| e.params.app_name != app);
                    self.persist(&g);
                    (Some(Event::Finished { element: ended, error: result.error.clone() }), remove)
                }
            };
            if let Some((path, folder)) = remove {
                remove_partial_download(&path, &folder);
            }
            if let Some(ev) = finished_event {
                self.emit(ev);
            }
            self.emit(Event::Changed);
        }
        let mut g = self.shared.inner.lock();
        g.worker = false;
        if g.state != DmState::Paused {
            g.state = DmState::Idle;
        }
        drop(g);
        self.emit(Event::Changed);
    }

    /// `pauseCurrentDownload`: stops the process (legendary keeps its resume
    /// data); the element stays first in the queue.
    pub fn pause(&self) {
        {
            let mut g = self.shared.inner.lock();
            g.state = DmState::Paused;
            if let Some(c) = g.current.as_mut() {
                c.pause_requested = true;
                c.cancel.request();
            }
        }
        self.emit(Event::Changed);
    }

    /// `resumeCurrentDownload`.
    pub fn resume(&self) {
        {
            let mut g = self.shared.inner.lock();
            if g.state == DmState::Paused {
                g.state = DmState::Idle;
                if let Some(c) = g.current.as_mut() {
                    // Still stopping: it restarts as soon as the process is gone.
                    c.pause_requested = true;
                    g.state = DmState::Running;
                }
            }
        }
        self.start();
        self.emit(Event::Changed);
    }

    /// `cancelCurrentDownload`: stops the first element and removes it (and
    /// the DLCs queued with it); optionally deletes what was downloaded.
    pub fn cancel_current(&self, remove_downloaded: bool) {
        {
            let mut g = self.shared.inner.lock();
            let Some(first) = g.queue.first().cloned() else { return };
            let dlcs = first.params.install_dlcs.clone();
            g.queue.retain(|e| !dlcs.contains(&e.params.app_name));
            let remove = remove_downloaded
                .then(|| first.params.folder_name().map(|f| (first.params.path.clone(), f)))
                .flatten();
            match g.current.as_mut().filter(|c| c.app_name == first.params.app_name) {
                Some(c) => {
                    c.remove_on_abort = remove;
                    c.cancel.request();
                }
                None => {
                    // Not running (paused or idle): drop it right away.
                    g.queue.retain(|e| e.params.app_name != first.params.app_name);
                    if let Some((path, folder)) = remove {
                        remove_partial_download(&path, &folder);
                    }
                    if g.queue.is_empty() && g.state == DmState::Paused {
                        g.state = DmState::Idle;
                    }
                }
            }
            self.persist(&g);
        }
        self.emit(Event::Changed);
    }

    /// `removeFromQueue` for an element that is not running.
    pub fn remove(&self, app_name: &str) {
        {
            let mut g = self.shared.inner.lock();
            if g.current.as_ref().is_some_and(|c| c.app_name == app_name) {
                return;
            }
            let before = g.queue.len();
            g.queue.retain(|e| e.params.app_name != app_name);
            if g.queue.len() == before {
                return;
            }
            self.persist(&g);
        }
        self.emit(Event::Changed);
    }

    /// Moves a queued element (not the first) up or down.
    pub fn move_element(&self, app_name: &str, up: bool) {
        {
            let mut g = self.shared.inner.lock();
            let Some(i) = g.queue.iter().position(|e| e.params.app_name == app_name) else { return };
            let j = if up { i.checked_sub(1) } else { Some(i + 1) };
            let Some(j) = j.filter(|j| *j < g.queue.len() && *j >= 1 && i >= 1) else { return };
            g.queue.swap(i, j);
            self.persist(&g);
        }
        self.emit(Event::Changed);
    }

    pub fn clear_finished(&self) {
        {
            let mut g = self.shared.inner.lock();
            g.finished.clear();
            self.persist(&g);
        }
        self.emit(Event::Changed);
    }

    pub fn clear_finished_item(&self, app_name: &str) {
        {
            let mut g = self.shared.inner.lock();
            g.finished.retain(|e| e.params.app_name != app_name);
            self.persist(&g);
        }
        self.emit(Event::Changed);
    }

    /// The app is quitting: pauses the queue the way the user's pause does
    /// (the element stays first in the queue, legendary keeps its resume
    /// data) so the next start picks it up again. Does not block: poll
    /// [`is_idle`](Self::is_idle) to know when the worker is gone. The paused
    /// state itself is not persisted, only the queue.
    pub fn pause_for_shutdown(&self) {
        self.pause();
    }

    /// No worker thread and no element running: nothing is writing to the
    /// install folders because of the queue.
    pub fn is_idle(&self) -> bool {
        let g = self.shared.inner.lock();
        !g.worker && g.current.is_none()
    }

    /// Stops the running process and records it as aborted (it moves to
    /// `finished`, which defeats the resume: NOT for quitting the app, use
    /// [`pause_for_shutdown`](Self::pause_for_shutdown)).
    pub fn shutdown(&self) {
        let g = self.shared.inner.lock();
        if let Some(c) = g.current.as_ref() {
            c.cancel.request();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{ElementKind, InstallParams};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// A job that finishes instantly, or blocks until cancelled.
    struct FakeJob {
        blocking: Vec<String>,
        runs: Mutex<Vec<String>>,
        fail: Vec<String>,
        started: AtomicUsize,
    }

    impl FakeJob {
        fn new(blocking: &[&str], fail: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                blocking: blocking.iter().map(|s| s.to_string()).collect(),
                fail: fail.iter().map(|s| s.to_string()).collect(),
                runs: Mutex::new(Vec::new()),
                started: AtomicUsize::new(0),
            })
        }
    }

    impl Job for FakeJob {
        fn run(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
            let app = el.params.app_name.clone();
            self.runs.lock().push(app.clone());
            self.started.fetch_add(1, Ordering::SeqCst);
            on_progress(Progress { percent: 10., ..Default::default() });
            if self.blocking.contains(&app) {
                while !cancel.is_requested() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                return JobResult { status: DmStatus::Abort, error: None };
            }
            if self.fail.contains(&app) {
                return JobResult { status: DmStatus::Error, error: Some("boom".into()) };
            }
            JobResult { status: DmStatus::Done, error: None }
        }
    }

    fn el(app: &str) -> DmElement {
        DmElement::new(
            ElementKind::Install,
            InstallParams {
                app_name: app.into(),
                path: "/tmp/never-used".into(),
                runner: "legendary".into(),
                platform_to_install: "Windows".into(),
                game_info: json!({"title": app, "folder_name": app}),
                ..Default::default()
            },
        )
    }

    fn manager(job: Arc<FakeJob>) -> (DownloadManager, mpsc::Receiver<Event>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let m = DownloadManager::new(DataPaths::at(dir.path()), job, move |e| {
            let _ = tx.lock().send(e);
        });
        (m, rx, dir)
    }

    fn wait_until(m: &DownloadManager, what: &str, cond: impl Fn(&Snapshot) -> bool) {
        for _ in 0..400 {
            if cond(&m.snapshot()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timeout waiting for {what}: {:?}", m.snapshot().state);
    }

    #[test]
    fn elements_run_in_order_and_move_to_finished() {
        let job = FakeJob::new(&[], &["b"]);
        let (m, rx, dir) = manager(job.clone());
        m.add(el("a"));
        m.add(el("b"));
        m.add(el("c"));
        wait_until(&m, "all done", |s| s.finished.len() == 3 && s.state == DmState::Idle);
        let s = m.snapshot();
        assert!(s.queue.is_empty());
        assert_eq!(*job.runs.lock(), ["a", "b", "c"]);
        let st: Vec<_> = s.finished.iter().map(|e| (e.params.app_name.as_str(), e.status)).collect();
        assert_eq!(st, [("a", Some(DmStatus::Done)), ("b", Some(DmStatus::Error)), ("c", Some(DmStatus::Done))]);
        assert!(s.finished.iter().all(|e| e.end_time >= e.start_time && e.start_time > 0));
        let events: Vec<Event> = rx.try_iter().collect();
        assert!(events.iter().any(|e| matches!(e, Event::Progress { app_name, .. } if app_name == "a")));
        assert!(events.iter().any(|e| matches!(e, Event::Finished { error: Some(m), .. } if m == "boom")));
        // Persisted in the queue file.
        let file = queue::load(&DataPaths::at(dir.path()));
        assert_eq!(file.finished.len(), 3);
        assert!(file.queue.is_empty());
    }

    #[test]
    fn pause_keeps_the_element_and_resume_runs_it_again() {
        let job = FakeJob::new(&["a"], &[]);
        let (m, _rx, _dir) = manager(job.clone());
        m.add(el("a"));
        m.add(el("b"));
        wait_until(&m, "a running", |s| s.state == DmState::Running && job.started.load(Ordering::SeqCst) == 1);
        m.pause();
        wait_until(&m, "paused", |s| s.state == DmState::Paused);
        std::thread::sleep(Duration::from_millis(100));
        let s = m.snapshot();
        assert_eq!(s.queue.len(), 2, "the paused element stays queued");
        assert!(s.finished.is_empty());
        assert_eq!(s.queue[0].params.app_name, "a");
        // Resume: a blocks again (second start), then cancel it.
        m.resume();
        wait_until(&m, "a restarted", |s| s.state == DmState::Running && job.started.load(Ordering::SeqCst) == 2);
        m.cancel_current(false);
        wait_until(&m, "b done", |s| s.finished.iter().any(|e| e.params.app_name == "b") && s.state == DmState::Idle);
        let s = m.snapshot();
        assert_eq!(s.finished.iter().find(|e| e.params.app_name == "a").unwrap().status, Some(DmStatus::Abort));
        assert_eq!(s.finished.iter().find(|e| e.params.app_name == "b").unwrap().status, Some(DmStatus::Done));
        assert!(s.queue.is_empty());
    }

    #[test]
    fn resume_while_still_stopping_does_not_lose_the_element() {
        let job = FakeJob::new(&["a"], &[]);
        let (m, _rx, _dir) = manager(job.clone());
        m.add(el("a"));
        wait_until(&m, "a running", |_| job.started.load(Ordering::SeqCst) == 1);
        m.pause();
        m.resume();
        wait_until(&m, "a restarted", |_| job.started.load(Ordering::SeqCst) == 2);
        assert!(m.snapshot().finished.is_empty());
        m.cancel_current(false);
        wait_until(&m, "idle", |s| s.state == DmState::Idle && s.queue.is_empty());
    }

    #[test]
    fn cancel_removes_the_element_and_its_dlcs() {
        let job = FakeJob::new(&["a"], &[]);
        let (m, _rx, _dir) = manager(job.clone());
        let mut base = el("a");
        base.params.install_dlcs = vec!["dlc1".into()];
        m.add(base);
        m.add(el("dlc1"));
        m.add(el("z"));
        wait_until(&m, "a running", |_| job.started.load(Ordering::SeqCst) == 1);
        m.cancel_current(false);
        wait_until(&m, "z done", |s| s.finished.iter().any(|e| e.params.app_name == "z"));
        let s = m.snapshot();
        assert!(!s.finished.iter().any(|e| e.params.app_name == "dlc1"));
        assert!(!job.runs.lock().contains(&"dlc1".to_string()));
    }

    #[test]
    fn queued_elements_can_be_removed_reordered_and_replaced() {
        let job = FakeJob::new(&["a"], &[]);
        let (m, _rx, _dir) = manager(job.clone());
        m.add(el("a"));
        wait_until(&m, "a running", |_| job.started.load(Ordering::SeqCst) == 1);
        m.add(el("b"));
        m.add(el("c"));
        m.add(el("d"));
        m.move_element("d", true);
        let order = |m: &DownloadManager| -> Vec<String> { m.snapshot().queue.iter().map(|e| e.params.app_name.clone()).collect() };
        assert_eq!(order(&m), ["a", "b", "d", "c"]);
        m.move_element("b", true); // would swap with the running one: refused
        assert_eq!(order(&m), ["a", "b", "d", "c"]);
        m.remove("a"); // running: ignored
        m.remove("b");
        assert_eq!(order(&m), ["a", "d", "c"]);
        let mut newer = el("c");
        newer.params.path = "/other".into();
        m.add(newer);
        assert_eq!(m.snapshot().queue[2].params.path, "/other");
        m.pause();
        m.cancel_current(false);
        wait_until(&m, "stopped", |s| s.state != DmState::Running);
        m.clear_finished_item("a");
        m.clear_finished();
        assert!(m.snapshot().finished.is_empty());
    }

    #[test]
    fn a_persisted_queue_is_loaded_but_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        queue::save(&data_paths, &[el("a")], &[]).unwrap();
        let job = FakeJob::new(&[], &[]);
        let m = DownloadManager::new(data_paths, job.clone(), |_| {});
        assert_eq!(m.snapshot().queue.len(), 1);
        assert_eq!(m.snapshot().state, DmState::Idle);
        assert_eq!(job.started.load(Ordering::SeqCst), 0);
        m.start();
        wait_until(&m, "done", |s| s.finished.len() == 1);
    }

    #[test]
    fn pausing_for_shutdown_keeps_the_running_element_queued_on_disk() {
        let job = FakeJob::new(&["a"], &[]);
        let (m, _rx, dir) = manager(job.clone());
        m.add(el("a"));
        m.add(el("b"));
        wait_until(&m, "a running", |s| s.state == DmState::Running && job.started.load(Ordering::SeqCst) == 1);
        assert!(!m.is_idle());
        m.pause_for_shutdown();
        for _ in 0..400 {
            if m.is_idle() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(m.is_idle(), "the worker went away");
        // What the next start loads: nothing finished, both elements still queued, a first.
        let file = queue::load(&DataPaths::at(dir.path()));
        assert!(file.finished.is_empty());
        let names: Vec<_> = file.queue.iter().map(|e| e.params.app_name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        // And it resumes: a new manager on the same folder runs them.
        let job2 = FakeJob::new(&[], &[]);
        let m2 = DownloadManager::new(DataPaths::at(dir.path()), job2.clone(), |_| {});
        m2.start();
        wait_until(&m2, "both done", |s| s.finished.len() == 2);
        assert_eq!(*job2.runs.lock(), ["a", "b"]);
    }

    #[test]
    fn an_idle_manager_is_idle_and_pausing_it_for_shutdown_is_harmless() {
        let job = FakeJob::new(&[], &[]);
        let (m, _rx, _dir) = manager(job);
        assert!(m.is_idle());
        m.pause_for_shutdown();
        assert!(m.is_idle());
    }
}
