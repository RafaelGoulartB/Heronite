//! Minimize, maximize and close in the app's own style, at the right end of
//! the top bars, where the system draws no title bar: on Windows the window
//! has `titlebar: None`, so GPUI hides the system's.
//!
//! They are window control areas, not click handlers: Windows itself
//! minimizes, maximizes or restores (and offers its snap layouts), and
//! closing goes through the window's should-close check like Alt+F4. The
//! empty part of a top bar moves the window and a double click on it
//! maximizes ([`drag_area`]).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::icon;
use crate::icons::Icon;
use crate::theme;

/// Whether the app draws the window's controls: on Windows, outside
/// fullscreen.
pub fn shown(window: &Window) -> bool {
    cfg!(windows) && !window.is_fullscreen()
}

/// The three buttons, for the right end of a top bar.
pub fn controls(window: &Window) -> Option<Div> {
    if !shown(window) {
        return None;
    }
    let (max_glyph, max_tip) = if window.is_maximized() { (restore(), tr!("Restore")) } else { (maximize(), tr!("Maximize")) };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .ml(px(6.))
            .child(button("win-minimize", WindowControlArea::Min, minimize(), tr!("Minimize"), theme::ink_a(0.08)))
            .child(button("win-maximize", WindowControlArea::Max, max_glyph, max_tip, theme::ink_a(0.08)))
            .child(button(
                "win-close",
                WindowControlArea::Close,
                icon(Icon::Close, theme::TOOL_ICON, theme::tool_icon()),
                tr!("Close"),
                theme::danger_fill_hover(),
            )),
    )
}

/// `el` moves the window when dragged, where the app draws the controls.
pub fn drag_area<E: InteractiveElement>(el: E, window: &Window) -> E {
    if shown(window) {
        el.window_control_area(WindowControlArea::Drag)
    } else {
        el
    }
}

/// The header's tool button (`super::tool_button_base`), with its own
/// hover colour.
fn button(id: &'static str, area: WindowControlArea, glyph: impl IntoElement, tip: &'static str, hover: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(theme::TOOL_BTN)
        .rounded(px(theme::R_ITEM))
        .hover(move |s| s.bg(hover))
        .window_control_area(area)
        .tooltip(super::tooltip(tip))
        .child(glyph)
}

/// The glyphs, drawn in the tool icons' colour and weight.
fn line(w: f32, h: f32) -> Div {
    div().w(px(w)).h(px(h)).rounded(px(1.)).bg(theme::tool_icon())
}

fn square(size: f32) -> Div {
    div().size(px(size)).rounded(px(2.5)).border(px(1.6)).border_color(theme::tool_icon())
}

fn minimize() -> Div {
    line(12., 1.6)
}

fn maximize() -> Div {
    square(12.)
}

/// A square with the top and right edges of another behind it.
fn restore() -> Div {
    div()
        .relative()
        .size(px(13.))
        .child(square(9.5).absolute().bottom_0().left_0())
        .child(line(9.5, 1.6).absolute().top_0().right_0())
        .child(line(1.6, 9.5).absolute().top_0().right_0())
}
