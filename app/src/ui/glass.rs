//! Glass surfaces ("Vidro"). GPUI has no `backdrop-filter`, so a glass panel
//! paints the selected game's art, pre-blurred on the CPU at a tiny
//! resolution (`Variant::Glass`, see `CollectionView::glass_image`), exactly
//! aligned with the window and clipped to the panel's rounded bounds, then the
//! translucent fill, a border and the top highlight. Without art (or while it
//! loads) the fill is nearly opaque. The Interface settings (Solid or
//! Transparent, opacity, blur: `theme::appearance`) drive all of it.
//!
//! The image is owned by `CollectionView`, which publishes it every frame
//! with [`set_backdrop`]; the UI runs on one thread, so a thread-local is
//! enough and every helper here works without a `cx`.

use std::cell::RefCell;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

thread_local! {
    static BACKDROP: RefCell<Option<Arc<RenderImage>>> = const { RefCell::new(None) };
}

/// Saturation of the glass backdrop (`backdrop-filter: blur(30px)
/// saturate(1.5)` in the mockups). The blur is a setting
/// (`theme::glass_blur_px`, Settings → Appearance).
pub const GLASS_SATURATION: f32 = 1.5;

/// Publishes the blurred art that glass surfaces paint behind themselves.
pub fn set_backdrop(img: Option<Arc<RenderImage>>) {
    BACKDROP.with(|b| *b.borrow_mut() = img);
}

/// The blurred art currently published (`None`: solid fallback).
pub fn backdrop() -> Option<Arc<RenderImage>> {
    // Solid / fully opaque panels never show the art, even if a stale one is published.
    if !theme::glass_shows_art() {
        return None;
    }
    BACKDROP.with(|b| b.borrow().clone())
}

/// Absolute layer filling its parent with the blurred art, aligned to the
/// window and clipped to the parent's bounds with `radius` corners.
pub fn backdrop_layer(radius: f32) -> Option<Canvas<()>> {
    let img = backdrop()?;
    Some(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let full = Bounds { origin: point(px(0.), px(0.)), size: window.viewport_size() };
                window.paint_image(bounds, full, Corners::all(px(radius)), img, 0, false).ok();
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// The layers every glass surface starts with: blurred art (when available),
/// the translucent fill and the 1px top highlight. `radius` is the outer
/// radius of the surface; the layers sit inside its 1px border.
fn layers(radius: f32) -> [AnyElement; 4] {
    let inner = (radius - 1.).max(0.);
    let art = backdrop_layer(inner);
    let fill = if art.is_some() { theme::glass_fill() } else { theme::glass_fill_solid() };
    let dim = if art.is_some() { theme::glass_art_dim() } else { theme::black_a(0.) };
    [
        art.map(IntoElement::into_any_element).unwrap_or_else(|| div().into_any_element()),
        // The real backdrop is the dark, scrimmed Collection; the raw art is brighter.
        div().absolute().inset_0().rounded(px(inner)).bg(dim).into_any_element(),
        div().absolute().inset_0().rounded(px(inner)).bg(fill).into_any_element(),
        div()
            .absolute()
            .top_0()
            .left(px(radius))
            .right(px(radius))
            .h(px(1.))
            .bg(theme::glass_highlight())
            .into_any_element(),
    ]
}

/// A big glass surface (popover, sheet, drawer, modal): border, big shadow
/// and the glass layers. Children added by the caller paint above the layers.
/// Corners are not clipped for children: give edge-to-edge children their own
/// radius.
pub fn glass_panel(radius: f32) -> Div {
    let mut d = div()
        .relative()
        .rounded(px(radius))
        .border_1()
        .border_color(theme::glass_border())
        .shadow(theme::glass_shadow());
    for l in layers(radius) {
        d = d.child(l);
    }
    d
}

/// Compact glass surface for menus and dropdown lists (smaller shadow).
pub fn glass_menu_surface(radius: f32) -> Div {
    let mut d = div()
        .relative()
        .rounded(px(radius))
        .border_1()
        .border_color(theme::glass_border())
        .shadow(theme::glass_shadow_sm());
    for l in layers(radius) {
        d = d.child(l);
    }
    d
}

/// Full-window glass for big sheets (like the Settings sheet of the mockups):
/// the blurred art under a dark tint, with no border and no rounding.
pub fn glass_fullscreen() -> Div {
    let art = backdrop_layer(0.);
    let has_art = art.is_some();
    div()
        .absolute()
        .inset_0()
        .children(art)
        .child(div().absolute().inset_0().bg(if has_art { theme::black_a(0.55) } else { theme::glass_fill_solid() }))
}

/// Veil darkening the Collection behind modals and drawers.
pub fn veil() -> Div {
    div().absolute().inset_0().bg(theme::glass_backdrop_veil())
}

/// Target size (physical px) of the glass backdrop request for a window of
/// `w x h` logical px: the window size rounded to 32px steps, so resizing does
/// not queue a new blur for every pixel (the result is ~1/5 of that anyway).
pub fn glass_target(w: f32, h: f32, scale: f32) -> (u32, u32) {
    let step = 32.;
    let q = |v: f32| (((v * scale) / step).round().max(1.) * step) as u32;
    (q(w), q(h))
}

#[cfg(test)]
mod tests {
    use super::glass_target;

    #[test]
    fn glass_target_snaps_to_32px_steps() {
        assert_eq!(glass_target(1280., 800., 1.), (1280, 800));
        assert_eq!(glass_target(1290., 805., 1.), (1280, 800));
        assert_eq!(glass_target(3428., 1402., 1.), (3424, 1408));
        assert_eq!(glass_target(1., 1., 1.), (32, 32));
    }
}
