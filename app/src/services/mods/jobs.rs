//! Downloads and installs run on threads of their own and report back with
//! messages; the service applies them on the UI thread. Downloads of one
//! game may overlap, installs take turns (the game's `install` lock).

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, MutexGuard};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::{CollectionGame, DataPaths};
use heronite_i18n::{tr, trn};
use heronite_mods::nexus::StoredAccount;
use heronite_mods::{ModsRoot, Progress};

use super::edit::GameShared;
use super::{FomodRequest, Job, JobStage, ModsEvent, ModsService};

/// At most this many files download at once, over all games.
const MAX_DOWNLOADS: usize = 3;
/// A finished job stays in the list this long.
const KEEP_DONE: Duration = Duration::from_secs(8);
/// Progress reaches the UI at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What the user answered a FOMOD installer.
pub(super) enum Answer {
    Finish,
    Plain,
    Cancel,
}

pub(super) enum Change {
    Name(String),
    Stage(JobStage),
    Progress(Progress),
    Dependencies(usize),
}

pub(super) enum Outcome {
    Installed {
        name: String,
        updated: bool,
        dependencies: usize,
        /// Its files do not look like they belong in the game's folders.
        misplaced: bool,
    },
    /// Done, with a line for a toast.
    Finished(String),
    Cancelled,
    Failed(String),
}

pub(super) enum Msg {
    Update {
        job: u64,
        change: Change,
    },
    Fomod {
        request: FomodRequest,
        answer: mpsc::Sender<Answer>,
    },
    /// A worker changed the game's list.
    ListChanged {
        game_key: String,
    },
    /// An installed mod was checked against the game's folders.
    Checked {
        game_key: String,
        mod_id: String,
        misplaced: bool,
    },
    Finished {
        job: u64,
        outcome: Outcome,
    },
    Notice {
        game_key: String,
        message: String,
        error: bool,
    },
}

/// The channel the threads report on, and what the service keeps per job.
pub(super) struct Work {
    pub tx: UnboundedSender<Msg>,
    cancels: HashMap<u64, Arc<AtomicBool>>,
    answers: HashMap<u64, mpsc::Sender<Answer>>,
    /// FOMOD installers waiting behind the one on screen.
    fomods: VecDeque<(FomodRequest, mpsc::Sender<Answer>)>,
}

impl Work {
    pub fn cancels_insert(&mut self, job: u64, flag: Arc<AtomicBool>) {
        self.cancels.insert(job, flag);
    }

    pub fn new() -> (Self, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded();
        (Self { tx, cancels: HashMap::new(), answers: HashMap::new(), fomods: VecDeque::new() }, rx)
    }
}

/// Everything a job thread needs; cheap to clone.
#[derive(Clone)]
pub(super) struct JobEnv {
    pub id: u64,
    pub key: String,
    pub tx: UnboundedSender<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub shared: Arc<GameShared>,
    pub root: ModsRoot,
    pub paths: DataPaths,
    pub game: Arc<CollectionGame>,
    pub account: Option<StoredAccount>,
}

impl JobEnv {
    pub fn send(&self, msg: Msg) {
        // The service is gone: the app is closing.
        let _ = self.tx.unbounded_send(msg);
    }

    pub fn stage(&self, stage: JobStage) {
        self.send(Msg::Update { job: self.id, change: Change::Stage(stage) });
    }

    pub fn name(&self, name: &str) {
        self.send(Msg::Update { job: self.id, change: Change::Name(name.to_string()) });
    }

    pub fn dependencies(&self, n: usize) {
        self.send(Msg::Update { job: self.id, change: Change::Dependencies(n) });
    }

    pub fn notice(&self, message: String, error: bool) {
        self.send(Msg::Notice { game_key: self.key.clone(), message, error });
    }

