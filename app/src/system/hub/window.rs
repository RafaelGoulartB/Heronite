//! The main window: opened on demand and dropped when it is closed to the tray.

use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::ui_state::{self, WindowPlacement};
use heronite_data::AppPaths;

use super::quit::{allow_user_close, maybe_exit_idle, request_quit};
use super::start::STARTED;
use super::{hub, Hub, MainWindow};
use crate::collection::CollectionView;

/// The window's size when it opens, and its smallest one.
const OPEN_SIZE: (f32, f32) = (1600., 1000.);
const MIN_SIZE: (f32, f32) = (640., 480.);

/// The screen the window most likely opens on: the primary one, else (Wayland
/// names none) the smallest, so the window fits whichever it is.
fn screen(cx: &App) -> Option<Size<Pixels>> {
    let area = |s: &Size<Pixels>| f32::from(s.width) * f32::from(s.height);
    cx.primary_display()
        .map(|d| d.visible_bounds().size)
        .or_else(|| cx.displays().iter().map(|d| d.visible_bounds().size).min_by(|a, b| area(a).total_cmp(&area(b))))
}

/// [`OPEN_SIZE`] and [`MIN_SIZE`], within 95% of `screen` (GPUI px, so the
/// interface scale counts): a 4K TV at 300% is 1280 × 720. A 1080p screen
/// keeps the full size.
fn fitted_sizes(screen: Option<Size<Pixels>>) -> (Size<Pixels>, Size<Pixels>) {
    let (open, min) = (size(px(OPEN_SIZE.0), px(OPEN_SIZE.1)), size(px(MIN_SIZE.0), px(MIN_SIZE.1)));
    let Some(screen) = screen.filter(|s| f32::from(s.width) > 0. && f32::from(s.height) > 0.) else { return (open, min) };
    let room = size(px((f32::from(screen.width) * 0.95).floor()), px((f32::from(screen.height) * 0.95).floor()));
    (open.min(&room), min.min(&room))
}

/// Least of a remembered window that must still be on a screen (each way, GPUI
/// px) for it to reopen there: enough to grab it.
const ON_SCREEN: f32 = 48.;

/// How long the window's placement rests before it is written (a drag or a
/// resize sends many).
const PLACEMENT_SAVE_DELAY: Duration = Duration::from_millis(500);

/// The main window's latest placement and the timer that writes it.
#[derive(Default)]
pub(super) struct PlacementSave {
    latest: Option<WindowPlacement>,
    unsaved: bool,
    timer: Option<Task<()>>,
}

/// Where a remembered window reopens (GPUI px; `zoom` is the interface's):
/// on the screen it overlaps most (its index in `screens`), at least `min`
/// big, shrunk and moved to fit that screen. `None` when it is on no screen
/// any more (a monitor that was unplugged).
fn restored_bounds(saved: &WindowPlacement, zoom: f32, min: Size<Pixels>, screens: &[Bounds<Pixels>]) -> Option<(usize, Bounds<Pixels>)> {
    let (x, y, w, h) = (saved.x / zoom, saved.y / zoom, saved.width / zoom, saved.height / zoom);
    let edges = |s: &Bounds<Pixels>| {
        let (sx, sy) = (f32::from(s.origin.x), f32::from(s.origin.y));
        (sx, sy, sx + f32::from(s.size.width), sy + f32::from(s.size.height))
    };
    let overlap = |s: &Bounds<Pixels>| {
        let (left, top, right, bottom) = edges(s);
        let (ow, oh) = ((x + w).min(right) - x.max(left), (y + h).min(bottom) - y.max(top));
        (ow >= ON_SCREEN && oh >= ON_SCREEN).then_some(ow * oh)
    };
    let (i, screen) = screens
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((i, s, overlap(s)?)))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(i, s, _)| (i, s))?;
    let (left, top, right, bottom) = edges(screen);
    let w = w.max(f32::from(min.width)).min(right - left);
    let h = h.max(f32::from(min.height)).min(bottom - top);
    let (x, y) = (x.min(right - w).max(left), y.min(bottom - h).max(top));
    Some((i, Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }))
}

/// The window's bounds when it opens, the screen they are on (Windows opens a
/// window on the primary screen unless told another) and whether it opens
/// maximized: as it was left, else centered at [`OPEN_SIZE`].
fn opening_bounds(
    saved: Option<&WindowPlacement>,
    open_size: Size<Pixels>,
    min_size: Size<Pixels>,
    cx: &App,
) -> (Bounds<Pixels>, Option<DisplayId>, bool) {
    let displays = cx.displays();
    let screens: Vec<_> = displays.iter().map(|d| d.visible_bounds()).collect();
    match saved {
        Some(p) => match restored_bounds(p, gpui_zoom::zoom(), min_size, &screens) {
            Some((i, bounds)) => (bounds, Some(displays[i].id()), p.maximized),
            None => (Bounds::centered(None, open_size, cx), None, p.maximized),
        },
        None => (Bounds::centered(None, open_size, cx), None, false),
    }
}

