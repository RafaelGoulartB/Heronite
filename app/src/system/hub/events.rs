//! Launch events from the runtime, tray commands and second-invocation requests.

use gpui_kit::*;
use heronite_launch::LaunchEvent;

use super::launch::launch;
use super::quit::{maybe_exit_idle, request_quit};
use super::window::{close_window_keep_alive, has_window, show_window};
use super::hub;
use crate::system::cli::{Cli, LaunchRequest};
use crate::system::single_instance::Message;
use crate::system::tray::TrayCmd;

pub(super) fn on_event(cx: &mut App, ev: LaunchEvent) {
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
    tell_compression(cx, &ev);
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

/// Capture compression pauses while any game runs.
fn tell_compression(cx: &mut App, ev: &LaunchEvent) {
    let Some(compression) = crate::services::compression::get(cx) else { return };
    let (app_name, running) = match ev {
        LaunchEvent::Launching { app_name } | LaunchEvent::Running { app_name, .. } => (app_name, true),
        LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => (app_name, false),
        LaunchEvent::Status { .. } => return,
    };
    compression.update(cx, |c, cx| c.set_game_running(app_name, running, cx));
}

/// With the window closed to the tray nobody sees toasts: problems of a
/// launch (it failed, cloud saves could not be synced) become notifications.
fn notify_without_window(ev: &LaunchEvent) {
    match ev {
        LaunchEvent::Error { message, .. } => crate::system::notify::send("Heronite", message.clone()),
        LaunchEvent::Status { message, warning: true, .. } => crate::system::notify::send("Heronite", message.clone()),
        _ => {}
    }
}

pub(super) fn on_tray_cmd(cx: &mut App, cmd: TrayCmd) {
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
pub(super) fn on_message(cx: &mut App, msg: Message) {
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
