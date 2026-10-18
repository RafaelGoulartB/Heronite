//! The in-game overlay (beta, off by default; Settings → Overlay): a panel
//! the controller's Guide button opens over a game launched by the app, plus
//! two shortcuts that work without opening it (Guide + RB: screenshot,
//! Guide + ↑ / ↓: volume).
//!
//! The service follows the launch events (from the hub, like the post-game
//! work), so it works with the main window closed to the tray. While a game
//! runs and the overlay is on, `heronite_overlay::input` reads the
//! controllers on its own thread; while the panel is open, or Guide is held,
//! the controllers are grabbed and the game gets none of their input.
//!
//! The panel and the notices are Wayland layer-shell surfaces
//! ([`window`]): they float over a fullscreen game without taking focus
//! from it. Desktops without layer-shell (GNOME, X11) cannot show them.

mod debug;
pub mod panel;
mod toast;
mod window;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::overlay_prefs::OverlayPrefs;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_launch::LaunchEvent;
use heronite_media::capture::{CaptureOutcome, SkipReason};
use heronite_overlay::input::{Controller, Event, Options, Watcher};

use self::panel::OverlayPanel;
use self::toast::Notice;
use crate::icons::Icon;
use crate::runtime::{GameRuntime, SessionInfo};

/// Time for the compositor to take the panel off screen before a capture.
const HIDE_BEFORE_CAPTURE: Duration = Duration::from_millis(300);

/// A game launched by the app that is running.
#[derive(Clone, Debug)]
pub struct RunningGame {
    pub app_name: String,
    /// Process (group leader) once known.
    pub pid: Option<u32>,
    pub started: Instant,
}

/// What the panel shows about the game it is open over.
#[derive(Clone, Debug)]
pub struct GameNow {
    pub game: RunningGame,
    pub info: Option<SessionInfo>,
}

impl GameNow {
    pub fn title(&self) -> String {
        self.info.as_ref().map(|i| i.title.clone()).unwrap_or_else(|| self.game.app_name.clone())
    }
}

pub struct OverlayService {
    data_paths: DataPaths,
    runtime: Arc<GameRuntime>,
    prefs: OverlayPrefs,
    /// Oldest first; the panel is about the last one.
    games: Vec<RunningGame>,
    watcher: Option<Watcher>,
    controllers: Vec<Controller>,
    panel: Option<WindowHandle<OverlayPanel>>,
    notice: Option<toast::NoticeWindow>,
    /// `false` once the compositor refused a layer-shell surface.
    layer_shell: bool,
    /// Debug: draw the panel in a normal window (`HERONITE_DEBUG_OVERLAY=window`).
    debug_window: bool,
}

struct OverlayGlobal(Entity<OverlayService>);
impl Global for OverlayGlobal {}

/// Creates the service (once). Called by the hub with the shared runtime.
pub fn init(cx: &mut App, data_paths: DataPaths, runtime: Arc<GameRuntime>) {
    if cx.has_global::<OverlayGlobal>() {
        return;
    }
    // A tiny file in the app's config folder.
    let prefs = OverlayPrefs::load(&AppPaths::for_data(&data_paths));
    let entity = cx.new(|_| OverlayService {
        data_paths,
        runtime,
        prefs,
        games: Vec::new(),
        watcher: None,
        controllers: Vec::new(),
        panel: None,
        notice: None,
        layer_shell: window::wayland(),
        debug_window: false,
    });
    cx.set_global(OverlayGlobal(entity.clone()));
    debug::start(&entity, cx);
}

pub fn get(cx: &App) -> Option<Entity<OverlayService>> {
    cx.try_global::<OverlayGlobal>().map(|g| g.0.clone())
}

impl OverlayService {
    /// Settings changed them (already saved).
    pub fn set_prefs(&mut self, prefs: OverlayPrefs, cx: &mut Context<Self>) {
        self.prefs = prefs;
        if let Some(w) = &self.watcher {
            w.set_options(options(&prefs));
        }
        if !prefs.enabled {
            self.close_panel(cx);
        }
        self.sync_watcher(cx);
        cx.notify();
    }

