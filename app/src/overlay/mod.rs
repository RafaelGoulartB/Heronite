//! The in-game overlay (off by default; Settings → Overlay): a panel
//! the controller's Guide button (or a keyboard shortcut, Super + G unless
//! changed) opens over a game launched by the app, plus two shortcuts that
//! work without opening it (Guide + RB, or the button chosen: screenshot;
//! Guide + ↑ / ↓: volume).
//!
//! The service follows the launch events (from the hub, like the post-game
//! work), so it works with the main window closed to the tray. While a game
//! runs and the overlay is on, `heronite_overlay::input` reads the
//! controllers on its own thread. Linux grabs them while the panel is open
//! or Guide is held. Windows uses Back+Start as Guide through XInput, which
//! has no grab: the open panel takes the foreground instead, and most games
//! stop reading the controller in the background.
//!
//! Settings can let it open with no game running too ("without a game"):
//! then the input thread and the shortcut run while the app does, and the
//! panel shows what has no game (volume, controllers, Heronite).
//!
//! Guide + Y marks a moment in the game's journal (`moment`); the panel's
//! Notes tab writes it. Guide + X saves the replay (the last seconds of the
//! game, `services::replay`) and marks it as a moment too (`replay`).
//!
//! The keyboard shortcut is registered with the desktop only while such a
//! game runs (or always, without a game) (a temporary Hyprland bind, else the GlobalShortcuts portal,
//! as the screenshot shortcut). Opened from the keyboard, the panel takes
//! the keyboard until it closes, so it can be driven without a controller.
//!
//! Windows uses native topmost tool windows over borderless/windowed games,
//! not exclusive fullscreen. Its keyboard default is Ctrl+Shift+G; the panel
//! takes the foreground however it opened, notices never activate. Linux uses Wayland layer-shell surfaces ([`window`]);
//! desktops without layer-shell (GNOME, X11) cannot show them.

mod debug;
pub mod labels;
mod moment;
pub mod panel;
mod replay;
mod toast;
mod window;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_achievements::prefs::NoticeFields;
use heronite_data::overlay_prefs::OverlayPrefs;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_launch::live_fps::LiveFps;
use heronite_launch::LaunchEvent;
use heronite_media::capture::{CaptureOutcome, GlobalHotkey, HotkeySpec, ShortcutBackend, SkipReason};
use heronite_overlay::input::{Controller, Event, Options, Watcher};

use self::panel::OverlayPanel;
use self::toast::Notice;
pub use self::toast::{made_up_unlocks, Unlock};
use crate::icons::Icon;
use crate::runtime::{GameRuntime, SessionInfo};

/// Time for the compositor to take the panel off screen before a capture.
const HIDE_BEFORE_CAPTURE: Duration = Duration::from_millis(300);
/// Size of an achievement's icon in its notice (design px).
const ICON_PX: f32 = 44.;
/// An achievement's notice waits this long at most for its icon.
const ICON_WAIT: Duration = Duration::from_millis(1500);
/// Windows: how long after the panel opens it checks that it still has the
/// foreground ([`heronite_platform::overlay_window::FocusReturn::hold`]).
#[cfg(windows)]
const FOREGROUND_CHECK: Duration = Duration::from_millis(250);

/// A game launched by the app that is running.
#[derive(Clone, Debug)]
pub struct RunningGame {
    pub app_name: String,
    /// Process (group leader) once known.
    pub pid: Option<u32>,
    pub started: Instant,
}

/// The keyboard shortcut, for Settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum HotkeyState {
    /// Not registered: off, or no game running.
    #[default]
    Idle,
    Registering,
    Ready(ShortcutBackend),
    Failed(String),
}

