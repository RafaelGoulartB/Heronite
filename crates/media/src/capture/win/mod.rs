//! The capture service on Windows: the same sessions, settings, events,
//! file names and folders as Linux's (`service.rs`), with Windows means:
//!
//! - the shortcut is a `RegisterHotKey` hotkey on a thread of its own
//!   ([`hotkey`]), registered while a game runs;
//! - the capture is DXGI Desktop Duplication of the game's monitor, GDI as a
//!   fallback ([`screen`]), turned into an 8-bit sRGB PNG ([`pixels`], which
//!   also says what happens on an HDR monitor);
//! - the feedback is an on-screen notice that does not take the focus
//!   ([`osd`]), else a notification.
//!
//! Everything but [`pixels`] talks to the desktop; the tests capture the
//! primary monitor where there is one.

mod hotkey;
mod osd;
mod pixels;
mod screen;

pub use hotkey::GlobalHotkey;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use heronite_i18n::tr;

use super::{
    capture_folder, current_game, temporary_name, unique_screenshot_path, ActiveGame, CaptureConfig, CaptureEvent, CaptureOutcome,
    CaptureStatus, HotkeySpec, ServiceOptions, SkipReason, BIND_DESCRIPTION,
};
use crate::list::resolve_path;

#[derive(Default)]
struct State {
    config: CaptureConfig,
    games: HashMap<String, ActiveGame>,
    hotkey: Option<GlobalHotkey>,
    /// The accelerator Windows accepted.
    registered: Option<String>,
    registration_error: Option<String>,
    last_notified_error: Option<String>,
    /// Bumped on every (re)configuration so late answers are ignored.
    epoch: u64,
}

struct Inner {
    events: Mutex<Sender<CaptureEvent>>,
    options: ServiceOptions,
    state: Mutex<State>,
    busy: AtomicBool,
}

/// Captures the screen while a launched game runs. Cheap to clone.
#[derive(Clone)]
pub struct CaptureService {
    inner: Arc<Inner>,
}

impl CaptureService {
    pub fn new(events: Sender<CaptureEvent>, options: ServiceOptions) -> Self {
        let inner = Inner { events: Mutex::new(events), options, state: Mutex::new(State::default()), busy: AtomicBool::new(false) };
        Self { inner: Arc::new(inner) }
    }

    fn send(&self, event: CaptureEvent) {
        let _ = self.inner.events.lock().unwrap().send(event);
    }

    /// Applies new settings (`screenshots.*`) and re-registers the shortcut.
    pub fn set_config(&self, config: CaptureConfig) {
        self.inner.state.lock().unwrap().config = config;
        self.reconfigure();
    }

    /// A game launched: it receives captures until [`end_session`](Self::end_session).
    pub fn begin_session(&self, game: ActiveGame) {
        log::info!("Collection screenshot session started for {} ({})", game.title, game.key());
        self.inner.state.lock().unwrap().games.insert(game.key(), game);
        self.reconfigure();
    }

    pub fn end_session(&self, runner: &str, app_name: &str) {
        log::info!("Collection screenshot session ended for {runner}:{app_name}");
        self.inner.state.lock().unwrap().games.remove(&format!("{runner}:{app_name}"));
        self.reconfigure();
    }

    /// Unregisters the shortcut.
    pub fn shutdown(&self) {
        self.inner.state.lock().unwrap().games.clear();
        self.unregister();
    }

    pub fn status(&self) -> CaptureStatus {
        let st = self.inner.state.lock().unwrap();
        CaptureStatus {
            registered: st.registered.is_some(),
            accelerator: st.config.accelerator.clone(),
            active_game: current_game(st.games.values()).map(|g| (g.app_name.clone(), g.runner.clone(), g.title.clone())),
            error: st.registration_error.clone(),
        }
    }

    fn unregister(&self) {
        let hotkey = {
            let mut st = self.inner.state.lock().unwrap();
            st.epoch += 1;
            st.registered = None;
            st.registration_error = None;
            st.hotkey.take()
        };
        // Joins the shortcut's thread, which may be waiting for the state
        // lock: never dropped while holding it.
        drop(hotkey);
    }

