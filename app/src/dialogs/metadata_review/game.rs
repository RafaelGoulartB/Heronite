//! The game side of the review: the strip of the games of this review on
//! top ("1 of 2"), and the game shown on the left: its cover, title, where
//! it comes from and why it is asked.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{Answer, MetadataReview};
use crate::dialogs::metadata_update::words;
use crate::icons::Icon;
use crate::services::widgets::cover_box;
use crate::theme;
use crate::ui::icon;

/// Covers in the strip around the game shown.
const STRIP: usize = 14;
const COVER_W: f32 = 148.;
const COVER_H: f32 = 208.;

impl MetadataReview {
    pub(super) fn render_strip(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let n = self.games.len();
        let first = self.at.saturating_sub(STRIP / 2).min(n.saturating_sub(STRIP));
        let scale = window.scale_factor();
        let mut row = div().flex().items_center().gap(px(6.));
        for i in first..(first + STRIP).min(n) {
            let g = &self.games[i];
            let src = self.covers.get_chain(&format!("rv:{}", g.key), &g.cover, 26., 35., scale);
            let current = i == self.at;
            let answered = self.answers[i].is_some_and(|a| a != Answer::Skipped);
            let free = self.busy.is_none();
            row = row.child(
                div()
                    .id(("rv-strip", i))
                    .flex_none()
                    .p(px(2.))
                    .rounded(px(7.))
                    .border_2()
                    .border_color(if current { theme::accent().into() } else { transparent_black() })
                    .when(answered && !current, |d| d.opacity(0.4))
                    .when(!current && free, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.go_to(i, cx))))
                    .tooltip(crate::ui::tooltip(g.item.title.clone()))
                    .child(cover_box(src, 26., 35.).rounded(px(4.))),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .mb(px(18.))
            .child(row)
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("{n} of {total}", n = self.at + 1, total = n)))
    }

    /// The game on the left.
    pub(super) fn render_game(&mut self, window: &Window) -> Div {
        let Some(g) = self.game().cloned() else { return div() };
        let src = self.covers.get_chain(&format!("rv:{}", g.key), &g.cover, COVER_W, COVER_H, window.scale_factor());
        let cover = if g.cover.is_empty() {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(COVER_W))
                .h(px(COVER_H))
                .rounded(px(8.))
                .bg(theme::ink_a(0.05))
                .border_1()
                .border_color(theme::ink_a(0.08))
                .child(icon(Icon::SportsEsports, px(34.), theme::text_dim()))
        } else {
            cover_box(src, COVER_W, COVER_H).rounded(px(8.))
        };
        let why = match self.reason {
            Some(r) => words::review_why(r).to_string(),
            None if self.loading => tr!("Asking the sources\u{2026}").to_string(),
            None => String::new(),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(220.))
            .gap(px(6.))
            .child(cover)
            .child(
                div()
                    .mt(px(10.))
                    .text_size(px(16.))
                    .line_height(px(21.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_bright())
                    .child(g.item.title.clone()),
            )
            .when(!g.origin.is_empty(), |d| d.child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(g.origin.clone())))
            .when(!why.is_empty(), |d| {
                d.child(div().mt(px(14.)).text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Why you are asked")))
                    .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(why))
            })
    }
}
