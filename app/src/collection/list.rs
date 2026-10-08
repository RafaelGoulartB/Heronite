//! Group header row (`.collection__groupToggle`) and the sticky overlay.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::vm::GroupVm;
use crate::theme;

/// `.collection__groupToggle`: 13px/600, .78 white, 7px dot, count pill.
pub(super) fn group_header(g: &GroupVm, stuck: bool) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(6.5))
        .py(px(6.4))
        .w_full()
        .text_size(px(13.))
        .line_height(px(15.6))
        .font_weight(theme::W_BOLD)
        .text_color(theme::white_a(0.78))
        .when(stuck, |d| {
            // `.is-stuck::before`: gradient that extends 12px below the row
            // and to both list edges (blur approximated by the gradient).
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom(px(-12.))
                    .left(px(-13.6))
                    .right(px(-16.))
                    // CSS: rgba(7,9,12,.32)→.14→0 over a 7px backdrop blur.
                    // Without the blur a darker ramp keeps the label legible.
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::rgb_a(7, 9, 12, 0.78), 0.),
                        linear_color_stop(theme::rgb_a(7, 9, 12, 0.), 1.),
                    )),
            )
        })
        .child(div().size(px(7.)).flex_none().rounded_full().bg(g.color).opacity(0.9))
        .child(g.name.clone())
        .child(
            div()
                .ml(px(1.75))
                .px(px(7.))
                .py(px(1.))
                .rounded(px(2.75))
                .bg(theme::white_a(0.08))
                .text_size(px(11.))
                .font_weight(theme::W_REGULAR)
                .text_color(theme::white_a(0.8))
                .child(g.cards.len().to_string()),
        )
}

/// Height of one group header row (padding + line).
pub(super) const GROUP_HEADER_H: f32 = 6.4 * 2. + 15.6;
