//! Runtime appearance (Settings → Appearance) and the glass surfaces derived
//! from it: fill, art dimming, borders, shadows and the page veil, each with
//! a dark and a light value. Also the accent colour and the high-contrast
//! switch the palette reads.
//!
//! The appearance is one process-wide value read by every glass helper (no
//! `cx` needed, no I/O while rendering). The Collection loads it at start and
//! the Appearance page updates it while a slider moves.

use gpui_kit::{px, Hsla};
use heronite_data::appearance::{readable_on, Accent, Appearance, InterfaceMode, Palette, Rgb, SolidTint};

use super::{black_a, custom_palette, ink_a, is_light, readable_on_light, rgba_hsla, shadow_a, white_a, with_alpha, LIGHT_PAGE};

static APPEARANCE: std::sync::RwLock<Appearance> = std::sync::RwLock::new(Appearance::DEFAULT);

/// Colour of the selected game's art for [`Accent::Game`], and that colour
/// darkened for the light theme; `None` while there is none (no game, art
/// still loading).
type GameAccent = Option<((u8, u8, u8), (u8, u8, u8))>;
static GAME_ACCENT: std::sync::RwLock<GameAccent> = std::sync::RwLock::new(None);

/// The desktop's colours, for "From the desktop" (read by
/// `system::desktop_colours`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DesktopColours {
    /// Its accent colour.
    pub accent: Option<Rgb>,
    /// Its windows' background on its dark style and on its light one.
    pub window_dark: Option<Rgb>,
    pub window_light: Option<Rgb>,
}

static DESKTOP: std::sync::RwLock<DesktopColours> =
    std::sync::RwLock::new(DesktopColours { accent: None, window_dark: None, window_light: None });

pub fn desktop_colours() -> DesktopColours {
    *DESKTOP.read().unwrap_or_else(|e| e.into_inner())
}

/// Replaces the desktop's colours. True when they changed.
pub fn set_desktop_colours(c: DesktopColours) -> bool {
    let mut cur = DESKTOP.write().unwrap_or_else(|e| e.into_inner());
    std::mem::replace(&mut *cur, c) != c
}

/// The desktop's window colour on the current theme's side, if it has one.
pub fn desktop_panel() -> Option<Rgb> {
    let d = desktop_colours();
    if is_light() {
        d.window_light
    } else {
        d.window_dark
    }
}

/// The current appearance settings.
pub fn appearance() -> Appearance {
    *APPEARANCE.read().unwrap_or_else(|e| e.into_inner())
}

/// Replaces the appearance; out-of-range values are clamped on the way in.
pub fn set_appearance(a: Appearance) {
    *APPEARANCE.write().unwrap_or_else(|e| e.into_inner()) = a.normalized();
}

