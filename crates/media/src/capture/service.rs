//! The capture service: session bookkeeping, shortcut registration, capture
//! and feedback. Everything here talks to the desktop (hyprctl, D-Bus, grim,
//! notifications) and is deliberately not exercised by the unit tests.
//!
//! The callers are on the UI thread, and the desktop can be slow to answer
//! (`hyprctl` has a 2 s timeout per call, a portal may wait for the user), so
//! the shortcut is registered and removed on a thread of its own: a change
//! only records what is wanted and wakes it.

use super::portal::PortalShortcut;
use super::{
    capture_folder, current_game, feedback, grim, hyprland, portal, temporary_name, to_hyprland_binding, to_portal_trigger,
    unique_screenshot_path, ActiveGame, Asked, CaptureConfig, CaptureEvent, CaptureOutcome, CaptureStatus, ServiceOptions, ShortcutBackend,
    SkipReason,
};
use crate::list::resolve_path;
use heronite_i18n::tr;
use signal_hook::consts::signal::SIGUSR2;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
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
    /// What the desktop says triggers the portal's shortcut, if it names it.
    portal_trigger: Option<String>,
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
    /// Wakes the registration thread; `None` after [`CaptureService::shutdown`].
    wake: Mutex<Option<Sender<()>>>,
    /// Held while the shortcut is registered or removed: the registration
    /// thread and `shutdown` never interleave.
    registering: Mutex<()>,
}

