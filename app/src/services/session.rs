//! Post-game work that must run whether or not a window exists (the window
//! can be closed to the tray while a game runs): the save-state key of
//! emulated games, the Ludusavi backup, and the screenshot capture session
//! of the game. The hub feeds it every launch event; the Collection only
//! observes its [`SessionEvent`]s (toasts, saved captures).
//!
//! Playtime and the local session record are written by the launcher thread
//! itself (`heronite-launch`), and cloud saves go through its prepare/after
//! exit steps, so neither needs anything here.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_emulation::ludusavi::{
    backup_after_play, notification, AfterPlay, CommandRunner, LudusaviBackupResult, LudusaviEnv, SystemRunner,
};
use heronite_emulation::{remember_save_state_key, HostEnv};
use heronite_i18n::tr;
use heronite_launch::probe::Probe;
use heronite_launch::{LaunchEvent, SystemProbe};
use heronite_media::capture::{ActiveGame, CaptureConfig, CaptureEvent, CaptureService, ServiceOptions};

use crate::runtime::{GameRuntime, SessionInfo};

/// Post-game jobs in flight. The hub does not quit an idle headless launch
/// while any is running (a Ludusavi backup can take a while).
static PENDING: AtomicUsize = AtomicUsize::new(0);

pub fn busy() -> bool {
    PENDING.load(Ordering::SeqCst) > 0
}

/// What the Collection may want to show.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// Result of the post-game work (backup saved / failed).
    Toast { text: String, error: bool },
    /// The capture service reported something (a saved screenshot...).
    Capture(CaptureEvent),
}

pub struct Session {
    data_paths: DataPaths,
    runtime: Arc<GameRuntime>,
    capture: Option<CaptureService>,
    capture_cfg: Option<CaptureConfig>,
    /// Runner of each game with a capture session (`app_name` -> runner).
    capture_games: HashMap<String, String>,
    /// Games launched and not exited yet (`app_name`).
    running: HashSet<String>,
}

impl EventEmitter<SessionEvent> for Session {}

struct SessionGlobal(Entity<Session>);
impl Global for SessionGlobal {}

/// Creates the service (once). Called by the hub with the shared runtime.
pub fn init(cx: &mut App, data_paths: DataPaths, runtime: Arc<GameRuntime>) -> Entity<Session> {
    if let Some(g) = cx.try_global::<SessionGlobal>() {
        return g.0.clone();
    }
    let entity = cx.new(|_| Session {
        data_paths,
        runtime,
        capture: None,
        capture_cfg: None,
        capture_games: HashMap::new(),
        running: HashSet::new(),
    });
    cx.set_global(SessionGlobal(entity.clone()));
    entity
}

/// The service, once the hub created it.
pub fn get(cx: &App) -> Option<Entity<Session>> {
    cx.try_global::<SessionGlobal>().map(|g| g.0.clone())
}

/// Whether a game this app launched is still running (the save restore
/// waits for it to close).
pub fn is_running(cx: &App, app_name: &str) -> bool {
    get(cx).is_some_and(|s| s.read(cx).running.contains(app_name))
}

/// The capture service, once a game was launched (the settings dialog shows
/// its status).
pub fn capture_of(cx: &App) -> Option<CaptureService> {
    get(cx)?.read(cx).capture.clone()
}