/// What the panel shows about the game it is open over.
#[derive(Clone, Debug)]
pub struct GameNow {
    pub game: RunningGame,
    pub info: Option<SessionInfo>,
    /// How its FPS limit can change now, and the limit it has (`0` = none).
    pub fps: Option<(LiveFps, u32)>,
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
    #[cfg(windows)]
    focus_return: Option<heronite_platform::overlay_window::FocusReturn>,
    /// When the panel opened (a replay saved from it cuts that time off).
    panel_opened: Option<Instant>,
    notice: Option<toast::NoticeWindow>,
    /// Achievement notices waiting for the screen.
    achievement_queue: toast::Queue,
    /// `false` once the compositor refused a layer-shell surface.
    layer_shell: bool,
    /// Debug: draw the panel in a normal window (`HERONITE_DEBUG_OVERLAY=window`).
    debug_window: bool,
    /// `HERONITE_DEBUG_OVERLAY_FPS`: a limiter the debug game pretends to have.
    debug_live_fps: Option<LiveFps>,
    /// FPS limits set from the panel, by game, while it runs.
    fps_limits: HashMap<String, u32>,
    /// The keyboard shortcut registered (and its accelerator) while a game runs.
    hotkey: Option<(String, GlobalHotkey)>,
    hotkey_state: HotkeyState,
    /// Bumped per registration, so a late answer of an old one is ignored.
    hotkey_epoch: u64,
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
        #[cfg(windows)]
        focus_return: None,
        panel_opened: None,
        notice: None,
        achievement_queue: toast::Queue::default(),
        layer_shell: window::supported(),
        debug_window: false,
        debug_live_fps: None,
        fps_limits: HashMap::new(),
        hotkey: None,
        hotkey_state: HotkeyState::Idle,
        hotkey_epoch: 0,
    });
    cx.set_global(OverlayGlobal(entity.clone()));
    // Without a game it is ready from the start.
    entity.update(cx, |s, cx| s.sync_watcher(cx));
    debug::start(&entity, cx);
}

