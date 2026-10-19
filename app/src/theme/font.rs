//! Font families and weights.

use heronite_data::appearance::FontChoice;

/// GPUI's name for the desktop's interface font.
const SYSTEM_FONT: &str = ".SystemUIFont";

/// Family of the text (Settings → Appearance → Font).
pub fn font_body() -> &'static str {
    match super::appearance().font {
        FontChoice::Default => "Cabin",
        FontChoice::Rubik => "Rubik",
        FontChoice::System => SYSTEM_FONT,
    }
}

/// Family of the titles.
pub fn font_heading() -> &'static str {
    match super::appearance().font {
        FontChoice::Default | FontChoice::Rubik => "Rubik",
        FontChoice::System => SYSTEM_FONT,
    }
}
/// Keyboard hint chips.
pub const FONT_MONO: &str = "Noto Sans Mono";

/// Heronite only ships the 400 weights of Rubik/Cabin; Chromium fakes bold for
/// CSS weights >= 600 and renders 500 as regular. The real 600 cut is the
/// closest match to that synthetic bold.
pub const W_REGULAR: gpui_kit::FontWeight = gpui_kit::FontWeight::NORMAL;
/// CSS `font-weight: 500`.
pub const W_500: gpui_kit::FontWeight = gpui_kit::FontWeight::NORMAL;
/// CSS `font-weight: 600 | 700`.
pub const W_BOLD: gpui_kit::FontWeight = gpui_kit::FontWeight::SEMIBOLD;
