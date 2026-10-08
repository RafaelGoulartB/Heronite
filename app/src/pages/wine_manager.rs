//! Wine Manager page (placeholder until implemented).

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::theme;

pub struct WineManagerPage {
    #[allow(dead_code)]
    ctx: PageCtx,
}

impl EventEmitter<PageEvent> for WineManagerPage {}

impl WineManagerPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { ctx }
    }
}

impl Render for WineManagerPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        page_body("page-wine_manager").child(
            page_column().child(div().text_size(px(15.2)).text_color(theme::white_a(0.6)).child("Coming soon")),
        )
    }
}
