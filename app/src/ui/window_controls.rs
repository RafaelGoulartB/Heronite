//! Minimize, maximize and close in the app's own style, at the right end of
//! the top bars, where the system draws no title bar: the window has
//! `titlebar: None` (Windows) and asks a Linux compositor to leave its frame
//! to the app. A compositor that keeps drawing its own (X11 without a
//! compositor, one that ignores the request) gets no buttons from the app.
//!
//! On Windows they are window control areas: Windows itself minimizes,
//! maximizes or restores (and offers its snap layouts), and closing goes
//! through the window's should-close check like Alt+F4. Linux has no such
//! areas, so the buttons ask the compositor, and close asks first like the
//! compositor's close. There the buttons follow the desktop: those of its
//! button layout (GNOME shows only close) that the compositor supports;
//! close is always there.
//!
//! The empty part of a top bar moves the window and a double click on it
//! maximizes ([`drag_area`]); on Linux a right click opens the window menu.

use std::cell::Cell;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::icon;
use crate::icons::Icon;
use crate::theme;

/// Whether the app draws the window's controls: on Windows, and on Linux
/// when the compositor left the frame to the app; never in fullscreen.
pub fn shown(window: &Window) -> bool {
    let own_frame = cfg!(windows) || (cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. }));
    own_frame && !window.is_fullscreen()
}

/// Which of minimize and maximize show (close always does).
#[cfg(target_os = "linux")]
fn wanted(window: &Window, cx: &App) -> (bool, bool) {
    let layout = cx.button_layout().unwrap_or_else(WindowButtonLayout::linux_default);
    let listed = |b| layout.left.iter().chain(&layout.right).any(|x| *x == Some(b));
    let can = window.window_controls();
    (listed(WindowButton::Minimize) && can.minimize, listed(WindowButton::Maximize) && can.maximize)
}

#[cfg(not(target_os = "linux"))]
fn wanted(_: &Window, _: &App) -> (bool, bool) {
    (true, true)
}

/// The buttons, for the right end of a top bar, in Windows' order.
pub fn controls(window: &Window, cx: &App) -> Option<Div> {
    if !shown(window) {
        return None;
    }
    let (min, max) = wanted(window, cx);
    let (max_glyph, max_tip) = if window.is_maximized() { (restore(), tr!("Restore")) } else { (maximize(), tr!("Maximize")) };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .ml(px(6.))
            .when(min, |d| {
                d.child(button("win-minimize", WindowControlArea::Min, minimize(), tr!("Minimize"), theme::ink_a(0.08), |w, _| {
                    w.minimize_window()
                }))
            })
            .when(max, |d| {
                d.child(button("win-maximize", WindowControlArea::Max, max_glyph, max_tip, theme::ink_a(0.08), |w, _| w.zoom_window()))
            })
            .child(button(
                "win-close",
                WindowControlArea::Close,
                icon(Icon::Close, theme::TOOL_ICON, theme::tool_icon()),
                tr!("Close"),
                theme::danger_fill_hover(),
                crate::system::close_window,
            )),
    )
}

thread_local! {
    /// A left press on a drag area that has not moved yet: the first motion
    /// hands the window to the compositor (a press alone stays a click, so a
    /// double click can maximize).
    static MOVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// `el` moves the window when dragged, where the app draws the controls.
pub fn drag_area<E: InteractiveElement>(el: E, window: &Window) -> E {
    if !shown(window) {
        return el;
    }
    if cfg!(windows) {
        return el.window_control_area(WindowControlArea::Drag);
    }
    el.on_mouse_down(MouseButton::Left, |ev, window, _| {
        MOVE_PENDING.set(ev.click_count < 2);
        if ev.click_count == 2 && window.window_controls().maximize {
            window.zoom_window();
        }
    })
    .on_mouse_up(MouseButton::Left, |_, _, _| MOVE_PENDING.set(false))
    .on_mouse_move(|ev, window, _| {
        if ev.pressed_button == Some(MouseButton::Left) && MOVE_PENDING.replace(false) {
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |ev, window, _| {
        if window.window_controls().window_menu {
            window.show_window_menu(ev.position);
        }
    })
}

/// The header's tool button (`super::tool_button_base`), with its own
/// hover colour. On Windows the system acts on `area`; elsewhere a click
/// runs `act`.
fn button(
    id: &'static str,
    area: WindowControlArea,
    glyph: impl IntoElement,
    tip: &'static str,
    hover: Hsla,
    act: fn(&mut Window, &mut App),
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(theme::TOOL_BTN)
        .rounded(px(theme::R_ITEM))
        .hover(move |s| s.bg(hover))
        .when(cfg!(windows), |d| d.window_control_area(area))
        .when(!cfg!(windows), |d| d.on_click(move |_, window, cx| act(window, cx)))
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
