//! Application lifecycle: the main window (created on demand and dropped when
//! it is closed to the tray), the game runtime shared by every window, launch
//! requests from the command line / protocol URLs / other instances / the
//! tray, and when the process quits.
//!
//! Quitting has two parts. A *request* (window closed without a tray to stay
//! in, tray "Quit") asks first when games are running or post-game work is in
//! flight ([`request_quit`]). The *graceful quit* ([`graceful_quit`], also run
//! by the signals and by the confirmation's "Quit anyway") saves the running
//! games' playtime, pauses the Epic queue in order, releases the
//! single-instance socket and the tray, and only then ends the loop. Games are
//! independent processes: quitting never stops them.
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
    /// The graceful quit began: no new windows, requests or quits.
    quitting: bool,
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
pub fn init(
    cx: &mut App,
    cli: Cli,
    started: Instant,
    messages: Option<UnboundedReceiver<Message>>,
    signals: Option<UnboundedReceiver<i32>>,
) {
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
        quitting: false,
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

    // SIGTERM / SIGINT / SIGHUP: quit without asking.
    if let Some(mut rx) = signals {
        cx.spawn(async move |cx| {
            while let Some(signal) = rx.next().await {
                cx.update(|cx| {
                    log::info!("quitting because of signal {signal}");
                    graceful_quit(cx);
                });
            }
        })
        .detach();
    }

    cx.on_window_closed(on_window_closed).detach();

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
            cx.update(graceful_quit);
        })
        .detach();
    }
    // As the tray's Quit (asks first when a game runs).
    if let Some(ms) = after("HERONITE_DEBUG_REQUEST_QUIT_AFTER_MS") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            cx.update(request_quit);
        })
        .detach();
    }
    // As the user pressing the window's close button.
    if let Some(ms) = after("HERONITE_DEBUG_USER_CLOSE_AFTER_MS") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            cx.update(|cx| {
                log::info!("[debug] the user closes the window");
                if allow_user_close(cx) {
                    let handle = hub(cx).0.borrow().window.as_ref().map(|w| w.handle);
                    if let Some(handle) = handle {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                }
            });
        })
        .detach();
    }
    // `<app_name>:<seconds>@<delay_ms>`: a game "runs" for that long (nothing
    // is started) so the post-game work can be watched with or without a window.
    if let Ok(spec) = std::env::var("HERONITE_DEBUG_FAKE_SESSION") {
        fake_session(cx, &spec);
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

fn fake_session(cx: &mut App, spec: &str) {
    let parse = || -> Option<(String, u64, u64)> {
        let (app, rest) = spec.split_once(':')?;
        let (secs, delay) = rest.split_once('@')?;
        Some((app.to_string(), secs.parse().ok()?, delay.parse().ok()?))
    };
    let Some((app_name, secs, delay)) = parse() else {
        log::warn!("HERONITE_DEBUG_FAKE_SESSION: expected <app_name>:<seconds>@<delay_ms>");
        return;
    };
    let state = hub(cx);
    let heroic = {
        let mut s = state.0.borrow_mut();
        if s.exit_when_idle {
            // A headless launch (`--launch` with FAKE_LAUNCH) would otherwise
            // exit before the fake session even starts.
            s.running.entry(app_name.clone()).or_insert(None);
        }
        s.heroic.clone()
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(delay)).await;
        let found = cx
            .background_spawn(async move {
                Library::load(&heroic).ok().and_then(|lib| lib.games.iter().find(|g| g.app_name == app_name).cloned())
            })
            .await;
        let Some(game) = found else {
            log::warn!("[fake-session] game not found");
            return;
        };
        let app = game.app_name.clone();
        log::info!("[fake-session] {} starts (window: {})", game.title, cx.update(|cx| has_window(cx)));
        cx.update(|cx| {
            let runtime = hub(cx).0.borrow().runtime.clone();
            runtime.debug_register_session(&game);
            on_event(cx, LaunchEvent::Launching { app_name: app.clone() });
            on_event(cx, LaunchEvent::Running { app_name: app.clone(), pid: 1 });
        });
        cx.background_executor().timer(Duration::from_secs(secs)).await;
        cx.update(|cx| {
            log::info!("[fake-session] {} exits (window: {})", game.title, has_window(cx));
            on_event(cx, LaunchEvent::Exited { app_name: app, elapsed: Duration::from_secs(secs), exit: None, record: None });
        });
    })
    .detach();
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
        // The close button / compositor "close": ask first when quitting
        // would leave games or post-game work behind.
        window.on_window_should_close(cx, |_, cx| allow_user_close(cx));
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
        if s.quitting {
            return;
        }
        s.tray.is_some() && (hiding || settings.exit_to_tray)
    };
    if keep {
        log::info!("window closed, staying in the tray");
        // Nothing left to wait for when the last game already ended.
        maybe_exit_idle(cx);
    } else {
        log::info!("window closed, quitting");
        // The close button already asked; this covers a window that went away
        // by other means while games run.
        request_quit(cx);
    }
}

/// Games the launcher is tracking (titles, sorted) and whether post-game work
/// (backups, cloud saves) is still running: what a quit would leave behind.
fn pending_work(cx: &App) -> (Vec<String>, bool) {
    let hub = hub(cx);
    let (running, runtime) = {
        let s = hub.0.borrow();
        (s.running.keys().cloned().collect::<Vec<_>>(), s.runtime.clone())
    };
    let mut titles: Vec<String> =
        running.into_iter().map(|app| runtime.session_info(&app).map(|i| i.title).filter(|t| !t.is_empty()).unwrap_or(app)).collect();
    titles.sort();
    (titles, crate::services::session::busy())
}

