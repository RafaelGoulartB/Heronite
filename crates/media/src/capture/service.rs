//! The capture service: session bookkeeping, shortcut registration, capture
//! and feedback. Everything here talks to the desktop (hyprctl, D-Bus, grim,
//! notifications) and is deliberately not exercised by the unit tests.

use super::portal::PortalShortcut;
use super::{
    capture_folder, current_game, feedback, grim, hyprland, portal, temporary_name, to_hyprland_binding, to_portal_trigger,
    unique_screenshot_path, ActiveGame, CaptureConfig, CaptureEvent, CaptureOutcome, CaptureStatus, ServiceOptions, ShortcutBackend,
    SkipReason,
};
use crate::list::resolve_path;
use heronite_i18n::tr;
use signal_hook::consts::signal::SIGUSR2;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
struct State {
    config: CaptureConfig,
    games: HashMap<String, ActiveGame>,
    /// `(keys, accelerator)` of the Hyprland bind we installed.
    hyprland: Option<(String, String)>,
    portal: Option<PortalShortcut>,
    /// Accelerator the portal session was asked for, once it confirmed.
    portal_registered: Option<String>,
    registration_error: Option<String>,
    last_notified_error: Option<String>,
    /// Bumped on every (re)configuration so late portal answers are ignored.
    epoch: u64,
}

struct Inner {
    events: Mutex<Sender<CaptureEvent>>,
    options: ServiceOptions,
    state: Mutex<State>,
    busy: AtomicBool,
    signals: Mutex<Option<signal_hook::iterator::Handle>>,
}

/// Captures the screen while a launched game runs. Cheap to clone.
///
/// Create one per process: it installs a SIGUSR2 handler (used by the
/// Hyprland bind) that must be in place before any bind exists, because the
/// default action of SIGUSR2 is to terminate the process.
#[derive(Clone)]
pub struct CaptureService {
    inner: Arc<Inner>,
}

impl CaptureService {
    pub fn new(events: Sender<CaptureEvent>, options: ServiceOptions) -> Self {
        let inner = Arc::new(Inner {
            events: Mutex::new(events),
            options,
            state: Mutex::new(State::default()),
            busy: AtomicBool::new(false),
            signals: Mutex::new(None),
        });
        let service = Self { inner };
        service.install_signal_handler();
        service
    }