    fn reconfigure(&self) {
        self.unregister();
        let (config, has_game, epoch) = {
            let st = self.inner.state.lock().unwrap();
            (st.config.clone(), current_game(st.games.values()).is_some(), st.epoch)
        };
        if !config.enabled || !has_game {
            return;
        }
        let accelerator = config.accelerator.clone();
        log::info!("Registering Collection screenshot shortcut {accelerator}");
        let weak = Arc::downgrade(&self.inner);
        let weak_ready = weak.clone();
        let accel_press = accelerator.clone();
        let accel_ready = accelerator.clone();
        let spec = HotkeySpec {
            id: "screenshot",
            description: BIND_DESCRIPTION,
            accelerator,
            consume: true,
            portal_app_id: self.inner.options.portal_app_id.clone(),
        };
        let hotkey = GlobalHotkey::start(
            spec,
            Arc::new(move || {
                if let Some(inner) = weak.upgrade() {
                    CaptureService { inner }.shortcut_pressed(&accel_press);
                }
            }),
            Box::new(move |result| {
                let Some(inner) = weak_ready.upgrade() else { return };
                let svc = CaptureService { inner };
                if svc.inner.state.lock().unwrap().epoch != epoch {
                    return; // superseded
                }
                match result {
                    Ok(backend) => {
                        {
                            let mut st = svc.inner.state.lock().unwrap();
                            st.registered = Some(accel_ready.clone());
                            st.last_notified_error = None;
                        }
                        log::info!("Collection screenshot shortcut {accel_ready} registered with Windows");
                        svc.send(CaptureEvent::ShortcutRegistered { accelerator: accel_ready, backend });
                    }
                    Err(e) => svc.registration_failed(e),
                }
            }),
        );
        let mut st = self.inner.state.lock().unwrap();
        if st.epoch == epoch {
            st.hotkey = Some(hotkey);
        } else {
            drop(st);
            drop(hotkey);
        }
    }

    fn registration_failed(&self, error: String) {
        log::error!("Collection screenshot shortcut registration failed: {error}");
        let fresh = {
            let mut st = self.inner.state.lock().unwrap();
            st.registration_error = Some(error.clone());
            let fresh = current_game(st.games.values()).is_some() && st.last_notified_error.as_deref() != Some(&error);
            if fresh {
                st.last_notified_error = Some(error.clone());
            }
            fresh
        };
        if fresh {
            let m = &self.inner.options.messages;
            notify(tr(&m.shortcut_failed_title), &tr(&m.shortcut_failed_body).replace("{error}", &error));
        }
        self.send(CaptureEvent::ShortcutError { error });
    }

    fn shortcut_pressed(&self, accelerator: &str) {
        log::info!("Collection screenshot shortcut {accelerator} pressed");
        let svc = self.clone();
        let spawned = std::thread::Builder::new().name("heronite-capture".into()).spawn(move || {
            svc.capture_now();
        });
        if let Err(e) = spawned {
            log::error!("could not start the capture thread: {e}");
        }
    }

    /// Captures the game's monitor into the active game's folder. Blocking;
    /// also what the shortcut triggers.
    pub fn capture_now(&self) -> CaptureOutcome {
        self.capture(true)
    }

    /// Like [`capture_now`](Self::capture_now) for the in-game overlay, which
    /// has its own switch and shows its own notice: it works with the
    /// shortcut turned off and shows no notice or notification.
    pub fn capture_quiet(&self) -> CaptureOutcome {
        self.capture(false)
    }

    fn capture(&self, shortcut: bool) -> CaptureOutcome {
        let (config, game) = {
            let st = self.inner.state.lock().unwrap();
            (st.config.clone(), current_game(st.games.values()).cloned())
        };
        if shortcut && !config.enabled {
            return CaptureOutcome::Skipped(SkipReason::Disabled);
        }
        let Some(game) = game else { return CaptureOutcome::Skipped(SkipReason::NoActiveGame) };
        if config.folder.as_os_str().is_empty() {
            return CaptureOutcome::Skipped(SkipReason::FolderNotConfigured);
        }
        if self.inner.busy.swap(true, Ordering::SeqCst) {
            return CaptureOutcome::Skipped(SkipReason::Busy);
        }
        let outcome = match capture_into(&config, &game) {
            Ok((path, monitor)) => {
                log::info!("Collection screenshot saved to {}", path.display());
                self.send(CaptureEvent::Saved { app_name: game.app_name.clone(), runner: game.runner.clone(), path: path.clone() });
                let m = &self.inner.options.messages;
                if shortcut && !osd::show(monitor, tr(&m.saved_title), &game.title) {
                    notify(
                        tr(&m.saved_title),
                        &tr(&m.saved_body).replace("{game}", &game.title).replace("{path}", &path.to_string_lossy()),
                    );
                }
                CaptureOutcome::Saved(path)
            }
            Err(error) => {
                log::error!("Collection screenshot capture failed: {error}");
                if shortcut {
                    let m = &self.inner.options.messages;
                    notify(tr(&m.failed_title), &tr(&m.failed_body).replace("{error}", &error));
                }
                self.send(CaptureEvent::Failed { error: error.clone() });
                CaptureOutcome::Failed(error)
            }
        };
        self.inner.busy.store(false, Ordering::SeqCst);
        outcome
    }
}

/// Captures and writes the PNG (through a hidden temporary file, like Linux)
/// into the game's folder. Returns the file and the monitor captured.
fn capture_into(config: &CaptureConfig, game: &ActiveGame) -> Result<(PathBuf, isize), String> {
    let root = resolve_path(&config.folder);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let folder = capture_folder(game, &root);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let shot = screen::capture_game_monitor()?;
    log::info!("captured {}x{} with {:?}", shot.image.width, shot.image.height, shot.method);
    let png = pixels::encode_png(&shot.image)?;
    let target = unique_screenshot_path(&folder, chrono::Local::now().naive_local());
    let temporary = folder.join(temporary_name(&game.app_name));
    std::fs::write(&temporary, &png).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &target).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        e.to_string()
    })?;
    Ok((target, shot.monitor))
}