    /// A mod cannot be laid out without the game's folder to match against.
    pub fn require_game_dir(&self) -> Result<(), String> {
        match super::facts::resolve(&self.paths, &self.game).dir {
            Some(_) => Ok(()),
            None => Err(super::install::no_game_folder(&self.game.title)),
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// A progress callback that sends at most ten updates a second (and the last one).
    pub fn progress(&self) -> impl FnMut(Progress) {
        throttled(self.tx.clone(), self.id)
    }

    /// Waits for a place among the downloads.
    pub fn download_slot(&self) -> Option<Slot> {
        loop {
            let n = DOWNLOADS.load(Ordering::SeqCst);
            if n < MAX_DOWNLOADS && DOWNLOADS.compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                return Some(Slot);
            }
            if self.cancelled() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// Waits for the turn to install into this game (`Waiting` meanwhile).
    pub fn install_turn(&self) -> Option<MutexGuard<'_, ()>> {
        let mut shown = false;
        loop {
            if let Some(guard) = self.shared.try_install() {
                return Some(guard);
            }
            if self.cancelled() {
                return None;
            }
            if !shown {
                shown = true;
                self.stage(JobStage::Waiting);
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// Waits for the user's answer to a FOMOD installer.
    pub fn ask_fomod(&self, request: FomodRequest) -> Option<Answer> {
        let (tx, rx) = mpsc::channel();
        self.send(Msg::Fomod { request, answer: tx });
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(answer) => return Some(answer),
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
                Err(mpsc::RecvTimeoutError::Timeout) if self.cancelled() => return None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn throttled(tx: UnboundedSender<Msg>, job: u64) -> impl FnMut(Progress) {
    let mut last: Option<Instant> = None;
    move |p: Progress| {
        let end = p.total > 0 && p.done >= p.total;
        if !end && last.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) {
            return;
        }
        last = Some(Instant::now());
        let _ = tx.unbounded_send(Msg::Update { job, change: Change::Progress(p) });
    }
}

static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);

/// A place among the downloads, given back when dropped.
pub(super) struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        DOWNLOADS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ModsService {
    /// Applies what the job threads report. Runs for as long as the service lives.
    pub(super) fn listen(&mut self, mut messages: UnboundedReceiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = messages.next().await {
                if this.update(cx, |s, cx| s.on_message(msg, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_message(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Update { job, change } => {
                let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) else { return };
                if matches!(j.stage, JobStage::Done | JobStage::Failed | JobStage::Cancelled) {
                    return;
                }
                match change {
                    Change::Name(name) => j.name = name,
                    Change::Stage(stage) => {
                        j.stage = stage;
                        if stage != JobStage::Downloading && stage != JobStage::Tool {
                            j.progress = Progress::default();
                        }
                    }
                    Change::Progress(p) => j.progress = p,
                    Change::Dependencies(n) => j.dependencies = n,
                }
                cx.emit(ModsEvent::Jobs);
            }
            Msg::Fomod { request, answer } => {
                let job_id = request.job_id;
                self.work.answers.insert(job_id, answer.clone());
                if self.fomod.is_none() {
                    self.fomod = Some(request);
                    cx.emit(ModsEvent::Fomod);
                } else {
                    self.work.fomods.push_back((request, answer));
                }
            }
            Msg::ListChanged { game_key } => self.refresh_from_shared(&game_key, cx),
            Msg::Checked { game_key, mod_id, misplaced } => {
                let flagged = self.flagged.entry(game_key).or_default();
                if misplaced {
                    flagged.insert(mod_id);
                } else {
                    flagged.remove(&mod_id);
                }
            }
            Msg::Notice { game_key, message, error } => cx.emit(ModsEvent::Notice { game_key, message, error }),
            Msg::Finished { job, outcome } => self.finish_job(job, outcome, cx),
        }
    }

    fn finish_job(&mut self, id: u64, outcome: Outcome, cx: &mut Context<Self>) {
        self.work.cancels.remove(&id);
        self.work.answers.remove(&id);
        self.drop_fomod_of(id, cx);
        let Some(at) = self.jobs.iter().position(|j| j.id == id) else { return };
        let game_key = self.jobs[at].game_key.clone();
        if self.jobs[at].stage == JobStage::Tool {
            // fuse-overlayfs is there now (or still not).
            self.refresh_support(cx);
        }
        let title = self.game_title(&game_key);
        let job = &mut self.jobs[at];
        let notice = match outcome {
            Outcome::Installed { name, updated, dependencies, misplaced } => {
                job.stage = JobStage::Done;
                job.name = name.clone();
                job.progress = Progress::default();
                job.dependencies = dependencies;
                self.prune_later(id, cx);
                // The warning takes the place of the success line: a toast shows one at a time.
                match misplaced {
                    true => Some((misplaced_line(&name), true)),
                    false => Some((installed_line(&name, &title, updated, dependencies), false)),
                }
            }
            Outcome::Finished(line) => {
                job.stage = JobStage::Done;
                self.prune_later(id, cx);
                Some((line, false))
            }
            Outcome::Cancelled => {
                job.stage = JobStage::Cancelled;
                None
            }
            Outcome::Failed(error) => {
                job.stage = JobStage::Failed;
                job.error = Some(error.clone());
                Some((tr!("Could not install {name}: {error}", name = job.name.clone(), error = error), true))
            }
        };
        cx.emit(ModsEvent::Jobs);
        if let Some((message, error)) = notice {
            cx.emit(ModsEvent::Notice { game_key, message, error });
        }
    }

    fn prune_later(&mut self, id: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEEP_DONE).await;
            this.update(cx, |s, cx| {
                let before = s.jobs.len();
                s.jobs.retain(|j| !(j.id == id && j.stage == JobStage::Done));
                if s.jobs.len() != before {
                    cx.emit(ModsEvent::Jobs);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The game's title for a sentence: the loaded list's, else the Collection's.
    pub(super) fn game_title(&self, game_key: &str) -> String {
        match (self.games.get(game_key), self.sources.get(game_key)) {
            (Some(g), _) => g.info.title.clone(),
            (None, Some(s)) => s.title.clone(),
            (None, None) => game_key.to_string(),
        }
    }

    pub(super) fn next_job_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Starts `run` on a thread of its own as a new job of the game. The
    /// game must be known (loaded, or found for a link).
    pub(super) fn start_job(
        &mut self,
        game_key: &str,
        name: String,
        cx: &mut Context<Self>,
        run: impl FnOnce(&JobEnv) -> Result<Outcome, String> + Send + 'static,
    ) -> Option<u64> {
        let Some(game) = self.sources.get(game_key).cloned() else {
            log::warn!("mods: a job for {game_key}, which is not known");
            cx.emit(ModsEvent::Notice {
                game_key: game_key.to_string(),
                message: tr!("This game is not in the Collection, so no mod can be added").to_string(),
                error: true,
            });
            return None;
        };
        let id = self.next_job_id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.work.cancels.insert(id, cancel.clone());
        let env = JobEnv {
            id,
            key: game_key.to_string(),
            tx: self.work.tx.clone(),
            cancel,
            shared: self.shared_for(game_key),
            root: self.root.clone(),
            paths: self.data_paths.clone(),
            game,
            account: self.account.clone(),
        };
        self.jobs.push(Job {
            id,
            game_key: game_key.to_string(),
            name,
            stage: JobStage::Waiting,
            progress: Progress::default(),
            error: None,
            dependencies: 0,
        });
        let thread_env = env.clone();
        let spawned = std::thread::Builder::new().name("mods-job".into()).spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                thread_env.require_game_dir()?;
                run(&thread_env)
            }));
            let outcome = match result {
                _ if thread_env.cancelled() => Outcome::Cancelled,
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => Outcome::Failed(error),
                Err(_) => Outcome::Failed(tr!("Something went wrong inside Heronite; the log has the details").to_string()),
            };
            thread_env.send(Msg::Finished { job: id, outcome });
        });
        if let Err(e) = spawned {
            env.send(Msg::Finished { job: id, outcome: Outcome::Failed(e.to_string()) });
        }
        cx.emit(ModsEvent::Jobs);
        Some(id)
    }

    pub(super) fn cancel(&mut self, job_id: u64, cx: &mut Context<Self>) {
        if let Some(flag) = self.work.cancels.get(&job_id) {
            flag.store(true, Ordering::Relaxed);
        }
        // Dropping the sender wakes a thread waiting for the installer's answer.
        self.work.answers.remove(&job_id);
        self.drop_fomod_of(job_id, cx);
    }

    pub(super) fn dismiss(&mut self, job_id: u64, cx: &mut Context<Self>) {
        let before = self.jobs.len();
        self.jobs.retain(|j| !(j.id == job_id && matches!(j.stage, JobStage::Done | JobStage::Failed | JobStage::Cancelled)));
        if self.jobs.len() != before {
            cx.emit(ModsEvent::Jobs);
        }
    }

    pub(super) fn answer_fomod(&mut self, job_id: u64, answer: Answer, cx: &mut Context<Self>) {
        if matches!(answer, Answer::Cancel) {
            // The user closed the installer: the job ends.
            if let Some(flag) = self.work.cancels.get(&job_id) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        if let Some(tx) = self.work.answers.remove(&job_id) {
            let _ = tx.send(answer);
        }
        self.drop_fomod_of(job_id, cx);
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id && j.stage == JobStage::NeedsChoices) {
            job.stage = JobStage::Installing;
            cx.emit(ModsEvent::Jobs);
        }
    }

    /// Takes the job's installer off the screen (or out of the line) and
    /// shows the next one.
    fn drop_fomod_of(&mut self, job_id: u64, cx: &mut Context<Self>) {
        self.work.fomods.retain(|(r, _)| r.job_id != job_id);
        if self.fomod.as_ref().is_some_and(|f| f.job_id == job_id) {
            self.fomod = self.work.fomods.pop_front().map(|(r, _)| r);
            cx.emit(ModsEvent::Fomod);
        }
    }
}

/// The mod went in, but nothing in it looks like it belongs in the game's folders.
fn misplaced_line(name: &str) -> String {
    tr!(
        "{name} was installed, but its files don't match the game's folders. Check the mod's instructions; you can open its folder from the list.",
        name = name
    )
}

/// "SkyUI installed for Skyrim Special Edition", with the dependencies that came along.
fn installed_line(name: &str, game: &str, updated: bool, dependencies: usize) -> String {
    match (updated, dependencies) {
        (true, _) => tr!("{name} updated for {game}", name = name, game = game),
        (false, 0) => tr!("{name} installed for {game}", name = name, game = game),
        (false, n) => trn!(
            n,
            "{name} and {n} dependency installed for {game}",
            "{name} and {n} dependencies installed for {game}",
            name = name,
            game = game
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{installed_line, throttled};
    use futures::channel::mpsc::unbounded;
    use heronite_mods::Progress;

    #[test]
    fn the_notice_says_what_was_installed() {
        assert_eq!(installed_line("SkyUI", "Skyrim", false, 0), "SkyUI installed for Skyrim");
        assert_eq!(installed_line("SkyUI", "Skyrim", true, 0), "SkyUI updated for Skyrim");
        assert_eq!(installed_line("MoreCompany", "Lethal Company", false, 1), "MoreCompany and 1 dependency installed for Lethal Company");
        assert_eq!(
            installed_line("MoreCompany", "Lethal Company", false, 3),
            "MoreCompany and 3 dependencies installed for Lethal Company"
        );
    }

    #[test]
    fn the_progress_callback_skips_the_crowd_but_not_the_end() {
        let (tx, mut rx) = unbounded();
        let mut progress = throttled(tx, 1);
        for done in 0..50 {
            progress(Progress { done, total: 50 });
        }
        progress(Progress { done: 50, total: 50 });
        drop(progress);
        let mut seen = 0;
        while rx.try_recv().is_ok() {
            seen += 1;
        }
        assert!((2..=3).contains(&seen), "{seen}");
    }
}
