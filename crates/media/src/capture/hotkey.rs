//! A global keyboard shortcut besides the screenshot one (the in-game
//! overlay's), registered the same two ways: a temporary Hyprland bind that
//! sends this process a signal, else the XDG GlobalShortcuts portal.
//! Like the rest of the service, not exercised by the unit tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use heronite_i18n::tr;

use super::portal::PortalShortcut;
use super::{hyprland, to_hyprland_binding, to_portal_trigger, HotkeySpec, ShortcutBackend};

#[derive(Default)]
struct Shared {
    stopped: AtomicBool,
    /// Keys of the Hyprland bind, once installed.
    hyprland: Mutex<Option<String>>,
    portal: Mutex<Option<PortalShortcut>>,
    signals: Mutex<Option<signal_hook::iterator::Handle>>,
}

/// A registered shortcut. Dropping it removes it, which blocks briefly
/// (`hyprctl`, the portal session): drop it off the UI thread.
pub struct GlobalHotkey {
    shared: Arc<Shared>,
}

type Ready = Box<dyn FnOnce(Result<ShortcutBackend, String>) + Send>;

impl GlobalHotkey {
    /// Registers `spec` in the background. `on_press` runs (on a thread of
    /// its own) for each press; `on_ready` gets the registration result once.
    pub fn start(spec: HotkeySpec, on_press: Arc<dyn Fn() + Send + Sync>, on_ready: Ready) -> Self {
        let shared = Arc::new(Shared::default());
        if hyprland::is_hyprland_session() {
            start_hyprland(&shared, spec, on_press, on_ready);
        } else if let Some(trigger) = to_portal_trigger(&spec.accelerator) {
            let portal = PortalShortcut::start(
                spec.portal_app_id,
                (spec.id, spec.description),
                trigger,
                on_press,
                Box::new(move |r| on_ready(r.map(|()| ShortcutBackend::Portal))),
            );
            *shared.portal.lock().unwrap() = Some(portal);
        } else {
            on_ready(Err(tr!("The shortcut {accelerator} is unavailable", accelerator = &spec.accelerator)));
        }
        Self { shared }
    }
}

fn start_hyprland(shared: &Arc<Shared>, spec: HotkeySpec, on_press: Arc<dyn Fn() + Send + Sync>, on_ready: Ready) {
    let Some(binding) = to_hyprland_binding(&spec.accelerator) else {
        on_ready(Err(tr!("The shortcut {accelerator} is not supported by Hyprland", accelerator = &spec.accelerator)));
        return;
    };
    // In place before the bind exists: the default action of SIGUSR1 ends
    // the process. signal-hook keeps its handler after the iterator closes, so
    // a late signal is harmless too.
    let mut signals = match signal_hook::iterator::Signals::new([spec.signal]) {
        Ok(s) => s,
        Err(e) => {
            on_ready(Err(e.to_string()));
            return;
        }
    };
    *shared.signals.lock().unwrap() = Some(signals.handle());
    let weak = Arc::downgrade(shared);
    let listening = std::thread::Builder::new().name("heronite-hotkey-signal".into()).spawn(move || {
        for _ in signals.forever() {
            let Some(shared) = weak.upgrade() else { break };
            if !shared.stopped.load(Ordering::SeqCst) {
                on_press();
            }
        }
    });
    if let Err(e) = listening {
        on_ready(Err(e.to_string()));
        return;
    }
    let shared = shared.clone();
    let spawned = std::thread::Builder::new().name("heronite-hotkey-bind".into()).spawn(move || {
        let command = format!("kill -{} {}", spec.signal, std::process::id());
        let result = hyprland::register_as(&binding, &spec.accelerator, &command, spec.description, spec.consume);
        if result.is_ok() {
            let mut keys = shared.hyprland.lock().unwrap();
            if shared.stopped.load(Ordering::SeqCst) {
                // Dropped while it was being registered.
                drop(keys);
                let _ = hyprland::unregister(&binding.keys);
            } else {
                *keys = Some(binding.keys.clone());
            }
        }
        on_ready(result.map(|()| ShortcutBackend::Hyprland));
    });
    if let Err(e) = spawned {
        log::warn!("could not start the shortcut thread: {e}");
    }
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        let keys = {
            let mut keys = self.shared.hyprland.lock().unwrap();
            self.shared.stopped.store(true, Ordering::SeqCst);
            keys.take()
        };
        if let Some(keys) = keys {
            if let Err(e) = hyprland::unregister(&keys) {
                log::warn!("Could not remove the Hyprland shortcut {keys}: {e}");
            }
        }
        // Joins the portal thread.
        drop(self.shared.portal.lock().unwrap().take());
        if let Some(handle) = self.shared.signals.lock().unwrap().take() {
            handle.close();
        }
    }
}
