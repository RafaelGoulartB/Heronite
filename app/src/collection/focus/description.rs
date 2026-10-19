//! The overview text: HTML blocks as styled text and sized images.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Display size of a description image: its HTML size, shrunk to the column.
pub fn html_image_size(w: u32, h: u32, column: f32) -> (f32, f32) {
    let (w, h) = if w == 0 || h == 0 { (column, column * 9. / 16.) } else { (w as f32, h as f32) };
    let scale = (column / w).min(1.);
    (w * scale, h * scale)
}

pub(super) fn render_html(blocks: &[crate::collection::html::Block], images: &dyn Fn(&str) -> Option<ImageSource>, column: f32) -> Div {
    use crate::collection::html::{Block, Span};
    let text_of = |spans: &[Span]| -> StyledText {
        let mut text = String::new();
        let mut highlights = Vec::new();
        for s in spans {
            let start = text.len();
            text.push_str(&s.text);
            if s.bold || s.italic {
                highlights.push((
                    start..text.len(),
                    HighlightStyle {
                        font_weight: s.bold.then_some(theme::W_BOLD),
                        font_style: s.italic.then_some(FontStyle::Italic),
                        ..Default::default()
                    },
                ));
            }
        }
        StyledText::new(text).with_highlights(highlights)
    };
    let mut out = div().flex().flex_col().text_size(px(15.2)).line_height(px(15.2 * 1.65)).text_color(theme::white_a(0.92));
    for (i, b) in blocks.iter().enumerate() {
        out = out.child(match b {
            Block::Heading(spans) => div()
                .when(i > 0, |d| d.mt(px(18.4)))
                .mb(px(7.2))
                .font_family(theme::font_heading())
                .text_size(px(16.32))
                .font_weight(theme::W_BOLD)
                .text_color(white())
                .child(text_of(spans))
                .into_any_element(),
            Block::Para(spans) => div().mb(px(12.)).child(text_of(spans)).into_any_element(),
            Block::ListItem(spans) => div()
                .flex()
                .gap(px(8.))
                .pl(px(6.))
                .mb(px(4.))
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w_0().child(text_of(spans)))
                .into_any_element(),
            Block::Image { url, width, height } => {
                let (w, h) = html_image_size(*width, *height, column);
                div()
                    .mt(px(13.6))
                    .mb(px(17.6))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .children(images(url).map(|src| img(src).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover)))
                    .into_any_element()
            }
        });
    }
    out
}
