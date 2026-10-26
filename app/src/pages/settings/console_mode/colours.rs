//! The colours of a theme on the desktop: the focus colour as dots, the
//! ready-made colour sets, each colour of the theme as a swatch that opens a
//! colour picker (with its hex and a way back to the built-in one), and a
//! warning with a fix when a text colour does not read on what it sits on.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{parse_hex, Rgb};
use heronite_data::console_theme::ConsoleTheme;
use heronite_i18n::tr;

use super::form::{reset_button, text_input};
use super::{card_rows, send, SettingsPage};
use crate::console::list::{Outcome, Row, Value};
use crate::console::look::ACCENTS;
use crate::console::theme_rows::{self, Arg, Card, Command, COLOUR_SETS};
use crate::icons::Icon;
use crate::theme;
use crate::ui::colour_picker::{ColourPicker, PickerEvent};
use crate::ui::controls::{button_sm, card_box, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

fn rgba((r, g, b): Rgb) -> Rgba {
    theme::rgb_a(r, g, b, 1.)
}

/// The colour the row `id` holds now (`None`: the built-in one).
fn current(theme: &ConsoleTheme, id: &str) -> Option<Rgb> {
    let c = &theme.colors;
    parse_hex(match id {
        "interface.accent" => {
            let a = theme.interface.accent.as_str();
            return ACCENTS.iter().find(|(n, _)| *n == a).map(|(_, v)| ((v >> 16) as u8, (v >> 8) as u8, *v as u8)).or_else(|| parse_hex(a));
        }
        "colors.background" => &c.background,
        "colors.panel" => &c.panel,
        "colors.text" => &c.text,
        "colors.muted" => &c.muted,
        "colors.focus_text" => &c.focus_text,
        "colors.on" => &c.on,
        _ => return None,
    })
}

/// Opens (or closes, when it is open) the picker of the colour row `id`.
fn toggle_picker(d: &mut SettingsPage, id: &str, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(s) = d.console_mode.as_mut() else { return };
    if s.colour_open.as_deref() == Some(id) {
        s.colour_open = None;
        s.picker = None;
        cx.notify();
        return;
    }
    open_picker(d, id, window, cx);
}

/// Opens the picker of the colour row `id`.
pub(super) fn open_picker(d: &mut SettingsPage, id: &str, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(s) = d.console_mode.as_mut() else { return };
    let start = current(&s.theme.prefs.theme, id).unwrap_or((0x5b, 0x8d, 0xd6));
    let picker = cx.new(|cx| ColourPicker::new(start, window, cx));
    let target = if id == "interface.accent" { "interface.accent_hex".to_string() } else { id.to_string() };
    cx.subscribe_in(&picker, window, move |d: &mut SettingsPage, _, ev: &PickerEvent, window, cx| {
        let c = match *ev {
            PickerEvent::Change(c) | PickerEvent::Commit(c) => c,
        };
        theme_rows::text_done(d, &target, heronite_data::appearance::hex(c), window, cx);
    })
    .detach();
    s.colour_open = Some(id.to_string());
    s.picker = Some(picker);
    cx.notify();
}

/// The open picker of the row `id`, moved to the colour the row holds now
/// (a ↺, Undo or another theme changed it under the picker).
pub(super) fn open_picker_of(d: &SettingsPage, id: &str, window: &mut Window, cx: &mut Context<SettingsPage>) -> Option<Entity<ColourPicker>> {
    let s = d.console_mode.as_ref()?;
    let picker = s.picker.clone().filter(|_| s.colour_open.as_deref() == Some(id))?;
    if let Some(c) = current(&s.theme.prefs.theme, id) {
        if picker.read(cx).colour() != c {
            picker.update(cx, |p, cx| p.set(c, window, cx));
        }
    }
    Some(picker)
}

/// The open picker under a row, when it belongs to the row `id`.
fn picker_under(d: &SettingsPage, id: &str, window: &mut Window, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let picker = open_picker_of(d, id, window, cx)?;
    let close = button_sm(SharedString::from(format!("cm-{id}-picker-done")), ButtonKind::Glass, tr!("Done")).on_click(cx.listener(
        |d, _, _, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.colour_open = None;
                s.picker = None;
            }
            cx.notify();
        },
    ));
    Some(div().flex().items_end().gap(px(12.)).pb(px(12.)).child(div().w(px(300.)).flex_none().child(picker)).child(close))
}

