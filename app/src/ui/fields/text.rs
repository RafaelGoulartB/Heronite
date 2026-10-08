//! Dialog typography, field labels and the grey panel box.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// `--text-default` on nord-dark.
pub fn text_default() -> Rgba {
    rgb(0xcaf3fd)
}

/// Dialog paragraph (`p`, 16px, `--text-default`).
pub fn paragraph(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(text_default()).child(text.into())
}

/// Same, but `--text-secondary` (white on nord-dark), as the settings tabs use.
pub fn paragraph_secondary(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(white()).child(text.into())
}

/// Small meta line (`--text-sm`, 13.33px).
pub fn meta_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.33)).line_height(px(16.).max(px(13.33))).text_color(white()).child(text.into())
}

/// Field label (`label`, 16px, `--text-default`).
pub fn field_label(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(text_default()).child(text.into())
}

/// A label above any control (an input, a row of input + button, a select).
pub fn labeled(label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div().flex().flex_col().child(field_label(label)).child(control)
}

/// Grey bordered box (`rgba(255,255,255,.04)`, 1px `.08`, radius 4, padding 12).
pub fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(4.))
        .bg(theme::white_a(0.04))
        .border_1()
        .border_color(theme::white_a(0.08))
}
