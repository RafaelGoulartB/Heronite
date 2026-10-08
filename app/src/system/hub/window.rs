//! The main window: opened on demand and dropped when it is closed to the tray.

use std::time::Instant;

use gpui_kit::*;

use super::quit::{allow_user_close, maybe_exit_idle, request_quit};
use super::start::STARTED;
use super::{hub, Hub, MainWindow};
use crate::collection::CollectionView;

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
pub(super) fn show_window(cx: &mut App) -> Option<WeakEntity<CollectionView>> {
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
pub(super) fn close_window_keep_alive(cx: &mut App) -> bool {
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

pub(super) fn on_window_closed(cx: &mut App, id: WindowId) {
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

/// Whether the main window exists (otherwise results go to desktop
/// notifications).
pub fn has_window(cx: &App) -> bool {
    cx.try_global::<Hub>().is_some_and(|h| h.0.borrow().window.is_some())
}
