//! The replay buffer (Settings → Replay, off by default; a game can have it
//! on or off on its own in Game settings → Performance): while a game
//! launched by the app runs, gpu-screen-recorder keeps its last seconds
//! (`heronite_overlay::replay`), and a save moves them into the game's
//! captures folder, under `replays/`. The in-game overlay asks for the saves
//! (Guide + X, the panel), marks them as moments and can turn the replay off
//! until the game ends.
//!
//! The service follows the launch events (from the hub), so it works with
//! the window closed to the tray. The recorder starts a few seconds after
//! the game is running (it does not compete with the game's loading), ends
//! with the last game (or a game that has it off) and starts again when the
//! options change. One recorder at a time: a new one waits for the last one
//! to end, and a save in progress is never cut short by a restart.
//!
//! The service also owns `replay.json`: Settings and the game settings change
//! it through [`change`], and it is written in order.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures::channel::mpsc::unbounded;
use futures::future::{FutureExt, Shared};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::journal::collection_key;
use heronite_data::replay_prefs::{Audio, Codec, Options, ReplayPrefs};
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_launch::LaunchEvent;
use heronite_media::capture::{replay_folder, unique_capture_path, ActiveGame};
use heronite_media::list::resolve_path;
use heronite_overlay::replay::plan::buffer_seconds;
use heronite_overlay::replay::{self as backend, Encoding, Info, Plan, Program, Recorder, State};

use crate::runtime::GameRuntime;

/// After the game is running: its loading goes first.
const START_AFTER: Duration = Duration::from_secs(8);
/// A change of the options waits this long, so several clicks restart once.
const RESTART_AFTER: Duration = Duration::from_millis(1500);
/// Saves wait for the game's sound this long, then record without it.
const AUDIO_WAIT: Duration = Duration::from_secs(180);
const AUDIO_POLL: Duration = Duration::from_secs(5);
/// Hidden folder of the captures root where the recorder writes a save
/// before it moves into the game's folder (same disk: a rename), and its
/// buffer when kept on disk.
const STAGING: &str = ".heronite-replay";
/// A save left in [`STAGING`] this long was never moved (the app ended
/// meanwhile): it goes.
const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// What the replay is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Turned off in Settings (and for the game running, if any).
    Off,
    /// On, waiting for a game.
    Idle,
    /// The game running has it off (Game settings → Performance).
    OffForGame,
    /// Turned off from the overlay until the game ends.
    OffForSession,
    Starting,
    /// Keeping the last seconds of the screen `monitor`.
    Recording {
        monitor: String,
        codec: Codec,
        /// Through Vulkan video, else the card's usual encoder.
        vulkan: bool,
    },
    /// It cannot record here, and why.
    Unavailable(String),
}

/// What Settings shows about this PC (`detect`).
#[derive(Clone, Debug)]
pub struct Detected {
    pub program: Option<Program>,
    /// What it can do (`Err`: `--info` failed).
    pub info: Result<Info, String>,
}

/// Blocking: whether gpu-screen-recorder is installed and what it can do.
pub fn detect() -> Detected {
    match Program::find() {
        None => Detected { program: None, info: Err(tr!("gpu-screen-recorder is not installed").into()) },
        Some(p) => {
            let info = p.info();
            Detected { program: Some(p), info }
        }
    }
}

/// The replay as the overlay's panel shows it, when it applies to the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelState {
    /// What a save keeps.
    pub seconds: u32,
    /// Not turned off until the game ends.
    pub on: bool,
}

/// A saved replay.
#[derive(Clone, Debug)]
pub struct Saved {
    pub path: PathBuf,
    /// Seconds it holds once the panel's time is cut off.
    pub seconds: u32,
}

/// The running recorder and what it records.
struct Running {
    recorder: Arc<Recorder>,
    /// The captures root.
    root: PathBuf,
    monitor: String,
    codec: Codec,
}

/// A game running.
struct GameRun {
    app_name: String,
    /// `journal::collection_key`, once known: its own switch.
    key: Option<String>,
    pid: Option<u32>,
}