    fn install_signal_handler(&self) {
        let mut signals = match signal_hook::iterator::Signals::new([SIGUSR2]) {
            Ok(s) => s,
            Err(e) => {
                log::error!("could not install the SIGUSR2 handler: {e}");
                return;
            }
        };
        *self.inner.signals.lock().unwrap() = Some(signals.handle());
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("heronite-capture-signal".into()).spawn(move || {
            for _ in signals.forever() {
                let Some(inner) = weak.upgrade() else { break };
                let svc = CaptureService { inner };
                let accel = {
                    let st = svc.inner.state.lock().unwrap();
                    st.hyprland.as_ref().filter(|_| current_game(st.games.values()).is_some()).map(|h| h.1.clone())
                };
                if let Some(accel) = accel {
                    svc.shortcut_pressed(&accel, "Hyprland");
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("could not start the SIGUSR2 thread: {e}");
        }
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

    /// Removes the compositor bind and the portal session.
    pub fn shutdown(&self) {
        {
            let mut st = self.inner.state.lock().unwrap();
            st.games.clear();
        }
        self.unregister();
        if let Some(handle) = self.inner.signals.lock().unwrap().take() {
            handle.close();
        }
    }

    pub fn status(&self) -> CaptureStatus {
        let st = self.inner.state.lock().unwrap();
        CaptureStatus {
            registered: st.hyprland.is_some() || st.portal_registered.is_some(),
            accelerator: st.config.accelerator.clone(),
            active_game: current_game(st.games.values()).map(|g| (g.app_name.clone(), g.runner.clone(), g.title.clone())),
            error: st.registration_error.clone(),
        }
    }

    fn unregister(&self) {
        let (hypr, portal) = {
            let mut st = self.inner.state.lock().unwrap();
            st.epoch += 1;
            st.portal_registered = None;
            st.registration_error = None;
            (st.hyprland.take(), st.portal.take())
        };
        if let Some((keys, _)) = hypr {
            if let Err(e) = hyprland::unregister(&keys) {
                log::warn!("Could not remove the Hyprland screenshot shortcut: {e}");
            }
        }
        // Joins the portal thread; must not hold the state lock.
        drop(portal);
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

        if hyprland::is_hyprland_session() {
            let result = to_hyprland_binding(&accelerator)
                .ok_or_else(|| tr!("The shortcut {accelerator} is not supported by Hyprland", accelerator = &accelerator))
                .and_then(|b| hyprland::register(&b, &accelerator, std::process::id()).map(|_| b));
            match result {
                Ok(binding) => {
                    {
                        let mut st = self.inner.state.lock().unwrap();
                        st.hyprland = Some((binding.keys, accelerator.clone()));
                        st.last_notified_error = None;
                    }
                    log::info!("Collection screenshot shortcut {accelerator} registered through Hyprland");
                    self.send(CaptureEvent::ShortcutRegistered { accelerator, backend: ShortcutBackend::Hyprland });
                }
                Err(e) => self.registration_failed(e),
            }
            return;
        }

        let Some(trigger) = to_portal_trigger(&accelerator) else {
            self.registration_failed(tr!("The shortcut {accelerator} is unavailable", accelerator = &accelerator));
            return;
        };
        let weak = Arc::downgrade(&self.inner);
        let weak_ready = weak.clone();
        let accel_ready = accelerator.clone();
        let accel_press = accelerator.clone();
        let shortcut = PortalShortcut::start(
            self.inner.options.portal_app_id.clone(),
            (portal::SHORTCUT_ID, super::BIND_DESCRIPTION),
            trigger,
            Arc::new(move || {
                if let Some(inner) = weak.upgrade() {
                    CaptureService { inner }.shortcut_pressed(&accel_press, "portal");
                }
            }),
            Box::new(move |result| {
                let Some(inner) = weak_ready.upgrade() else { return };
                let svc = CaptureService { inner };
                if svc.inner.state.lock().unwrap().epoch != epoch {
                    return; // superseded
                }
                match result {
                    Ok(()) => {
                        {
                            let mut st = svc.inner.state.lock().unwrap();
                            st.portal_registered = Some(accel_ready.clone());
                            st.last_notified_error = None;
                        }
                        log::info!("Collection screenshot shortcut {accel_ready} registered");
                        svc.send(CaptureEvent::ShortcutRegistered { accelerator: accel_ready, backend: ShortcutBackend::Portal });
                    }
                    Err(e) => svc.registration_failed(e),
                }
            }),
        );
        let mut st = self.inner.state.lock().unwrap();
        if st.epoch == epoch {
            st.portal = Some(shortcut);
        } else {
            drop(st);
            drop(shortcut);
        }
    }

    fn registration_failed(&self, error: String) {
        log::error!("Collection screenshot shortcut registration failed: {error}");
        let notify = {
            let mut st = self.inner.state.lock().unwrap();
            st.registration_error = Some(error.clone());
            let fresh = current_game(st.games.values()).is_some() && st.last_notified_error.as_deref() != Some(&error);
            if fresh {
                st.last_notified_error = Some(error.clone());
            }
            fresh
        };
        if notify {
            let m = &self.inner.options.messages;
            feedback::notify(tr(&m.shortcut_failed_title), &tr(&m.shortcut_failed_body).replace("{error}", &error));
        }
        self.send(CaptureEvent::ShortcutError { error });
    }

    fn shortcut_pressed(&self, accelerator: &str, source: &str) {
        log::info!("Collection screenshot shortcut {accelerator} pressed via {source}");
        let svc = self.clone();
        std::thread::spawn(move || {
            svc.capture_now();
        });
    }

    /// Captures the display under the pointer into the active game's folder.
    /// Blocking; also what the shortcut triggers.
    pub fn capture_now(&self) -> CaptureOutcome {
        self.capture(true)
    }

    /// Like [`capture_now`](Self::capture_now) for the in-game overlay, which
    /// has its own switch and shows its own notice: it works with the
    /// shortcut turned off and shows no OSD or notification.
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
        let outcome = match self.capture_into(&config, &game) {
            Ok(path) => {
                log::info!("Collection screenshot saved to {}", path.display());
                self.send(CaptureEvent::Saved { app_name: game.app_name.clone(), runner: game.runner.clone(), path: path.clone() });
                let m = &self.inner.options.messages;
                if shortcut && !feedback::show_osd(tr(&m.saved_title), &game.title) {
                    feedback::notify(
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
                    feedback::notify(tr(&m.failed_title), &tr(&m.failed_body).replace("{error}", &error));
                }
                self.send(CaptureEvent::Failed { error: error.clone() });
                CaptureOutcome::Failed(error)
            }
        };
        self.inner.busy.store(false, Ordering::SeqCst);
        outcome
    }

    fn capture_into(&self, config: &CaptureConfig, game: &ActiveGame) -> Result<std::path::PathBuf, String> {
        let root = resolve_path(&config.folder);
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let folder = capture_folder(game, &root);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let target = unique_screenshot_path(&folder, chrono::Local::now().naive_local());
        let temporary = folder.join(temporary_name(&game.app_name));
        let png = capture_display_png()?;
        std::fs::write(&temporary, &png).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &target).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            e.to_string()
        })?;
        Ok(target)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(st) = self.state.get_mut() {
            if let Some((keys, _)) = st.hyprland.take() {
                let _ = hyprland::unregister(&keys);
            }
        }
        if let Ok(handle) = self.signals.get_mut() {
            if let Some(handle) = handle.take() {
                handle.close();
            }
        }
    }
}

/// PNG bytes of the display under the pointer: `grim` on Hyprland, else (or
/// when grim fails) the Screenshot portal.
fn capture_display_png() -> Result<Vec<u8>, String> {
    if hyprland::is_hyprland_session() {
        match hyprland::output_under_pointer().and_then(|name| grim::capture_png(Some(&name))) {
            Ok(png) => return Ok(png),
            Err(e) => log::warn!("Native Hyprland screenshot failed; trying the portal: {e}"),
        }
    }
    let path = portal::screenshot()?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()));
    // The portal leaves its file behind; remove it when it lives in a temp dir.
    if is_temporary(&path) {
        let _ = std::fs::remove_file(&path);
    }
    let bytes = bytes?;
    if bytes.is_empty() {
        return Err("the screenshot portal returned an empty image".into());
    }
    Ok(bytes)
}

fn is_temporary(path: &Path) -> bool {
    path.starts_with(std::env::temp_dir()) || std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| path.starts_with(d))
}
