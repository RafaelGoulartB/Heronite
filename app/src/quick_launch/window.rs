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

/// Opens the window with the view `build` makes.
pub(super) fn open<V: Render>(cx: &mut App, build: impl Fn(&mut Window, &mut App) -> Entity<V>) -> anyhow::Result<AnyWindowHandle> {
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
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display(cx), size, cx))),
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
    let handle = cx.open_window(options, |window, cx| {
        window.set_window_title("Heronite");
        build(window, cx)
    })?;
    cx.activate(true);
    Ok(handle.into())
}

/// Windows: the screen of the foreground window (where the user is), else
/// the primary one.
#[cfg(windows)]
fn display(cx: &App) -> Option<DisplayId> {
    let screen = heronite_platform::overlay_window::foreground_screen().ok()?;
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
