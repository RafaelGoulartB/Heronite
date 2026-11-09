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
    inset(window.viewport_size(), frame_insets(window.window_decorations(), gpui_kit::component::window_paddings(window)))
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
    fn server_decorations_leave_the_whole_window() {
        let b = inset(size(px(1920.), px(1080.)), frame_insets(Decorations::Server, pads(0.)));
        assert_eq!(b, Bounds { origin: point(px(0.), px(0.)), size: size(px(1920.), px(1080.)) });
    }

    #[test]
    fn a_floating_client_window_loses_its_shadow_and_border() {
        let e = frame_insets(Decorations::Client { tiling: Tiling::default() }, pads(20.));
        let b = inset(size(px(1874.), px(1000.)), e);
        assert_eq!(b.origin, point(px(21.), px(21.)));
        assert_eq!(b.size, size(px(1832.), px(958.)));
    }

    #[test]
    fn a_maximized_client_window_has_no_frame() {
        let b = inset(size(px(1920.), px(1080.)), frame_insets(Decorations::Client { tiling: Tiling::tiled() }, pads(0.)));
        assert_eq!(b.size, size(px(1920.), px(1080.)));
    }

    #[test]
    fn a_window_snapped_left_keeps_the_frame_on_its_free_side() {
        let tiling = Tiling { top: true, bottom: true, left: true, right: false };
        let paddings = Edges { top: px(0.), right: px(20.), bottom: px(0.), left: px(0.) };
        let b = inset(size(px(960.), px(1080.)), frame_insets(Decorations::Client { tiling }, paddings));
        assert_eq!(b.origin, point(px(0.), px(0.)));
        assert_eq!(b.size, size(px(939.), px(1080.)));
    }
}
