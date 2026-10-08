//! Application lifecycle: the main window (created on demand and dropped when
//! it is closed to the tray), the game runtime shared by every window, launch
//! requests from the command line / protocol URLs / other instances / the
//! tray, and when the process quits.
//!
//! State lives in a global [`Hub`] (`Rc<RefCell<..>>`): borrow it only for
//! short scopes and never while calling into GPUI (opening a window builds
//! the Collection, which calls back into [`attach_runtime`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::{CollectionGame, HeroicPaths, Library};
use heronite_launch::LaunchEvent;

use super::cli::{Cli, LaunchRequest, RUNNERS};
use super::settings::SysSettings;
use super::single_instance::Message;
use super::tray::{self, TrayCmd, TrayHandle};
use crate::collection::CollectionView;
use crate::runtime::GameRuntime;

/// Process exit code (non-zero when a launch from outside failed).
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

pub fn exit_code() -> i32 {
    EXIT_CODE.load(Ordering::Relaxed)
}

struct MainWindow {
    handle: AnyWindowHandle,
    view: WeakEntity<CollectionView>,
}

struct State {
    heroic: HeroicPaths,
    runtime: Arc<GameRuntime>,
    window: Option<MainWindow>,
    /// The open Collection's channel for launch events.
    subscriber: Option<UnboundedSender<LaunchEvent>>,
    /// Games started by this process (pid once known).
    running: HashMap<String, Option<u32>>,
    tray: Option<TrayHandle>,
    /// Quit when the last game ends and no window was shown (started only to
    /// launch a game without UI).
    exit_when_idle: bool,
    /// The next window close is ours (tray / minimize on launch): stay alive.
    hiding: bool,
    fake_launch: bool,
    settings: SysSettings,
    settings_stamp: Option<SystemTime>,
}

impl State {
    /// Settings, re-read only when the file changed (a `stat` per call).
    fn settings(&mut self) -> SysSettings {
        let stamp = heronite_data::game_config::global_config_path(&self.heroic)
            .metadata()
            .and_then(|m| m.modified())
            .ok();
        if stamp != self.settings_stamp {
            self.settings = SysSettings::load(&self.heroic);
            self.settings_stamp = stamp;
        }
        self.settings
    }
}

#[derive(Clone)]
pub struct Hub(Rc<RefCell<State>>);
impl Global for Hub {}

fn hub(cx: &App) -> Hub {
    cx.global::<Hub>().clone()
}

/// Called by `CollectionView::new`: the shared runtime plus a fresh stream of
/// launch events (games already running are announced first).
pub fn attach_runtime(heroic: &HeroicPaths, cx: &mut App) -> (Arc<GameRuntime>, UnboundedReceiver<LaunchEvent>) {
    let Some(hub) = cx.try_global::<Hub>().cloned() else {
        let (runtime, rx) = GameRuntime::new(heroic.clone());
        return (Arc::new(runtime), rx);
    };
    let (tx, rx) = unbounded();
    let mut s = hub.0.borrow_mut();
    for (app_name, pid) in &s.running {
        let _ = tx.unbounded_send(LaunchEvent::Launching { app_name: app_name.clone() });
        if let Some(pid) = pid {
            let _ = tx.unbounded_send(LaunchEvent::Running { app_name: app_name.clone(), pid: *pid });
        }
    }
    s.subscriber = Some(tx);
    (s.runtime.clone(), rx)
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| !v.is_empty() && v != "0").unwrap_or(false)
}

