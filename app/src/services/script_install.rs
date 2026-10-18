//! The install of a game from an install script (`heronite_installer`),
//! owned here so it keeps going when its dialog closes. One at a time: the
//! worker thread runs the script, the dialog shows its stages, log and
//! questions, and the Collection hears when it ends ([`ScriptInstallEvent`]).
//!
//! The worker talks back through a channel drained by a pump on the UI
//! thread; a question blocks the worker until [`ScriptInstall::answer`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_installer::engine::{self, Failure};
use heronite_installer::{Answer, Event, Host, Job, Outcome, Question};

/// Lines of program output kept for the log view.
const LOG_LINES: usize = 400;

#[derive(Clone, Debug)]
pub enum ScriptInstallEvent {
    /// The run ended: the game was added (`error: None`), or not.
    Finished { title: String, error: Option<String>, cancelled: bool },
}

pub enum Status {
    Running,
    Done(Outcome),
    Failed(Failure),
}

pub struct Run {
    pub job: Arc<Job>,
    pub stages: Vec<String>,
    /// Index into `stages` of the current one.
    pub stage: usize,
    /// Progress inside the stage: `(fraction, text)`.
    pub progress: Option<(Option<f32>, String)>,
    pub log: VecDeque<String>,
    /// Lines received so far (the log keeps the last [`LOG_LINES`]).
    pub log_count: usize,
    pub question: Option<Question>,
    pub status: Status,
    pub stopping: bool,
    /// The game's folder is being removed after a failure.
    pub removing: bool,
    control: Arc<Control>,
}

impl Run {
    pub fn running(&self) -> bool {
        matches!(self.status, Status::Running)
    }
}

struct Control {
    cancel: AtomicBool,
    answers: Mutex<Option<Sender<Option<Answer>>>>,
}

enum Msg {
    Event(Event),
    Ask(Question),
    Done(Box<Result<Outcome, Failure>>),
}

pub struct ScriptInstall {
    run: Option<Run>,
}

impl EventEmitter<ScriptInstallEvent> for ScriptInstall {}

struct ScriptInstallGlobal(Entity<ScriptInstall>);
impl Global for ScriptInstallGlobal {}

/// The service (created on first use).
pub fn get(cx: &mut App) -> Entity<ScriptInstall> {
    if let Some(g) = cx.try_global::<ScriptInstallGlobal>() {
        return g.0.clone();
    }
    let entity = cx.new(|_| ScriptInstall { run: None });
    cx.set_global(ScriptInstallGlobal(entity.clone()));
    entity
}

struct WorkerHost {
    tx: UnboundedSender<Msg>,
    control: Arc<Control>,
    answers: Mutex<Receiver<Option<Answer>>>,
}

impl Host for WorkerHost {
    fn event(&self, event: Event) {
        let _ = self.tx.unbounded_send(Msg::Event(event));
    }

    fn cancelled(&self) -> bool {
        self.control.cancel.load(Ordering::SeqCst)
    }