/// A round swatch of `c`; the built-in colour (`None`) shows as a split
/// light / dark disc.
fn swatch(c: Option<Rgb>, size: f32) -> Div {
    let inner = div().size(px(size)).rounded_full().border_1().border_color(theme::ink_a(0.18));
    match c {
        Some(c) => inner.bg(rgba(c)),
        None => inner.bg(linear_gradient(
            135.,
            linear_color_stop(theme::rgb_a(238, 242, 247, 1.), 0.5),
            linear_color_stop(theme::rgb_a(21, 25, 31, 1.), 0.5),
        )),
    }
}

/// The focus colours as dots, then the theme's own colour (opens the picker).
/// `size` is the diameter of a dot with its ring.
pub(super) fn accent_dots(d: &SettingsPage, r: &Row, size: f32, cx: &mut Context<SettingsPage>) -> Div {
    let inner = size - 8.;
    let Value::Choice { options, index } = &r.value else { return div() };
    let accent = d.console_mode.as_ref().map(|s| s.theme.prefs.theme.interface.accent.clone()).unwrap_or_default();
    let open = d.console_mode.as_ref().is_some_and(|s| s.colour_open.as_deref() == Some("interface.accent"));
    let own = parse_hex(&accent);
    let mut row = div().flex().flex_wrap().items_center().gap(px(if size < 30. { 3. } else { 6. }));
    for (i, (_, c)) in ACCENTS.iter().enumerate() {
        let selected = i == *index;
        let fill: Background = if i == 1 {
            // The colour of the focused game: a blend stands for "any colour".
            linear_gradient(135., linear_color_stop(rgb(0xf7c46c), 0.), linear_color_stop(rgb(0xb9a4fb), 1.))
        } else {
            rgb(*c).into()
        };
        let options = options.clone();
        let name = options.get(i).cloned().unwrap_or_default();
        row = row.child(
            dot_frame(SharedString::from(format!("cm-accent-{i}")), selected, size)
                .tooltip(crate::ui::tooltip(name))
                .child(div().size(px(inner)).rounded_full().bg(fill).border_1().border_color(theme::ink_a(0.15)))
                .on_click(cx.listener(move |d, _, window, cx| {
                    send(d, Outcome::Changed("interface.accent".into(), Value::Choice { options: options.clone(), index: i }), window, cx);
                })),
        );
    }
    let custom = dot_frame("cm-accent-own", own.is_some() || open, size)
        .tooltip(crate::ui::tooltip(tr!("Your own colour")))
        .child(match own {
            Some(c) => swatch(Some(c), inner),
            None => div()
                .size(px(inner))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme::ink_a(0.08))
                .child(icon(Icon::Palette, px(13.), theme::text_muted())),
        })
        .on_click(cx.listener(|d, _, window, cx| toggle_picker(d, "interface.accent", window, cx)));
    row.child(div().w(px(1.)).h(px(20.)).mx(px(2.)).bg(theme::ink_a(0.12))).child(custom)
}

fn dot_frame(id: impl Into<ElementId>, on: bool, size: f32) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(if on { theme::text_primary().into() } else { gpui_kit::transparent_black() })
        .cursor_pointer()
}

/// The focus colour as a row of the Interface card, with its picker under it.
pub(super) fn accent_row(d: &mut SettingsPage, r: &Row, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let reset = reset_button(d, "interface.accent", cx);
    let dots = accent_dots(d, r, 30., cx);
    let line = crate::ui::controls::row(r.label.clone(), None, div().flex().items_center().gap(px(6.)).children(reset).child(dots));
    div().flex().flex_col().child(line).children(picker_under(d, "interface.accent", window, cx)).into_any_element()
}

