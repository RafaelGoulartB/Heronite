//! Visual tokens taken from Heronite's CSS (theme `nord-dark`, Collection
//! overlay styles). Values are px at a 16px root unless noted.

use gpui_kit::{hsla, px, rgb, rgba, Hsla, Pixels, Rgba};

pub const FONT_BODY: &str = "Cabin";
pub const FONT_HEADING: &str = "Rubik";
/// Keyboard hint chips.
pub const FONT_MONO: &str = "Noto Sans Mono";

/// `rgba(255,255,255,a)`
pub fn white_a(a: f32) -> Hsla {
    hsla(0., 0., 1., a)
}

/// `rgba(0,0,0,a)`
pub fn black_a(a: f32) -> Hsla {
    hsla(0., 0., 0., a)
}

/// `rgb(r,g,b)` with alpha.
pub fn rgb_a(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a }
}

pub fn page_bg() -> Rgba {
    rgb(0x07090c)
}

// nord-dark
pub fn accent() -> Rgba { rgb(0xa5dceb) }
pub fn accent_overlay() -> Rgba { rgb(0xcbeef8) }
pub fn navbar_background() -> Rgba { rgb(0x2e3440) }
pub fn navbar_active_background() -> Rgba { rgb(0x191c22) }
pub fn background() -> Rgba { rgb(0x191c22) }
pub fn success() -> Rgba { rgb(0x94f1ab) }
pub fn success_hover() -> Rgba { rgb(0x75e08f) }
pub fn danger() -> Rgba { rgb(0xbf616a) }
pub fn modal_backdrop() -> Rgba { rgba(0x000000cc) }

pub fn selected_ring() -> Rgba { rgb(0xe8dcc0) }

// Collection layout
pub const HEADER_H: Pixels = px(42.);
pub const LIST_COL_W: f32 = 432.; // 27rem
pub const LIST_COL_W_MD: f32 = 296.; // 18.5rem (container <= 1100px)
pub const LIST_COL_W_SM: f32 = 156.; // 9.75rem (container <= 780px)
pub const CARD_W: f32 = 125.;
pub const CARD_GAP_X: f32 = 12.;
pub const CARD_GAP_Y: f32 = 12.8;
pub const TOOL_BTN: Pixels = px(32.);
pub const TOOL_ICON: Pixels = px(18.4);

/// Status colours (`statusColors.ts`).
pub fn status_color(slug: &str) -> Rgba {
    match slug {
        "playing" => rgb(0x3ecf8e),
        "plan-to-play" => rgb(0x6cb6ff),
        "on-hold" => rgb(0xf0a202),
        "played" => rgb(0x8b9cff),
        "endless" => rgb(0x2ec4b6),
        "beaten" => rgb(0xf4d35e),
        "completed" => rgb(0xc084fc),
        "abandoned" => rgb(0x9aa0a6),
        "not-played" => rgb(0x6b7280),
        _ => rgb(0xe5e7eb),
    }
}

/// Heronite only ships the 400 weights of Rubik/Cabin; Chromium fakes bold for
/// CSS weights >= 600 and renders 500 as regular. The real 600 cut is the
/// closest match to that synthetic bold.
pub const W_REGULAR: gpui_kit::FontWeight = gpui_kit::FontWeight::NORMAL;
/// CSS `font-weight: 500`.
pub const W_500: gpui_kit::FontWeight = gpui_kit::FontWeight::NORMAL;
/// CSS `font-weight: 600 | 700`.
pub const W_BOLD: gpui_kit::FontWeight = gpui_kit::FontWeight::SEMIBOLD;

/// Colours of the gpui-component widgets we reuse (inputs, sliders, popovers)
/// so they look like Heroic's controls on nord-dark.
pub fn apply_component_theme(cx: &mut gpui_kit::App) {
    use gpui_kit::component::theme::{Theme, ThemeMode};
    Theme::change(ThemeMode::Dark, None, cx);
    let t = Theme::global_mut(cx);
    t.font_family = FONT_BODY.into();
    let c = &mut t.colors;
    c.background = background().into();
    c.foreground = rgb(0xcaf3fd).into();
    c.input = white_a(0.18);
    c.ring = rgb(0x1976d2).into();
    c.caret = white_a(0.9);
    c.selection = hsla(210. / 360., 0.8, 0.5, 0.35);
    c.primary = rgb(0x1976d2).into();
    c.slider_bar = rgb(0x1976d2).into();
    c.slider_thumb = rgb(0x1976d2).into();
    c.popover = navbar_background().into();
    c.popover_foreground = rgb(0xcaf3fd).into();
    c.border = white_a(0.12);
    c.muted_foreground = white_a(0.38);
}

