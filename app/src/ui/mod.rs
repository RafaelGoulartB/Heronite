//! Small styled building blocks shared by the Collection screens.

pub mod controls;
pub mod dialog;
pub mod dropdown;
pub mod fields;
pub mod glass;
pub mod ludusavi;
mod scrollbar;

pub use scrollbar::scrollbar_draggable;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;

/// Monochrome icon tinted with `color`.
pub fn icon(icon: Icon, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg().path(icon.path()).size(size).flex_none().text_color(color.into())
}

/// Empty header tool button: 32×32, radius 10, no fill until hovered or
/// `active` (white .08). The caller adds the icon.
pub fn tool_button_base(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(theme::TOOL_BTN)
        .rounded(px(theme::R_ITEM))
        .cursor_pointer()
        .when(active, |d| d.bg(theme::white_a(0.08)))
        .hover(|s| s.bg(theme::white_a(0.08)))
}

/// Header tool button: [`tool_button_base`] with an icon at .55 white
/// (white while `active`).
pub fn tool_button(id: impl Into<ElementId>, ic: Icon, active: bool) -> Stateful<Div> {
    let color = if active { theme::white_a(1.) } else { theme::tool_icon() };
    tool_button_base(id, active).child(icon(ic, theme::TOOL_ICON, color))
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