pub struct ReplayService {
    data_paths: DataPaths,
    runtime: Arc<GameRuntime>,
    prefs: ReplayPrefs,
    /// Games running, oldest first: the last one decides.
    games: Vec<GameRun>,
    running: Option<Running>,
    /// What the running (or starting) recorder records with.
    target: Option<Options>,
    status: Status,
    /// Bumped by every start and stop: late answers of an older one are dropped.
    epoch: u64,
    saving: bool,
    /// The recorder must change once the save in progress ends.
    resync: bool,
    /// Turned off from the overlay until the last game ends.
    session_off: bool,
    /// The recorders ending, one after the other: a new one starts after them.
    stopping: Option<Shared<Task<()>>>,
    /// What gpu-screen-recorder can do, kept while games run: a restart does
    /// not ask again (`--info` wakes the graphics card).
    detected: Option<(Program, Info)>,
    /// `replay.json` is being written; `dirty`: changed again meanwhile.
    writing: bool,
    dirty: bool,
}

struct ReplayGlobal(Entity<ReplayService>);
impl Global for ReplayGlobal {}

/// Creates the service (once). Called by the hub with the shared runtime.
pub fn init(cx: &mut App, data_paths: DataPaths, runtime: Arc<GameRuntime>) {
    if cx.has_global::<ReplayGlobal>() {
        return;
    }
    // A tiny file in the app's config folder.
    let prefs = ReplayPrefs::load(&AppPaths::for_data(&data_paths));
    let status = if prefs.any_enabled() { Status::Idle } else { Status::Off };
    let entity = cx.new(|_| ReplayService {
        data_paths,
        runtime,
        prefs,
        games: Vec::new(),
        running: None,
        target: None,
        status,
        epoch: 0,
        saving: false,
        resync: false,
        session_off: false,
        stopping: None,
        detected: None,
        writing: false,
        dirty: false,
    });
    cx.set_global(ReplayGlobal(entity));
}

pub fn get(cx: &App) -> Option<Entity<ReplayService>> {
    cx.try_global::<ReplayGlobal>().map(|g| g.0.clone())
}

/// The options as they are now (read from `app` without the service).
pub fn prefs(cx: &App, app: &AppPaths) -> ReplayPrefs {
    match get(cx) {
        Some(s) => s.read(cx).prefs.clone(),
        None => ReplayPrefs::load(app),
    }
}

/// Changes the options: the service writes them and restarts the recorder
/// when the game running records with other ones. Gives the new options.
pub fn change(cx: &mut App, app: &AppPaths, f: impl FnOnce(&mut ReplayPrefs)) -> ReplayPrefs {
    match get(cx) {
        Some(service) => service.update(cx, |s, cx| {
            s.change_prefs(f, cx);
            s.prefs.clone()
        }),
        None => {
            let mut prefs = ReplayPrefs::load(app);
            f(&mut prefs);
            if let Err(e) = prefs.save(app) {
                log::warn!("saving the replay options: {e:#}");
            }
            prefs
        }
    }
}

/// What the recorder found while a game runs, so Settings does not wake the
/// graphics card again.
pub fn cached_detected(cx: &App) -> Option<Detected> {
    let service = get(cx)?;
    let (program, info) = service.read(cx).detected.clone()?;
    Some(Detected { program: Some(program), info: Ok(info) })
}

/// The app is quitting: the recorder ends (off the UI thread; it saves nothing).
pub fn shutdown(cx: &mut App) {
    let Some(service) = get(cx) else { return };
    let running = service.update(cx, |s, _| {
        s.epoch += 1;
        s.target = None;
        s.running.take()
    });
    if let Some(r) = running {
        std::thread::spawn(move || drop(r));
    }
}

impl ReplayService {
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Applies `f` to the options, writes them and brings the recorder in
    /// line (after [`RESTART_AFTER`], so several changes restart once).
    pub fn change_prefs(&mut self, f: impl FnOnce(&mut ReplayPrefs), cx: &mut Context<Self>) {
        let before = self.prefs.clone();
        f(&mut self.prefs);
        if self.prefs == before {
            return;
        }
        self.persist(cx);
        self.sync(RESTART_AFTER, cx);
        cx.notify();
    }

    /// The replay in the overlay's panel: `None` when it does not apply to
    /// the game running.
    pub fn panel_state(&self) -> Option<PanelState> {
        let key = self.current_key();
        (!self.games.is_empty() && self.prefs.enabled_for(key))
            .then_some(PanelState { seconds: self.prefs.options.seconds, on: !self.session_off })
    }

    /// The overlay turns the replay off until the last game ends, or back on.
    pub fn set_session_on(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.session_off == !on {
            return;
        }
        self.session_off = !on;
        log::info!("replay: turned {} until the game ends", if on { "on" } else { "off" });
        self.sync(Duration::ZERO, cx);
        cx.notify();
    }

