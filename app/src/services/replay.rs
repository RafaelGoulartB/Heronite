//! The replay buffer (Settings → Replay, off by default): while a game
//! launched by the app runs, gpu-screen-recorder keeps its last seconds in
//! memory (`heronite_overlay::replay`), and a save moves them into the
//! game's captures folder, under `replays/`. The in-game overlay asks for
//! the saves (Guide + X, the panel) and marks them as moments.
//!
//! The service follows the launch events (from the hub), so it works with
//! the window closed to the tray. The recorder starts a few seconds after
//! the game is running (it does not compete with the game's loading) and
//! ends with the last game.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::replay_prefs::{Audio, ReplayPrefs};
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_launch::LaunchEvent;
use heronite_media::capture::{replay_folder, unique_capture_path, ActiveGame};
use heronite_media::list::resolve_path;
use heronite_overlay::replay::{self as backend, Info, Plan, Program, Recorder, State};

use crate::runtime::GameRuntime;

/// After the game is running: its loading goes first.
const START_AFTER: Duration = Duration::from_secs(8);
/// Saves wait for the game's sound this long, then record without it.
const AUDIO_WAIT: Duration = Duration::from_secs(180);
const AUDIO_POLL: Duration = Duration::from_secs(5);
/// Hidden folder of the captures root where the recorder writes a save
/// before it moves into the game's folder (same disk: a rename).
const STAGING: &str = ".heronite-replay";

