//! The launcher's surface. Wayland with layer-shell (Hyprland, KDE, Sway): a
//! layer surface centred on the focused screen, over every window, that takes
//! the keyboard. Elsewhere (GNOME, X11, Windows): a borderless window centred
//! on the screen. `HERONITE_DEBUG_QUICK_LAUNCH` always draws it in a normal
//! window, so captures can find it.

use gpui_kit::*;

use crate::collection::palette::PaletteView;

/// Opens the window with the view `build` makes.
pub(super) fn open<V: Render>(
    cx: &mut App,
    build: impl Fn(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<(AnyWindowHandle, Entity<V>)> {
    let size = PaletteView::launcher_size();
    #[cfg(target_os = "linux")]
    if crate::overlay::window::supported() && crate::env::value("HERONITE_DEBUG_QUICK_LAUNCH").is_none() {
        match gpui_kit::open_window(layer_options(size), cx, &build) {
            Err(e) if crate::overlay::window::is_layer_shell_missing(&e) => {
                log::info!("quick launcher: no layer-shell here, a normal window instead");
            }
            opened => return opened,
        }
    }
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("app.heronite.Heronite".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        // Windows: a pop-up stays out of the taskbar and over the others.
        kind: if cfg!(windows) { WindowKind::PopUp } else { WindowKind::Normal },
        is_resizable: false,
        focus: true,
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, &build)?;
    cx.activate(true);
    Ok(opened)
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