/// Fades and transitions on/off. `HERONITE_NO_ANIMATIONS=1` disables them
/// (reduced motion; also needed for off-screen screenshots, where the
/// compositor stops sending frames and fades would stay at opacity 0).
pub fn animations() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("HERONITE_NO_ANIMATIONS").is_none_or(|v| v.is_empty() || v == "0"))
}

// ---------------------------------------------------------------------------
// Glass design system (docs/agents/glass-common.md, "OPTION 1 — Vidro").
// Everything the menus, dialogs, pages and settings share lives here; never
// hard-code one of these values at a call site.
// ---------------------------------------------------------------------------

/// `rgba(r,g,b,a)` as `Hsla` (what `white_a` returns), for the glass tokens.
pub fn rgba_hsla(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    rgb_a(r, g, b, a).into()
}

/// Glass surface laid over the blurred art: `rgba(22,27,36,.84)`. With
/// [`glass_art_dim`] about 8% of the art shows through: a tint of the game's
/// colours without bright art hurting legibility (the mockup's .62 did).
pub fn glass_fill() -> Hsla {
    rgba_hsla(22, 27, 36, 0.84)
}
/// Same surface when there is no art to blur behind it: `rgba(22,27,36,.94)`.
pub fn glass_fill_solid() -> Hsla {
    rgba_hsla(22, 27, 36, 0.94)
}
/// Darkening of the blurred art under the glass fill (stands in for the
/// Collection's scrims that sit between the art and a real backdrop blur).
pub fn glass_art_dim() -> Hsla {
    black_a(0.5)
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

// Radii (px).
pub const R_POPOVER: f32 = 20.;
pub const R_MENU: f32 = 14.;
pub const R_SHEET: f32 = 26.;
pub const R_DRAWER: f32 = 24.;
pub const R_CARD: f32 = 16.;
pub const R_TILE: f32 = 14.;
pub const R_INPUT: f32 = 12.;
pub const R_ITEM: f32 = 10.;

/// Backdrop veil behind modals and drawers (the Collection stays visible).
pub fn glass_backdrop_veil() -> Hsla {
    black_a(0.55)
}

// Text.
pub fn text_primary() -> Rgba { rgb(0xe8edf4) }
/// Captions, leads, labels above fields (`#9aa6b6`).
pub fn text_muted() -> Rgba { rgb(0x9aa6b6) }
/// Row descriptions (`#8c97a8`).
pub fn text_dim() -> Rgba { rgb(0x8c97a8) }
pub fn text_placeholder() -> Rgba { rgb(0x6f7a8a) }
/// Text drawn on an accent fill.
pub fn on_accent() -> Rgba { rgb(0x0c1a20) }

// Card (a group of rows) and its rows.
pub fn card_fill() -> Hsla { white_a(0.05) }
pub fn card_border() -> Hsla { white_a(0.07) }
pub fn row_separator() -> Hsla { white_a(0.06) }

// Toggle switch: 40x24 pill, 18px knob inset 3px.
pub const TOGGLE_W: f32 = 40.;
pub const TOGGLE_H: f32 = 24.;
pub const TOGGLE_KNOB: f32 = 18.;
pub fn toggle_off() -> Hsla { white_a(0.16) }

// Buttons (pill, 36px high; 30px small).
pub const BUTTON_H: f32 = 36.;
pub const BUTTON_H_SM: f32 = 30.;
pub fn button_fill() -> Hsla { white_a(0.09) }
pub fn button_fill_hover() -> Hsla { white_a(0.14) }
pub fn button_border() -> Hsla { white_a(0.10) }
pub fn button_text() -> Rgba { rgb(0xeef2f7) }
pub fn danger_fill() -> Hsla { rgba_hsla(191, 97, 106, 0.20) }
pub fn danger_fill_hover() -> Hsla { rgba_hsla(191, 97, 106, 0.30) }
pub fn danger_text() -> Rgba { rgb(0xf2a5ad) }

// Segmented tabs.
pub fn seg_track() -> Hsla { black_a(0.28) }
pub fn seg_active() -> Hsla { white_a(0.14) }
pub fn seg_text() -> Rgba { rgb(0xb5bfcc) }

// Inputs / selects.
pub const INPUT_H: f32 = 40.;
pub fn input_fill() -> Hsla { black_a(0.25) }
pub fn input_border() -> Hsla { white_a(0.08) }
/// Border of a focused input: the accent at 60%.
pub fn input_border_focus() -> Hsla {
    let a = accent();
    Rgba { a: 0.6, ..a }.into()
}

// Side navigation pills and the accent tint of active items.
pub fn nav_text() -> Rgba { rgb(0xc3cbd8) }
pub fn accent_tint() -> Hsla { rgba_hsla(165, 220, 235, 0.16) }
pub fn accent_tint_border() -> Hsla { rgba_hsla(165, 220, 235, 0.40) }
pub fn menu_item_hover() -> Hsla { white_a(0.07) }
