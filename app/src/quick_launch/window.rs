//! The launcher's surface. Wayland with layer-shell (Hyprland, KDE, Sway): a
//! layer surface centred on the focused screen, over every window, that takes
//! the keyboard. Elsewhere (GNOME, X11, Windows): a borderless window centred
//! on the screen (on Windows the foreground window's, as a topmost pop-up
//! out of the taskbar). `HERONITE_DEBUG_QUICK_LAUNCH` always draws it in a
//! normal window, so captures can find it.
//!
//! The window is the panel's size and holds the view alone, like the
//! overlay's: the component library's `Root` would paint the theme's
//! background and a window frame around it.

use gpui_kit::*;

use crate::collection::palette::PaletteView;

/// Opens the window with the view `build` makes. `activation_token`: the
/// xdg-activation token of the process that asked for it (see [`activated`]).
pub(super) fn open<V: Render>(
    cx: &mut App,
    activation_token: Option<String>,
    build: impl Fn(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<AnyWindowHandle> {
    let size = PaletteView::launcher_size();
    #[cfg(target_os = "linux")]
    if crate::overlay::window::supported() && crate::env::value("HERONITE_DEBUG_QUICK_LAUNCH").is_none() {
        match cx.open_window(layer_options(size), &build) {
            Err(e) if crate::overlay::window::is_layer_shell_missing(&e) => {
                log::info!("quick launcher: no layer-shell here, a normal window instead");
            }
            opened => return Ok(opened?.into()),
        }
    }
    // The bounds are that screen's: GPUI checks them against `display_id`
    // (the primary screen without one) and, off it, opens its default size
    // there instead (Windows).
    let display = display(cx);
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, size, cx))),
        display_id: display,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("app.heronite.Heronite".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        // Windows: a pop-up stays out of the taskbar and over the others.
        kind: if cfg!(windows) { WindowKind::PopUp } else { WindowKind::Normal },
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        ..Default::default()
    };
    let handle = activated(activation_token, || {
        cx.open_window(options, |window, cx| {
            window.set_window_title("Heronite");
            build(window, cx)
        })
    })?;
    #[cfg(windows)]
    finish(cx, handle.into());
    cx.activate(true);
    Ok(handle.into())
}

/// Linux (Wayland): `open` activates the window it opens with `token`, the
/// one the process that forwarded the request was started with (the
/// desktop's key binding, `xdg-open` for `heronite://quick-launch`). The
/// keyboard is a focus a compositor that prevents focus stealing (GNOME)
/// only gives with a token: without one the launcher opens behind the
/// focused window, and typing goes to that one. A layer surface has the
/// keyboard anyway, so only the normal window gets it.
#[cfg(target_os = "linux")]
fn activated<T>(token: Option<String>, open: impl FnOnce() -> T) -> T {
    let Some(token) = token else { return open() };
    gpui_linux::set_activation_token(Some(token));
    let opened = open();
    // Not used when the window did not open: no later window takes it.
    gpui_linux::set_activation_token(None);
    opened
}

#[cfg(not(target_os = "linux"))]
fn activated<T>(_token: Option<String>, open: impl FnOnce() -> T) -> T {
    open()
}

/// Windows: no frame around the panel (see `overlay_window::drop_frame`),
/// and the foreground with the keyboard, which Windows refuses a window
/// opened on a request from outside (`heronite://quick-launch` through the
/// socket: no input came to Heronite; `cx.activate` does nothing there).
/// Run once the borrows unwind, as both send GPUI synchronous messages.
#[cfg(windows)]
fn finish(cx: &mut App, handle: AnyWindowHandle) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let hwnd = handle.update(cx, |_, window, _| match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(raw)) => Some(raw.hwnd),
        _ => None,
    });
    let Ok(Some(hwnd)) = hwnd else {
        log::warn!("quick launcher: no Windows window handle, its frame stays");
        return;
    };
    cx.foreground_executor()
        .spawn(async move {
            if let Err(e) = heronite_platform::overlay_window::drop_frame(hwnd) {
                log::warn!("quick launcher: could not take the frame off: {e}");
            }
            match heronite_platform::overlay_window::take_foreground(hwnd) {
                Ok(true) => {}
                Ok(false) => log::info!("quick launcher: Windows kept the foreground elsewhere; a click gives it the keyboard"),
                Err(e) => log::warn!("quick launcher: could not take the foreground: {e}"),
            }
        })
        .detach();
}

/// Windows: the screen of the foreground window (where the user is), else
/// the primary one; the overlay's (`HERONITE_DEBUG_OVERLAY_MONITOR` picks
/// another in debug runs).
#[cfg(windows)]
fn display(cx: &App) -> Option<DisplayId> {
    let screen = crate::overlay::window::target_screen().ok()?;
    let scale = screen.scale.max(0.1);
    let (x, y) = ((screen.left as f32 + screen.width as f32 / 2.) / scale, (screen.top as f32 + screen.height as f32 / 2.) / scale);
    let distance = |b: Bounds<Pixels>| {
        let c = b.center();
        (f32::from(c.x) - x).powi(2) + (f32::from(c.y) - y).powi(2)
    };
    cx.displays().into_iter().min_by(|a, b| distance(a.bounds()).total_cmp(&distance(b.bounds()))).map(|d| d.id())
}

/// Elsewhere the desktop places a normal window: the primary screen.
#[cfg(not(windows))]
fn display(_cx: &App) -> Option<DisplayId> {
    None
}

#[cfg(target_os = "linux")]
fn layer_options(size: Size<Pixels>) -> WindowOptions {
    use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size })),
        app_id: Some("app.heronite.Heronite".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        // No anchor: centred on the screen the compositor picks (the focused one).
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "heronite-quick-launch".into(),
            layer: Layer::Overlay,
            anchor: Anchor::empty(),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        focus: true,
        ..Default::default()
    }
}