    /// Every launch event, from the hub.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Running { app_name, pid } => {
                let key = self.runtime.session_info(app_name).map(|i| collection_key(&i.runner, &i.app_name));
                match self.games.iter_mut().find(|g| &g.app_name == app_name) {
                    Some(g) => {
                        g.pid = Some(*pid);
                        g.key = key.or(g.key.take());
                    }
                    None => self.games.push(GameRun { app_name: app_name.clone(), key, pid: Some(*pid) }),
                }
                // One recorder records the screen, whatever runs on it.
                self.sync(START_AFTER, cx);
            }
            LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => {
                self.games.retain(|g| &g.app_name != app_name);
                if self.games.is_empty() {
                    self.session_off = false;
                    // The monitors may change before the next game.
                    self.detected = None;
                }
                self.sync(RESTART_AFTER, cx);
            }
            LaunchEvent::Launching { .. } | LaunchEvent::Preparing { .. } | LaunchEvent::Status { .. } => {}
        }
    }

    /// The game that decides: the last one started.
    fn current_key(&self) -> Option<&str> {
        self.games.last().and_then(|g| g.key.as_deref())
    }

    /// The options to record with now; `None`: no recorder should run.
    fn wanted(&self) -> Option<Options> {
        decide(&self.prefs, !self.games.is_empty(), self.current_key(), self.session_off).ok()
    }

    /// The status without a recorder.
    fn idle_status(&self) -> Status {
        decide(&self.prefs, !self.games.is_empty(), self.current_key(), self.session_off).err().unwrap_or(Status::Idle)
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }

    /// Brings the recorder in line with [`Self::wanted`]: starts it (after
    /// `delay`), starts it again with other options, or ends it. Waits for a
    /// save in progress.
    fn sync(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let wanted = self.wanted();
        if wanted == self.target {
            if wanted.is_none() {
                let idle = self.idle_status();
                self.set_status(idle, cx);
            }
            return;
        }
        if self.saving {
            self.resync = true;
            return;
        }
        self.stop(cx);
        if let Some(opts) = wanted {
            self.start(opts, delay, cx);
        }
    }

    /// Starts the recorder again with the same options (the game's sound
    /// appeared).
    fn restart(&mut self, cx: &mut Context<Self>) {
        self.target = None;
        self.sync(Duration::ZERO, cx);
    }

    /// Ends the recorder.
    fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.target = None;
        if let Some(r) = self.running.take() {
            self.retire(r.recorder, cx);
        }
        let idle = self.idle_status();
        self.set_status(idle, cx);
    }

    /// Ends a recorder off the UI thread, after the ones already ending.
    fn retire(&mut self, recorder: Arc<Recorder>, cx: &mut Context<Self>) {
        let before = self.stopping.take();
        let ended = cx.background_spawn(async move {
            if let Some(before) = before {
                before.await;
            }
            drop(recorder);
        });
        self.stopping = Some(ended.shared());
    }

    /// Writes `replay.json` off the UI thread, one write at a time, the last
    /// options last.
    fn persist(&mut self, cx: &mut Context<Self>) {
        if self.writing {
            self.dirty = true;
            return;
        }
        self.writing = true;
        let prefs = self.prefs.clone();
        let app = AppPaths::for_data(&self.data_paths);
        cx.spawn(async move |this, cx| {
            if let Err(e) = cx.background_spawn(async move { prefs.save(&app) }).await {
                log::warn!("saving the replay options: {e:#}");
            }
            this.update(cx, |s, cx| {
                s.writing = false;
                if std::mem::take(&mut s.dirty) {
                    s.persist(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Starts the recorder with `opts` after `delay`, once the last one has
    /// ended: finds the program, the monitor and the game's sound off the UI
    /// thread.
    fn start(&mut self, opts: Options, delay: Duration, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.target = Some(opts.clone());
        self.set_status(Status::Starting, cx);
        let data_paths = self.data_paths.clone();
        let game_pid = self.games.last().and_then(|g| g.pid);
        let stopping = self.stopping.clone();
        let cached = self.detected.clone();
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            if this.update(cx, |s, _| s.epoch != epoch).unwrap_or(true) {
                return;
            }
            // Never two recorders at once (twice the memory, one socket).
            if let Some(stopping) = stopping {
                stopping.await;
            }
            if this.update(cx, |s, _| s.epoch != epoch).unwrap_or(true) {
                return;
            }
            let (tx, mut rx) = unbounded();
            let launched = cx
                .background_spawn(async move {
                    launch(&data_paths, &opts, game_pid, cached, move |state| {
                        let _ = tx.unbounded_send(state.clone());
                    })
                })
                .await;
            let ok = this
                .update(cx, |s, cx| {
                    if s.epoch != epoch {
                        // Stopped meanwhile: let it go.
                        if let Ok(l) = launched {
                            s.retire(Arc::new(l.recorder), cx);
                        }
                        return false;
                    }
                    match launched {
                        Ok(l) => {
                            s.detected = Some(l.detected);
                            s.running = Some(Running { recorder: Arc::new(l.recorder), root: l.root, monitor: l.monitor, codec: l.codec });
                            if l.waits_for_game_audio {
                                s.wait_for_game_audio(epoch, cx);
                            }
                            true
                        }
                        Err(why) => {
                            log::warn!("replay: cannot record: {why}");
                            s.target = None;
                            s.detected = None;
                            s.set_status(Status::Unavailable(why), cx);
                            false
                        }
                    }
                })
                .unwrap_or(false);
            if !ok {
                return;
            }
            // The recorder's own news: recording, or it gave up after dying.
            while let Some(state) = rx.next().await {
                let current = this
                    .update(cx, |s, cx| {
                        if s.epoch != epoch {
                            return false;
                        }
                        match state {
                            State::Failed(why) => {
                                // Its thread has ended: the next change or game tries again.
                                if let Some(r) = s.running.take() {
                                    s.retire(r.recorder, cx);
                                }
                                s.target = None;
                                s.detected = None;
                                s.set_status(Status::Unavailable(why), cx);
                            }
                            State::Starting => s.set_status(Status::Starting, cx),
                            State::Recording { vulkan } => {
                                if let Some(r) = &s.running {
                                    let status = Status::Recording { monitor: r.monitor.clone(), codec: r.codec, vulkan };
                                    s.set_status(status, cx);
                                }
                            }
                            State::Stopped => {}
                        }
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    break;
                }
            }
        })
        .detach();
    }

    /// The game played nothing when the recorder started: once it does,
    /// start again with its sound (the few seconds kept so far had none).
    fn wait_for_game_audio(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let Some(pid) = self.games.last().and_then(|g| g.pid) else { return };
        cx.spawn(async move |this, cx| {
            let until = Instant::now() + AUDIO_WAIT;
            while Instant::now() < until {
                cx.background_executor().timer(AUDIO_POLL).await;
                if this.update(cx, |s, _| s.epoch != epoch).unwrap_or(true) {
                    return;
                }
                let names = cx.background_spawn(async move { heronite_overlay::audio::game_stream_names(pid) }).await;
                if names.is_empty() {
                    continue;
                }
                this.update(cx, |s, cx| {
                    if s.epoch == epoch {
                        log::info!("replay: the game plays sound now ({}): recording it", names.join(", "));
                        // After the save in progress, if any.
                        s.restart(cx);
                    }
                })
                .ok();
                return;
            }
        })
        .detach();
    }

    /// Saves the last seconds of the game `app_name` plus `extra` seconds
    /// (the time the panel was open, cut off later) into its `replays`
    /// folder.
    pub fn save(&mut self, app_name: &str, extra: Duration, cx: &mut Context<Self>) -> Task<Result<Saved, String>> {
        let (Some(running), Some(opts)) = (&self.running, &self.target) else {
            let why = match &self.status {
                Status::Off => tr!("Replay is off (Settings → Replay)").to_string(),
                Status::OffForGame => tr!("Replay is off for this game (Game settings → Performance)").to_string(),
                Status::OffForSession => tr!("Replay is off until the game ends: turn it on in the overlay").to_string(),
                Status::Unavailable(why) => why.clone(),
                _ => tr!("Still starting: try again in a moment").to_string(),
            };
            return Task::ready(Err(why));
        };
        if self.saving {
            return Task::ready(Err(tr!("Already saving a replay").to_string()));
        }
        let Some(info) = self.runtime.session_info(app_name) else {
            return Task::ready(Err(tr!("No game to save a replay of").to_string()));
        };
        let game = ActiveGame {
            app_name: info.app_name,
            runner: info.runner,
            title: info.title,
            aliases: info.aliases,
            steam_app_id: info.steam_app_id,
            started_at: SystemTime::now(),
        };
        let recorder = running.recorder.clone();
        let root = running.root.clone();
        // Whole seconds, rounding the panel's time up so it is all cut off.
        let extra = extra.as_secs_f64().ceil() as u32;
        let seconds = opts.seconds + extra;
        // The buffer may hold less than the panel's time asks for.
        let kept = opts.seconds.min(buffer_seconds(opts).saturating_sub(extra));
        self.saving = true;
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move {
                    let staged = recorder.save(seconds)?;
                    move_into(&staged, &replay_folder(&game, &root))
                })
                .await;
            this.update(cx, |s, cx| {
                s.saving = false;
                if std::mem::take(&mut s.resync) {
                    s.sync(Duration::ZERO, cx);
                }
            })
            .ok();
            match &saved {
                Ok(path) => log::info!("replay: saved {}", path.display()),
                Err(e) => log::warn!("replay: saving failed: {e}"),
            }
            saved.map(|path| Saved { path, seconds: kept })
        })
    }
}

/// What the replay does with a game running (`playing`; `key`: the last one
/// started): record with these options, or not, and its status then.
fn decide(prefs: &ReplayPrefs, playing: bool, key: Option<&str>, session_off: bool) -> Result<Options, Status> {
    if !playing {
        return Err(if prefs.any_enabled() { Status::Idle } else { Status::Off });
    }
    if !prefs.enabled_for(key) {
        let off_here = key.and_then(|k| prefs.game(k)) == Some(false);
        return Err(if off_here { Status::OffForGame } else { Status::Off });
    }
    if session_off {
        return Err(Status::OffForSession);
    }
    Ok(prefs.options.clone())
}

/// A recorder started, and what the service keeps of it.
struct Launched {
    recorder: Recorder,
    /// The captures root.
    root: PathBuf,
    monitor: String,
    codec: Codec,
    detected: (Program, Info),
    /// It records without the game's sound, which is not playing yet.
    waits_for_game_audio: bool,
}

/// Blocking: everything a recording needs, then the recorder. `cached` is
/// what an earlier start found out.
fn launch(
    data_paths: &DataPaths,
    opts: &Options,
    game_pid: Option<u32>,
    cached: Option<(Program, Info)>,
    on_state: impl Fn(&State) + Send + Sync + 'static,
) -> Result<Launched, String> {
    let settings = heronite_data::settings::load(data_paths).map_err(|e| e.to_string())?;
    let folder = settings.screenshots.folder.trim().to_string();
    if folder.is_empty() {
        return Err(tr!("Choose a screenshots folder in Settings → Screenshots: replays are saved there").into());
    }
    let root = resolve_path(Path::new(&folder));
    let (program, info) = match cached {
        Some(found) => found,
        None => {
            let program = Program::find().ok_or_else(|| tr!("gpu-screen-recorder is not installed").to_string())?;
            let info = program.info()?;
            (program, info)
        }
    };
    if !info.recent_enough() {
        return Err(tr!(
            "gpu-screen-recorder {found} is too old: {needed} or newer is needed",
            found = info.version_text(),
            needed = format!("{}.{}", backend::MIN_VERSION.0, backend::MIN_VERSION.1)
        ));
    }
    let codec =
        info.codec_for(opts.codec).ok_or_else(|| tr!("The graphics card has no video encoder gpu-screen-recorder can use").to_string())?;
    let encoding = Encoding { codec, vulkan: info.uses_vulkan(opts.encoder, codec) };
    let screen = pick_screen(&info, opts, game_pid).ok_or_else(|| tr!("No monitor to record was found").to_string())?;
    let names = match (opts.audio, game_pid) {
        (Audio::Game, Some(pid)) if info.app_audio => heronite_overlay::audio::game_stream_names(pid),
        _ => Vec::new(),
    };
    let game_audio = match opts.audio {
        // Without PipeWire one app cannot be recorded alone: everything is.
        Audio::Game if !info.app_audio => Some("default_output".to_string()),
        Audio::Game => backend::plan::app_audio(&names),
        _ => None,
    };
    let waits_for_game_audio = opts.audio == Audio::Game && game_audio.is_none();
    let monitor = screen.name.clone();
    let staging = root.join(STAGING);
    clean_staging(&staging);
    let plan = Plan::new(opts, screen, encoding, game_audio, staging, program.socket_path());
    let recorder = Recorder::start(program.clone(), plan, on_state).map_err(|e| e.to_string())?;
    Ok(Launched { recorder, root, monitor, codec, detected: (program, info), waits_for_game_audio })
}

/// The monitor chosen in the options, else the game's, else the first.
fn pick_screen(info: &Info, opts: &Options, game_pid: Option<u32>) -> Option<backend::Screen> {
    let named = |name: &str| info.screens.iter().find(|s| s.name == name).cloned();
    (!opts.monitor.is_empty())
        .then(|| named(&opts.monitor))
        .flatten()
        .or_else(|| backend::target::game_monitor(game_pid).and_then(|n| named(&n)))
        .or_else(|| info.screens.first().cloned())
}

/// Removes what earlier recorders left in the staging folder: their buffers
/// on disk (`gsr-replay-*`, left when one was killed) and saves never moved
/// (`Replay_*`, older than [`STALE_AFTER`]). Called with no recorder running.
fn clean_staging(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let stale = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_AFTER);
        let removed = if name.starts_with("gsr-replay-") && path.is_dir() {
            std::fs::remove_dir_all(&path).is_ok()
        } else if name.starts_with("Replay_") && stale {
            std::fs::remove_file(&path).is_ok()
        } else {
            false
        };
        if removed {
            log::info!("replay: removed the leftover {}", path.display());
        }
    }
}

