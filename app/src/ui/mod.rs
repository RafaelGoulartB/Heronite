//! Small styled building blocks shared by the Collection screens.

pub mod dropdown;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;

/// Monochrome icon tinted with `color`.
pub fn icon(icon: Icon, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg().path(icon.path()).size(size).flex_none().text_color(color.into())
}

/// Header tool button: 32×32, radius 4, icon .58 white, hover white on .08.
pub fn tool_button(id: impl Into<ElementId>, ic: Icon, active: bool) -> Stateful<Div> {
    let color = if active { theme::white_a(1.) } else { theme::white_a(0.58) };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(theme::TOOL_BTN)
        .rounded(px(4.))
        .cursor_pointer()
        .when(active, |d| d.bg(theme::white_a(0.08)))
        .hover(|s| s.bg(theme::white_a(0.08)))
        .child(icon(ic, theme::TOOL_ICON, color))
}

/// Round translucent button used in the focus panel (tools 36px, footer 42px).
pub fn round_button(id: impl Into<ElementId>, ic: Icon, diameter: f32, bg_alpha: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(diameter))
        .rounded_full()
        .cursor_pointer()
        .bg(theme::rgb_a(16, 18, 22, bg_alpha))
        .hover(|s| s.bg(theme::rgb_a(10, 12, 14, 0.9)))
        .child(icon(ic, px(diameter * 0.52), rgb(0xf4f6f8)))
}

/// Pill chip (`.collectionFocus__chip`).
pub fn chip(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.72))
        .py(px(1.92))
        .rounded(px(4.))
        .bg(theme::black_a(0.48))
        .text_color(white())
        .text_size(px(12.))
        .line_height(px(16.2))
        .child(text.into())
}
