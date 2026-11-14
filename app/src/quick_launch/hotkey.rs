//! The launcher's keyboard shortcut, registered while the app runs (in the
//! tray too) the way the overlay's and the replay's are: `RegisterHotKey` on
//! Windows, a temporary bind on Hyprland, the GlobalShortcuts portal
//! elsewhere. None unless set in Settings → Quick launcher.

use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::quick_launch_prefs::QuickLaunchPrefs;
use heronite_data::{AppPaths, DataPaths};
use heronite_media::capture::{GlobalHotkey, HotkeySpec};

use crate::overlay::HotkeyState;

pub struct QuickLaunchService {
    app_paths: AppPaths,
    prefs: QuickLaunchPrefs,
    /// The shortcut registered, and its accelerator.
    hotkey: Option<(String, GlobalHotkey)>,
    hotkey_state: HotkeyState,
    /// Bumped per registration, so a late answer of an old one is ignored.
    hotkey_epoch: u64,
}

struct ServiceGlobal(Entity<QuickLaunchService>);
impl Global for ServiceGlobal {}

/// Creates the service (once) and registers the shortcut set in Settings.
pub fn init(cx: &mut App, data_paths: &DataPaths) {
    if cx.has_global::<ServiceGlobal>() {
        return;
    }
    let app_paths = AppPaths::for_data(data_paths);
    let prefs = QuickLaunchPrefs::load(&app_paths);
    let entity = cx.new(|_| QuickLaunchService { app_paths, prefs, hotkey: None, hotkey_state: HotkeyState::Idle, hotkey_epoch: 0 });
    cx.set_global(ServiceGlobal(entity.clone()));
    entity.update(cx, |s, cx| s.sync_hotkey(cx));
}

pub fn get(cx: &App) -> Option<Entity<QuickLaunchService>> {
    cx.try_global::<ServiceGlobal>().map(|g| g.0.clone())
}

/// The app is quitting: the shortcut, for the caller to drop off the UI
/// thread (a Hyprland bind would otherwise outlive the app).
pub fn take_hotkey(cx: &mut App) -> Option<GlobalHotkey> {
    let service = get(cx)?;
    service.update(cx, |s, _| {
        s.hotkey_epoch += 1;
        s.hotkey_state = HotkeyState::Idle;
        s.hotkey.take().map(|(_, h)| h)
    })
}

impl QuickLaunchService {
    /// The shortcut (Electron-style), empty for none.
    pub fn accelerator(&self) -> &str {
        &self.prefs.keyboard_accelerator
    }

    pub fn hotkey_state(&self) -> &HotkeyState {
        &self.hotkey_state
    }

    /// Settings changed the shortcut (empty removes it): saved and registered.
    pub fn set_accelerator(&mut self, accelerator: String, cx: &mut Context<Self>) {
        self.prefs.keyboard_accelerator = accelerator.trim().to_string();
        if let Err(e) = self.prefs.save(&self.app_paths) {
            log::warn!("quick launcher: saving its options: {e:#}");
        }
        self.sync_hotkey(cx);
        cx.notify();
    }

    fn sync_hotkey(&mut self, cx: &mut Context<Self>) {
        let accelerator = self.prefs.keyboard_accelerator.trim();
        let wanted = (!accelerator.is_empty()).then(|| accelerator.to_string());
        // The same keys again after a failure: tried once more.
        let failed = matches!(self.hotkey_state, HotkeyState::Failed(_));
        if wanted.as_deref() == self.hotkey.as_ref().map(|(a, _)| a.as_str()) && !failed {
            return;
        }
        self.hotkey_epoch += 1;
        if let Some((_, old)) = self.hotkey.take() {
            // Removing it waits for hyprctl / the portal.
            cx.background_spawn(async move { drop(old) }).detach();
        }
        let Some(accelerator) = wanted else {
            self.hotkey_state = HotkeyState::Idle;
            cx.notify();
            return;
        };
        log::info!("quick launcher: registering the keyboard shortcut {accelerator}");
        let (press_tx, mut presses) = unbounded();
        let (ready_tx, mut ready) = unbounded();
        let spec = HotkeySpec {
            id: "quick-launch",
            description: "Heronite: quick launcher",
            // SIGUSR1 is the overlay's, SIGUSR2 the screenshot's, SIGRTMIN+1 the replay's.
            #[cfg(target_os = "linux")]
            signal: libc::SIGRTMIN() + 2,
            accelerator: accelerator.clone(),
            consume: true,
            portal_app_id: Some("app.heronite.Heronite".into()),
        };
        let hotkey = GlobalHotkey::start(
            spec,
            Arc::new(move || {
                let _ = press_tx.unbounded_send(());
            }),
            Box::new(move |result| {
                let _ = ready_tx.unbounded_send(result);
            }),
        );
        self.hotkey = Some((accelerator, hotkey));
        self.hotkey_state = HotkeyState::Registering;
        cx.notify();
        let epoch = self.hotkey_epoch;
        cx.spawn(async move |this, cx| {
            if let Some(result) = ready.next().await {
                let current = this
                    .update(cx, |s, cx| {
                        if s.hotkey_epoch != epoch {
                            return false;
                        }
                        s.hotkey_state = match result {
                            Ok(backend) => HotkeyState::Ready(backend),
                            Err(e) => {
                                log::warn!("quick launcher: keyboard shortcut: {e}");
                                HotkeyState::Failed(e)
                            }
                        };
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
            }
            while presses.next().await.is_some() {
                if !this.read_with(cx, |s, _| s.hotkey_epoch == epoch).unwrap_or(false) {
                    break;
                }
                cx.update(super::toggle);
            }
        })
        .detach();
    }
}