/// Clears `busy` however the capture ends (a panic included).
struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
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
        let (wake, woken) = mpsc::channel();
        let inner = Arc::new(Inner {
            events: Mutex::new(events),
            options,
            state: Mutex::new(State::default()),
            busy: AtomicBool::new(false),
            signals: Mutex::new(None),
            wake: Mutex::new(Some(wake)),
            registering: Mutex::new(()),
        });
        let service = Self { inner };
        service.install_signal_handler();
        service.start_registration_thread(woken);
        service
    }

    /// The thread ends when the service is shut down or its last clone goes.
    fn start_registration_thread(&self, woken: mpsc::Receiver<()>) {
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let spawned = std::thread::Builder::new().name("heronite-capture-shortcut".into()).spawn(move || {
            while woken.recv().is_ok() {
                // Several changes in a row need one registration.
                while woken.try_recv().is_ok() {}
                let Some(inner) = weak.upgrade() else { break };
                CaptureService { inner }.reconfigure();
            }
        });
        if let Err(e) = spawned {
            log::error!("could not start the screenshot shortcut thread: {e}");
        }
    }

    /// Asks the registration thread to make the shortcut match the state.
    fn request_reconfigure(&self) {
        if let Some(wake) = self.inner.wake.lock().unwrap().as_ref() {
            let _ = wake.send(());
        }
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
        self.request_reconfigure();
    }

    /// A game launched: it receives captures until [`end_session`](Self::end_session).
    pub fn begin_session(&self, game: ActiveGame) {
        log::info!("Collection screenshot session started for {} ({})", game.title, game.key());
        self.inner.state.lock().unwrap().games.insert(game.key(), game);
        self.request_reconfigure();
    }

    pub fn end_session(&self, runner: &str, app_name: &str) {
        log::info!("Collection screenshot session ended for {runner}:{app_name}");
        self.inner.state.lock().unwrap().games.remove(&format!("{runner}:{app_name}"));
        self.request_reconfigure();
    }

    /// Removes the compositor bind and the portal session, here and now (the
    /// process may be about to exit). Blocking.
    pub fn shutdown(&self) {
        {
            let mut st = self.inner.state.lock().unwrap();
            st.games.clear();
        }
        // No game is left, so a registration still queued only removes too.
        drop(self.inner.wake.lock().unwrap().take());
        {
            let _registering = self.inner.registering.lock().unwrap_or_else(|e| e.into_inner());
            self.unregister();
        }
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
            desktop_trigger: st.portal_registered.as_ref().map(|_| st.portal_trigger.clone().unwrap_or_default()),
        }
    }

    fn unregister(&self) {
        let (hypr, portal) = {
            let mut st = self.inner.state.lock().unwrap();
            st.epoch += 1;
            st.portal_registered = None;
            st.portal_trigger = None;
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

    /// Blocking: runs on the registration thread.
    fn reconfigure(&self) {
        let _registering = self.inner.registering.lock().unwrap_or_else(|e| e.into_inner());
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

        let mut hyprland_error = None;
        if hyprland::is_hyprland_session() {
            let result = to_hyprland_binding(&accelerator)
                .ok_or_else(|| {
                    hyprland::Failure::Refused(tr!("The shortcut {accelerator} is not supported by Hyprland", accelerator = &accelerator))
                })
                .and_then(|b| hyprland::register(&b, &accelerator).map(|_| b));
            match result {
                Ok(binding) => {
                    {
                        let mut st = self.inner.state.lock().unwrap();
                        st.hyprland = Some((binding.keys, accelerator.clone()));
                        st.last_notified_error = None;
                    }
                    log::info!("Collection screenshot shortcut {accelerator} registered through Hyprland");
                    self.send(CaptureEvent::ShortcutRegistered { accelerator, backend: ShortcutBackend::Hyprland });
                    return;
                }
                // Hyprland's portal may take the shortcut where `hyprctl`
                // cannot; what Hyprland itself refused stays refused.
                Err(hyprland::Failure::Unavailable(e)) => {
                    log::warn!("Hyprland could not be asked for the shortcut; trying the desktop portal: {e}");
                    hyprland_error = Some(e);
                }
                Err(hyprland::Failure::Refused(e)) => return self.registration_failed(e),
            }
        }

        let Some(trigger) = to_portal_trigger(&accelerator) else {
            let unavailable = tr!("The shortcut {accelerator} is unavailable", accelerator = &accelerator);
            return self.registration_failed(hyprland_error.unwrap_or(unavailable));
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
                    Ok(desktop_trigger) => {
                        log::info!("Collection screenshot shortcut {accel_ready} registered; the desktop's trigger: {desktop_trigger:?}");
                        {
                            let mut st = svc.inner.state.lock().unwrap();
                            st.portal_registered = Some(accel_ready.clone());
                            st.portal_trigger = desktop_trigger;
                            st.last_notified_error = None;
                        }
                        svc.send(CaptureEvent::ShortcutRegistered { accelerator: accel_ready, backend: ShortcutBackend::Portal });
                    }
                    // Hyprland's reason first: that is the session's own way.
                    Err(e) => svc.registration_failed(hyprland_error.unwrap_or(e)),
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

    /// Captures the screen into the active game's folder: the display under
    /// the pointer on Hyprland, what the desktop's portal captures elsewhere
    /// (as a rule every display). Blocking; also what the shortcut triggers.
    pub fn capture_now(&self) -> CaptureOutcome {
        self.capture(Asked::Shortcut)
    }

    /// Like [`capture_now`](Self::capture_now) for the in-game overlay, which
    /// has its own switch and shows its own notice: it works with the
    /// shortcut turned off and shows no OSD or notification.
    pub fn capture_quiet(&self) -> CaptureOutcome {
        self.capture(Asked::Overlay)
    }

    /// Like [`capture_now`](Self::capture_now) for a capture asked for from
    /// outside the app (`heronite://screenshot`, which a key bound in the
    /// desktop's settings runs where no shortcut can be registered): it
    /// works with the shortcut turned off.
    pub fn capture_requested(&self) -> CaptureOutcome {
        self.capture(Asked::Command)
    }

    fn capture(&self, asked: Asked) -> CaptureOutcome {
        let (config, game) = {
            let st = self.inner.state.lock().unwrap();
            (st.config.clone(), current_game(st.games.values()).cloned())
        };
        if asked.needs_the_switch() && !config.enabled {
            return CaptureOutcome::Skipped(SkipReason::Disabled);
        }
        let Some(game) = game else { return CaptureOutcome::Skipped(SkipReason::NoActiveGame) };
        if config.folder.as_os_str().is_empty() {
            return CaptureOutcome::Skipped(SkipReason::FolderNotConfigured);
        }
        if self.inner.busy.swap(true, Ordering::SeqCst) {
            return CaptureOutcome::Skipped(SkipReason::Busy);
        }
        let _busy = Busy(&self.inner.busy);
        match self.capture_into(&config, &game) {
            Ok(path) => {
                log::info!("Collection screenshot saved to {}", path.display());
                self.send(CaptureEvent::Saved { app_name: game.app_name.clone(), runner: game.runner.clone(), path: path.clone() });
                let m = &self.inner.options.messages;
                if asked.shows_the_result() && !feedback::show_osd(tr(&m.saved_title), &game.title) {
                    feedback::notify(
                        tr(&m.saved_title),
                        &tr(&m.saved_body).replace("{game}", &game.title).replace("{path}", &path.to_string_lossy()),
                    );
                }
                CaptureOutcome::Saved(path)
            }
            Err(error) => {
                log::error!("Collection screenshot capture failed: {error}");
                if asked.shows_the_result() {
                    let m = &self.inner.options.messages;
                    feedback::notify(tr(&m.failed_title), &tr(&m.failed_body).replace("{error}", &error));
                }
                self.send(CaptureEvent::Failed { error: error.clone() });
                CaptureOutcome::Failed(error)
            }
        }
    }

    fn capture_into(&self, config: &CaptureConfig, game: &ActiveGame) -> Result<std::path::PathBuf, String> {
        let root = resolve_path(&config.folder);
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let folder = capture_folder(game, &root);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let target = unique_screenshot_path(&folder, chrono::Local::now().naive_local());
        let temporary = folder.join(temporary_name(&game.app_name));
        let png = capture_display_png(self.inner.options.portal_app_id.as_deref())?;
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

/// PNG bytes of the screen. Hyprland: the display under the pointer with
/// `grim`, else the Screenshot portal. Elsewhere: the portal, else `grim`
/// with every display, which the compositors of the wlroots family and niri
/// take where they have no Screenshot portal.
fn capture_display_png(portal_app_id: Option<&str>) -> Result<Vec<u8>, String> {
    let hyprland = hyprland::is_hyprland_session();
    if hyprland {
        match hyprland::output_under_pointer().and_then(|name| grim::capture_png(Some(&name))) {
            Ok(png) => return Ok(png),
            Err(e) => log::warn!("Native Hyprland screenshot failed; trying the portal: {e}"),
        }
    }
    let portal_error = match portal_png(portal_app_id) {
        Ok(png) => return Ok(png),
        Err(e) => e,
    };
    if hyprland {
        return Err(portal_error);
    }
    log::warn!("The screenshot portal failed; trying grim: {portal_error}");
    grim::capture_png(None).map_err(|e| {
        log::warn!("grim failed too: {e}");
        portal_error
    })
}

/// PNG bytes from the Screenshot portal, which writes a file.
fn portal_png(portal_app_id: Option<&str>) -> Result<Vec<u8>, String> {
    let path = portal::screenshot(portal_app_id)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{HyprlandBind, BIND_DESCRIPTION};
    use std::time::{Duration, Instant};

    /// The first event `pick` takes, within 20 s.
    fn wait_for<T>(events: &mpsc::Receiver<CaptureEvent>, mut pick: impl FnMut(CaptureEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let event = events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("the service answers in time");
            if let Some(found) = pick(event) {
                return found;
            }
        }
    }

    fn our_binds() -> Vec<String> {
        let ours = |b: &HyprlandBind| b.description.as_deref() == Some(BIND_DESCRIPTION);
        hyprland::binds().unwrap().iter().filter(|b| ours(b)).filter_map(HyprlandBind::keys).collect()
    }

    /// The whole way on a Hyprland session: the shortcut is bound without
    /// the caller waiting, a bind left by an earlier run goes, the command
    /// Hyprland runs for the keys reaches this process and the screen is
    /// captured, and the bind goes with the game. Binds unlikely keys for a
    /// moment and captures the screen into a temporary folder.
    #[test]
    #[ignore = "binds keys in the running Hyprland and captures the screen"]
    fn live_hyprland_shortcut_captures() {
        if !hyprland::is_hyprland_session() {
            return eprintln!("not a Hyprland session; skipped");
        }
        let folder = tempfile::tempdir().unwrap();
        let (tx, events) = mpsc::channel();
        let service = CaptureService::new(tx, ServiceOptions::default());
        // As a Heronite that crashed with another shortcut set would leave it.
        hyprland::register(&to_hyprland_binding("CommandOrControl+Alt+Shift+F8").unwrap(), "CommandOrControl+Alt+Shift+F8").unwrap();
        assert_eq!(our_binds(), ["CTRL + ALT + SHIFT + F8"]);

        service.set_config(CaptureConfig {
            enabled: true,
            folder: folder.path().into(),
            accelerator: "Shift+Alt+CommandOrControl+F9".into(),
        });
        let called = Instant::now();
        service.begin_session(ActiveGame {
            app_name: "live-test".into(),
            runner: "sideload".into(),
            title: "Live test".into(),
            aliases: vec![],
            steam_app_id: None,
            started_at: std::time::SystemTime::now(),
        });
        eprintln!("begin_session returned after {:?}", called.elapsed());
        let backend = wait_for(&events, |e| match e {
            CaptureEvent::ShortcutRegistered { backend, .. } => Some(backend),
            CaptureEvent::ShortcutError { error } => panic!("{error}"),
            _ => None,
        });
        eprintln!("registered {:?} after the call", called.elapsed());
        assert_eq!(backend, ShortcutBackend::Hyprland);
        assert!(service.status().registered);
        assert_eq!(our_binds(), ["CTRL + ALT + SHIFT + F9"], "the stale bind went, whatever its keys");

        let command = hyprland::signal_command(SIGUSR2);
        eprintln!("the bind runs: {command}");
        assert!(std::process::Command::new("sh").args(["-c", &command]).status().unwrap().success());
        let path = wait_for(&events, |e| match e {
            CaptureEvent::Saved { path, .. } => Some(path),
            CaptureEvent::Failed { error } => panic!("{error}"),
            _ => None,
        });
        assert!(path.starts_with(folder.path().join("Live test")), "{}", path.display());
        assert!(std::fs::read(&path).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));

        service.end_session("sideload", "live-test");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !our_binds().is_empty() {
            assert!(Instant::now() < deadline, "the bind is removed with the game: {:?}", our_binds());
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!service.status().registered);
    }
}
