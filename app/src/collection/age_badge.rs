//! The age rating above Play: the board and its code on a badge coloured by
//! age, what the code means and why it was given; every board's rating in
//! the tooltip. A game no board rated says so in one quiet line.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::vm::AgeVm;
use crate::theme;
use crate::ui::tooltip;

pub(super) fn age_badge(age: Option<&AgeVm>) -> AnyElement {
    let Some(age) = age else {
        return div()
            .id("focus-age")
            .px(px(4.))
            .text_size(px(12.5))
            .text_color(theme::text_dim())
            .child(tr!("No age rating"))
            .tooltip(tooltip(tr!("No age rating was found for this game.")))
            .into_any_element();
    };
    let tone = theme::age_tone(age.min_age);
    let badge = div()
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .justify_center()
        .min_w(px(44.))
        .h(px(46.))
        .px(px(5.))
        .rounded(px(9.))
        .border_2()
        .border_color(tone)
        .bg(Hsla::from(tone).opacity(0.14))
        .child(div().text_size(px(9.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(age.board.clone()))
        .child(
            div()
                .font_family(theme::font_heading())
                .font_weight(theme::W_BOLD)
                .text_size(px(if age.badge.len() > 3 { 13. } else { 18. }))
                .line_height(px(20.))
                .text_color(theme::text_bright())
                .child(age.badge.clone()),
        );
    let reasons =
        (!age.reasons.is_empty()).then(|| div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(age.reasons.clone()));
    let tip = if age.reasons.is_empty() { age.all.clone() } else { format!("{}\n\n{}", age.all, age.reasons).into() };
    div()
        .id("focus-age")
        .flex()
        .items_center()
        .gap(px(10.))
        .min_w_0()
        .child(badge)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .truncate()
                        .child(age.title.clone()),
                )
                .children(reasons),
        )
        .tooltip(tooltip(tip))
        .into_any_element()
}
