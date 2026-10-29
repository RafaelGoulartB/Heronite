//! Release notes (`services::update::notes`) drawn as lines: Settings →
//! About's card and the "What's new" dialog.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::services::update::notes;
use crate::theme;

/// The notes, one element per line. `paragraphs`: the lines that are neither
/// headings nor list items too (the download guide, the closing line).
pub fn list(markdown: &str, paragraphs: bool) -> Div {
    let mut col = div().flex().flex_col().gap(px(4.)).py(px(12.));
    let mut after_list = false;
    for note in notes::parse(markdown) {
        if !paragraphs && matches!(note, notes::Note::Text(_)) {
            continue;
        }
        let bullet = matches!(note, notes::Note::Bullet(_));
        col = col.child(match note {
            notes::Note::Heading(t) => {
                div().pt(px(6.)).text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(t).into_any_element()
            }
            notes::Note::Bullet(t) => div()
                .flex()
                .gap(px(8.))
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(theme::text_muted())
                .child(div().flex_none().text_color(theme::text_dim()).child("\u{2022}"))
                .child(div().flex_1().min_w_0().child(t))
                .into_any_element(),
            // A paragraph after a list (the closing line) stands apart from it.
            notes::Note::Text(t) => div()
                .when(after_list, |d| d.pt(px(6.)))
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(theme::text_muted())
                .child(t)
                .into_any_element(),
        });
        after_list = bullet;
    }
    col
}