    /// Every launch event, from the hub.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Running { app_name, pid } => {
                match self.games.iter_mut().find(|g| &g.app_name == app_name) {
                    Some(g) => g.pid = Some(*pid),
                    None => self.games.push(RunningGame { app_name: app_name.clone(), pid: Some(*pid), started: Instant::now() }),
                }
                self.sync_watcher(cx);
            }
            LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => {
                self.games.retain(|g| &g.app_name != app_name);
                if self.games.is_empty() {
                    self.close_panel(cx);
                }
                self.sync_watcher(cx);
            }
            LaunchEvent::Launching { .. } | LaunchEvent::Preparing { .. } | LaunchEvent::Status { .. } => {}
        }
    }

    /// The game the panel is about: the one started last.
    pub fn current(&self) -> Option<GameNow> {
        let game = self.games.last()?.clone();
        let info = self.runtime.session_info(&game.app_name);
        Some(GameNow { game, info })
    }

    /// The input thread runs while a game runs and the overlay is on.
    fn sync_watcher(&mut self, cx: &mut Context<Self>) {
        let wanted = self.prefs.enabled && !self.games.is_empty();
        if wanted == self.watcher.is_some() {
            return;
        }
        if !wanted {
            // Dropping it stops the thread and releases every grab.
            self.watcher = None;
            self.controllers.clear();
            return;
        }
        let (tx, mut rx) = unbounded();
        match Watcher::start(options(&self.prefs), move |ev| {
            let _ = tx.unbounded_send(ev);
        }) {
            Ok(w) => self.watcher = Some(w),
            Err(e) => {
                log::warn!("overlay: cannot read controllers: {e}");
                return;
            }
        }
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                if this.update(cx, |this, cx| this.on_input(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_input(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Open => self.open_panel(cx),
            Event::Close => self.close_panel(cx),
            Event::Screenshot => self.screenshot(cx),
            Event::Volume(dir) => self.step_volume(dir, cx),
            Event::Press(b) => {
                if let Some(panel) = self.panel {
                    let pad = panel::pad_of(b);
                    // The compositor may have closed the surface: let go of the controllers.
                    if panel.update(cx, |p, window, cx| p.on_pad(pad, window, cx)).is_err() {
                        self.close_panel(cx);
                    }
                }
            }
            Event::Controllers(list) => {
                self.controllers = list;
                if let Some(panel) = self.panel {
                    let list = self.controllers.clone();
                    let _ = panel.update(cx, |p, _, cx| p.set_controllers(list, cx));
                }
                cx.notify();
            }
        }
    }

    fn open_panel(&mut self, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            return;
        }
        let Some(game) = self.current() else {
            self.release_input();
            return;
        };
        if !self.layer_shell && !self.debug_window {
            log::info!("overlay: this desktop cannot show it (no layer-shell)");
            self.release_input();
            return;
        }
        let service = cx.entity().downgrade();
        let controllers = self.controllers.clone();
        let (paths, step) = (self.data_paths.clone(), self.prefs.volume_step);
        let layer = !self.debug_window;
        match window::open_panel(cx, self.prefs.side, layer, move |_, cx| {
            cx.new(|cx| OverlayPanel::new(service, paths, step, game, controllers, cx))
        }) {
            Ok(handle) => self.panel = Some(handle),
            Err(e) => {
                log::warn!("overlay: cannot open the panel: {e:#}");
                if window::is_layer_shell_missing(&e) {
                    self.layer_shell = false;
                    crate::system::notify::send(
                        tr!("The overlay cannot open"),
                        tr!("This desktop cannot draw over games (it lacks the layer-shell protocol). Hyprland, KDE Plasma and Sway can."),
                    );
                }
                self.release_input();
            }
        }
    }

    /// Closes the panel (and gives the controllers back to the game).
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.release_input();
        if let Some(handle) = self.panel.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn release_input(&self) {
        if let Some(w) = &self.watcher {
            w.close();
        }
    }

    /// Captures the screen: the panel goes away first so it is not in the picture.
    pub fn screenshot(&mut self, cx: &mut Context<Self>) {
        let was_open = self.panel.is_some();
        self.close_panel(cx);
        let Some(capture) = crate::services::session::capture_of(cx) else {
            self.notify(Notice::text(Icon::PhotoCamera, tr!("Screenshots are not available"), true), cx);
            return;
        };
        let title = self.current().map(|g| g.title()).unwrap_or_default();
        cx.spawn(async move |this, cx| {
            if was_open {
                cx.background_executor().timer(HIDE_BEFORE_CAPTURE).await;
            }
            let outcome = cx.background_spawn(async move { capture.capture_quiet() }).await;
            let notice = match outcome {
                CaptureOutcome::Saved(_) => Notice::text(Icon::PhotoCamera, tr!("Screenshot saved"), false).sub(title),
                CaptureOutcome::Skipped(SkipReason::FolderNotConfigured) => {
                    Notice::text(Icon::PhotoCamera, tr!("Choose a screenshots folder in Settings → Screenshots"), true)
                }
                CaptureOutcome::Skipped(SkipReason::Busy) => return,
                CaptureOutcome::Skipped(_) => Notice::text(Icon::PhotoCamera, tr!("No game to capture"), true),
                CaptureOutcome::Failed(e) => Notice::text(Icon::PhotoCamera, tr!("Screenshot failed"), true).sub(e),
            };
            this.update(cx, |this, cx| this.notify(notice, cx)).ok();
        })
        .detach();
    }

    /// One volume step up (`1`) or down (`-1`) of the default output.
    fn step_volume(&mut self, dir: i8, cx: &mut Context<Self>) {
        let delta = i32::from(dir) * i32::from(self.prefs.volume_step);
        cx.spawn(async move |this, cx| {
            let level = cx.background_spawn(async move { heronite_overlay::audio::step_volume(delta) }).await;
            let notice = match level {
                Some(v) => Notice::volume(v),
                None => Notice::text(Icon::VolumeUp, tr!("Volume control is not available"), true),
            };
            this.update(cx, |this, cx| this.notify(notice, cx)).ok();
        })
        .detach();
    }

    /// Shows a short notice in a corner of the screen (replacing the one shown).
    fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        if !self.layer_shell && !self.debug_window {
            log::info!("overlay notice: {}", notice.title);
            return;
        }
        let layer = !self.debug_window;
        self.show_notice(notice, layer, cx);
    }

    /// Ends the game the panel is about.
    pub fn stop_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        self.close_panel(cx);
        log::info!("overlay: stopping {app_name}");
        if !self.runtime.stop(app_name) {
            self.runtime.force_clear(app_name);
        }
    }

    /// Brings the main window up (the game keeps running).
    pub fn show_launcher(&mut self, cx: &mut Context<Self>) {
        self.close_panel(cx);
        crate::system::show_main_window(cx);
    }

    /// A game's status changed here: the Collection reloads.
    pub fn library_changed(&self, cx: &mut Context<Self>) {
        if let Some(session) = crate::services::session::get(cx) {
            session.update(cx, |_, cx| cx.emit(crate::services::session::SessionEvent::LibraryChanged));
        }
    }
}

fn options(prefs: &OverlayPrefs) -> Options {
    Options { screenshot: prefs.screenshot_shortcut, volume: prefs.volume_shortcut }
}