/// Notes the window's placement after a move or resize; it is written once
/// it rests ([`PLACEMENT_SAVE_DELAY`]) or the window closes.
fn remember_placement(window: &Window, cx: &mut App) {
    // Fullscreen (F11, `--fullscreen`) is not remembered: the window reopens
    // as it was before. (X11 reports it as windowed or maximized.)
    if window.is_fullscreen() {
        return;
    }
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (b, false),
        WindowBounds::Maximized(b) => (b, true),
        WindowBounds::Fullscreen(_) => return,
    };
    let zoom = gpui_zoom::zoom();
    let mut placement = WindowPlacement {
        x: f32::from(bounds.origin.x) * zoom,
        y: f32::from(bounds.origin.y) * zoom,
        width: f32::from(bounds.size.width) * zoom,
        height: f32::from(bounds.size.height) * zoom,
        maximized,
    };
    let hub = hub(cx);
    let latest = hub.0.borrow().placement.latest;
    // Maximized keeps the bounds it restores to. Windows reports those; X11
    // reports the maximized ones, so on Linux the last known ones are kept.
    if cfg!(target_os = "linux") && maximized {
        if let Some(l) = latest {
            placement = WindowPlacement { maximized, ..l };
        }
    }
    if latest == Some(placement) {
        return;
    }
    let timer = cx.spawn(async move |cx| {
        cx.background_executor().timer(PLACEMENT_SAVE_DELAY).await;
        cx.update(save_placement);
    });
    let mut s = hub.0.borrow_mut();
    s.placement.latest = Some(placement);
    s.placement.unsaved = true;
    // Replacing the timer cancels the previous one.
    s.placement.timer = Some(timer);
}

/// Writes the window's placement if it changed since the last write.
pub(super) fn save_placement(cx: &mut App) {
    let hub = hub(cx);
    let (placement, paths) = {
        let mut s = hub.0.borrow_mut();
        if !std::mem::take(&mut s.placement.unsaved) {
            return;
        }
        (s.placement.latest, AppPaths::for_data(&s.data_paths))
    };
    if let Some(p) = placement {
        if let Err(err) = ui_state::set_window_placement(&paths, p) {
            log::warn!("saving the window's placement: {err:#}");
        }
    }
}

fn open_main_window(cx: &mut App, started: Instant, fullscreen: bool) -> Option<MainWindow> {
    let (open_size, min_size) = fitted_sizes(screen(cx));
    let saved = {
        let hub = hub(cx);
        let mut s = hub.0.borrow_mut();
        if s.placement.latest.is_none() {
            s.placement.latest = ui_state::window_placement(&AppPaths::for_data(&s.data_paths));
        }
        s.placement.latest
    };
    let (bounds, display_id, maximized) = opening_bounds(saved.as_ref(), open_size, min_size, cx);
    let options = WindowOptions {
        display_id,
        // Fullscreen keeps `bounds` for when F11 switches back.
        window_bounds: Some(if fullscreen {
            WindowBounds::Fullscreen(bounds)
        } else if maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }),
        // No system title bar: the app draws its own controls
        // (`ui::window_controls`). `titlebar: None` hides it on Windows; on
        // Linux the compositor is asked to leave the frame to the app.
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("app.heronite.Heronite".into()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(min_size),
        // X11 only; Wayland's compositor takes the `.desktop` entry's.
        icon: cfg!(target_os = "linux").then(crate::system::app_icon::x11_window_icon).flatten(),
        ..Default::default()
    };
    // The library loads while the window and the GPU come up.
    crate::collection::preload_library(cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Heronite");
        // The close button / compositor "close": ask first when quitting
        // would leave games or post-game work behind.
        window.on_window_should_close(cx, |_, cx| allow_user_close(cx));
        cx.new(|cx| {
            // Moves, resizes, maximizing and restoring; the observer goes with
            // the view.
            cx.observe_window_bounds(window, |_, window, cx| remember_placement(window, cx)).detach();
            CollectionView::new(started, window, cx)
        })
    });
    crate::collection::discard_preload();
    match opened {
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
    show_window_for(cx, false)
}

/// `--fullscreen` was asked for in this run: the windows open fullscreen and
/// console mode leaves fullscreen alone.
pub fn fullscreen_asked(cx: &App) -> bool {
    cx.try_global::<Hub>().is_some_and(|h| h.0.borrow().fullscreen)
}

/// [`show_window`]; `console`: the window is about to enter console mode (a
/// new one then opens fullscreen when console mode asks for it).
fn show_window_for(cx: &mut App, console: bool) -> Option<WeakEntity<CollectionView>> {
    let hub = hub(cx);
    let existing = hub.0.borrow().window.as_ref().map(|w| (w.handle, w.view.clone()));
    if let Some((handle, view)) = existing {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            hub.0.borrow_mut().exit_when_idle = false;
            return Some(view);
        }
    }
    let started = STARTED.with(|s| s.take()).unwrap_or_else(Instant::now);
    let (asked, paths) = {
        let s = hub.0.borrow();
        (s.fullscreen, AppPaths::for_data(&s.data_paths))
    };
    // Console mode's own fullscreen: set from the start, as switching a
    // window that is still being mapped can switch it back.
    let fullscreen = asked || crate::console::startup::opens_fullscreen(&paths, console);
    let window = open_main_window(cx, started, fullscreen)?;
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

