//! Look of console mode (proposal "2 · Console", `o2-kit.jpg`): big type,
//! one focused row at a time drawn as a light pill, On / Off pills,
//! `‹ value ›` choices, coloured controller glyphs and a hint bar.
//!
//! Every size goes through [`u`], which scales the 1280×800 design to the
//! window height ([`set_scale`] runs once per frame), so a TV reads like the
//! mock-up.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::motion;
use super::{Btn, Hint};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_data::console_theme::{Glyphs, Layout};
use heronite_i18n::tr;

static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

thread_local! {
    /// Focus pills drawn this frame (keeps their animation ids apart).
    static PILLS: Cell<u32> = const { Cell::new(0) };
}

/// Scales the design to the window: `height / 800`, between 1 and 2. Runs
/// once at the start of every console frame.
pub fn set_scale(window: &Window) {
    let h = f32::from(window.viewport_size().height);
    SCALE.store(((h / 800.).clamp(1., 2.) * super::look::scale()).to_bits(), Ordering::Relaxed);
    PILLS.with(|c| c.set(0));
}

/// Sets the scale directly, for windows whose height is not the display's
/// (the in-game overlay's panel and notices).
pub fn set_scale_value(scale: f32) {
    SCALE.store(scale.clamp(1., 2.).to_bits(), Ordering::Relaxed);
    PILLS.with(|c| c.set(0));
}

pub fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// A design size (px at 1280×800) for this window.
pub fn u(v: f32) -> Pixels {
    px(v * scale())
}

/// A light console theme is on (dark text over washed-out art): the surfaces
/// and tints below flip their ink.
pub fn light() -> bool {
    super::look::light()
}

/// Ink of the console at alpha `a`: white on a dark console, near black on a
/// light one. Faint fills, borders, dividers and idle text of the surfaces;
/// what sits on a picture with its own dark gradient keeps `white()`.
pub fn ink(a: f32) -> Hsla {
    if light() {
        theme::rgba_hsla(0x15, 0x19, 0x1f, a)
    } else {
        theme::white_a(a)
    }
}

/// Text a step above the primary one on a picture or tile that is washed
/// like the console (white on a dark console, the text colour on a light one).
pub fn strong() -> Hsla {
    if light() {
        theme::text_primary().into()
    } else {
        white()
    }
}

/// Text on a fill of colour `bg` (a badge, the On pill): the focus text, or
/// near-black when the console is light and `bg` is too bright for white.
pub fn on_fill(bg: Rgba) -> Rgba {
    if light() && 0.2126 * bg.r + 0.7152 * bg.g + 0.0722 * bg.b > 0.6 {
        rgb(0x15191f)
    } else {
        theme::console_on_focus()
    }
}

/// A status or score colour as text or icon on the console: darkened on a
/// light one, where the pale colours would vanish.
pub fn tint(c: Rgba) -> Rgba {
    if light() {
        motion::mix(c, rgb(0x15191f), 0.4)
    } else {
        c
    }
}

/// A backing behind small controls (the hint bar, keys): black at `a` on a
/// dark console, white on a light one.
pub fn shade(a: f32) -> Hsla {
    if light() {
        theme::white_a((a * 1.6).min(1.))
    } else {
        theme::black_a(a)
    }
}

/// A recess in a panel (a text field, a picture's backing): black at `a` on
/// a dark console, a faint ink on a light one.
pub fn well(a: f32) -> Hsla {
    if light() {
        ink(a * 0.2)
    } else {
        theme::black_a(a)
    }
}

/// A dim over content (behind modals and menus): black at `a` on a dark
/// console, the background colour (a wash) on a light one.
pub fn dim(a: f32) -> Hsla {
    if light() {
        veil(a * 0.8)
    } else {
        theme::black_a(a)
    }
}

/// The console's background colour at alpha `a` on a light console, washing
/// out what is behind it; `black_a` on a dark one.
pub fn veil(a: f32) -> Hsla {
    if light() {
        let mut c = theme::console_bg();
        c.a = a;
        c.into()
    } else {
        theme::black_a(a)
    }
}

/// Soft drop shadow (`y` and `blur` in design px).
pub fn shadow(alpha: f32, y: f32, blur: f32) -> Vec<BoxShadow> {
    let color = if light() { ink(alpha * 0.4) } else { theme::black_a(alpha) };
    vec![BoxShadow { color, offset: point(px(0.), u(y)), blur_radius: u(blur), spread_radius: px(0.), inset: false }]
}

