//! Spotlight's top bar: the tabs as names between LB and RB, the current one
//! in a pill, centred over the screen, and a large clock at the right. Both
//! step away while the page has risen (`lift`, 0..=1), so the page has the
//! whole window.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::console::kit::{self, u};
use crate::console::pointer::{self, Clickable};
use crate::console::{look, motion, Btn, Pad};
use crate::theme;

/// The tabs, centred in the top bar (it is `relative`).
pub(in crate::collection::console) fn spotlight_tabs(names: &[SharedString], active: usize, lift: f32) -> Div {
    spotlight_tabs_in(names, active, lift, theme::text_muted().into())
}

/// [`spotlight_tabs`] with the tabs that are not current in `idle`.
pub(in crate::collection::console) fn spotlight_tabs_in(names: &[SharedString], active: usize, lift: f32, idle: Hsla) -> Div {
    let tabs = div()
        .flex()
        .items_center()
        .gap(u(4.))
        .child(kit::glyph(Btn::LB).mr(u(10.)).on_pads([Pad::LB]))
        .children(names.iter().enumerate().map(|(i, name)| {
            let on = i == active;
            let label = name.clone();
            let id = SharedString::from(format!("spot-tab-{label}"));
            let tab = div().relative().on_pads(pointer::along(active, i, Pad::LB, Pad::RB));
            motion::spring(id, tab, motion::SOFT, on, move |d, k| {
                let k = k.clamp(0., 1.);
                let focus: Hsla = theme::console_focus().into();
                d.child(div().absolute().inset_0().rounded_full().bg(Hsla { a: focus.a * k, ..focus })).child(
                    div()
                        .relative()
                        .px(u(15.))
                        .py(u(5.))
                        .text_size(u(14.5))
                        .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                        .text_color(motion::mix(idle, theme::console_on_focus(), k))
                        .whitespace_nowrap()
                        .child(label),
                )
            })
        }))
        .child(kit::glyph(Btn::RB).ml(u(10.)).on_pads([Pad::RB]));
    let shown = 1. - lift.clamp(0., 1.);
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .flex()
        .items_center()
        .justify_center()
        .opacity(shown)
        .when(shown > 0.05, |d| d.child(tabs))
}

/// The clock, larger than the other layouts' (nothing when the theme hides it).
pub(in crate::collection::console) fn spotlight_clock(lift: f32) -> Div {
    div().text_size(u(23.)).text_color(kit::strong()).opacity(1. - lift.clamp(0., 1.)).children(look::clock_text(chrono::Local::now()))
}
