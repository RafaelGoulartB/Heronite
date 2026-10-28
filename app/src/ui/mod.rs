//! Small styled building blocks shared by the Collection screens.

pub mod achievements;
pub mod colour_picker;
pub mod controls;
pub mod dialog;
pub mod dropdown;
pub mod fields;
pub mod find;
pub mod game_actions;
pub mod glass;
pub mod lsfg;
pub mod ludusavi;
mod scrollbar;

pub use scrollbar::{scrollbar_draggable, NoDragView};

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;

/// Opens `target` (a web page, a `steam://` link) with the desktop's handler,
/// without waiting.
pub fn open_external(target: &str) {
    if let Err(e) = std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        log::warn!("xdg-open {target}: {e}");
    }
}

/// Monochrome icon tinted with `color`.
pub fn icon(icon: Icon, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg().path(icon.path()).size(size).flex_none().text_color(color.into())
}

/// Empty header tool button: 32×32, radius 10, no fill until hovered or
/// `active` (ink .08). The caller adds the icon.
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
        .when(active, |d| d.bg(theme::ink_a(0.08)))
        .hover(|s| s.bg(theme::ink_a(0.08)))
}

/// Header tool button: [`tool_button_base`] with an icon at .55 ink
/// (bright while `active`).
pub fn tool_button(id: impl Into<ElementId>, ic: Icon, active: bool) -> Stateful<Div> {
    let color = if active { theme::text_bright() } else { theme::tool_icon() };
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
        .bg(theme::round_fill(bg_alpha))
        .hover(|s| s.bg(theme::round_fill_hover()))
        .child(icon(ic, px(diameter * 0.52), theme::round_icon()))
}

/// Pill chip (`.collectionFocus__chip`).
pub fn chip(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.72))
        .py(px(1.92))
        .rounded(px(4.))
        .bg(theme::focus_chip_fill())
        .text_color(theme::text_bright())
        .text_size(px(12.))
        .line_height(px(16.2))
        .child(text.into())
}

/// Hover text for `.tooltip(..)` of a stateful element: a small glass box,
/// 320px wide at most, shown after GPUI's hover delay.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        glass::glass_menu_surface(theme::R_ITEM)
            .max_w(px(320.))
            .px(px(10.))
            .py(px(7.))
            .text_size(px(12.5))
            .line_height(px(17.))
            .text_color(theme::text_primary())
            .child(self.0.clone())
    }
}
