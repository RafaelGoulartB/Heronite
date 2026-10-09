//! Look of console mode (proposal "2 · Console", `o2-kit.jpg`): big type,
//! one focused row at a time drawn as a light pill, On / Off pills,
//! `‹ value ›` choices, coloured controller glyphs and a hint bar.
//!
//! Every size goes through [`u`], which scales the 1280×800 design to the
//! window height ([`set_scale`] runs once per frame), so a TV reads like the
//! mock-up.

use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Btn, Hint};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

/// Scales the design to the window: `height / 800`, between 1 and 2.
pub fn set_scale(window: &Window) {
    let h = f32::from(window.viewport_size().height);
    SCALE.store((h / 800.).clamp(1., 2.).to_bits(), Ordering::Relaxed);
}

pub fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// A design size (px at 1280×800) for this window.
pub fn u(v: f32) -> Pixels {
    px(v * scale())
}

/// Puts child `ix` of a scroll area at its top, from the child positions of
/// the last frame (GPUI's `scroll_to_top_of_item` misplaces it once the
/// area has scrolled). `false` before the first layout: try next frame.
pub fn scroll_to_top(handle: &ScrollHandle, ix: usize) -> bool {
    if ix == 0 {
        handle.set_offset(point(handle.offset().x, px(0.)));
        return true;
    }
    match (handle.bounds_for_item(0), handle.bounds_for_item(ix)) {
        (Some(first), Some(item)) => {
            let max = handle.max_offset().y;
            let y = (item.top() - first.top()).min(max);
            handle.set_offset(point(handle.offset().x, -y));
            true
        }
        _ => false,
    }
}

/// Soft drop shadow (`y` and `blur` in design px).
pub fn shadow(alpha: f32, y: f32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color: theme::black_a(alpha), offset: point(px(0.), u(y)), blur_radius: u(blur), spread_radius: px(0.), inset: false }]
}

/// Letter-spaced small caps title ("HERONITE", "SETTINGS").
pub fn wordmark(text: &str) -> Div {
    let spaced: String = text.chars().flat_map(|c| [c, '\u{2009}']).collect();
    div()
        .font_family(theme::FONT_HEADING)
        .font_weight(theme::W_BOLD)
        .text_size(u(14.))
        .text_color(theme::text_muted())
        .child(spaced.trim_end().to_string())
}

/// Small caps kicker above a title ("GAME SETTINGS").
pub fn kicker(text: &str) -> Div {
    div().text_size(u(12.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(text.to_uppercase())
}

/// Rubik title.
pub fn title(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .font_family(theme::FONT_HEADING)
        .font_weight(theme::W_BOLD)
        .text_size(u(size))
        .line_height(u(size * 1.15))
        .text_color(theme::text_primary())
        .child(text.into())
}

pub fn body_text(text: impl Into<SharedString>) -> Div {
    div().text_size(u(15.)).line_height(u(23.)).text_color(theme::text_muted()).child(text.into())
}

/// Controller glyph: coloured circle for A/B/X/Y, dark keycap for the rest.
pub fn glyph(btn: Btn) -> Div {
    let circle = |letter: &str, color: Rgba| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(u(22.))
            .rounded_full()
            .bg(color)
            .text_size(u(11.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::console_on_focus())
            .child(letter.to_string())
    };
    let cap = |label: &str| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(u(22.))
            .min_w(u(26.))
            .px(u(5.))
            .rounded(u(5.))
            .bg(theme::white_a(0.16))
            .text_size(u(10.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_primary())
            .child(label.to_string())
    };
    match btn {
        Btn::A => circle("A", theme::console_btn_a()),
        Btn::B => circle("B", theme::console_btn_b()),
        Btn::X => circle("X", theme::console_btn_x()),
        Btn::Y => circle("Y", theme::console_btn_y()),
        Btn::LB => cap("LB"),
        Btn::RB => cap("RB"),
        Btn::LT => cap("LT"),
        Btn::Start => cap("\u{2630}"),
        Btn::Bumpers => div().flex().gap(u(4.)).child(cap("LB")).child(cap("RB")),
        Btn::LeftRight => div().flex().gap(u(4.)).child(cap("\u{25c0}")).child(cap("\u{25b6}")),
    }
}

/// Footer with the button hints of the current screen.
pub fn hint_bar(hints: &[Hint]) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(u(26.))
        .h(u(60.))
        .px(u(46.))
        .border_t_1()
        .border_color(theme::white_a(0.08))
        .bg(theme::black_a(0.35))
        .children(hints.iter().map(|h| {
            div()
                .flex()
                .items_center()
                .gap(u(8.))
                .text_size(u(14.))
                .text_color(theme::text_muted())
                .child(glyph(h.btn))
                .child(h.label.clone())
        }))
}

/// Base of a focusable row: transparent, or the light pill when focused.
pub fn row_base(focused: bool) -> Div {
    let d = div().flex().items_center().gap(u(16.)).min_h(u(56.)).px(u(22.)).rounded(u(14.));
    if focused {
        d.bg(theme::console_focus())
            .text_color(theme::console_on_focus())
            .border_2()
            .border_color(theme::white_a(0.55))
            .shadow(shadow(0.45, 8., 24.))
    } else {
        d.text_color(theme::text_primary()).border_2().border_color(transparent_black())
    }
}

/// "On" (green) / "Off" (grey) pill.
pub fn toggle_pill(on: bool, focused: bool) -> Div {
    let d = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(u(64.))
        .h(u(30.))
        .rounded_full()
        .text_size(u(14.))
        .font_weight(theme::W_BOLD);
    if on {
        d.bg(theme::console_on())
            .text_color(theme::console_on_focus())
            .when(focused, |d| d.border_2().border_color(theme::console_on_focus()))
            .child("On")
    } else {
        d.bg(if focused { theme::black_a(0.12) } else { theme::white_a(0.1) })
            .text_color(if focused { theme::console_on_focus() } else { theme::text_muted() })
            .child("Off")
    }
}

/// `‹ value ›` (arrows dimmed at the ends).
pub fn choice(value: impl Into<SharedString>, focused: bool, can_left: bool, can_right: bool) -> Div {
    let color: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_primary().into() };
    let arrow = |ic: Icon, on: bool| icon(ic, u(18.), if on { color } else { Hsla { a: 0.25, ..color } });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(u(14.))
        .child(arrow(Icon::ChevronLeft, can_left))
        .child(
            div()
                .min_w(u(150.))
                .max_w(u(360.))
                .flex()
                .justify_center()
                .truncate()
                .text_size(u(15.))
                .font_weight(theme::W_BOLD)
                .text_color(color)
                .child(value.into()),
        )
        .child(arrow(Icon::ChevronRight, can_right))
}

