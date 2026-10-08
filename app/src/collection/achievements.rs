//! Achievements panel of the focus panel's side column
//! (`collectionFocus__panel` with `collectionFocus__achievements`): a title
//! with `unlocked/total`, then up to 8 rows (unlocked first), locked ones
//! dimmed.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_metadata::achievements::{panel_rows, Achievement};

use super::focus::{h3, panel};
use crate::theme;

/// One row ready to draw.
pub struct Row {
    pub name: SharedString,
    pub unlocked: bool,
    pub image: Option<ImageSource>,
    /// Image URL, so the caller can request the texture.
    pub image_url: String,
}

/// The rows of the panel and the `unlocked/total` counters.
pub fn rows(items: &[Achievement]) -> (usize, Vec<Row>) {
    let (unlocked, shown) = panel_rows(items);
    let rows = shown
        .into_iter()
        .map(|a| Row {
            name: a.display_name().to_string().into(),
            unlocked: a.unlocked(),
            image: None,
            image_url: a.image_url().to_string(),
        })
        .collect();
    (unlocked, rows)
}

pub fn view(unlocked: usize, total: usize, rows: Vec<Row>) -> AnyElement {
    let mut list = div().flex().flex_col().gap(px(6.));
    for row in rows {
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.8))
                .min_w_0()
                .text_size(px(13.33))
                .text_color(white())
                .opacity(if row.unlocked { 1. } else { 0.55 })
                .child(
                    div()
                        .size(px(28.))
                        .flex_none()
                        .rounded(px(4.))
                        .overflow_hidden()
                        .bg(theme::white_a(0.06))
                        .children(row.image.map(|src| img(src).size_full().rounded(px(4.)).object_fit(ObjectFit::Cover))),
                )
                .child(div().min_w_0().truncate().child(row.name)),
        );
    }
    panel(false)
        .child(
            h3("Achievements")
                .flex()
                .justify_between()
                .gap(px(12.8))
                .child(
                    div()
                        .text_size(px(13.6))
                        .font_weight(theme::W_500)
                        .text_color(theme::white_a(0.62))
                        .child(format!("{unlocked}/{total}")),
                ),
        )
        .child(list)
        .into_any_element()
}