/// What the replay is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Turned off in Settings.
    Off,
    /// On, waiting for a game.
    Idle,
    Starting,
    /// Keeping the last seconds of the screen (its name).
    Recording {
        monitor: String,
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

/// The running recorder and what it records.
struct Running {
    recorder: Arc<Recorder>,
    /// The captures root.
    root: PathBuf,
    monitor: String,
}

pub struct ReplayService {
    data_paths: DataPaths,
    runtime: Arc<GameRuntime>,
    prefs: ReplayPrefs,
    /// Games running (oldest first) and their process, once known.
    games: Vec<(String, Option<u32>)>,
    running: Option<Running>,
    status: Status,
    /// Bumped by every start and stop: late answers of an older one are dropped.
    epoch: u64,
    saving: bool,
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
    let status = if prefs.enabled { Status::Idle } else { Status::Off };
    let entity =
        cx.new(|_| ReplayService { data_paths, runtime, prefs, games: Vec::new(), running: None, status, epoch: 0, saving: false });
    cx.set_global(ReplayGlobal(entity));
}

pub fn get(cx: &App) -> Option<Entity<ReplayService>> {
    cx.try_global::<ReplayGlobal>().map(|g| g.0.clone())
}

/// The app is quitting: the recorder ends (off the UI thread; it saves nothing).
pub fn shutdown(cx: &mut App) {
    let Some(service) = get(cx) else { return };
    let running = service.update(cx, |s, _| {
        s.epoch += 1;
        s.running.take()
    });
    if let Some(r) = running {
        std::thread::spawn(move || drop(r));
    }
}

impl ReplayService {
    pub fn prefs(&self) -> &ReplayPrefs {
        &self.prefs
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Settings changed them (already saved): a running recorder starts again
    /// with them.
    pub fn set_prefs(&mut self, prefs: ReplayPrefs, cx: &mut Context<Self>) {
        if prefs == self.prefs {
            return;
        }
        self.prefs = prefs;
        self.stop(cx);
        if self.prefs.enabled && !self.games.is_empty() {
            self.start(Duration::ZERO, cx);
        }
        cx.notify();
    }

    /// Every launch event, from the hub.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Running { app_name, pid } => {
                match self.games.iter_mut().find(|(a, _)| a == app_name) {
                    Some(g) => g.1 = Some(*pid),
                    None => self.games.push((app_name.clone(), Some(*pid))),
                }
                // One recorder records the screen, whatever runs on it.
                if self.prefs.enabled && self.running.is_none() && self.status != Status::Starting {
                    self.start(START_AFTER, cx);
                }
            }
            LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => {
                self.games.retain(|(a, _)| a != app_name);
                if self.games.is_empty() {
                    self.stop(cx);
                }
            }
            LaunchEvent::Launching { .. } | LaunchEvent::Preparing { .. } | LaunchEvent::Status { .. } => {}
        }
    }

    fn idle_status(&self) -> Status {
        if self.prefs.enabled {
            Status::Idle
        } else {
            Status::Off
        }
    }

    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }

    /// Ends the recorder (off the UI thread).
    fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        if let Some(r) = self.running.take() {
            cx.background_spawn(async move { drop(r) }).detach();
        }
        let idle = self.idle_status();
        self.set_status(idle, cx);
    }

    /// Starts the recorder after `delay`: finds the program, the monitor and
    /// the game's sound off the UI thread.
    fn start(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.set_status(Status::Starting, cx);
        let prefs = self.prefs.clone();
        let data_paths = self.data_paths.clone();
        let game_pid = self.games.last().and_then(|g| g.1);
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            if this.update(cx, |s, _| s.epoch != epoch).unwrap_or(true) {
                return;
            }
            let (tx, mut rx) = unbounded();
            let started = cx
                .background_spawn(async move {
                    launch(&data_paths, &prefs, game_pid, move |state| {
                        let _ = tx.unbounded_send(state.clone());
                    })
                })
                .await;
            let ok = this
                .update(cx, |s, cx| {
                    if s.epoch != epoch {
                        // Stopped meanwhile: let it go (off the UI thread).
                        if let Ok((r, ..)) = started {
                            cx.background_spawn(async move { drop(r) }).detach();
                        }
                        return false;
                    }
                    match started {
                        Ok((recorder, root, monitor, game_audio)) => {
                            s.running = Some(Running { recorder: Arc::new(recorder), root, monitor });
                            if !game_audio && s.prefs.audio == Audio::Game {
                                s.wait_for_game_audio(epoch, cx);
                            }
                            true
                        }
                        Err(why) => {
                            log::warn!("replay: cannot record: {why}");
                            s.set_status(Status::Unavailable(why), cx);
                            false
                        }
                    }
                })
                .unwrap_or(false);
            if !ok {
                return;
            }
            // The recorder's own news: it failed, or it gave up after dying.
            while let Some(state) = rx.next().await {
                let current = this
                    .update(cx, |s, cx| {
                        if s.epoch != epoch {
                            return false;
                        }
                        match state {
                            State::Failed(why) => {
                                // Its thread has ended: the next game tries again.
                                if let Some(r) = s.running.take() {
                                    cx.background_spawn(async move { drop(r) }).detach();
                                }
                                s.set_status(Status::Unavailable(why), cx);
                            }
                            State::Starting => s.set_status(Status::Starting, cx),
                            State::Recording => {
                                if !matches!(s.status, Status::Recording { .. }) {
                                    let monitor = s.running.as_ref().map(|r| r.monitor.clone()).unwrap_or_default();
                                    s.set_status(Status::Recording { monitor }, cx);
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
        let Some(pid) = self.games.last().and_then(|g| g.1) else { return };
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
                    if s.epoch == epoch && !s.saving {
                        log::info!("replay: the game plays sound now ({}): recording it", names.join(", "));
                        s.stop(cx);
                        s.start(Duration::ZERO, cx);
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
    /// folder. Resolves to the saved file.
    pub fn save(&mut self, app_name: &str, extra: Duration, cx: &mut Context<Self>) -> Task<Result<PathBuf, String>> {
        let Some(running) = &self.running else {
            let why = match &self.status {
                Status::Off => tr!("Replay is off (Settings → Replay)").to_string(),
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
        let seconds = self.prefs.seconds + extra.as_secs_f64().ceil() as u32;
        self.saving = true;
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move {
                    let staged = recorder.save(seconds)?;
                    move_into(&staged, &replay_folder(&game, &root))
                })
                .await;
            this.update(cx, |s, _| s.saving = false).ok();
            match &saved {
                Ok(path) => log::info!("replay: saved {}", path.display()),
                Err(e) => log::warn!("replay: saving failed: {e}"),
            }
            saved
        })
    }
}

/// Blocking: everything a recording needs, then the recorder. Returns it,
/// the captures root, the monitor and whether the game's sound is in it.
fn launch(
    data_paths: &DataPaths,
    prefs: &ReplayPrefs,
    game_pid: Option<u32>,
    on_state: impl Fn(&State) + Send + Sync + 'static,
) -> Result<(Recorder, PathBuf, String, bool), String> {
    let settings = heronite_data::settings::load(data_paths).map_err(|e| e.to_string())?;
    let folder = settings.screenshots.folder.trim().to_string();
    if folder.is_empty() {
        return Err(tr!("Choose a screenshots folder in Settings → Screenshots: replays are saved there").into());
    }
    let root = resolve_path(Path::new(&folder));
    let program = Program::find().ok_or_else(|| tr!("gpu-screen-recorder is not installed").to_string())?;
    let info = program.info()?;
    if !info.recent_enough() {
        return Err(tr!(
            "gpu-screen-recorder {found} is too old: {needed} or newer is needed",
            found = info.version_text(),
            needed = format!("{}.{}", backend::MIN_VERSION.0, backend::MIN_VERSION.1)
        ));
    }
    let codec =
        info.codec_for(prefs.codec).ok_or_else(|| tr!("The graphics card has no video encoder gpu-screen-recorder can use").to_string())?;
    let screen = pick_screen(&info, prefs, game_pid).ok_or_else(|| tr!("No monitor to record was found").to_string())?;
    let names = match (prefs.audio, game_pid) {
        (Audio::Game, Some(pid)) if info.app_audio => heronite_overlay::audio::game_stream_names(pid),
        _ => Vec::new(),
    };
    let game_audio = match prefs.audio {
        // Without PipeWire one app cannot be recorded alone: everything is.
        Audio::Game if !info.app_audio => Some("default_output".to_string()),
        Audio::Game => backend::plan::app_audio(&names),
        _ => None,
    };
    let with_game_audio = prefs.audio != Audio::Game || game_audio.is_some();
    let monitor = screen.name.clone();
    let plan = Plan::new(prefs, screen, codec, game_audio, root.join(STAGING), program.socket_path());
    let recorder = Recorder::start(program, plan, on_state).map_err(|e| e.to_string())?;
    Ok((recorder, root, monitor, with_game_audio))
}

/// The monitor chosen in Settings, else the game's, else the first.
fn pick_screen(info: &Info, prefs: &ReplayPrefs, game_pid: Option<u32>) -> Option<backend::Screen> {
    let named = |name: &str| info.screens.iter().find(|s| s.name == name).cloned();
    (!prefs.monitor.is_empty())
        .then(|| named(&prefs.monitor))
        .flatten()
        .or_else(|| backend::target::game_monitor(game_pid).and_then(|n| named(&n)))
        .or_else(|| info.screens.first().cloned())
}

/// Moves a saved replay into `folder` under the captures' name (its date
/// and time), copying when the folders are on different disks.
fn move_into(staged: &Path, folder: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let ext = staged.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let target = unique_capture_path(folder, chrono::Local::now().naive_local(), ext);
    if std::fs::rename(staged, &target).is_err() {
        std::fs::copy(staged, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        let _ = std::fs::remove_file(staged);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude has a `test` attribute of its own.
    use super::{move_into, STAGING};

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
}
