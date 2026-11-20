//! "Which one is it?": a radio card per game the sources proposed (its
//! picture, name, year, its name in another script and platform, the
//! sources that agree on it and a
//! pill when one found it by id or hash), then the search of every source by
//! another name. While the game's proposals load, a busy line; a load that
//! failed (offline) says so plainly.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::MatchChoice;

use super::MetadataReview;
use crate::dialogs::metadata_update::words;
use crate::dialogs::pick_rom::radio;
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::services::widgets::cover_box;
use crate::theme;
use crate::ui::fields::search_box;
use crate::ui::icon;

const THUMB_W: f32 = 44.;
const THUMB_H: f32 = 60.;

impl MetadataReview {
    fn choice_card(&mut self, i: usize, c: &MatchChoice, scale: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let on = self.picked == Some(i);
        let thumb = match c.thumb_url.as_deref() {
            Some(url) => cover_box(self.covers.get(Some(url), THUMB_W, THUMB_H, scale), THUMB_W, THUMB_H).rounded(px(5.)),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(THUMB_W))
                .h(px(THUMB_H))
                .rounded(px(5.))
                .border_1()
                .border_color(theme::ink_a(0.1))
                .bg(theme::ink_a(0.04))
                .child(icon(Icon::Image, px(18.), theme::text_dim())),
        };
        let line = [c.year.map(|y| y.to_string()), c.alt_title.clone(), c.platform.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let service = self.hub.read(cx).service.clone();
        let marks = c.hits.iter().map(|h| {
            let name = words::source_name(&service, &h.source);
            div()
                .id(SharedString::from(format!("rv-mark-{i}-{}", h.source)))
                .tooltip(crate::ui::tooltip(name.clone()))
                .child(labels::mark(&h.source, &name, 18.))
        });
        let n = c.hits.len();
        let agree = trn!(n, "{n} source found it", "{n} sources agree");
        let sure = c.hits.iter().filter_map(|h| h.matched_by).find_map(words::sure_pill);
        div()
            .id(("rv-choice", i))
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(theme::R_TILE))
            .border_1()
            .cursor_pointer()
            .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
            .when(!on, |d| d.bg(theme::card_fill()).border_color(theme::card_border()).hover(|s| s.bg(theme::ink_a(0.06))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.picked = Some(i);
                cx.notify();
            }))
            .child(radio(on))
            .child(thumb)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(div().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(c.title.clone().unwrap_or_default()))
                    .when(!line.is_empty(), |d| d.child(div().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(line)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .mt(px(2.))
                            .children(marks)
                            .child(div().ml(px(4.)).text_size(px(11.5)).text_color(theme::text_dim()).child(agree)),
                    ),
            )
            .children(sure.map(|text| {
                let ok: Hsla = theme::success().into();
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_full()
                    .bg(Hsla { a: 0.14, ..ok })
                    .text_size(px(11.5))
                    .text_color(theme::readable(ok))
                    .child(text)
            }))
    }

    /// A line in the place of the cards (busy, failed, nothing proposed).
    fn notice(text: String, busy: bool) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .py(px(14.))
            .rounded(px(theme::R_TILE))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .when(busy, |d| d.child(icon(Icon::FaSpinner, px(13.), theme::accent())))
            .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text))
    }

    pub(super) fn render_choices(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let scale = window.scale_factor();
        let mut list = div().id("rv-choices").flex().flex_col().gap(px(8.)).flex_1().min_h_0().overflow_y_scroll();
        if self.loading {
            let game = self.game().is_some_and(|g| !g.queued);
            let text =
                if game { tr!("Asking the sources about this game\u{2026}") } else { tr!("Reading what the sources proposed\u{2026}") };
            list = list.child(Self::notice(text.to_string(), true));
        } else if let Some(err) = self.load_error.clone() {
            list = list.child(Self::notice(err, false));
        } else if self.searching {
            list = list.child(Self::notice(tr!("Searching every source\u{2026}").to_string(), true));
        } else if self.choices.is_empty() {
            let text = if self.search_note.is_some() {
                String::new()
            } else {
                tr!("No source found a game for this name. Search another name, or say it is in no source.").to_string()
            };
            if !text.is_empty() {
                list = list.child(Self::notice(text, false));
            }
        } else {
            let choices = self.choices.clone();
            for (i, c) in choices.iter().enumerate() {
                list = list.child(self.choice_card(i, c, scale, cx));
            }
        }
        let note = self.search_note.clone().map(|n| div().text_size(px(12.)).text_color(theme::caution_text()).child(n));
        let error = self.error.clone().map(|e| {
            div()
                .px(px(14.))
                .py(px(9.))
                .rounded(px(theme::R_INPUT))
                .bg(theme::danger_fill())
                .text_size(px(12.5))
                .text_color(theme::danger_text())
                .child(e)
        });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(10.))
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("Which one is it?")))
            .child(list)
            .child(search_box(&self.search).flex_none().w_full())
            .children(note)
            .children(error)
    }
}
