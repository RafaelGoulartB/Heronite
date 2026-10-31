//! The overlay's surfaces: Wayland layer-shell windows on the overlay layer,
//! drawn over everything (fullscreen games included) without keyboard focus,
//! so the game keeps it; only a panel opened from the keyboard takes it
//! (until it closes). Sizes follow the display's height, like console
//! mode, smaller since the panel floats over a game (1440 px tall → 1.16).

#[cfg(target_os = "linux")]
use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellNotSupportedError, LayerShellOptions};
use gpui_kit::*;
use heronite_data::overlay_prefs::Side;

/// Size of the panel in design px (console mode's 1280×800 scale). The
/// height fits the tallest tab (Home with the FPS limit and "Mark this
/// moment" rows and its recent screenshots); every tab
/// fills it, so the panel never changes size while it loads or between tabs.
pub const PANEL_W: f32 = 440.;
pub const PANEL_H: f32 = 912.;
/// Gap between the surfaces and the screen edges, in design px.
pub const EDGE: f32 = 16.;
/// Size of a notice, in design px.
pub const NOTICE_W: f32 = 360.;
pub const NOTICE_H: f32 = 76.;

/// Display height (px) per unit of the console scale.
const PX_PER_SCALE: f32 = 1240.;

/// The console scale for the display the overlay shows on.
pub fn scale(cx: &App) -> f32 {
    (display_height(cx) / PX_PER_SCALE).clamp(1., 2.)
}

/// Height of the tallest display. Wayland has no primary display (GPUI
/// returns none), and the compositor puts the surface on the focused one,
/// which this cannot know: one display is the usual case.
/// `HERONITE_DEBUG_OVERLAY_DISPLAY=<px>` stands in for it (sizes of other
/// screens in debug captures).
fn display_height(cx: &App) -> f32 {
    if let Some(h) = crate::env::parse::<f32>("HERONITE_DEBUG_OVERLAY_DISPLAY") {
        return h;
    }
    cx.displays().iter().map(|d| f32::from(d.bounds().size.height)).fold(0., f32::max).max(720.)
}

#[cfg(target_os = "linux")]
fn side_anchor(side: Side) -> Anchor {
    match side {
        Side::Left => Anchor::LEFT,
        Side::Right => Anchor::RIGHT,
    }
}

/// A layer-shell surface sits at the top, along `side`.
fn options(namespace: &str, size: Size<Pixels>, side: Side, margin: Pixels, layer: bool, keyboard: bool) -> WindowOptions {
    let kind = if layer { layer_shell(namespace, side, margin, keyboard) } else { WindowKind::Normal };
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size })),
        app_id: Some(if layer { "com.heronite.rs".into() } else { format!("com.heronite.{namespace}") }),
        window_background: WindowBackgroundAppearance::Transparent,
        kind,
        focus: !layer || keyboard,
        ..Default::default()
    }
}

#[cfg(target_os = "linux")]
fn layer_shell(namespace: &str, side: Side, margin: Pixels, keyboard: bool) -> WindowKind {
    WindowKind::LayerShell(LayerShellOptions {
        namespace: namespace.to_string(),
        layer: Layer::Overlay,
        anchor: side_anchor(side) | Anchor::TOP,
        margin: Some((margin, margin, margin, margin)),
        keyboard_interactivity: if keyboard { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None },
        ..Default::default()
    })
}

/// Layer-shell is Wayland's: [`wayland`] is false elsewhere, so it is never
/// asked for.
#[cfg(not(target_os = "linux"))]
fn layer_shell(_namespace: &str, _side: Side, _margin: Pixels, _keyboard: bool) -> WindowKind {
    WindowKind::Normal
}

/// Opens the panel along one side of the screen. `layer = false` draws it in
/// a normal window (debug captures); `keyboard`: it takes the keyboard.
pub fn open_panel<V: Render>(
    cx: &mut App,
    side: Side,
    layer: bool,
    keyboard: bool,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let s = scale(cx);
    let height = (PANEL_H * s).min(panel_max_height(cx));
    let size = size(px(PANEL_W * s), px(height));
    cx.open_window(options("overlay", size, side, px(EDGE * s), layer, keyboard), build)
}

/// The tallest the panel may be: the display minus the edge gaps (a Steam
/// Deck's 800 px is a little short of the full height).
fn panel_max_height(cx: &App) -> f32 {
    (display_height(cx) - 2. * EDGE * scale(cx)).max(400.)
}

/// Opens a notice in the top corner of `side`.
pub fn open_notice<V: Render>(
    cx: &mut App,
    side: Side,
    layer: bool,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let s = scale(cx);
    let size = size(px(NOTICE_W * s), px(NOTICE_H * s));
    // Away from the panel's side, so both can show.
    let corner = match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    };
    cx.open_window(options("overlay-notice", size, corner, px(24. * s), layer, false), build)
}

/// The compositor refused a layer-shell surface (GNOME's has none).
#[cfg(target_os = "linux")]
pub fn is_layer_shell_missing(e: &anyhow::Error) -> bool {
    e.downcast_ref::<LayerShellNotSupportedError>().is_some()
}

/// No layer-shell surface is asked for off Linux.
#[cfg(not(target_os = "linux"))]
pub fn is_layer_shell_missing(_e: &anyhow::Error) -> bool {
    false
}

/// Layer-shell only exists on Wayland (an X11 session cannot show the overlay).
pub fn wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}
