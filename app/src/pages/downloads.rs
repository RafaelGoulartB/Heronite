//! Downloads page (placeholder until implemented).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::theme;

pub struct DownloadsPage {
    #[allow(dead_code)]
    ctx: PageCtx,
}

impl EventEmitter<PageEvent> for DownloadsPage {}

impl DownloadsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { ctx }
    }
}

impl Render for DownloadsPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        page_body("page-downloads").child(
            page_column().child(div().text_size(px(15.2)).text_color(theme::white_a(0.6)).child("Coming soon")),
        )
    }
}