/// Opens the quit confirmation in the existing main window.
fn ask_quit_confirm(cx: &mut App, titles: Vec<String>, busy: bool) -> bool {
    let hub = hub(cx);
    let (view, tray) = {
        let s = hub.0.borrow();
        (s.window.as_ref().map(|w| w.view.clone()), s.tray.is_some())
    };
    let Some(view) = view else { return false };
    view.update(cx, |v, cx| v.system_quit_confirm(titles, busy, tray, cx)).is_ok()
}

/// The close button was pressed (called by the platform before the window
/// goes away). Returns whether it may close. The window stays when it was
/// meant to quit the app while games run: the confirmation opens in it.
fn allow_user_close(cx: &mut App) -> bool {
    let hub = hub(cx);
    let stays_alive = {
        let mut s = hub.0.borrow_mut();
        if s.quitting {
            return true;
        }
        let settings = s.settings();
        s.tray.is_some() && (s.hiding || settings.exit_to_tray)
    };
    if stays_alive {
        return true;
    }
    let (titles, busy) = pending_work(cx);
    if titles.is_empty() && !busy {
        return true;
    }
    log::info!("close requested while {} game(s) run (post-game work: {busy}): asking", titles.len());
    // Never `show_window` from here: the window is being updated right now.
    ask_quit_confirm(cx, titles, busy);
    false
}

/// A user's quit (tray menu, a window closed by other means). Asks first when
/// games run or post-game work is in flight; otherwise quits.
pub fn request_quit(cx: &mut App) {
    if hub(cx).0.borrow().quitting {
        return;
    }
    let (titles, busy) = pending_work(cx);
    if titles.is_empty() && !busy {
        return graceful_quit(cx);
    }
    log::info!("quit requested while {} game(s) run (post-game work: {busy}): asking", titles.len());
    // The window may be hidden (tray): bring it up for the question.
    let asked = show_window(cx).is_some() && ask_quit_confirm(cx, titles, busy);
    if !asked {
        log::warn!("could not show the quit confirmation: quitting");
        graceful_quit(cx);
    }
}

/// The confirmation's "Quit anyway".
pub fn quit_anyway(cx: &mut App) {
    graceful_quit(cx);
}

/// The confirmation's "Keep running in tray": the window goes, the app and
/// the games stay.
pub fn keep_in_tray(cx: &mut App) {
    // The click that asked is being handled inside the window: close it after.
    cx.defer(|cx| {
        if !close_window_keep_alive(cx) {
            log::warn!("could not hide the window to the tray");
        }
    });
}

/// How long the Epic queue gets to stop its process after the pause request.
const EPIC_STOP_WAIT: Duration = Duration::from_secs(5);
/// A quit that has not finished by then is forced (a stuck step).
const QUIT_WATCHDOG: Duration = Duration::from_secs(15);

/// Ends the app in order, without asking, and never blocks the UI thread:
/// stop accepting second invocations, save the running games' playtime, pause
/// the Epic queue (its element stays queued and resumes at the next start),
/// shut the tray, then end the loop. Games keep running. A second call is a
/// no-op; a signal while this runs ends the process at once.
pub fn graceful_quit(cx: &mut App) {
    let hub = hub(cx);
    let (runtime, tray) = {
        let mut s = hub.0.borrow_mut();
        if s.quitting {
            return;
        }
        s.quitting = true;
        (s.runtime.clone(), s.tray.take())
    };
    log::info!("quitting");
    super::signals::quit_started();
    // A second process from now on finds no socket instead of a dying listener.
    super::single_instance::close_active();
    if let Some(t) = tray {
        t.shutdown();
    }
    let downloads = crate::services::try_downloads(cx);
    if let Some(d) = &downloads {
        // Only asks (legendary gets SIGTERM); waited for below.
        d.update(cx, |d, _| d.pause_for_shutdown());
    }
    // If a step hangs, the process still ends.
    let _ = std::thread::Builder::new().name("quit-watchdog".into()).spawn(|| {
        std::thread::sleep(QUIT_WATCHDOG);
        log::error!("the quit did not finish in {QUIT_WATCHDOG:?}: exiting");
        std::process::exit(0);
    });
    cx.spawn(async move |cx| {
        let saved = cx.background_spawn(async move { runtime.checkpoint_all() }).await;
        if saved > 0 {
            log::info!("saved the playtime of {saved} running game(s)");
        }
        if let Some(d) = downloads {
            let deadline = Instant::now() + EPIC_STOP_WAIT;
            while !cx.update(|cx| d.read(cx).queue_idle()) {
                if Instant::now() >= deadline {
                    log::warn!("the Epic queue did not stop in {EPIC_STOP_WAIT:?}; legendary is stopped when the app exits");
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
        }
        log::info!("goodbye");
        cx.update(|cx| cx.quit());
    })
    .detach();
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
        graceful_quit(cx);
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
        TrayCmd::Quit => request_quit(cx),
        TrayCmd::Launch(app_name) => launch(cx, LaunchRequest { app_name, ..Default::default() }, true, false),
    }
}

/// A second invocation reached this instance.
fn on_message(cx: &mut App, msg: Message) {
    if hub(cx).0.borrow().quitting {
        log::info!("request from another invocation ignored: quitting");
        return;
    }
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
