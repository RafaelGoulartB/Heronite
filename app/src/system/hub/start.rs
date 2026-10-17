//! Setup: the runtime, the tray, the global [`Hub`] and the first window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::DataPaths;

use super::debug::debug_hooks;
use super::events::{on_event, on_message, on_tray_cmd};
use super::launch::launch;
use super::quit::graceful_quit;
use super::window::{apply_window_flags, on_window_closed, show_window};
use super::{hub, Hub, State};
use crate::runtime::GameRuntime;
use crate::system::cli::Cli;
use crate::system::settings::SysSettings;
use crate::system::single_instance::Message;
use crate::system::tray;

/// Sets everything up and opens the first window (unless the invocation or
/// the settings ask for none).
pub fn init(
    cx: &mut App,
    cli: Cli,
    started: Instant,
    messages: Option<UnboundedReceiver<Message>>,
    signals: Option<UnboundedReceiver<i32>>,
) {
    let data_paths = DataPaths::detect();
    let settings = SysSettings::load(&data_paths);
    // Points the entry at this executable (a new AppImage may live elsewhere).
    // Only when on: a run against another data folder must not remove it.
    if settings.start_with_system {
        std::thread::spawn(|| crate::system::autostart::sync(true));
    }
    let (runtime, events) = GameRuntime::new(data_paths.clone());
    let runtime = Arc::new(runtime);
    let adopting = runtime.clone();
    // Post-game work (backup, capture session) runs here, with or without a window.
    crate::services::session::init(cx, data_paths.clone(), runtime.clone());

    let (tray_tx, tray_rx) = unbounded();
    let tray = (!settings.no_tray_icon && !crate::env::flag("HERONITE_NO_TRAY"))
        .then(|| {
            tray::spawn(data_paths.clone(), settings.dark_tray_icon, settings.exit_to_tray, settings.max_recent, false, tray_tx.clone())
        })
        .flatten();
    let fake_tray = crate::env::flag("HERONITE_DEBUG_FAKE_TRAY");
    let tray_alive = tray.is_some() || fake_tray;

    let debug_hidden = crate::env::flag("HERONITE_DEBUG_START_HIDDEN");
    let hide = cli.hide_window(settings.hide_window_on_protocol_launch);
    // "Start minimized" only for the start that comes with the session.
    let start_in_tray = (cli.autostart && settings.start_in_tray && tray_alive) || debug_hidden;
    // `--console` / `--fullscreen` show the window even when it would start in the tray.
    let want_window = !hide && (cli.wants_window() || !start_in_tray);
    // Nothing would keep the app reachable: show the window after all.
    let unreachable = !want_window && !tray_alive && cli.launch.is_none() && !debug_hidden;

    cx.set_global(Hub(Rc::new(RefCell::new(State {
        data_paths: data_paths.clone(),
        runtime,
        window: None,
        subscriber: None,
        running: HashMap::new(),
        steps: HashMap::new(),
        tray,
        fake_tray,
        exit_when_idle: cli.launch.is_some() && !want_window && !start_in_tray,
        hiding: false,
        quitting: false,
        fake_launch: crate::env::flag("HERONITE_DEBUG_FAKE_LAUNCH"),
        // The first window opens fullscreen right away.
        fullscreen: cli.fullscreen && !hide,
        settings,
        settings_stamp: data_paths.global_config().metadata().and_then(|m| m.modified()).ok(),
    }))));
    STARTED.with(|s| s.set(Some(started)));

    cx.spawn(async move |cx| {
        let mut events = events;
        while let Some(ev) = events.next().await {
            cx.update(|cx| on_event(cx, ev));
        }
    })
    .detach();

    cx.spawn(async move |cx| {
        let mut rx = tray_rx;
        while let Some(cmd) = rx.next().await {
            cx.update(|cx| on_tray_cmd(cx, cmd));
        }
    })
    .detach();

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

    // The Epic service (download queue, startup update check, auto-updates),
    // the capture compression and the app's own updates belong to the app,
    // not to the window. A one-shot launch has no use for them.
    if !hub(cx).0.borrow().exit_when_idle {
        crate::services::downloads(cx);
        crate::services::compression::init(cx, data_paths.clone());
        crate::services::update::init(cx, heronite_data::AppPaths::for_data(&data_paths));
    }
    // Games still running from before the app restarted; after the services
    // above, which follow running games through the same events.
    cx.background_spawn(async move {
        let n = adopting.adopt_leftovers();
        if n > 0 {
            log::info!("following {n} game(s) left running");
        }
    })
    .detach();

    if want_window || unreachable {
        show_window(cx);
    }
    if !hide {
        apply_window_flags(cx, cli.console, false);
    }
    if let Some(req) = cli.launch {
        launch(cx, req, hide, false);
    }
    debug_hooks(cx, tray_tx);
}

thread_local! {
    pub(super) static STARTED: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}