/// Sets everything up and opens the first window (unless the invocation or
/// the settings ask for none).
pub fn init(cx: &mut App, cli: Cli, started: Instant, messages: Option<UnboundedReceiver<Message>>) {
    let heroic = HeroicPaths::detect();
    let settings = SysSettings::load(&heroic);
    let (runtime, events) = GameRuntime::new(heroic.clone());
    let runtime = Arc::new(runtime);
    // Post-game work (backup, capture session) runs here, with or without a window.
    crate::services::session::init(cx, heroic.clone(), runtime.clone());

    let (tray_tx, tray_rx) = unbounded();
    let tray = (!settings.no_tray_icon && !env_flag("HERONITE_NO_TRAY"))
        .then(|| tray::spawn(heroic.clone(), settings.dark_tray_icon, settings.exit_to_tray, settings.max_recent, false, tray_tx.clone()))
        .flatten();
    let tray_alive = tray.is_some();

    let debug_hidden = env_flag("HERONITE_DEBUG_START_HIDDEN");
    let hide = cli.hide_window(settings.hide_window_on_protocol_launch);
    let start_in_tray = (settings.start_in_tray && tray_alive) || debug_hidden;
    let want_window = !(hide || start_in_tray);
    // Nothing would keep the app reachable: show the window after all.
    let unreachable = !want_window && !tray_alive && cli.launch.is_none() && !debug_hidden;

    cx.set_global(Hub(Rc::new(RefCell::new(State {
        heroic: heroic.clone(),
        runtime,
        window: None,
        subscriber: None,
        running: HashMap::new(),
        tray,
        exit_when_idle: cli.launch.is_some() && !want_window && !start_in_tray,
        hiding: false,
        fake_launch: env_flag("HERONITE_DEBUG_FAKE_LAUNCH"),
        settings,
        settings_stamp: heronite_data::game_config::global_config_path(&heroic).metadata().and_then(|m| m.modified()).ok(),
    }))));
    STARTED.with(|s| s.set(Some(started)));

    // Launch events from the runtime.
    cx.spawn(async move |cx| {
        let mut events = events;
        while let Some(ev) = events.next().await {
            cx.update(|cx| on_event(cx, ev));
        }
    })
    .detach();

    // Tray clicks.
    cx.spawn(async move |cx| {
        let mut rx = tray_rx;
        while let Some(cmd) = rx.next().await {
            cx.update(|cx| on_tray_cmd(cx, cmd));
        }
    })
    .detach();

    // Requests from second invocations.
    if let Some(mut rx) = messages {
        cx.spawn(async move |cx| {
            while let Some(msg) = rx.next().await {
                cx.update(|cx| on_message(cx, msg));
            }
        })
        .detach();
    }

    cx.on_window_closed(|cx, id| on_window_closed(cx, id)).detach();

    // The Epic service (download queue, startup update check, auto-updates)
    // belongs to the app, not to the window. A one-shot launch has no use for it.
    if !hub(cx).0.borrow().exit_when_idle {
        crate::services::downloads(cx);
    }

    if want_window || unreachable {
        show_window(cx);
    }
    if let Some(req) = cli.launch {
        launch(cx, req, hide, false);
    }
    debug_hooks(cx, tray_tx);
}

thread_local! {
    static STARTED: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// `HERONITE_DEBUG_*` helpers to exercise the window lifecycle without a
/// mouse: `SHOW_AFTER_MS`, `CLOSE_AFTER_MS` (as if the user closed it),
/// `QUIT_AFTER_MS`, `TRAY_CMDS=show@500,toggle@1500,launch:<app>@2000,quit@3000`.
fn debug_hooks(cx: &mut App, tray_tx: UnboundedSender<TrayCmd>) {
    let after = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
    if let Some(ms) = after("HERONITE_DEBUG_SHOW_AFTER_MS") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            cx.update(|cx| {
                log::info!("[debug] showing the window");
                show_window(cx);
            });
        })
        .detach();
    }
    if let Some(ms) = after("HERONITE_DEBUG_CLOSE_AFTER_MS") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            cx.update(|cx| {
                // Like the user closing the window: `exitToTray` decides whether the app stays.
                log::info!("[debug] closing the window");
                let handle = hub(cx).0.borrow().window.as_ref().map(|w| w.handle);
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            });
        })
        .detach();
    }
    if let Some(ms) = after("HERONITE_DEBUG_QUIT_AFTER_MS") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            cx.update(|cx| quit(cx));
        })
        .detach();
    }
    if let Ok(list) = std::env::var("HERONITE_DEBUG_TRAY_CMDS") {
        for item in list.split(',').filter(|s| !s.is_empty()) {
            let Some((cmd, ms)) = item.split_once('@') else { continue };
            let Ok(ms) = ms.parse::<u64>() else { continue };
            let cmd = match cmd {
                "show" => TrayCmd::Show,
                "toggle" => TrayCmd::Toggle,
                "quit" => TrayCmd::Quit,
                other => match other.strip_prefix("launch:") {
                    Some(app) => TrayCmd::Launch(app.to_string()),
                    None => continue,
                },
            };
            let tx = tray_tx.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                let _ = tx.unbounded_send(cmd);
            })
            .detach();
        }
    }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

