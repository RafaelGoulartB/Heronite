//! Group header row (`.collection__groupToggle`) and the sticky overlay.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::vm::GroupVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// `.collection__groupToggle`: 13px/600, .78 white, 7px dot, count pill;
/// then `extra` (the "Select group" chip) and, while the group is folded, a
/// chevron at the end of the line saying it opens.
pub(super) fn group_header(g: &GroupVm, extra: Option<impl IntoElement>) -> Div {
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
        .text_color(theme::ink_a(0.78))
        .child(match g.icon {
            Some(ic) => icon(ic, px(14.), theme::ink_a(0.55)).into_any_element(),
            None => div().size(px(7.)).flex_none().rounded_full().bg(g.color).opacity(0.9).into_any_element(),
        })
        .child(g.name.clone())
        .child(
            div()
                .ml(px(1.75))
                .px(px(7.))
                .py(px(1.))
                .rounded(px(2.75))
                .bg(theme::ink_a(0.08))
                .text_size(px(11.))
                .font_weight(theme::W_REGULAR)
                .text_color(theme::ink_a(0.8))
                .child(g.cards.len().to_string()),
        )
        .children(extra)
        .when(g.collapsed, |d| d.child(div().flex_1()).child(icon(Icon::ExpandMore, px(18.), theme::ink_a(0.55))))
}

/// `.is-stuck::before`: the shade behind the pinned header, across the whole
/// list column and 12px below the row (blur approximated by the gradient).
pub(super) fn stuck_scrim(grid: bool) -> Div {
    let shade = |from: f32, to: f32| {
        linear_gradient(180., linear_color_stop(theme::stage_ink_a(from), 0.), linear_color_stop(theme::stage_ink_a(to), 1.))
    };
    let scrim = div().absolute().top_0().left_0().right_0().bottom(px(-12.));
    if !grid {
        // CSS: rgba(7,9,12,.32)→.14→0 over a 7px backdrop blur.
        // Without the blur a darker ramp keeps the label legible.
        return scrim.bg(shade(0.78, 0.));
    }
    // The grid has no column background under it: a shade that starts dark
    // reads as a band along the header, so it fades in, then out.
    scrim
        .bottom(px(-20.))
        .flex()
        .flex_col()
        .child(div().h(px(GROUP_HEADER_H / 2.)).flex_none().bg(shade(0.12, 0.5)))
        .child(div().flex_1().bg(shade(0.5, 0.)))
}

/// [`stuck_scrim`] over the Focus list's rows: their titles would read
/// through the header's label, so the shade stays dense under it.
pub(super) fn stuck_scrim_rows() -> Div {
    let dense = theme::stage_ink_a(0.92);
    div().absolute().top_0().left_0().right_0().bottom(px(-10.)).flex().flex_col().child(div().flex_1().bg(dense)).child(
        div().h(px(10.)).flex_none().bg(linear_gradient(180., linear_color_stop(dense, 0.), linear_color_stop(theme::stage_ink_a(0.), 1.))),
    )
}

/// Height of one group header row (padding + line).
pub(super) const GROUP_HEADER_H: f32 = 6.4 * 2. + 15.6;