/// Whether this desktop can draw the overlay over games: it needs Wayland's
/// layer-shell, which GNOME does not have.
pub fn desktop_status() -> (String, bool) {
    if cfg!(windows) {
        return (tr!("Supported over windowed and borderless games. Exclusive fullscreen is not supported.").into(), true);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return (tr!("Not supported on X11: the overlay needs a Wayland session.").into(), false);
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop.to_ascii_lowercase().contains("gnome") {
        return (tr!("GNOME cannot draw over games (it lacks the layer-shell protocol).").into(), false);
    }
    (tr!("Supported (Wayland with layer-shell, like Hyprland, KDE Plasma and Sway).").into(), true)
}

pub fn get(cx: &App) -> Option<Entity<OverlayService>> {
    cx.try_global::<OverlayGlobal>().map(|g| g.0.clone())
}

/// The app is quitting: the keyboard shortcut, for the caller to drop off
/// the UI thread (that removes it from the desktop; a Hyprland bind would
/// otherwise outlive the app).
pub fn take_hotkey(cx: &mut App) -> Option<GlobalHotkey> {
    let service = get(cx)?;
    service.update(cx, |s, _| {
        s.hotkey_epoch += 1;
        s.hotkey_state = HotkeyState::Idle;
        s.hotkey.take().map(|(_, h)| h)
    })
}

impl OverlayService {
    /// Settings changed them (already saved).
    pub fn set_prefs(&mut self, prefs: OverlayPrefs, cx: &mut Context<Self>) {
        if let Some(w) = &self.watcher {
            w.set_options(options(&prefs));
        }
        let enabled = prefs.enabled;
        self.prefs = prefs;
        if !enabled {
            self.close_panel(cx);
            self.achievement_queue.clear();
        }
        self.sync_watcher(cx);
        cx.notify();
    }

    pub fn hotkey_state(&self) -> &HotkeyState {
        &self.hotkey_state
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
                self.fps_limits.remove(app_name);
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
        let live = self.debug_live_fps.clone().or_else(|| self.runtime.live_fps(&game.app_name));
        // gamescope's limit is set with xprop.
        let live = live.filter(|l| !l.is_gamescope() || heronite_overlay::gamescope::available());
        let fps = live.map(|l| {
            let limit = self.fps_limits.get(&game.app_name).copied().unwrap_or_else(|| l.limit());
            (l, limit)
        });
        Some(GameNow { game, info, fps })
    }

    /// The panel changed the running game's FPS limit (kept for this session).
    pub fn fps_limit_changed(&mut self, app_name: &str, limit: u32) {
        self.fps_limits.insert(app_name.to_string(), limit);
    }

    /// Whether it can open now: on, and a game runs (or it may open without).
    fn active(&self) -> bool {
        self.prefs.enabled && (!self.games.is_empty() || self.prefs.without_game)
    }

    /// The input thread runs, and the keyboard shortcut is registered,
    /// while it can open.
    fn sync_watcher(&mut self, cx: &mut Context<Self>) {
        self.sync_hotkey(cx);
        let wanted = self.active();
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

    fn sync_hotkey(&mut self, cx: &mut Context<Self>) {
        let p = &self.prefs;
        let wanted =
            (self.active() && p.keyboard_shortcut && (self.layer_shell || self.debug_window)).then(|| p.keyboard_accelerator.clone());
        if wanted.as_deref() == self.hotkey.as_ref().map(|(a, _)| a.as_str()) {
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
        log::info!("overlay: registering the keyboard shortcut {accelerator}");
        let (press_tx, mut presses) = unbounded();
        let (ready_tx, mut ready) = unbounded();
        let spec = HotkeySpec {
            id: "overlay",
            description: "Heronite overlay",
            #[cfg(target_os = "linux")]
            signal: signal_hook::consts::SIGUSR1,
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
        let epoch = self.hotkey_epoch;
        cx.spawn(async move |this, cx| {
            if let Some(result) = ready.next().await {
                let current = this
                    .update(cx, |this, cx| {
                        if this.hotkey_epoch != epoch {
                            return false;
                        }
                        this.hotkey_state = match result {
                            Ok(backend) => HotkeyState::Ready(backend),
                            Err(e) => {
                                log::warn!("overlay: keyboard shortcut: {e}");
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
                let current = this
                    .update(cx, |this, cx| {
                        if this.hotkey_epoch == epoch {
                            this.toggle_from_keyboard(cx);
                        }
                        this.hotkey_epoch == epoch
                    })
                    .unwrap_or(false);
                if !current {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// The keyboard shortcut: opens the panel (taking the keyboard) or closes it.
    fn toggle_from_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            self.close_panel(cx);
            return;
        }
        // The controllers are held as with Guide (and Guide closes it).
        if let Some(w) = &self.watcher {
            w.open();
        }
        self.open_panel_with(true, cx);
    }

    fn on_input(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Open => self.open_panel(cx),
            Event::Close => self.close_panel(cx),
            Event::Screenshot => self.screenshot(cx),
            Event::Volume(dir) => self.step_volume(dir, cx),
            Event::Moment => self.mark_moment(cx),
            Event::Replay => self.save_replay(cx),
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
        self.open_panel_with(false, cx);
    }

    /// `keyboard`: opened from the keyboard, the panel takes it while open.
    fn open_panel_with(&mut self, keyboard: bool, cx: &mut Context<Self>) {
        if self.panel.is_some() {
            return;
        }
        let game = self.current();
        if game.is_none() && !self.prefs.without_game {
            self.release_input();
            return;
        }
        if !self.layer_shell && !self.debug_window {
            log::info!("overlay: this desktop cannot show it (no layer-shell)");
            self.release_input();
            return;
        }
        let layer = !self.debug_window;
        if layer && window::exclusive_fullscreen() {
            self.release_input();
            return;
        }
        let service = cx.entity().downgrade();
        let controllers = self.controllers.clone();
        let (paths, step) = (self.data_paths.clone(), self.prefs.volume_step);
        let display = window::Display::current(cx);
        let scale = display.scale();
        match window::open_panel(cx, display, self.prefs.side, layer, keyboard, move |window, cx| {
            #[cfg(windows)]
            {
                let closing = service.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    if let Some(service) = closing.upgrade() {
                        cx.defer(move |cx| service.update(cx, |s, cx| s.close_panel(cx)));
                    }
                    false
                });
            }
            cx.new(|cx| OverlayPanel::new(service, paths, step, game, controllers, keyboard, scale, window, cx))
        }) {
            Ok(opened) => {
                self.panel = Some(opened.handle);
                #[cfg(windows)]
                if let Some(shown) = opened.shown {
                    let handle = opened.handle;
                    cx.spawn(async move |this, cx| match shown.await {
                        Ok(mut focus) => {
                            let kept = this
                                .update(cx, |s, _| {
                                    if s.panel != Some(handle) {
                                        return false;
                                    }
                                    s.focus_return = focus.take();
                                    s.focus_return.is_some()
                                })
                                .unwrap_or(false);
                            drop(focus);
                            // Something can take the foreground back right after
                            // (the main window, when the shortcut was pressed
                            // in it): once more, the keys then reach the panel.
                            if kept {
                                cx.background_executor().timer(FOREGROUND_CHECK).await;
                                this.update(cx, |s, _| {
                                    if let Some(focus) = s.focus_return.as_ref().filter(|_| s.panel == Some(handle)) {
                                        focus.hold();
                                    }
                                })
                                .ok();
                            }
                        }
                        Err(error) => {
                            log::warn!("overlay: cannot show the native panel: {error:#}");
                            this.update(cx, |s, cx| {
                                if s.panel == Some(handle) {
                                    s.close_panel(cx);
                                }
                            })
                            .ok();
                        }
                    })
                    .detach();
                }
                self.panel_opened = Some(Instant::now());
            }
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
        self.panel_opened = None;
        if let Some(handle) = self.panel.take() {
            #[cfg(windows)]
            {
                let focus = self.focus_return.take();
                // Focus restoration also sends native messages; do it outside
                // the App borrow, before GPUI destroys the panel's HWND.
                cx.spawn(async move |_, cx| {
                    drop(focus);
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                })
                .detach();
            }
            #[cfg(not(windows))]
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

    /// Whether notices can be drawn (a layer-shell surface, or the debug window).
    fn can_notify(&self) -> bool {
        self.layer_shell || self.debug_window
    }

    /// Shows a short notice in a corner of the screen (replacing the one shown).
    fn notify(&mut self, notice: Notice, cx: &mut Context<Self>) {
        if !self.can_notify() {
            log::info!("overlay notice: {}", notice.title);
            return;
        }
        self.show_notice(notice, !self.debug_window, cx);
    }

    /// Shows a new achievement over the game, after the ones already
    /// waiting, with the lines of `fields` its source gives. Its icon (a URL
    /// or a file; may be empty) is loaded first, for at most a moment: the
    /// notice never waits long for it. `false` when the overlay is off,
    /// cannot draw here or the game is in exclusive fullscreen (Windows),
    /// for the caller to notify the desktop instead.
    pub fn show_achievement(&mut self, unlock: Unlock, fields: NoticeFields, cx: &mut Context<Self>) -> bool {
        // The debug window shows it whatever the saved options say.
        if !(self.prefs.enabled || self.debug_window) || !self.can_notify() {
            return false;
        }
        let layer = !self.debug_window;
        if layer && window::exclusive_fullscreen() {
            return false;
        }
        let opts = heronite_images::Opts::new(self.data_paths.images_cache_dir(), Some(AppPaths::for_data(&self.data_paths).thumbs_dir()));
        let side = (ICON_PX * window::Display::current(cx).scale()).round() as u32;
        let ticket = self.achievement_queue.ticket();
        let icon_url = unlock.achievement.icon.clone();
        cx.spawn(async move |this, cx| {
            let image = if icon_url.is_empty() {
                None
            } else {
                let load = cx.background_spawn(async move {
                    let chain = if icon_url.starts_with("http") {
                        heronite_images::ImageRef::Url(icon_url)
                    } else {
                        heronite_images::ImageRef::File(icon_url.into())
                    };
                    heronite_images::load_cover(&[chain], side, side, &opts).ok().map(|decoded| {
                        let frame = image::Frame::new(decoded.into_image_buffer());
                        Arc::new(RenderImage::new(smallvec::smallvec![frame]))
                    })
                });
                let timeout = cx.background_executor().timer(ICON_WAIT);
                match futures::future::select(load, timeout).await {
                    futures::future::Either::Left((image, _)) => image,
                    futures::future::Either::Right(_) => None,
                }
            };
            this.update(cx, |this, cx| this.queue_achievement(ticket, Notice::achievement(&unlock, fields, image), layer, cx)).ok();
        })
        .detach();
        true
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
    Options {
        screenshot: prefs.screenshot_shortcut,
        screenshot_button: prefs.screenshot_button.into(),
        volume: prefs.volume_shortcut,
        moment: prefs.moment_shortcut,
        replay: prefs.replay_shortcut && heronite_platform::supported(heronite_platform::Feature::Replay),
    }
}
