//! Colours: the `nord-dark` palette, text, and the fills of cards, buttons,
//! tabs and inputs.

use gpui_kit::{hsla, rgb, Hsla, Rgba};

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

/// `rgba(r,g,b,a)` as `Hsla` (what `white_a` returns).
pub fn rgba_hsla(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    rgb_a(r, g, b, a).into()
}

// Palette (`nord-dark`).
pub fn page_bg() -> Rgba { rgb(0x07090c) }
pub fn accent() -> Rgba { rgb(0xa5dceb) }
pub fn accent_overlay() -> Rgba { rgb(0xcbeef8) }
pub fn navbar_background() -> Rgba { rgb(0x2e3440) }
pub fn background() -> Rgba { rgb(0x191c22) }
pub fn success() -> Rgba { rgb(0x94f1ab) }
pub fn danger() -> Rgba { rgb(0xbf616a) }
pub fn selected_ring() -> Rgba { rgb(0xe8dcc0) }

/// Console mode (proposal "2 · Console"): dark surfaces, a light pill for
/// the focused row, controller button colours.
pub fn console_bg() -> Rgba { rgb(0x0b0e13) }
pub fn console_sidebar() -> Rgba { rgb(0x0e1218) }
pub fn console_focus() -> Rgba { rgb(0xeef2f7) }
pub fn console_on_focus() -> Rgba { rgb(0x0d1117) }
pub fn console_on() -> Rgba { rgb(0x3dd68c) }
pub fn console_btn_a() -> Rgba { rgb(0x3ecf8e) }
pub fn console_btn_b() -> Rgba { rgb(0xf0727a) }
pub fn console_btn_x() -> Rgba { rgb(0x6cb6ff) }
pub fn console_btn_y() -> Rgba { rgb(0xf5cd5b) }

/// Statistics charts: one colour per top game (most played first); the rest
/// share [`chart_other`].
pub fn chart_series(i: usize) -> Rgba {
    match i {
        0 => rgb(0xa5dceb),
        1 => rgb(0xc6a9ee),
        2 => rgb(0x94e8b0),
        3 => rgb(0xf2cd85),
        _ => rgb(0xf09f8f),
    }
}
pub fn chart_other() -> Hsla { white_a(0.22) }

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

// Text.
pub fn text_primary() -> Rgba { rgb(0xe8edf4) }
/// Captions, leads, labels above fields (`#9aa6b6`).
pub fn text_muted() -> Rgba { rgb(0x9aa6b6) }
/// Row descriptions (`#8c97a8`).
pub fn text_dim() -> Rgba { rgb(0x8c97a8) }
/// Text drawn on an accent fill.
pub fn on_accent() -> Rgba { rgb(0x0c1a20) }

// Card (a group of rows) and its rows.
pub fn card_fill() -> Hsla { white_a(0.05) }
pub fn card_border() -> Hsla { white_a(0.07) }
pub fn row_separator() -> Hsla { white_a(0.06) }

pub fn toggle_off() -> Hsla { white_a(0.16) }

// Buttons.
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

// Inputs and selects.
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
