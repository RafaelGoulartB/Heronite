//! Runtime appearance (Settings → Appearance) and the glass surfaces derived
//! from it: fill, art dimming, borders, shadows and the page veil.
//!
//! The appearance is one process-wide value read by every glass helper (no
//! `cx` needed, no I/O while rendering). The Collection loads it at start and
//! the Appearance page updates it while a slider moves.

use gpui_kit::{px, Hsla};
use heronite_data::appearance::{Accent, Appearance, InterfaceMode};

use super::{black_a, rgba_hsla, white_a};

static APPEARANCE: std::sync::RwLock<Appearance> = std::sync::RwLock::new(Appearance::DEFAULT);

/// Colour of the selected game's art for [`Accent::Game`]; `None` while
/// there is none (no game, art still loading).
static GAME_ACCENT: std::sync::RwLock<Option<(u8, u8, u8)>> = std::sync::RwLock::new(None);

/// The current appearance settings.
pub fn appearance() -> Appearance {
    *APPEARANCE.read().unwrap_or_else(|e| e.into_inner())
}

/// Replaces the appearance; out-of-range values are clamped on the way in.
pub fn set_appearance(a: Appearance) {
    *APPEARANCE.write().unwrap_or_else(|e| e.into_inner()) = a.normalized();
}

/// Opacity of the glass panels as a fraction (the Solid mode is fully opaque).
fn panel_opacity() -> f32 {
    let a = appearance();
    match a.interface_mode {
        InterfaceMode::Solid => 1.,
        InterfaceMode::Transparent => a.interface_opacity / 100.,
    }
}

/// The default panel opacity (`rgba(22,27,36,.84)`).
const GLASS_OPACITY_DEFAULT: f32 = 0.84;

/// Do the glass surfaces show blurred art at all? No in Solid mode and when
/// the panels are fully opaque: then nothing needs to be generated or kept.
pub fn glass_shows_art() -> bool {
    panel_opacity() < 0.995
}

/// Blur (logical px) of the art behind the glass panels.
pub fn glass_blur_px() -> f32 {
    appearance().interface_blur
}

fn solid_mode() -> bool {
    appearance().interface_mode == InterfaceMode::Solid
}

fn glass_tint(a: f32) -> Hsla {
    // Transparent glass always uses the slate tint; the presets are for Solid.
    let (r, g, b) = if solid_mode() { appearance().solid_tint.rgb() } else { (22, 27, 36) };
    rgba_hsla(r, g, b, a)
}

/// Glass surface laid over the blurred art: `rgba(22,27,36,.84)` by default
/// (the Interface opacity). With [`glass_art_dim`] about 8% of the art shows
/// through at the default: a tint of the game's colours without bright art
/// hurting legibility. Solid mode: the opaque tint.
pub fn glass_fill() -> Hsla {
    glass_tint(panel_opacity())
}
/// Same surface when there is no art to blur behind it: `rgba(22,27,36,.94)`
/// (never more transparent than the glass itself is opaque).
pub fn glass_fill_solid() -> Hsla {
    glass_tint(panel_opacity().max(0.94))
}
/// Darkening of the blurred art under the glass fill (stands in for the
/// Collection's scrims that sit between the art and a real backdrop blur):
/// 50% up to the default opacity, then rising to 100% so that fully opaque
/// panels hide the art completely.
pub fn glass_art_dim() -> Hsla {
    let o = panel_opacity();
    let t = ((o - GLASS_OPACITY_DEFAULT) / (1. - GLASS_OPACITY_DEFAULT)).clamp(0., 1.);
    black_a(0.5 + 0.5 * t)
}
/// Veil over the Collection behind the pages (☰ pages, Settings): `.55` at
/// the default panel opacity, lighter for more transparent panels and fully
/// opaque for Solid panels (the pages then show no art at all).
pub fn page_backdrop() -> Hsla {
    let o = panel_opacity();
    let a = if o >= GLASS_OPACITY_DEFAULT {
        0.55 + 0.45 * ((o - GLASS_OPACITY_DEFAULT) / (1. - GLASS_OPACITY_DEFAULT))
    } else {
        0.35 + 0.20 * ((o - 0.5) / (GLASS_OPACITY_DEFAULT - 0.5)).clamp(0., 1.)
    };
    // Solid: the plain page colour, no art behind the sheet.
    let (r, g, b) = if solid_mode() { (7, 9, 12) } else { (10, 12, 16) };
    rgba_hsla(r, g, b, a)
}
pub fn glass_border() -> Hsla {
    white_a(0.11)
}
/// 1px highlight along the top edge of a glass surface (inset shadow in CSS).
pub fn glass_highlight() -> Hsla {
    white_a(0.08)
}
/// `0 30px 80px rgba(0,0,0,.45)`.
pub fn glass_shadow() -> Vec<gpui_kit::BoxShadow> {
    vec![gpui_kit::BoxShadow {
        color: black_a(0.45),
        offset: gpui_kit::point(px(0.), px(30.)),
        blur_radius: px(80.),
        spread_radius: px(0.),
        inset: false,
    }]
}
/// Smaller shadow of the compact menus (context menu, dropdown, select list).
pub fn glass_shadow_sm() -> Vec<gpui_kit::BoxShadow> {
    vec![gpui_kit::BoxShadow {
        color: black_a(0.42),
        offset: gpui_kit::point(px(0.), px(14.)),
        blur_radius: px(36.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Backdrop veil behind modals and drawers (the Collection stays visible).
pub fn glass_backdrop_veil() -> Hsla {
    black_a(0.55)
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    /// One test only: the appearance is process-wide.
    #[test]
    fn glass_tokens_follow_the_appearance() {
        set_appearance(Appearance::default());
        assert!((glass_fill().a - 0.84).abs() < 1e-6);
        assert!((glass_fill_solid().a - 0.94).abs() < 1e-6);
        assert!((glass_art_dim().a - 0.5).abs() < 1e-6);
        assert!((page_backdrop().a - 0.55).abs() < 1e-6);
        assert!(glass_shows_art());
        assert_eq!(glass_blur_px(), 30.);

        set_appearance(Appearance { interface_opacity: 100., ..Appearance::default() });
        assert!(!glass_shows_art());
        assert!((glass_fill().a - 1.).abs() < 1e-6);
        assert!((glass_art_dim().a - 1.).abs() < 1e-6);

        set_appearance(Appearance { interface_opacity: 50., ..Appearance::default() });
        assert!((glass_fill().a - 0.5).abs() < 1e-6);
        assert!((glass_fill_solid().a - 0.94).abs() < 1e-6);
        assert!(page_backdrop().a < 0.55);

        set_appearance(Appearance { interface_mode: InterfaceMode::Solid, ..Appearance::default() });
        assert!(!glass_shows_art());
        assert!((glass_fill().a - 1.).abs() < 1e-6);
        assert!((page_backdrop().a - 1.).abs() < 1e-6);

        // Out-of-range values are clamped on the way in.
        set_appearance(Appearance { interface_opacity: 5., interface_blur: 500., ..Appearance::default() });
        assert!((glass_fill().a - 0.5).abs() < 1e-6);
        assert_eq!(glass_blur_px(), 60.);
        set_appearance(Appearance::default());
    }
}
