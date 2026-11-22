//! The part of the window the app's views get to draw in.
//!
//! With client-side decorations (GNOME and other compositors without
//! `xdg-decoration`), a window that is not maximized or tiled is wrapped in
//! gpui-component's `window_border`: a 20px shadow and a 1px border on every
//! side the window is not tiled against. `window.viewport_size()` still covers
//! that frame, so layout math that sizes things from it lays them out wider
//! and taller than the room the views have (the grid ran under the drawer).
//! Size layouts with [`content_size`]; place things in window coordinates
//! (mouse positions, element bounds) against [`content_bounds`].

use gpui_kit::*;

/// The frame's border, drawn inside its shadow (gpui-component's
/// `window_border::BORDER_SIZE`).
const FRAME_BORDER: f32 = 1.;

/// Where the views are drawn, in window coordinates.
pub fn content_bounds(window: &Window) -> Bounds<Pixels> {
    // Only `window_border` sets the client inset (every frame, before the
    // views render). A window opened without gpui-kit's Root, like the
    // overlay's panel and notices, draws no frame even when it reports
    // client-side decorations (a Wayland layer surface starts as Client).
    let decorations = if window.client_inset().is_some() { window.window_decorations() } else { Decorations::Server };
    inset(window.viewport_size(), frame_insets(decorations, gpui_kit::component::window_paddings(window)))
}

/// The size of [`content_bounds`]: what layouts fit in.
pub fn content_size(window: &Window) -> Size<Pixels> {
    content_bounds(window).size
}

/// The window frame on each side: the shadow `paddings` gpui-component's
/// `window_paddings` gives, plus the border on every side that is not tiled.
fn frame_insets(decorations: Decorations, paddings: Edges<Pixels>) -> Edges<Pixels> {
    let Decorations::Client { tiling } = decorations else {
        return Edges::all(px(0.));
    };
    let side = |pad: Pixels, tiled: bool| if tiled { pad } else { pad + px(FRAME_BORDER) };
    Edges {
        top: side(paddings.top, tiling.top),
        right: side(paddings.right, tiling.right),
        bottom: side(paddings.bottom, tiling.bottom),
        left: side(paddings.left, tiling.left),
    }
}

fn inset(viewport: Size<Pixels>, e: Edges<Pixels>) -> Bounds<Pixels> {
    let width = (viewport.width - e.left - e.right).max(px(0.));
    let height = (viewport.height - e.top - e.bottom).max(px(0.));
    Bounds { origin: point(e.left, e.top), size: size(width, height) }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{frame_insets, inset};
    use gpui_kit::{point, px, size, Bounds, Decorations, Edges, Pixels, Tiling};

    fn pads(v: f32) -> Edges<Pixels> {
        Edges::all(px(v))
    }

    #[test]
    fn the_drawable_area_leaves_out_the_frame() {
        let free_right = Tiling { top: true, bottom: true, left: true, right: false };
        let right_pad = Edges { top: px(0.), right: px(20.), bottom: px(0.), left: px(0.) };
        for (what, decorations, paddings, window, origin, area) in [
            ("server decorations leave the whole window", Decorations::Server, pads(0.), (1920., 1080.), (0., 0.), (1920., 1080.)),
            (
                "a floating client window loses its shadow and border",
                Decorations::Client { tiling: Tiling::default() },
                pads(20.),
                (1874., 1000.),
                (21., 21.),
                (1832., 958.),
            ),
            (
                "a maximized client window has no frame",
                Decorations::Client { tiling: Tiling::tiled() },
                pads(0.),
                (1920., 1080.),
                (0., 0.),
                (1920., 1080.),
            ),
            (
                "a window snapped left keeps the frame on its free side",
                Decorations::Client { tiling: free_right },
                right_pad,
                (960., 1080.),
                (0., 0.),
                (939., 1080.),
            ),
        ] {
            let b = inset(size(px(window.0), px(window.1)), frame_insets(decorations, paddings));
            assert_eq!(b, Bounds { origin: point(px(origin.0), px(origin.1)), size: size(px(area.0), px(area.1)) }, "{what}");
        }
    }
}