/// Tabs switched with LB / RB; the active one is white and underlined.
pub fn tabs(items: &[SharedString], active: usize) -> Div {
    div()
        .flex()
        .items_center()
        .gap(u(26.))
        .child(glyph(Btn::LB))
        .children(items.iter().enumerate().map(|(i, t)| {
            let on = i == active;
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(u(4.))
                .child(
                    div()
                        .text_size(u(17.))
                        .text_color(if on { white() } else { theme::text_muted().into() })
                        .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                        .whitespace_nowrap()
                        .child(t.clone()),
                )
                .child(div().h(u(3.)).w(u(28.)).rounded_full().bg(if on { white() } else { transparent_black() }))
        }))
        .child(glyph(Btn::RB))
}

/// "21:04".
pub fn clock() -> Div {
    div()
        .text_size(u(16.))
        .text_color(theme::text_primary())
        .child(chrono::Local::now().format("%H:%M").to_string())
}

/// Section caption inside a list ("Display", "Controls").
pub fn caption(text: impl Into<SharedString>) -> Div {
    div().pt(u(14.)).pb(u(4.)).px(u(22.)).text_size(u(13.)).font_weight(theme::W_BOLD).text_color(theme::text_dim()).child(text.into())
}

/// Status / info pill ("1 active", "Installed").
pub fn badge(text: impl Into<SharedString>, color: Rgba) -> Div {
    let c: Hsla = color.into();
    div()
        .flex_none()
        .px(u(10.))
        .py(u(3.))
        .rounded_full()
        .bg(c)
        .text_size(u(12.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::console_on_focus())
        .child(text.into())
}

/// Full-window dim behind an overlay.
pub fn scrim() -> Div {
    div().absolute().inset_0().bg(theme::black_a(0.62))
}

/// Centred empty state.
pub fn empty(ic: Icon, title_text: impl Into<SharedString>, body: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .py(u(60.))
        .child(icon(ic, u(40.), theme::text_dim()))
        .child(title(title_text, 22.))
        .child(body_text(body))
}

/// Centred modal panel over a dim (sheets, console dialogs).
pub fn modal(heading: impl Into<SharedString>, width: f32, body: impl IntoElement) -> Div {
    div().absolute().inset_0().child(scrim()).child(
        div().absolute().inset_0().flex().items_center().justify_center().pb(u(60.)).child(
            div()
                .w(u(width))
                .max_w(relative(0.94))
                .max_h(relative(0.86))
                .flex()
                .flex_col()
                .gap(u(14.))
                .p(u(26.))
                .rounded(u(24.))
                .bg(theme::console_sidebar())
                .border_1()
                .border_color(theme::white_a(0.1))
                .child(title(heading, 26.))
                .child(div().flex().flex_col().flex_1().min_h_0().gap(u(14.)).child(body)),
        ),
    )
}

/// Draws `child` over the whole window, whatever the caller's bounds (for
/// overlays of views embedded in a smaller area, like a settings panel).
pub fn window_layer(window: &Window, child: impl IntoElement) -> AnyElement {
    let size = window.viewport_size();
    deferred(anchored().position(point(px(0.), px(0.))).child(div().relative().w(size.width).h(size.height).child(child)))
        .with_priority(5)
        .into_any_element()
}