/// Letter-spaced small caps title ("HERONITE", "SETTINGS").
pub fn wordmark(text: &str) -> Div {
    let spaced: String = text.chars().flat_map(|c| [c, '\u{2009}']).collect();
    div()
        .font_family(heading_font())
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
        .font_family(heading_font())
        .font_weight(theme::W_BOLD)
        .text_size(u(size))
        .line_height(u(size * 1.15))
        .text_color(theme::text_primary())
        .child(text.into())
}

pub fn body_text(text: impl Into<SharedString>) -> Div {
    div().text_size(u(15.)).line_height(u(23.)).text_color(theme::text_muted()).child(text.into())
}

/// Which of the four face buttons, by position on the pad.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Face {
    South,
    East,
    West,
    North,
}

/// What a hint glyph draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mark {
    /// A letter on a round button.
    Letter(&'static str, Face),
    /// A PlayStation symbol (cross, circle, square, triangle) on a round button.
    Shape(Face),
    /// A dark keycap with a label.
    Cap(&'static str),
    /// A keycap with the menu icon.
    Menu,
    Pair(&'static str, &'static str),
    Arrows,
}

/// The button a hint stands for, by position: A confirms (south, or east when
/// the theme swaps A and B), B goes back.
pub(crate) fn face(btn: Btn, swap_ab: bool) -> Option<Face> {
    Some(match (btn, swap_ab) {
        (Btn::A, false) | (Btn::B, true) => Face::South,
        (Btn::B, false) | (Btn::A, true) => Face::East,
        (Btn::X, _) => Face::West,
        (Btn::Y, _) => Face::North,
        _ => return None,
    })
}

/// What `btn` looks like in `style` (`Auto` is resolved by the caller).
pub(crate) fn mark(style: Glyphs, btn: Btn, swap_ab: bool) -> Mark {
    if let Some(f) = face(btn, swap_ab) {
        return match style {
            Glyphs::PlayStation => Mark::Shape(f),
            // The labels follow the positions: Nintendo's south button is B.
            Glyphs::Nintendo => Mark::Letter(["B", "A", "Y", "X"][f as usize], f),
            _ => Mark::Letter(["A", "B", "X", "Y"][f as usize], f),
        };
    }
    // Xbox, Steam Deck and Auto (resolved before) share the names; the others differ.
    let names = |xbox: [&'static str; 3], ps: [&'static str; 3], nin: [&'static str; 3]| match style {
        Glyphs::PlayStation => ps,
        Glyphs::Nintendo => nin,
        Glyphs::Deck => ["L1", "R1", "L2"],
        _ => xbox,
    };
    let [lb, rb, lt] = names(["LB", "RB", "LT"], ["L1", "R1", "L2"], ["L", "R", "ZL"]);
    match btn {
        Btn::LB => Mark::Cap(lb),
        Btn::RB => Mark::Cap(rb),
        Btn::Bumpers => Mark::Pair(lb, rb),
        Btn::LT => Mark::Cap(lt),
        Btn::Start => match style {
            Glyphs::PlayStation => Mark::Cap("Options"),
            Glyphs::Nintendo => Mark::Cap("+"),
            Glyphs::Deck => Mark::Menu,
            _ => Mark::Cap("\u{2630}"),
        },
        Btn::LeftRight => Mark::Arrows,
        _ => unreachable!("face buttons are handled above"),
    }
}

/// Controller glyph in the style the console theme asks for (`Auto` follows
/// the connected controller): Xbox coloured letters, PlayStation symbols,
/// Nintendo and Steam Deck letters, or the keyboard keys.
pub fn glyph(btn: Btn) -> Div {
    let style = super::look::glyphs();
    if style == Glyphs::Keyboard {
        return key_glyph(btn);
    }
    draw(mark(style, btn, super::look::theme().controls.swap_ab), style)
}

fn draw(m: Mark, style: Glyphs) -> Div {
    match m {
        Mark::Letter(text, f) => letter_button(text, f, style),
        Mark::Shape(f) => shape_button(f),
        Mark::Cap(label) => key_cap(label),
        Mark::Menu => arrow_cap(Icon::Menu),
        Mark::Pair(a, b) => div().flex().gap(u(4.)).child(key_cap(a)).child(key_cap(b)),
        // Icons: the fonts have no ◀ ▶, and the fallback draws them as emoji.
        Mark::Arrows => div().flex().gap(u(4.)).child(arrow_cap(Icon::ChevronLeft)).child(arrow_cap(Icon::ChevronRight)),
    }
}

/// The round base of a face button.
fn round_button() -> Div {
    div().relative().flex().flex_none().items_center().justify_center().size(u(22.)).rounded_full()
}

fn letter_button(text: &str, f: Face, style: Glyphs) -> Div {
    let (bg, fg): (Hsla, Hsla) = match style {
        Glyphs::Nintendo => (rgb(0xe3e6ea).into(), rgb(0x1a1d23).into()),
        Glyphs::Deck => (ink(0.2), theme::text_primary().into()),
        _ => (
            match f {
                Face::South => theme::console_btn_a(),
                Face::East => theme::console_btn_b(),
                Face::West => theme::console_btn_x(),
                Face::North => theme::console_btn_y(),
            }
            .into(),
            theme::console_on_focus().into(),
        ),
    };
    round_button().bg(bg).text_size(u(11.)).font_weight(theme::W_BOLD).text_color(fg).child(text.to_string())
}

/// PlayStation colours of the symbols.
fn shape_color(f: Face) -> Rgba {
    match f {
        Face::South => rgb(0x7aa8ff),
        Face::East => rgb(0xff6f7d),
        Face::West => rgb(0xf4a3d4),
        Face::North => rgb(0x5fe0b4),
    }
}

/// A PlayStation button. The symbols are drawn: the fonts have no ✕ ○ □ △,
/// and the fallback draws emoji.
fn shape_button(f: Face) -> Div {
    let color: Hsla = shape_color(f).into();
    // The symbols are light: on a dark disc when the console is light.
    round_button()
        .bg(if light() { ink(0.82) } else { theme::white_a(0.16) })
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_shape(bounds, f, color, window)).absolute().inset_0())
}

