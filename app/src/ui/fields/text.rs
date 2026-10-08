//! Dialog typography, field labels and the card box (glass look).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Primary text colour of dialogs (`#e8edf4`).
pub fn text_default() -> Rgba {
    theme::text_primary()
}

/// Dialog paragraph: 14px.
pub fn paragraph(text: impl Into<SharedString>) -> Div {
    div().text_size(px(14.)).line_height(px(20.)).text_color(theme::text_primary()).child(text.into())
}

/// Secondary paragraph (muted, 13px), as the settings tabs use.
pub fn paragraph_secondary(text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.into())
}

/// Small meta line (12px, dim).
pub fn meta_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text.into())
}

/// Field label above a control: 12px muted.
pub fn field_label(text: impl Into<SharedString>) -> Div {
    div().mb(px(6.)).text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(text.into())
}

/// A label above any control (an input, a row of input + button, a select).
pub fn labeled(label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div().flex().flex_col().child(field_label(label)).child(control)
}

/// Grouped box: card fill, 1px border, radius 16, padding 14.
pub fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(14.))
        .rounded(px(theme::R_CARD))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
}
