//! Sizes and radii.

use gpui_kit::{px, Pixels};

// Collection layout.
pub const HEADER_H: Pixels = px(42.);
pub const LIST_COL_W: f32 = 432.; // 27rem
pub const LIST_COL_W_MD: f32 = 296.; // 18.5rem (container <= 1100px)
pub const LIST_COL_W_SM: f32 = 156.; // 9.75rem (container <= 780px)
pub const CARD_W: f32 = 125.;
pub const CARD_GAP_X: f32 = 12.;
pub const CARD_GAP_Y: f32 = 12.8;
pub const TOOL_BTN: Pixels = px(32.);
pub const TOOL_ICON: Pixels = px(18.4);

// Radii.
pub const R_POPOVER: f32 = 20.;
pub const R_MENU: f32 = 14.;
pub const R_SHEET: f32 = 26.;
pub const R_DRAWER: f32 = 24.;
pub const R_CARD: f32 = 16.;
pub const R_TILE: f32 = 14.;
pub const R_INPUT: f32 = 12.;
pub const R_ITEM: f32 = 10.;

/// Toggle switch: 40x24 pill, 18px knob inset 3px.
pub const TOGGLE_W: f32 = 40.;
pub const TOGGLE_H: f32 = 24.;
pub const TOGGLE_KNOB: f32 = 18.;

/// Buttons are pills, 36px high (30px small).
pub const BUTTON_H: f32 = 36.;
pub const BUTTON_H_SM: f32 = 30.;

pub const INPUT_H: f32 = 40.;
