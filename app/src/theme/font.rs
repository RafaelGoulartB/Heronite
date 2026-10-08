//! Font families and weights.

pub const FONT_BODY: &str = "Cabin";
pub const FONT_HEADING: &str = "Rubik";
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