/// The Colours card: the sets, then one row per colour, then what does not read.
pub(super) fn card(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(theme) = d.console_mode.as_ref().map(|s| s.theme.prefs.theme.clone()) else { return div() };
    let mut col = card_box().py(px(12.)).gap(px(4.));
    col = col.child(meta_text(tr!(
        "A light background makes the whole console light. A colour left empty keeps the built-in one, which always reads."
    )));

    let mut sets = div().flex().flex_wrap().gap(px(8.)).pt(px(8.)).pb(px(6.));
    for set in &COLOUR_SETS {
        let on = set.is_on(&theme);
        let dots = [set.background, set.panel, set.text]
            .iter()
            .map(|c| swatch(parse_hex(c), 12.).ml(px(-4.)).into_any_element())
            .collect::<Vec<_>>();
        let id = set.id;
        sets = sets.child(
            div()
                .id(SharedString::from(format!("cm-colour-set-{id}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(12.))
                .pr(px(12.))
                .h(px(32.))
                .rounded_full()
                .border_1()
                .border_color(if on { theme::accent().into() } else { theme::card_border() })
                .bg(if on { theme::accent_tint() } else { theme::ink_a(0.03) })
                .cursor_pointer()
                .hover(|s| s.border_color(theme::accent_alpha(0.7)))
                .child(div().flex().items_center().pl(px(4.)).children(dots))
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(tr(set.name)))
                .on_click(cx.listener(move |d, _, _, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.colour_open = None;
                        s.picker = None;
                    }
                    theme_rows::change(d, &Command::new("colors.set", Arg::Pick(id.into())), cx);
                })),
        );
    }
    col = col.child(sets);

    let rows = card_rows(d, Card::Colours);
    for r in rows {
        col = col.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(colour_row(d, &r, &theme, window, cx));
    }

    let problems = theme_rows::hard_to_read(&theme);
    if !problems.is_empty() {
        let (text, under, ratio) = problems[0];
        let what = match (text, under) {
            ("text", "background") => tr!("The text is hard to read on the background ({ratio}:1; 4.5:1 reads).", ratio = format!("{ratio:.1}")),
            ("text", _) => tr!("The text is hard to read on the panels ({ratio}:1; 4.5:1 reads).", ratio = format!("{ratio:.1}")),
            (_, "background") => {
                tr!("The secondary text is hard to read on the background ({ratio}:1; 4.5:1 reads).", ratio = format!("{ratio:.1}"))
            }
            _ => tr!("The secondary text is hard to read on the panels ({ratio}:1; 4.5:1 reads).", ratio = format!("{ratio:.1}")),
        };
        col = col.child(
            div()
                .mt(px(8.))
                .mb(px(4.))
                .p(px(10.))
                .rounded(px(theme::R_ITEM))
                .bg(theme::ink_a(0.04))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(icon(Icon::Warning, px(16.), theme::warning()))
                .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(theme::text_primary()).child(what))
                .child(
                    button_sm("cm-colours-readable", ButtonKind::Glass, tr!("Make it readable"))
                        .on_click(cx.listener(|d, _, _, cx| theme_rows::run_press(d, "colors.readable", cx))),
                ),
        );
    }
    col
}

/// Where the colour of the row `id` shows.
fn role_note(id: &str) -> Option<&'static str> {
    Some(match id {
        "colors.background" => tr!("Behind the art, and the whole screen with a plain background."),
        "colors.panel" => tr!("Menus, sheets and the hint bar."),
        "colors.text" => tr!("Names, titles and the menus."),
        "colors.muted" => tr!("Facts, hints and labels."),
        "colors.focus_text" => tr!("Text on the focus colour: buttons and the item in focus."),
        "colors.on" => tr!("Switches that are on."),
        _ => return None,
    })
}

/// One colour: its swatch (opens the picker), its hex, back to the built-in one.
fn colour_row(d: &mut SettingsPage, r: &Row, theme: &ConsoleTheme, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let id = r.id.to_string();
    let c = current(theme, &id);
    let open = d.console_mode.as_ref().is_some_and(|s| s.colour_open.as_deref() == Some(id.as_str()));
    let pick = {
        let id = id.clone();
        div()
            .id(SharedString::from(format!("cm-{id}-swatch")))
            .p(px(3.))
            .rounded_full()
            .border_2()
            .border_color(if open { theme::accent().into() } else { gpui_kit::transparent_black() })
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .tooltip(crate::ui::tooltip(tr!("Pick a colour")))
            .child(swatch(c, 24.))
            .on_click(cx.listener(move |d, _, window, cx| toggle_picker(d, &id, window, cx)))
    };
    let hex = text_input(d, &id, &id, &c.map(heronite_data::appearance::hex).unwrap_or_default(), tr!("Built-in"), window, cx).w(px(120.));
    let builtin = c.is_some().then(|| {
        let id = id.clone();
        button_sm(SharedString::from(format!("cm-{id}-builtin")), ButtonKind::Ghost, tr!("Built-in")).on_click(cx.listener(
            move |d, _, window, cx| {
                if let Some(s) = d.console_mode.as_mut().filter(|s| s.colour_open.as_deref() == Some(id.as_str())) {
                    s.colour_open = None;
                    s.picker = None;
                }
                theme_rows::text_done(d, &id, String::new(), window, cx);
            },
        ))
    });
    let reset = reset_button(d, &id, cx);
    let control = div().flex().items_center().gap(px(8.)).children(reset).children(builtin).child(hex).child(pick);
    let line = crate::ui::controls::row(r.label.clone(), role_note(&id), control);
    div().flex().flex_col().child(line).children(picker_under(d, &id, window, cx)).into_any_element()
}