fn paint_shape(bounds: Bounds<Pixels>, f: Face, color: Hsla, window: &mut Window) {
    let w = f32::from(bounds.size.width);
    let c = bounds.center();
    let at = |dx: f32, dy: f32| point(c.x + px(dx * w), c.y + px(dy * w));
    let stroke = |points: &[Point<Pixels>], closed: bool, window: &mut Window| {
        let mut b = PathBuilder::stroke(px(w * 0.1));
        for (i, p) in points.iter().enumerate() {
            if i == 0 {
                b.move_to(*p);
            } else {
                b.line_to(*p);
            }
        }
        if closed {
            b.line_to(points[0]);
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    match f {
        Face::South => {
            stroke(&[at(-0.19, -0.19), at(0.19, 0.19)], false, window);
            stroke(&[at(0.19, -0.19), at(-0.19, 0.19)], false, window);
        }
        Face::East => {
            let ring: Vec<_> = (0..32)
                .map(|i| {
                    let a = i as f32 / 32. * std::f32::consts::TAU;
                    at(0.25 * a.cos(), 0.25 * a.sin())
                })
                .collect();
            stroke(&ring, true, window);
        }
        Face::West => stroke(&[at(-0.2, -0.2), at(0.2, -0.2), at(0.2, 0.2), at(-0.2, 0.2)], true, window),
        Face::North => stroke(&[at(0., -0.27), at(0.25, 0.17), at(-0.25, 0.17)], true, window),
    }
}

/// [`glyph`] for the keyboard: the key `console::input::pad_from_key` turns
/// into that button.
pub fn key_glyph(btn: Btn) -> Div {
    match btn {
        Btn::A => key_cap("Enter"),
        Btn::B => key_cap("Esc"),
        Btn::X => key_cap("X"),
        Btn::Y => key_cap("Y"),
        Btn::LB => key_cap("Q"),
        Btn::RB => key_cap("E"),
        Btn::Bumpers => draw(Mark::Pair("Q", "E"), Glyphs::Keyboard),
        Btn::Start => key_cap("M"),
        // No key for these: the pad's own look.
        Btn::LT | Btn::LeftRight => draw(mark(Glyphs::Xbox, btn, false), Glyphs::Xbox),
    }
}

/// A dark keycap with a label.
fn key_cap(label: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(u(22.))
        .min_w(u(26.))
        .px(u(5.))
        .rounded(u(5.))
        .bg(ink(0.16))
        .text_size(u(10.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(label.to_string())
}

/// A dark keycap holding an icon.
fn arrow_cap(ic: Icon) -> Div {
    div().flex().flex_none().items_center().justify_center().h(u(22.)).min_w(u(26.)).rounded(u(5.)).bg(ink(0.16)).child(icon(
        ic,
        u(16.),
        theme::text_primary(),
    ))
}

/// Footer with the button hints of the current screen.
pub fn hint_bar(hints: &[Hint]) -> Div {
    div().flex().flex_none().items_center().gap(u(26.)).h(u(60.)).px(u(46.)).border_t_1().border_color(ink(0.08)).bg(shade(0.35)).children(
        hints.iter().map(|h| {
            div()
                .flex()
                .items_center()
                .gap(u(8.))
                .text_size(u(14.))
                .text_color(theme::text_muted())
                .child(glyph(h.btn))
                .child(h.label.clone())
        }),
    )
}

/// The light focus pill behind a row. It pops in (grows from a little
/// inset, fading up) when the focus arrives and on every press, so moving
/// and changing values both answer at once.
pub fn focus_pill(radius: f32) -> AnyElement {
    let n = PILLS.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    let edge = -2.;
    let pill = div()
        .absolute()
        .top(px(edge))
        .bottom(px(edge))
        .left(px(edge))
        .right(px(edge))
        .rounded(u(radius))
        .bg(theme::console_focus())
        .border_2()
        .border_color(theme::white_a(0.55))
        .shadow(shadow(0.45, 8., 24.));
    if !motion::on() {
        return pill.into_any_element();
    }
    let id = SharedString::from(format!("pill-{n}-{}", motion::press_epoch()));
    pill.with_animation(id, Animation::new(std::time::Duration::from_millis(200)).with_easing(ease_out_quint()), move |d, t| {
        let inset = px(edge) + u(4.) * (1. - t);
        d.opacity(0.55 + 0.45 * t).top(inset).bottom(inset).left(inset).right(inset)
    })
    .into_any_element()
}

/// Base of a focusable row: transparent, or the light pill when focused.
pub fn row_base(focused: bool) -> Div {
    let d = div()
        .relative()
        .flex()
        .items_center()
        .gap(u(16.))
        .min_h(u(56.))
        .px(u(22.))
        .rounded(u(14.))
        .border_2()
        .border_color(transparent_black());
    if focused {
        d.text_color(theme::console_on_focus()).child(focus_pill(14.))
    } else {
        d.text_color(theme::text_primary())
    }
}

/// "On" (green) / "Off" (grey) pill.
pub fn toggle_pill(on: bool, focused: bool) -> Div {
    let d = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(u(64.))
        .px(u(10.))
        .h(u(30.))
        .rounded_full()
        .text_size(u(14.))
        .font_weight(theme::W_BOLD);
    if on {
        d.bg(theme::console_on())
            .text_color(on_fill(theme::console_on()))
            .when(focused, |d| d.border_2().border_color(theme::console_on_focus()))
            .child(tr!("On"))
    } else {
        d.bg(if focused {
            if light() {
                theme::white_a(0.16)
            } else {
                theme::black_a(0.12)
            }
        } else {
            ink(0.1)
        })
        .text_color(if focused { theme::console_on_focus() } else { theme::text_muted() })
        .child(tr!("Off"))
    }
}

/// `‹ value ›` (arrows dimmed at the ends).
pub fn choice(value: impl Into<SharedString>, focused: bool, can_left: bool, can_right: bool) -> Div {
    choice_sized(value, focused, can_left, can_right, 150.)
}

/// [`choice`] whose value takes at least `min_w` design px (narrow for short
/// values, so the label keeps its room).
pub fn choice_sized(value: impl Into<SharedString>, focused: bool, can_left: bool, can_right: bool, min_w: f32) -> Div {
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
                .min_w(u(min_w))
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

/// Tabs switched with LB / RB; the active one is white and underlined
/// (the underline grows in from the centre while the old one shrinks).
pub fn tabs(items: &[SharedString], active: usize) -> Div {
    div()
        .flex()
        .items_center()
        .gap(u(26.))
        .child(glyph(Btn::LB))
        .children(items.iter().enumerate().map(|(i, t)| {
            let on = i == active;
            let label = t.clone();
            let id = SharedString::from(format!("tab-{label}"));
            motion::spring(id, div().flex().flex_col().items_center().gap(u(4.)), motion::SOFT, on, move |d, k| {
                let k = k.clamp(0., 1.);
                d.child(
                    div()
                        .text_size(u(17.))
                        .text_color(motion::mix(theme::text_muted(), strong(), k))
                        .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                        .whitespace_nowrap()
                        .child(label),
                )
                .child(div().h(u(3.)).w(u(28. * k)).rounded_full().bg(theme::console_focus()).opacity(k))
            })
        }))
        .child(glyph(Btn::RB))
}

/// The tabs as names alone, larger, without LB / RB (`tab_bar: names`).
pub fn tab_names(items: &[SharedString], active: usize) -> Div {
    div().flex().items_center().gap(u(34.)).children(items.iter().enumerate().map(|(i, t)| {
        let on = i == active;
        let label = t.clone();
        let id = SharedString::from(format!("tab-name-{label}"));
        motion::spring(id, div(), motion::SOFT, on, move |d, k| {
            d.text_size(u(22.))
                .text_color(motion::mix(theme::text_muted(), strong(), k.clamp(0., 1.)))
                .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                .whitespace_nowrap()
                .child(label)
        })
    }))
}

/// The icon of a tab: by what it shows (its `source`), else by its layout.
pub fn tab_icon(source: &str, layout: Layout) -> Icon {
    let source = source.trim();
    let kind = source.split_once(':').map_or(source, |(k, _)| k);
    match kind {
        "all" => Icon::Collections,
        "installed" => Icon::Download,
        "not-installed" => Icon::CloudDownload,
        "favourites" => Icon::Favorite,
        "pc" => Icon::Tv,
        "emulated" | "platform" => Icon::SportsEsports,
        "status" => Icon::Flag,
        "category" => Icon::Label,
        "store" => Icon::Storefront,
        _ => match layout {
            Layout::Grid => Icon::GridView,
            Layout::Dashboard => Icon::Home,
            _ => Icon::Collections,
        },
    }
}

/// Spaced capitals ("A L L  G A M E S"): GPUI has no letter spacing, so the
/// letters get thin spaces between them.
pub fn spaced_caps(text: &str) -> String {
    let words: Vec<String> =
        text.split_whitespace().map(|w| w.to_uppercase().chars().map(String::from).collect::<Vec<_>>().join("\u{2009}")).collect();
    words.join("\u{2002}")
}

/// The tabs as a bar along the bottom: an icon and a name each, the current
/// one bright, the others dim, LB / RB at the ends.
pub fn tabs_bottom(items: &[(SharedString, Icon)], active: usize) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(u(40.))
        .h(u(56.))
        .px(u(46.))
        .border_t_1()
        .border_color(ink(0.08))
        .child(glyph(Btn::LB))
        .children(items.iter().enumerate().map(|(i, (name, ic))| {
            let on = i == active;
            let (name, ic) = (name.clone(), *ic);
            let id = SharedString::from(format!("tab-bottom-{name}"));
            motion::spring(id, div().flex().items_center().gap(u(9.)), motion::SOFT, on, move |d, k| {
                let k = k.clamp(0., 1.);
                d.opacity(0.4 + 0.6 * k).child(icon(ic, u(20.), theme::text_primary())).child(
                    div()
                        .text_size(u(17.))
                        .text_color(theme::text_primary())
                        .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                        .whitespace_nowrap()
                        .child(name),
                )
            })
        }))
        .child(glyph(Btn::RB))
}

/// The current tab alone, big and centred, with LB / RB at its sides when
/// there are others (`spaced`: in spaced capitals).
pub fn tab_title(name: &str, ic: Icon, more: bool, spaced: bool) -> Div {
    let text = if spaced { spaced_caps(name) } else { name.to_string() };
    div()
        .flex()
        .items_center()
        .gap(u(18.))
        .when(more, |d| d.child(glyph(Btn::LB)))
        .child(
            div().flex().items_center().gap(u(12.)).child(icon(ic, u(26.), theme::text_primary())).child(
                div()
                    .font_family(heading_font())
                    .font_weight(theme::W_BOLD)
                    .text_size(u(28.))
                    .text_color(theme::text_primary())
                    .whitespace_nowrap()
                    .child(text),
            ),
        )
        .when(more, |d| d.child(glyph(Btn::RB)))
}

/// "21:04" (or 12 h, with the date, or nothing: the console theme says).
pub fn clock() -> Div {
    div().text_size(u(16.)).text_color(theme::text_primary()).children(super::look::clock_text(chrono::Local::now()))
}

/// Title font of console mode: the console theme's while it is on, else the app's.
pub fn heading_font() -> SharedString {
    if super::active() {
        super::look::fonts().0.into()
    } else {
        theme::font_heading().into()
    }
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
        .text_color(on_fill(color))
        .child(text.into())
}

/// Full-window dim behind an overlay (fades in).
pub fn scrim() -> AnyElement {
    motion::fade_in("scrim", div().absolute().inset_0().bg(dim(0.62)), 180)
}

/// A centred panel's entrance: fades in while rising into place.
pub fn panel_in(id: &'static str, panel: Div) -> AnyElement {
    motion::enter(id, panel.relative(), 260, 0., 18.)
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
        div().absolute().inset_0().flex().items_center().justify_center().pb(u(60.)).child(panel_in(
            "modal-panel",
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
                .border_color(ink(0.1))
                .shadow(shadow(0.5, 18., 48.))
                .child(title(heading, 26.))
                .child(div().flex().flex_col().flex_1().min_h_0().gap(u(14.)).child(body)),
        )),
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

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{mark, Face, Mark};
    use crate::console::Btn;
    use heronite_data::console_theme::Glyphs;

    #[test]
    fn xbox_and_deck_name_the_buttons_by_letter() {
        assert_eq!(mark(Glyphs::Xbox, Btn::A, false), Mark::Letter("A", Face::South));
        assert_eq!(mark(Glyphs::Deck, Btn::Y, false), Mark::Letter("Y", Face::North));
        assert_eq!(mark(Glyphs::Xbox, Btn::Bumpers, false), Mark::Pair("LB", "RB"));
        assert_eq!(mark(Glyphs::Deck, Btn::LB, false), Mark::Cap("L1"));
        assert_eq!(mark(Glyphs::Deck, Btn::Start, false), Mark::Menu);
    }

    #[test]
    fn nintendo_labels_follow_the_positions() {
        // Confirm is the south button, which Nintendo calls B.
        assert_eq!(mark(Glyphs::Nintendo, Btn::A, false), Mark::Letter("B", Face::South));
        assert_eq!(mark(Glyphs::Nintendo, Btn::B, false), Mark::Letter("A", Face::East));
        assert_eq!(mark(Glyphs::Nintendo, Btn::X, false), Mark::Letter("Y", Face::West));
        assert_eq!(mark(Glyphs::Nintendo, Btn::Y, false), Mark::Letter("X", Face::North));
        assert_eq!(mark(Glyphs::Nintendo, Btn::LT, false), Mark::Cap("ZL"));
        assert_eq!(mark(Glyphs::Nintendo, Btn::Start, false), Mark::Cap("+"));
    }

    #[test]
    fn playstation_draws_symbols_and_names_l1_r1() {
        assert_eq!(mark(Glyphs::PlayStation, Btn::A, false), Mark::Shape(Face::South));
        assert_eq!(mark(Glyphs::PlayStation, Btn::B, false), Mark::Shape(Face::East));
        assert_eq!(mark(Glyphs::PlayStation, Btn::RB, false), Mark::Cap("R1"));
        assert_eq!(mark(Glyphs::PlayStation, Btn::Start, false), Mark::Cap("Options"));
    }

    #[test]
    fn swapping_a_and_b_moves_confirm_to_the_east_button() {
        assert_eq!(mark(Glyphs::Xbox, Btn::A, true), Mark::Letter("B", Face::East));
        assert_eq!(mark(Glyphs::Xbox, Btn::B, true), Mark::Letter("A", Face::South));
        assert_eq!(mark(Glyphs::Nintendo, Btn::A, true), Mark::Letter("A", Face::East));
        assert_eq!(mark(Glyphs::PlayStation, Btn::A, true), Mark::Shape(Face::East));
        // X and Y stay where they are.
        assert_eq!(mark(Glyphs::Xbox, Btn::X, true), Mark::Letter("X", Face::West));
    }
}