static CUSTOM_BACKGROUND_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Changes when the user picks another background picture (it is always
/// copied to the same file): part of its texture's cache id.
pub fn custom_background_epoch() -> u64 {
    CUSTOM_BACKGROUND_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn bump_custom_background() {
    CUSTOM_BACKGROUND_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Sets the colour [`Accent::Game`] uses. True when it changed.
pub fn set_game_accent(rgb: Option<(u8, u8, u8)>) -> bool {
    let mut cur = GAME_ACCENT.write().unwrap_or_else(|e| e.into_inner());
    let changed = cur.map(|(c, _)| c) != rgb;
    if changed {
        *cur = rgb.map(|c| (c, readable_on_light(c)));
    }
    changed
}

/// A preset accent in the current theme's shade, or the desktop's (`None`
/// for [`Accent::Game`], and for the desktop's while it has none).
pub fn accent_shade(a: Accent) -> Option<(u8, u8, u8)> {
    if a == Accent::Desktop {
        return desktop_colours().accent.map(own_accent_shade);
    }
    if is_light() {
        a.rgb_light()
    } else {
        a.rgb()
    }
}

/// An accent picked by hand in the current theme's shade: the same hue,
/// darkened until it reads on the light panels, lightened on the dark ones.
pub fn own_accent_shade(c: Rgb) -> Rgb {
    if is_light() {
        readable_on_light(c)
    } else {
        readable_on(c, &Palette::DARK.backgrounds(), false)
    }
}

/// The accent of `look` in the current theme's shade: its own colour or its
/// preset (`None` for [`Accent::Game`]).
pub fn look_accent(look: &Appearance) -> Option<Rgb> {
    match look.accent_own {
        Some(c) => Some(own_accent_shade(c)),
        None => accent_shade(look.accent),
    }
}

/// The accent colour (Settings → Appearance → Accent colour), in the shade
/// of the current theme.
pub fn accent_rgb() -> (u8, u8, u8) {
    if let Some(p) = custom_palette() {
        return p.accent;
    }
    let light = is_light();
    if let Some(c) = look_accent(&appearance()) {
        return c;
    }
    let game = *GAME_ACCENT.read().unwrap_or_else(|e| e.into_inner());
    match game {
        Some((dark, for_light)) => {
            if light {
                for_light
            } else {
                dark
            }
        }
        None => accent_shade(Accent::Ice).expect("a preset"),
    }
}

/// Stronger secondary text and borders (Settings → Appearance).
pub fn high_contrast() -> bool {
    appearance().high_contrast
}

/// `a` (the alpha of faint text or a line), raised halfway to opaque in high contrast.
pub fn contrast_a(a: f32) -> f32 {
    if high_contrast() {
        a + (1. - a) * 0.5
    } else {
        a
    }
}

/// Opacity of the glass panels as a fraction (the Solid mode is fully opaque).
fn panel_opacity() -> f32 {
    let a = appearance();
    match a.interface_mode {
        InterfaceMode::Solid => 1.,
        InterfaceMode::Transparent => a.interface_opacity / 100.,
    }
}

/// Panel opacity above which the art under the glass darkens towards fully
/// hidden at 100% (below it the dim stays at 50%). The default opacity (90%)
/// sits above it.
const DIM_KNEE: f32 = 0.84;

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

/// The solid panels' colour of the current theme (Settings → Appearance →
/// Interface background → Colour).
pub fn solid_tint() -> SolidTint {
    let a = appearance();
    if is_light() {
        a.solid_tint_light
    } else {
        a.solid_tint
    }
}

/// The panels' colour: the custom theme's, else the desktop's (when
/// chosen and it has one for this side), the one picked by hand or the
/// [`solid_tint`].
pub fn panel_rgb() -> Rgb {
    if let Some(p) = custom_palette() {
        return p.panel;
    }
    let a = appearance();
    let own = if is_light() { a.solid_own_light } else { a.solid_own };
    a.solid_desktop.then(desktop_panel).flatten().or(own).unwrap_or_else(|| solid_tint().rgb())
}

fn glass_tint(a: f32) -> Hsla {
    // Transparent glass always uses Slate (Paper on the light theme); the
    // presets and a colour picked by hand are for Solid.
    let (r, g, b) = match (custom_palette().is_some() || solid_mode(), is_light()) {
        (true, _) => panel_rgb(),
        (false, false) => SolidTint::Slate.rgb(),
        (false, true) => SolidTint::Paper.rgb(),
    };
    rgba_hsla(r, g, b, a)
}

/// Glass surface laid over the blurred art: `rgba(22,27,36,.90)` by default
/// (the Interface opacity). With [`glass_art_dim`] about 3% of the art shows
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
/// 50% up to [`DIM_KNEE`], then rising to 100% so that fully opaque
/// panels hide the art completely. The light theme fades the art instead.
pub fn glass_art_dim() -> Hsla {
    let o = panel_opacity();
    let t = ((o - DIM_KNEE) / (1. - DIM_KNEE)).clamp(0., 1.);
    let a = 0.5 + 0.5 * t;
    if let Some(p) = custom_palette() {
        return with_alpha(p.page, a);
    }
    if is_light() {
        with_alpha(LIGHT_PAGE, a)
    } else {
        black_a(a)
    }
}
/// Veil over the Collection behind the pages (☰ pages, Settings): `.55` at
/// [`DIM_KNEE`], darker above it, lighter for more transparent panels and fully
/// opaque for Solid panels (the pages then show no art at all).
pub fn page_backdrop() -> Hsla {
    let o = panel_opacity();
    let a = if o >= DIM_KNEE {
        0.55 + 0.45 * ((o - DIM_KNEE) / (1. - DIM_KNEE))
    } else {
        0.35 + 0.20 * ((o - 0.5) / (DIM_KNEE - 0.5)).clamp(0., 1.)
    };
    // Solid: the plain page colour, no art behind the sheet.
    if let Some(p) = custom_palette() {
        return with_alpha(p.page, a);
    }
    let (r, g, b) = match (is_light(), solid_mode()) {
        (false, true) => (7, 9, 12),
        (false, false) => (10, 12, 16),
        (true, true) => (220, 224, 230),
        (true, false) => LIGHT_PAGE,
    };
    rgba_hsla(r, g, b, a)
}
pub fn glass_border() -> Hsla {
    match (is_light(), high_contrast()) {
        (false, hc) => white_a(if hc { 0.22 } else { 0.11 }),
        (true, hc) => ink_a(if hc { 0.2 } else { 0.1 }),
    }
}
/// 1px highlight along the top edge of a glass surface (inset shadow in CSS).
pub fn glass_highlight() -> Hsla {
    white_a(if is_light() { 0.35 } else { 0.08 })
}
/// `0 30px 80px rgba(0,0,0,.45)`.
pub fn glass_shadow() -> Vec<gpui_kit::BoxShadow> {
    vec![gpui_kit::BoxShadow {
        color: shadow_a(0.45),
        offset: gpui_kit::point(px(0.), px(30.)),
        blur_radius: px(80.),
        spread_radius: px(0.),
        inset: false,
    }]
}
/// Smaller shadow of the compact menus (context menu, dropdown, select list).
pub fn glass_shadow_sm() -> Vec<gpui_kit::BoxShadow> {
    vec![gpui_kit::BoxShadow {
        color: shadow_a(0.42),
        offset: gpui_kit::point(px(0.), px(14.)),
        blur_radius: px(36.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Backdrop veil behind modals and drawers (the Collection stays visible).
pub fn glass_backdrop_veil() -> Hsla {
    if is_light() {
        ink_a(0.32)
    } else {
        black_a(0.55)
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;
    use crate::theme::{set_dark_only, set_system_appearance, theme_is_light};
    use gpui_kit::WindowAppearance;
    use heronite_data::appearance::Theme;

    /// One test only: the appearance is process-wide.
    #[test]
    fn glass_tokens_follow_the_appearance() {
        set_appearance(Appearance::default());
        assert!((glass_fill().a - 0.90).abs() < 1e-6);
        assert!((glass_fill_solid().a - 0.94).abs() < 1e-6);
        assert!(glass_art_dim().a > 0.5 && glass_art_dim().a < 1.);
        assert!(glass_shows_art());
        assert_eq!(glass_blur_px(), 30.);

        set_appearance(Appearance { interface_opacity: 84., ..Appearance::default() });
        assert!((glass_art_dim().a - 0.5).abs() < 1e-6);
        assert!((page_backdrop().a - 0.55).abs() < 1e-6);

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

        // The light theme: Paper glass, its own solid tint, dark windows stay dark.
        let light = Appearance { theme: Theme::Light, ..Appearance::default() };
        set_appearance(light);
        assert!(is_light());
        assert_eq!(glass_fill(), with_alpha(crate::theme::LIGHT_PAPER, 0.9));
        assert_eq!(accent_rgb(), Accent::Ice.rgb_light().unwrap());
        set_appearance(Appearance { interface_mode: InterfaceMode::Solid, solid_tint_light: SolidTint::Sand, ..light });
        assert_eq!(solid_tint(), SolidTint::Sand);
        set_dark_only(true);
        assert!(!is_light() && theme_is_light());
        assert_eq!(solid_tint(), SolidTint::Slate);
        assert_eq!(accent_rgb(), Accent::Ice.rgb().unwrap());
        set_dark_only(false);
        set_appearance(Appearance { theme: Theme::System, ..Appearance::default() });
        assert!(!is_light());
        assert!(set_system_appearance(WindowAppearance::VibrantLight));
        assert!(is_light());
        assert!(!set_system_appearance(WindowAppearance::Light));
        set_system_appearance(WindowAppearance::Dark);

        // An accent of your own: as picked when it reads, lightened on the
        // dark theme and darkened on the light one when it does not.
        set_appearance(Appearance { accent_own: Some((250, 120, 30)), ..Appearance::default() });
        assert_eq!(accent_rgb(), (250, 120, 30));
        let contrast = heronite_data::appearance::contrast;
        set_appearance(Appearance { accent_own: Some((20, 30, 90)), ..Appearance::default() });
        assert!(contrast(accent_rgb(), Palette::DARK.panel) >= 4.5);
        set_appearance(Appearance { theme: Theme::Light, accent_own: Some((255, 230, 80)), ..Appearance::default() });
        assert!(contrast(accent_rgb(), crate::theme::LIGHT_PAPER) >= 4.5);

        // The desktop's colours, while it has them; Ice and the tint without.
        set_appearance(Appearance {
            accent: Accent::Desktop,
            interface_mode: InterfaceMode::Solid,
            solid_desktop: true,
            ..Appearance::default()
        });
        assert_eq!((accent_rgb(), panel_rgb()), (Accent::Ice.rgb().unwrap(), SolidTint::Slate.rgb()));
        set_desktop_colours(DesktopColours { accent: Some((61, 174, 233)), window_dark: Some((32, 35, 38)), window_light: None });
        assert_eq!((accent_rgb(), panel_rgb()), ((61, 174, 233), (32, 35, 38)));
        set_desktop_colours(DesktopColours::default());

        // The custom theme: surfaces, text and accent as picked, ink in its text colour.
        let mut custom = heronite_data::appearance::Palette::DARK;
        custom.panel = (40, 20, 30);
        custom.accent = (250, 120, 30);
        set_appearance(Appearance { theme: Theme::Custom, custom, ..Appearance::default() });
        assert!(!is_light());
        assert_eq!(panel_rgb(), (40, 20, 30));
        assert_eq!(glass_fill(), with_alpha((40, 20, 30), 0.9));
        assert_eq!(accent_rgb(), (250, 120, 30));
        assert_eq!(crate::theme::ink_a(0.1), with_alpha(custom.text, 0.1));
        set_dark_only(true);
        assert!(crate::theme::custom_palette().is_none());
        set_dark_only(false);
        set_appearance(Appearance { theme: Theme::Custom, custom: heronite_data::appearance::Palette::LIGHT, ..Appearance::default() });
        assert!(is_light());
        set_appearance(Appearance::default());
    }
}
