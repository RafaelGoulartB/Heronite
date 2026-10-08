//! Visual tokens taken from Heronite's CSS (theme `nord-dark`, Collection
//! overlay styles). Values are px at a 16px root unless noted.

use gpui_kit::{hsla, px, rgb, rgba, Hsla, Pixels, Rgba};

pub const FONT_BODY: &str = "Cabin";
pub const FONT_HEADING: &str = "Rubik";

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