    fn ask(&self, question: Question) -> Option<Answer> {
        let _ = self.tx.unbounded_send(Msg::Ask(question));
        let rx = self.answers.lock().ok()?;
        loop {
            if self.cancelled() {
                return None;
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(answer) => return answer,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl ScriptInstall {
    pub fn run(&self) -> Option<&Run> {
        self.run.as_ref()
    }

    /// Starts `job`. Refused while another install runs.
    pub fn start(&mut self, job: Job, cx: &mut Context<Self>) -> Result<(), String> {
        if self.run.as_ref().is_some_and(Run::running) {
            return Err(tr!("Another game is being installed").into());
        }
        let job = Arc::new(job);
        let (answer_tx, answer_rx) = channel();
        let control = Arc::new(Control { cancel: AtomicBool::new(false), answers: Mutex::new(Some(answer_tx)) });
        let (tx, mut rx) = unbounded::<Msg>();
        self.run = Some(Run {
            job: job.clone(),
            stages: heronite_installer::stages(&job.script),
            stage: 0,
            progress: None,
            log: VecDeque::new(),
            log_count: 0,
            question: None,
            status: Status::Running,
            stopping: false,
            removing: false,
            control: control.clone(),
        });
        let metadata = crate::dialogs::metadata_wizard::hub(cx).read(cx).service.clone();
        let host = WorkerHost { tx: tx.clone(), control, answers: Mutex::new(answer_rx) };
        std::thread::Builder::new()
            .name("script-install".into())
            .spawn(move || {
                // The cover is looked up while the script runs.
                let art = {
                    let (paths, app_name, title) = (job.paths.clone(), job.app_name.clone(), job.title.clone());
                    std::thread::spawn(move || find_art(&metadata, &paths, &app_name, &title))
                };
                let result = engine::install(&job, &host);
                if let Ok(out) = &result {
                    if let Ok(Some((cover, square))) = art.join() {
                        if let Err(e) = heronite_data::sideload::fill_art(&job.paths, &out.app_name, &cover, &square) {
                            log::warn!("art of {}: {e:#}", out.title);
                        }
                    }
                    let shortcuts = crate::system::shortcuts::on_game_added(&job.paths, "sideload", &out.app_name);
                    for m in shortcuts.messages {
                        log::info!("shortcuts for {}: {}", out.title, m.text);
                    }
                }
                let _ = tx.unbounded_send(Msg::Done(Box::new(result)));
            })
            .map_err(|e| e.to_string())?;
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Program output comes in bursts: one redraw per batch.
                let mut batch = vec![first];
                while let Ok(m) = rx.try_recv() {
                    batch.push(m);
                    if batch.len() >= 500 {
                        break;
                    }
                }
                let alive = this.update(cx, |this, cx| {
                    for m in batch {
                        this.apply(m, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
        Ok(())
    }

    fn apply(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else { return };
        match msg {
            Msg::Event(Event::Stage(i)) => {
                run.stage = i;
                run.progress = None;
            }
            Msg::Event(Event::Progress { fraction, text }) => run.progress = Some((fraction, text)),
            Msg::Event(Event::Log(line)) => {
                if run.log.len() >= LOG_LINES {
                    run.log.pop_front();
                }
                run.log.push_back(line);
                run.log_count += 1;
            }
            Msg::Ask(q) => run.question = Some(q),
            Msg::Done(result) => {
                run.question = None;
                run.progress = None;
                let title = run.job.title.clone();
                let (error, cancelled) = match &*result {
                    Ok(_) => (None, false),
                    Err(f) => (Some(f.message.clone()), f.cancelled),
                };
                run.status = match *result {
                    Ok(o) => Status::Done(o),
                    Err(f) => Status::Failed(f),
                };
                cx.emit(ScriptInstallEvent::Finished { title, error, cancelled });
            }
        }
    }

    /// Answers the question on screen (`None` stops the install).
    pub fn answer(&mut self, answer: Option<Answer>, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else { return };
        run.question = None;
        if answer.is_none() {
            run.stopping = true;
            run.control.cancel.store(true, Ordering::SeqCst);
        }
        if let Some(tx) = run.control.answers.lock().ok().and_then(|g| g.clone()) {
            let _ = tx.send(answer);
        }
        cx.notify();
    }

    /// Stops the running install (the program running now is ended).
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.run.as_mut().filter(|r| r.running()) {
            run.stopping = true;
            run.control.cancel.store(true, Ordering::SeqCst);
            cx.notify();
        }
    }

    /// Runs a failed install again from the step that failed, with the
    /// answers given before.
    pub fn retry(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(run) = self.run.as_ref() else { return Ok(()) };
        let Status::Failed(f) = &run.status else { return Ok(()) };
        let Some(from) = f.step else { return Ok(()) };
        let mut job = (*run.job).clone();
        job.resume = Some(heronite_installer::Resume { from, answers: f.answers.clone(), created_dir: f.created_dir });
        self.run = None;
        self.start(job, cx)
    }

    /// Forgets a finished run.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.run.as_ref().is_some_and(|r| !r.running()) {
            self.run = None;
            cx.notify();
        }
    }

    /// After a failure: deletes the folder the install created, then
    /// forgets the run.
    pub fn remove_files(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else { return };
        if !matches!(&run.status, Status::Failed(f) if f.created_dir) || run.removing {
            return;
        }
        run.removing = true;
        let job = run.job.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { engine::remove_game_dir(&job) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    log::warn!("removing the install folder: {e}");
                    if let Some(run) = this.run.as_mut() {
                        run.removing = false;
                        run.log.push_back(e);
                    }
                } else {
                    this.run = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Cover and background for a new game by its title: `(art_cover,
/// art_square)`, `None` when nothing was found.
fn find_art(
    service: &heronite_metadata::service::MetadataService,
    paths: &heronite_data::DataPaths,
    app_name: &str,
    title: &str,
) -> Option<(String, String)> {
    let key = heronite_metadata::covers::steamgrid_api_key(paths);
    let meta = heronite_data::models::LocalGameMeta {
        app_name: app_name.to_string(),
        runner: "sideload".into(),
        title: title.to_string(),
        ..Default::default()
    };
    let game =
        heronite_data::models::GameInfo { runner: "sideload".into(), app_name: app_name.into(), title: title.into(), ..Default::default() };
    let found = heronite_metadata::covers::fetch_covers_for_game(&service.http, key.as_deref(), &meta, &game);
    let cover = found.art_cover.unwrap_or_default();
    let square = found.art_square.unwrap_or_default();
    (!cover.is_empty() || !square.is_empty()).then_some((cover, square))
}