/// Moves a saved replay into `folder` under the captures' name (its date
/// and time), copying when the folders are on different disks.
fn move_into(staged: &Path, folder: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let ext = staged.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let target = unique_capture_path(folder, chrono::Local::now().naive_local(), ext);
    if std::fs::rename(staged, &target).is_err() {
        if let Err(e) = std::fs::copy(staged, &target) {
            let _ = std::fs::remove_file(&target);
            return Err(tr!(
                "Saved in {staged} but not moved to {folder}: {err}",
                staged = staged.display(),
                folder = folder.display(),
                err = e
            ));
        }
        let _ = std::fs::remove_file(staged);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has a `test` attribute of its own.
    use heronite_data::replay_prefs::ReplayPrefs;

    use super::{clean_staging, decide, move_into, Status, STAGING};

    #[test]
    fn each_game_decides_whether_it_records() {
        let mut p = ReplayPrefs { enabled: true, ..Default::default() };
        p.set_game("steam_off", Some(false));
        let all = Ok(p.options.clone());

        assert_eq!(decide(&p, false, None, false), Err(Status::Idle), "waiting for a game");
        assert_eq!(decide(&p, true, Some("any"), false), all);
        assert_eq!(decide(&p, true, None, false), all, "a game not in the library");
        assert_eq!(decide(&p, true, Some("steam_off"), false), Err(Status::OffForGame));
        assert_eq!(decide(&p, true, Some("any"), true), Err(Status::OffForSession), "turned off in the overlay");

        p.enabled = false;
        assert_eq!(decide(&p, false, None, false), Err(Status::Off));
        assert_eq!(decide(&p, true, Some("gog_on"), false), Err(Status::Off));
        p.set_game("gog_on", Some(true));
        assert_eq!(decide(&p, false, None, false), Err(Status::Idle), "a game has it on");
        assert_eq!(decide(&p, true, Some("gog_on"), false), all, "with the options of every game");
    }

    #[test]
    fn a_save_moves_under_the_capture_name() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join(STAGING).join("Replay_2026-10-20_21-06-41.mp4");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"x").unwrap();
        let folder = dir.path().join("Game").join("replays");
        let saved = move_into(&staged, &folder).unwrap();
        assert_eq!(saved.parent(), Some(folder.as_path()));
        assert!(saved.file_name().unwrap().to_str().unwrap().ends_with(".mp4"));
        assert!(saved.is_file() && !staged.exists());
    }

    #[test]
    fn leftovers_of_earlier_recorders_go() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(STAGING);
        let buffer = staging.join("gsr-replay-2026-10-21_20-35-24.gsr");
        std::fs::create_dir_all(&buffer).unwrap();
        std::fs::write(buffer.join("Replay_0.gsr"), b"x").unwrap();
        let fresh = staging.join("Replay_2026-10-21_20-40-00.mp4");
        let old = staging.join("Replay_2026-10-20_10-00-00.mp4");
        let other = staging.join("notes.txt");
        for f in [&fresh, &old, &other] {
            std::fs::write(f, b"x").unwrap();
        }
        let day_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(day_ago).unwrap();
        clean_staging(&staging);
        assert!(!buffer.exists() && !old.exists());
        assert!(fresh.exists(), "a save may still be moving");
        assert!(other.exists(), "only what the recorder writes");
    }
}