impl Session {
    /// Every launch event, from the hub.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Launching { app_name } => {
                self.running.insert(app_name.clone());
                self.begin_capture(app_name, cx);
            }
            LaunchEvent::Exited { app_name, elapsed, .. } => {
                self.running.remove(app_name);
                self.end_capture(app_name);
                self.game_ended(app_name, *elapsed, cx);
            }
            LaunchEvent::Error { app_name, .. } => {
                self.running.remove(app_name);
                self.end_capture(app_name);
            }
            // A game picked up again after the app restarted starts here.
            LaunchEvent::Running { app_name, .. } if !self.running.contains(app_name) => {
                self.running.insert(app_name.clone());
                self.begin_capture(app_name, cx);
            }
            LaunchEvent::Running { .. } | LaunchEvent::Status { .. } | LaunchEvent::Preparing { .. } => {}
        }
    }

    /// The Collection applies changed settings while it is open.
    pub fn apply_capture_config(&mut self, settings: &heronite_data::models::ScreenshotsSettings) {
        let cfg = CaptureConfig::from_settings(settings);
        if self.capture_cfg.as_ref() == Some(&cfg) {
            return;
        }
        self.capture_cfg = Some(cfg.clone());
        if let Some(svc) = &self.capture {
            svc.set_config(cfg);
        }
    }

    fn capture_service(&mut self, cx: &mut Context<Self>) -> CaptureService {
        if let Some(svc) = &self.capture {
            return svc.clone();
        }
        let (std_tx, std_rx) = std::sync::mpsc::channel();
        let (tx, mut rx) = unbounded();
        let _ = std::thread::Builder::new().name("capture-events".into()).spawn(move || {
            for ev in std_rx {
                if tx.unbounded_send(ev).is_err() {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                if this.update(cx, |_, cx| cx.emit(SessionEvent::Capture(ev))).is_err() {
                    break;
                }
            }
        })
        .detach();
        let svc = CaptureService::new(std_tx, ServiceOptions { portal_app_id: Some("com.heronite.launcher".into()), ..Default::default() });
        if let Some(cfg) = self.capture_cfg.clone() {
            svc.set_config(cfg);
        }
        self.capture = Some(svc.clone());
        svc
    }

    /// Starts the capture session of a launched game. The settings come from
    /// disk (there may be no Collection to have loaded them).
    fn begin_capture(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let Some(info) = self.runtime.session_info(app_name) else { return };
        let started_at = SystemTime::now();
        self.capture_games.insert(info.app_name.clone(), info.runner.clone());
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let settings = cx.background_spawn(async move { heronite_data::settings::load(&data_paths) }).await;
            this.update(cx, |this, cx| {
                // The game may have ended while the settings were read.
                if !this.capture_games.contains_key(&info.app_name) {
                    return;
                }
                match settings {
                    Ok(s) => this.apply_capture_config(&s.screenshots),
                    Err(e) => log::warn!("reading the screenshot settings: {e:#}"),
                }
                let svc = this.capture_service(cx);
                svc.begin_session(ActiveGame {
                    app_name: info.app_name,
                    runner: info.runner,
                    title: info.title,
                    aliases: info.aliases,
                    steam_app_id: info.steam_app_id,
                    started_at,
                });
            })
            .ok();
        })
        .detach();
    }

    fn end_capture(&mut self, app_name: &str) {
        if let (Some(svc), Some(runner)) = (self.capture.clone(), self.capture_games.remove(app_name)) {
            svc.end_session(&runner, app_name);
        } else {
            self.capture_games.remove(app_name);
        }
    }

    /// The save-state key of emulated games and the Ludusavi backup, off the
    /// UI thread. Shows the result as a toast (Collection open) and a
    /// desktop notification (backup saved always; failures when no window).
    fn game_ended(&mut self, app_name: &str, elapsed: Duration, cx: &mut Context<Self>) {
        let Some(info) = self.runtime.take_session(app_name) else { return };
        let paths = self.data_paths.clone();
        PENDING.fetch_add(1, Ordering::SeqCst);
        cx.spawn(async move |this, cx| {
            let notice = cx
                .background_spawn(async move {
                    let runner = SystemRunner::default();
                    after_play(&paths, &info, elapsed, &runner, &SystemProbe)
                })
                .await;
            let had_notice = notice.is_some();
            this.update(cx, |_, cx| {
                if let Some(n) = notice {
                    if n.desktop || (n.error && !crate::system::has_window(cx)) {
                        crate::system::notify::send(n.title, n.text.clone());
                    }
                    cx.emit(SessionEvent::Toast { text: n.text, error: n.error });
                }
            })
            .ok();
            PENDING.fetch_sub(1, Ordering::SeqCst);
            log::info!("post-game work of the session ended (result shown: {had_notice})");
            cx.update(crate::system::post_game_idle);
        })
        .detach();
    }
}

/// What the post-game work has to say.
#[derive(Debug, PartialEq)]
pub struct Notice {
    pub title: String,
    pub text: String,
    pub error: bool,
    /// Worth a desktop notification even with the window open.
    pub desktop: bool,
}

/// Blocking: everything that follows a finished session.
pub fn after_play(
    paths: &DataPaths,
    info: &SessionInfo,
    elapsed: Duration,
    runner: &dyn CommandRunner,
    probe: &dyn Probe,
) -> Option<Notice> {
    let ludusavi = match heronite_data::settings::load(paths) {
        Ok(s) => s.ludusavi.enabled,
        Err(e) => {
            log::warn!("reading the settings after the game: {e:#}");
            false
        }
    };
    if info.is_emulator {
        let rom = info.rom.clone().or_else(|| info.selected_rom.clone());
        let started = SystemTime::now().checked_sub(elapsed).unwrap_or_else(SystemTime::now);
        if let Err(e) = remember_save_state_key(paths, &HostEnv::system(), &info.app_name, rom.as_deref(), started) {
            log::warn!("remembering save state key: {e:#}");
        }
    }
    if !ludusavi {
        return None;
    }
    let env = LudusaviEnv { paths, runner, probe };
    let result = backup_after_play(
        &env,
        &AfterPlay {
            app_name: info.app_name.clone(),
            title: info.title.clone(),
            runner: info.runner.clone(),
            elapsed_seconds: elapsed.as_secs(),
        },
    )?;
    backup_notice(&result, &info.title)
}

