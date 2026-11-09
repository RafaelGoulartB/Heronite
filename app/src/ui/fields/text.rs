//! Dialog typography, field labels and the card box (glass look).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Dialog paragraph: 14px.
pub fn paragraph(text: impl Into<SharedString>) -> Div {
    div().text_size(px(14.)).line_height(px(20.)).text_color(theme::text_primary()).child(text.into())
}

/// Small meta line (12px, dim).
pub fn meta_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text.into())
}

/// Field label above a control: 12px muted.
pub fn field_label(text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div()
        .relative()
        .children(crate::ui::find::flash(&text, 8.))
        .mb(px(6.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme::text_muted())
        .child(text)
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
