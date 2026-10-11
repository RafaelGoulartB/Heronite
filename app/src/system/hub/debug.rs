//! `HERONITE_DEBUG_*` helpers to exercise the window lifecycle without a mouse.

use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::*;
use heronite_data::Library;
use heronite_launch::LaunchEvent;

use super::events::on_event;
use super::quit::{allow_user_close, graceful_quit, request_quit};
use super::window::{has_window, show_window};
use super::hub;
use crate::system::tray::TrayCmd;

/// `HERONITE_DEBUG_*` helpers to exercise the window lifecycle without a
/// mouse: `SHOW_AFTER_MS`, `CLOSE_AFTER_MS` (as if the user closed it),
/// `QUIT_AFTER_MS`, `TRAY_CMDS=show@500,toggle@1500,launch:<app>@2000,quit@3000`.
pub(super) fn debug_hooks(cx: &mut App, tray_tx: UnboundedSender<TrayCmd>) {
    let after = |name: &str| crate::env::parse::<u64>(name);
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
    if let Some(spec) = crate::env::value("HERONITE_DEBUG_FAKE_SESSION") {
        fake_session(cx, &spec);
    }
    if let Some(list) = crate::env::value("HERONITE_DEBUG_TRAY_CMDS") {
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
    let data_paths = {
        let mut s = state.0.borrow_mut();
        if s.exit_when_idle {
            // A headless launch (`--launch` with FAKE_LAUNCH) would otherwise
            // exit before the fake session even starts.
            s.running.entry(app_name.clone()).or_insert(None);
        }
        s.data_paths.clone()
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(delay)).await;
        let found = cx
            .background_spawn(async move {
                Library::load(&data_paths).ok().and_then(|lib| lib.games.iter().find(|g| g.app_name == app_name).cloned())
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