/// Saved backups are announced on the desktop; problems show in the
/// Collection (and on the desktop only when it has no window).
pub fn backup_notice(result: &LudusaviBackupResult, title: &str) -> Option<Notice> {
    if let Some((t, body)) = notification(result, title) {
        return Some(Notice { title: t, text: body, error: false, desktop: true });
    }
    result.error.as_ref().map(|e| Notice {
        title: "Ludusavi".into(),
        text: tr!("Ludusavi backup failed: {error}", error = e),
        error: true,
        desktop: false,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use heronite_data::DataPaths;
    use heronite_emulation::ludusavi::{CommandOutput, CommandRunner};
    use heronite_launch::probe::FakeProbe;

    use super::{after_play, backup_notice, LudusaviBackupResult, Notice};
    use crate::runtime::SessionInfo;

    /// The backup has a process-wide in-flight guard: tests take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn turn() -> std::sync::MutexGuard<'static, ()> {
        TURN.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records the calls instead of running Ludusavi.
    #[derive(Default)]
    struct FakeLudusavi(Mutex<Vec<String>>);

    impl CommandRunner for FakeLudusavi {
        fn run(&self, _binary: &str, args: &[String]) -> Result<CommandOutput, String> {
            self.0.lock().unwrap().push(args.join(" "));
            let stdout = r#"{"overall":{"totalGames":1},"games":{"Hades":{}}}"#.to_string();
            Ok(CommandOutput { stdout, stderr: String::new(), code: 0 })
        }
    }

    fn info() -> SessionInfo {
        SessionInfo {
            app_name: "hades".into(),
            runner: "sideload".into(),
            title: "Hades".into(),
            steam_app_id: None,
            aliases: Vec::new(),
            is_emulator: false,
            rom: None,
            selected_rom: None,
        }
    }

    fn data_dir(settings: &str) -> (tempfile::TempDir, DataPaths) {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), settings).unwrap();
        (d, paths)
    }

    const ENABLED: &str = r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"binaryPath":"","backupPath":"/backups","format":"zip","compression":"zstd"}}"#;

    #[test]
    fn backup_after_play_needs_no_window_and_reports_the_result() {
        let _turn = turn();
        let (_d, paths) = data_dir(ENABLED);
        let runner = FakeLudusavi::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let n = after_play(&paths, &info(), Duration::from_secs(120), &runner, &probe).expect("a backup notice");
        assert!(!n.error && n.desktop);
        assert_eq!(n.title, "Ludusavi");
        assert!(n.text.contains("Hades"), "{}", n.text);
        assert!(runner.0.lock().unwrap().iter().any(|c| c.starts_with("backup")));
    }

    #[test]
    fn nothing_runs_when_ludusavi_is_off_or_the_session_was_short() {
        let _turn = turn();
        let (_d, paths) = data_dir(r#"{"ludusavi":{"enabled":false}}"#);
        let runner = FakeLudusavi::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        assert_eq!(after_play(&paths, &info(), Duration::from_secs(120), &runner, &probe), None);
        let (_d2, paths2) = data_dir(ENABLED);
        assert_eq!(after_play(&paths2, &info(), Duration::from_secs(2), &runner, &probe), None);
        assert!(runner.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_missing_binary_is_reported_as_an_error() {
        let _turn = turn();
        let (_d, paths) = data_dir(ENABLED);
        let runner = FakeLudusavi::default();
        let n = after_play(&paths, &info(), Duration::from_secs(120), &runner, &FakeProbe::new("/h")).expect("an error notice");
        assert!(n.error && !n.desktop);
        assert!(n.text.contains("not found"), "{}", n.text);
    }

    #[test]
    fn saved_backup_is_a_desktop_notice() {
        let r = LudusaviBackupResult { ran: true, game_name: Some("Hades".into()), backup_path: Some("/b".into()), ..Default::default() };
        let n = backup_notice(&r, "hades").unwrap();
        assert_eq!(n, Notice { title: "Ludusavi".into(), text: "Saved Hades to /b".into(), error: false, desktop: true });
    }

    #[test]
    fn failed_backup_is_an_error_only_for_the_desktop_without_window() {
        let r = LudusaviBackupResult { error: Some("boom".into()), ..Default::default() };
        let n = backup_notice(&r, "x").unwrap();
        assert!(n.error && !n.desktop);
        assert_eq!(n.text, "Ludusavi backup failed: boom");
    }

    #[test]
    fn skipped_backup_says_nothing() {
        let r = LudusaviBackupResult { skipped_reason: Some("disabled".into()), ..Default::default() };
        assert_eq!(backup_notice(&r, "x"), None);
    }
}