fn open_main_window(cx: &mut App, started: Instant) -> Option<MainWindow> {
    let bounds = Bounds::centered(None, size(px(1600.), px(1000.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        app_id: Some("com.heronite.rs".into()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(640.), px(480.))),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Heronite");
        cx.new(|cx| CollectionView::new(started, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.activate(true);
            Some(MainWindow { handle, view: view.downgrade() })
        }
        Err(err) => {
            log::error!("failed to open the window: {err:#}");
            None
        }
    }
}

/// Opens the main window, or brings the open one to the front. Returns the
/// Collection view.
pub fn show_window(cx: &mut App) -> Option<WeakEntity<CollectionView>> {
    let hub = hub(cx);
    let existing = hub.0.borrow().window.as_ref().map(|w| (w.handle, w.view.clone()));
    if let Some((handle, view)) = existing {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            hub.0.borrow_mut().exit_when_idle = false;
            return Some(view);
        }
    }
    let started = STARTED.with(|s| s.take()).unwrap_or_else(Instant::now);
    let window = open_main_window(cx, started)?;
    let view = window.view.clone();
    let mut s = hub.0.borrow_mut();
    s.window = Some(window);
    s.exit_when_idle = false;
    if let Some(t) = &s.tray {
        t.set_window_open(true);
    }
    log::info!("main window opened");
    Some(view)
}

/// Closes the window but keeps the app alive (needs the tray). Returns whether
/// a window was closed.
fn close_window_keep_alive(cx: &mut App) -> bool {
    let hub = hub(cx);
    let handle = {
        let mut s = hub.0.borrow_mut();
        if s.tray.is_none() {
            return false;
        }
        let Some(handle) = s.window.as_ref().map(|w| w.handle) else { return false };
        s.hiding = true;
        handle
    };
    if handle.update(cx, |_, window, _| window.remove_window()).is_err() {
        hub.0.borrow_mut().hiding = false;
        return false;
    }
    true
}

fn on_window_closed(cx: &mut App, id: WindowId) {
    let hub = hub(cx);
    let keep = {
        let mut s = hub.0.borrow_mut();
        if !s.window.as_ref().is_some_and(|w| w.handle.window_id() == id) {
            return;
        }
        s.window = None;
        s.subscriber = None;
        let hiding = std::mem::take(&mut s.hiding);
        let settings = s.settings();
        if let Some(t) = &s.tray {
            t.set_window_open(false);
        }
        s.tray.is_some() && (hiding || settings.exit_to_tray)
    };
    if keep {
        log::info!("window closed, staying in the tray");
        // Nothing left to wait for when the last game already ended.
        maybe_exit_idle(cx);
    } else {
        log::info!("window closed, quitting");
        quit(cx);
    }
}

pub fn quit(cx: &mut App) {
    let hub = hub(cx);
    if let Some(t) = hub.0.borrow_mut().tray.take() {
        t.shutdown();
    }
    cx.quit();
}

/// Whether the main window exists (otherwise results go to desktop
/// notifications).
pub fn has_window(cx: &App) -> bool {
    cx.try_global::<Hub>().is_some_and(|h| h.0.borrow().window.is_some())
}

/// The post-game work of a session finished: a headless launch can exit now.
pub fn post_game_idle(cx: &mut App) {
    maybe_exit_idle(cx);
}

fn maybe_exit_idle(cx: &mut App) {
    let hub = hub(cx);
    let idle = {
        let s = hub.0.borrow();
        s.exit_when_idle && s.window.is_none() && s.running.is_empty() && !crate::services::session::busy()
    };
    if idle {
        log::info!("game ended and no window was shown: exiting");
        quit(cx);
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

fn on_event(cx: &mut App, ev: LaunchEvent) {
    let hub = hub(cx);
    let (launching, ended, settings) = {
        let mut s = hub.0.borrow_mut();
        match &ev {
            LaunchEvent::Launching { app_name } => {
                s.running.entry(app_name.clone()).or_insert(None);
            }
            LaunchEvent::Running { app_name, pid } => {
                s.running.insert(app_name.clone(), Some(*pid));
            }
            LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => {
                s.running.remove(app_name);
            }
            LaunchEvent::Status { message, warning, .. } => {
                if s.window.is_none() {
                    if *warning {
                        log::warn!("{message}");
                    } else {
                        log::info!("{message}");
                    }
                }
            }
        }
        let delivered = s.subscriber.as_ref().is_some_and(|tx| tx.unbounded_send(ev.clone()).is_ok());
        let ended = matches!(ev, LaunchEvent::Exited { .. } | LaunchEvent::Error { .. });
        if !delivered {
            s.subscriber = None;
            if let LaunchEvent::Error { message, .. } = &ev {
                log::error!("{message}");
            }
        }
        (matches!(ev, LaunchEvent::Launching { .. }), ended, s.settings())
    };
    // Window-independent consumers: post-game work and the Epic service.
    if let Some(session) = crate::services::session::get(cx) {
        session.update(cx, |s, cx| s.on_launch_event(&ev, cx));
    }
    tell_downloads(cx, &ev);
    if !has_window(cx) {
        notify_without_window(&ev);
    }
    if launching && settings.minimize_on_launch {
        // Heroic's `minimizeOnLaunch`: needs the tray to come back from.
        close_window_keep_alive(cx);
    }
    if ended {
        maybe_exit_idle(cx);
    }
}

/// The Epic service must not update a game while it is played (it queues the
/// update once the game exits).
fn tell_downloads(cx: &mut App, ev: &LaunchEvent) {
    let Some(downloads) = crate::services::try_downloads(cx) else { return };
    let (app_name, running) = match ev {
        LaunchEvent::Launching { app_name } | LaunchEvent::Running { app_name, .. } => (app_name, true),
        LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => (app_name, false),
        LaunchEvent::Status { .. } => return,
    };
    downloads.update(cx, |d, cx| d.game_running(app_name, running, cx));
}

/// With the window closed to the tray nobody sees toasts: problems of a
/// launch (it failed, cloud saves could not be synced) become notifications.
fn notify_without_window(ev: &LaunchEvent) {
    match ev {
        LaunchEvent::Error { message, .. } => super::notify::send("Heronite", message.clone()),
        LaunchEvent::Status { message, warning: true, .. } => super::notify::send("Heronite", message.clone()),
        _ => {}
    }
}

fn on_tray_cmd(cx: &mut App, cmd: TrayCmd) {
    log::info!("tray: {cmd:?}");
    match cmd {
        TrayCmd::Show => {
            show_window(cx);
        }
        TrayCmd::Toggle => {
            let open = hub(cx).0.borrow().window.is_some();
            if open {
                close_window_keep_alive(cx);
            } else {
                show_window(cx);
            }
        }
        TrayCmd::Quit => quit(cx),
        TrayCmd::Launch(app_name) => launch(cx, LaunchRequest { app_name, ..Default::default() }, true, false),
    }
}

/// A second invocation reached this instance.
fn on_message(cx: &mut App, msg: Message) {
    log::info!("request from another invocation: {msg:?}");
    let hide_setting = hub(cx).0.borrow_mut().settings().hide_window_on_protocol_launch;
    let cli = Cli { launch: msg.launch.clone(), no_gui: msg.no_gui, ..Default::default() };
    let hide = cli.hide_window(hide_setting);
    match msg.launch {
        // Heroic's `second-instance`: show the window unless the launch hides it.
        None => {
            show_window(cx);
        }
        Some(req) => launch(cx, req, hide, true),
    }
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Keys of the library entries a request may mean, in search order.
pub fn candidate_keys(req: &LaunchRequest) -> Vec<String> {
    match &req.runner {
        Some(r) => vec![format!("{r}_{}", req.app_name)],
        None => RUNNERS.iter().map(|r| format!("{r}_{}", req.app_name)).collect(),
    }
}

fn find_game(lib: &Library, req: &LaunchRequest) -> Option<Arc<CollectionGame>> {
    candidate_keys(req).iter().find_map(|k| lib.get(k)).cloned()
}

/// Launches `req`. `hide`: keep the window out of it (headless launch);
/// `close_visible`: with `hide`, also close an open window (Heroic hides it)
/// when there is a tray to come back from.
pub fn launch(cx: &mut App, req: LaunchRequest, hide: bool, close_visible: bool) {
    log::info!("launch request: {req:?} hide={hide}");
    if !req.args.is_empty() || req.alt_exe.is_some() {
        log::warn!("launch arguments / altExe from the URL are not supported yet and are ignored");
    }
    if hide {
        if close_visible && close_window_keep_alive(cx) {
            log::info!("window hidden for the launch");
        }
        launch_headless(cx, req);
    } else if let Some(view) = show_window(cx) {
        let _ = view.update(cx, |v, cx| v.system_launch(req, cx));
    }
}

fn launch_headless(cx: &mut App, req: LaunchRequest) {
    let heroic = hub(cx).0.borrow().heroic.clone();
    cx.spawn(async move |cx| {
        let (h, r) = (heroic, req.clone());
        let found = cx
            .background_spawn(async move {
                Library::load(&h).map(|lib| find_game(&lib, &r)).map_err(|e| format!("could not read the library: {e:#}"))
            })
            .await;
        cx.update(|cx| on_headless_game(cx, req, found));
    })
    .detach();
}

fn fail(cx: &mut App, message: String) {
    log::error!("{message}");
    EXIT_CODE.store(1, Ordering::Relaxed);
    let hub = hub(cx);
    let view = hub.0.borrow().window.as_ref().map(|w| w.view.clone());
    if let Some(view) = view {
        let _ = view.update(cx, |v, cx| v.system_toast(message, true, cx));
    }
    maybe_exit_idle(cx);
}

fn on_headless_game(cx: &mut App, req: LaunchRequest, found: Result<Option<Arc<CollectionGame>>, String>) {
    let game = match found {
        Err(e) => return fail(cx, e),
        Ok(None) => return fail(cx, format!("Could not find the game {} in the library", req.app_name)),
        Ok(Some(g)) => g,
    };
    let hub = hub(cx);
    if !game.installed {
        // Heroic asks to install it (and shows the window for that).
        log::info!("{} is not installed: showing the window", game.title);
        if let Some(view) = show_window(cx) {
            let _ = view.update(cx, |v, cx| v.system_launch(req, cx));
        }
        return;
    }
    let (runtime, fake, running) = {
        let s = hub.0.borrow();
        (s.runtime.clone(), s.fake_launch, s.running.contains_key(&game.app_name))
    };
    if running {
        log::info!("{} is already running", game.title);
        return;
    }
    if fake {
        log::info!("[fake-launch] headless {} ({}/{})", game.title, game.runner, game.app_name);
        maybe_exit_idle(cx);
        return;
    }
    // `--no-gui` skips legendary's version check (Heroic does the same).
    let result = if game.runner == "legendary" { runtime.play_skip_version_check(&game) } else { runtime.play(&game) };
    match result {
        Ok(()) => {
            hub.0.borrow_mut().running.entry(game.app_name.clone()).or_insert(None);
        }
        Err(e) => fail(cx, format!("Could not launch {}: {e}", game.title)),
    }
}

#[cfg(test)]
mod tests {
    use super::{candidate_keys, LaunchRequest};

    #[test]
    fn candidate_keys_follow_runner_order() {
        let r = LaunchRequest { runner: Some("gog".into()), app_name: "123".into(), ..Default::default() };
        assert_eq!(candidate_keys(&r), vec!["gog_123"]);
        let r = LaunchRequest { runner: None, app_name: "Quail".into(), ..Default::default() };
        assert_eq!(
            candidate_keys(&r),
            vec!["legendary_Quail", "gog_Quail", "nile_Quail", "sideload_Quail", "zoom_Quail"]
        );
    }
}