/// `--console` / `--fullscreen` (at start or from another invocation): shows
/// the window and switches it. Console mode is remembered like when it is
/// picked in the app; fullscreen holds for the windows of this run.
pub(super) fn apply_window_flags(cx: &mut App, console: bool, fullscreen: bool) {
    if !(console || fullscreen) {
        return;
    }
    let hub = hub(cx);
    let existing = hub.0.borrow().window.as_ref().map(|w| w.handle);
    hub.0.borrow_mut().fullscreen |= fullscreen;
    // A new window already opens fullscreen; only an open one is switched
    // (asking a window that is still being mapped could switch it back).
    let Some(view) = show_window_for(cx, console) else { return };
    let Some(handle) = hub.0.borrow().window.as_ref().map(|w| w.handle) else { return };
    let opened = existing != Some(handle);
    let switch_fullscreen = fullscreen && !opened;
    handle
        .update(cx, |_, window, cx| {
            if switch_fullscreen && !window.is_fullscreen() {
                window.toggle_fullscreen();
            }
            if console {
                view.update(cx, |v, cx| v.enter_console_mode(opened, window, cx)).ok();
            }
        })
        .ok();
}

/// The app's own close button (Linux): asks first like the compositor's close,
/// then closes.
pub fn close_window(window: &mut Window, cx: &mut App) {
    if allow_user_close(cx) {
        window.remove_window();
    }
}

/// Closes the window but keeps the app alive (needs the tray). Returns whether
/// a window was closed.
pub(super) fn close_window_keep_alive(cx: &mut App) -> bool {
    let hub = hub(cx);
    let handle = {
        let mut s = hub.0.borrow_mut();
        if !s.has_tray() {
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
    if hub.0.borrow().window.as_ref().is_some_and(|w| w.handle.window_id() == id) {
        // The last move or resize may still be waiting for its timer.
        hub.0.borrow_mut().placement.timer = None;
        save_placement(cx);
    }
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
        s.has_tray() && (hiding || settings.exit_to_tray)
    };
    if keep {
        log::info!("window closed, staying in the tray");
        // The Collection, its textures and caches were just dropped; the
        // glass backdrop it published goes too.
        crate::ui::glass::set_backdrop(None);
        crate::trim_heap_later();
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

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` macro would shadow `#[test]`.
    use super::{fitted_sizes, restored_bounds, OPEN_SIZE};
    use gpui_kit::{point, px, size, Bounds, Pixels};
    use heronite_data::ui_state::WindowPlacement;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }
    }

    #[test]
    fn a_remembered_window_reopens_where_it_was_while_it_fits() {
        let min = size(px(640.), px(480.));
        // Two 1080p screens side by side (the taskbar takes 48 px).
        let screens = [rect(0., 0., 1920., 1032.), rect(1920., 0., 1920., 1032.)];
        let at = |x, y, width, height| WindowPlacement { x, y, width, height, maximized: false };
        assert_eq!(restored_bounds(&at(100., 50., 1200., 800.), 1., min, &screens), Some((0, rect(100., 50., 1200., 800.))));
        // On the second screen: opened there.
        assert_eq!(restored_bounds(&at(2000., 50., 1200., 800.), 1., min, &screens), Some((1, rect(2000., 50., 1200., 800.))));
        // The interface zoom does not change the size on screen.
        assert_eq!(restored_bounds(&at(200., 100., 1600., 1000.), 2., min, &screens), Some((0, rect(100., 50., 800., 500.))));
        // Hanging off the screen it is mostly on: moved and shrunk onto it.
        assert_eq!(restored_bounds(&at(-300., 600., 2400., 900.), 1., min, &screens), Some((0, rect(0., 132., 1920., 900.))));
        // The second screen was unplugged: nowhere to reopen.
        assert_eq!(restored_bounds(&at(2000., 50., 1200., 800.), 1., min, &screens[..1]), None);
        // Too small (an older size limit): grown to the smallest.
        assert_eq!(restored_bounds(&at(10., 10., 200., 100.), 1., min, &screens), Some((0, rect(10., 10., 640., 480.))));
    }

    #[test]
    fn the_window_fits_the_screen() {
        let open = size(px(OPEN_SIZE.0), px(OPEN_SIZE.1));
        assert_eq!(fitted_sizes(None).0, open);
        assert_eq!(fitted_sizes(Some(size(px(2560.), px(1440.)))).0, open);
        assert_eq!(fitted_sizes(Some(size(px(1920.), px(1080.)))).0, open);
        // A 4K TV at 300%.
        let (o, m) = fitted_sizes(Some(size(px(1280.), px(720.))));
        assert_eq!((o, m), (size(px(1216.), px(684.)), size(px(640.), px(480.))));
        // At 200% on top of that: the smallest size fits too.
        let (o, m) = fitted_sizes(Some(size(px(640.), px(360.))));
        assert_eq!((o, m), (size(px(608.), px(342.)), size(px(608.), px(342.))));
    }
}