/// A Windows notification (toast); it never takes the focus. Sent off the
/// calling thread.
fn notify(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let spawned = std::thread::Builder::new().name("heronite-capture-notify".into()).spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("Heronite").summary(&title).body(&body).show() {
            log::error!("Collection screenshot notification failed: {e}");
        }
    });
    if let Err(e) = spawned {
        log::error!("could not start the notification thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::ShortcutBackend;
    use std::time::SystemTime;

    fn game() -> ActiveGame {
        ActiveGame {
            app_name: "notepad_test".into(),
            runner: "sideload".into(),
            title: "Test: Game".into(),
            aliases: vec![],
            steam_app_id: None,
            started_at: SystemTime::now(),
        }
    }

    #[test]
    fn nothing_is_captured_without_a_game_or_a_folder() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let svc = CaptureService::new(tx, ServiceOptions::default());
        svc.set_config(CaptureConfig { enabled: true, folder: PathBuf::new(), accelerator: "F10".into() });
        assert_eq!(svc.capture_now(), CaptureOutcome::Skipped(SkipReason::NoActiveGame));
        svc.begin_session(game());
        assert_eq!(svc.capture_now(), CaptureOutcome::Skipped(SkipReason::FolderNotConfigured));
        svc.set_config(CaptureConfig { enabled: false, folder: "x".into(), accelerator: "F10".into() });
        assert_eq!(svc.capture_now(), CaptureOutcome::Skipped(SkipReason::Disabled));
        svc.shutdown();
        assert!(svc.status().active_game.is_none());
    }

    #[test]
    fn a_bad_shortcut_is_reported() {
        let (tx, rx) = std::sync::mpsc::channel();
        let svc = CaptureService::new(tx, ServiceOptions::default());
        svc.set_config(CaptureConfig { enabled: true, folder: "x".into(), accelerator: "Hyper+F10".into() });
        svc.begin_session(game());
        let status = svc.status();
        assert!(!status.registered);
        assert!(status.error.is_some_and(|e| e.contains("Hyper+F10")));
        assert!(rx.try_iter().any(|e| matches!(e, CaptureEvent::ShortcutError { .. })));
        svc.shutdown();
    }

    /// The shortcut exists only while a game runs, and a second program
    /// asking for the same keys hears they are taken.
    #[test]
    fn the_shortcut_follows_the_session() {
        if !screen::has_output() {
            eprintln!("no desktop here: hotkeys are not tried");
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let svc = CaptureService::new(tx, ServiceOptions::default());
        // An unusual combination nothing else holds.
        let accel = "CommandOrControl+Alt+Shift+F23";
        svc.set_config(CaptureConfig { enabled: true, folder: "x".into(), accelerator: accel.into() });
        assert!(!svc.status().registered);
        svc.begin_session(game());
        let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(event, CaptureEvent::ShortcutRegistered { accelerator: accel.into(), backend: ShortcutBackend::Windows });
        assert!(svc.status().registered);
        // Someone else asking for it now is told it is taken.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spec = HotkeySpec { id: "other", description: "", accelerator: accel.into(), consume: true, portal_app_id: None };
        let other = GlobalHotkey::start(spec, Arc::new(|| {}), Box::new(move |r| ready_tx.send(r).unwrap()));
        let err = ready_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap_err();
        assert!(err.contains("another program"), "{err}");
        drop(other);
        svc.end_session("sideload", "notepad_test");
        assert!(!svc.status().registered);
        // Free again.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spec = HotkeySpec { id: "again", description: "", accelerator: accel.into(), consume: true, portal_app_id: None };
        let again = GlobalHotkey::start(spec, Arc::new(|| {}), Box::new(move |r| ready_tx.send(r).unwrap()));
        assert_eq!(ready_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), Ok(ShortcutBackend::Windows));
        drop(again);
    }

    /// A capture lands in the game's folder under the shared name.
    #[test]
    fn captures_into_the_game_folder() {
        if !screen::has_output() {
            eprintln!("no desktop here: nothing to capture");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Test Game - Deluxe")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let svc = CaptureService::new(tx, ServiceOptions::default());
        svc.set_config(CaptureConfig { enabled: true, folder: dir.path().to_path_buf(), accelerator: "Ctrl+Alt+Shift+F22".into() });
        let mut g = game();
        g.title = "Test Game".into();
        svc.begin_session(g);
        let CaptureOutcome::Saved(path) = svc.capture_quiet() else { panic!("not saved") };
        assert_eq!(path.parent().unwrap(), dir.path().join("Test Game - Deluxe"));
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.ends_with(".png") && name.len() == "2026-01-01_00-00-00-000.png".len(), "{name}");
        assert!(image::open(&path).is_ok());
        assert!(rx.try_iter().any(|e| matches!(e, CaptureEvent::Saved { path: p, .. } if p == path)));
        // No temporary file left behind.
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        svc.shutdown();
    }
}
