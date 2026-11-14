//! The shortcuts of Settings → Controller for the tab shown: the action, the
//! controller's buttons and the keyboard's keys. Every shortcut that can
//! change is a chip with a chevron; a click opens its popover under it (one
//! at a time): the screenshot's button, the button held for console mode,
//! a key recorded in place, or only the on/off of a fixed button. A shortcut
//! that is off is a dashed chip that says so. The rows of "Overlay open" and
//! "Console mode" are read-only (fixed, and console mode's theme).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{AButton, ConsoleButton};
use heronite_data::overlay_prefs::{ChordButton, DEFAULT_KEYBOARD_ACCELERATOR, VOLUME_STEPS};
use heronite_data::settings::DEFAULT_CAPTURE_ACCELERATOR;
use heronite_i18n::{tr, tr as translate};
use heronite_platform::{supported, Feature};

use super::pad::Part;
use super::{console_button, glyph, toggle_selected, Show, GUIDE};
use crate::console::theme_rows::console_button_name;
use crate::icons::Icon;
use crate::pages::settings::{overlay, replay, screenshots, Section, SettingsPage};
use crate::theme;
use crate::ui::controls::{button_sm, mini_checkbox, ButtonKind};
use crate::ui::fields::accelerator_from_key;
use crate::ui::icon;

/// Widths of the controller and keyboard columns.
const PAD_W: f32 = 236.;
const KEY_W: f32 = 236.;
/// The popovers' width.
const POP_W: f32 = 330.;

/// A popover of the table: what it changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Edit {
    /// The button held with Guide for the screenshot.
    ShotButton,
    /// The on/off of a shortcut whose button is fixed.
    Fixed(Fixed),
    /// A keyboard shortcut, recorded in the popover.
    Key(Key),
    /// The button held to enter console mode.
    ConsoleButton,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Fixed {
    Replay,
    Moment,
    Volume,
    /// The overlay opens outside games too.
    Anywhere,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Key {
    Overlay,
    Shot,
    Replay,
}

/// A row of the table.
struct Line {
    /// The button it lights on the drawing.
    part: Option<Part>,
    /// The other button of a pair (LB of LB / RB): picking it lights the row too.
    also: Option<Part>,
    action: &'static str,
    note: Option<String>,
    /// Under the action (the volume's steps).
    extra: Option<AnyElement>,
    pad: AnyElement,
    key: AnyElement,
}

impl Line {
    fn new(action: &'static str, part: Option<Part>) -> Self {
        Self { part, also: None, action, note: None, extra: None, pad: dash(), key: dash() }
    }
}

/// "–": nothing in this column.
fn dash() -> AnyElement {
    div().text_size(px(13.)).text_color(theme::text_dim()).child("–").into_any_element()
}

fn plus() -> Div {
    div().text_size(px(11.)).text_color(theme::text_dim()).child("+")
}

/// Buttons pressed together, with a "+" between them.
fn chord(keys: &[&str]) -> Div {
    let mut row = div().flex().flex_none().items_center().gap(px(4.));
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            row = row.child(plus());
        }
        row = row.child(glyph(k));
    }
    row
}

/// Buttons side by side (each does its own thing).
fn glyphs(keys: &[&str]) -> Div {
    div().flex().flex_none().items_center().gap(px(4.)).children(keys.iter().map(|k| glyph(k)))
}

/// The Guide chord with `button`.
fn guide_with(button: &str) -> Div {
    let keys: Vec<&str> = GUIDE.iter().copied().chain([button]).collect();
    chord(&keys)
}

/// "Tap" or "Hold" before the buttons.
fn gesture(word: &'static str, keys: Div) -> Div {
    div().flex().items_center().gap(px(6.)).child(div().text_size(px(12.)).text_color(theme::text_muted()).child(word)).child(keys)
}

/// A key cap, as the keyboard writes it.
fn key_cap(text: impl Into<SharedString>) -> Div {
    crate::ui::controls::kbd(text)
}

/// An accelerator ("Super+Shift+R") as key caps with "+" between them.
fn key_caps(accelerator: &str) -> Div {
    let shown = accelerator.replace("CommandOrControl", "Ctrl");
    let mut row = div().flex().flex_none().items_center().gap(px(3.));
    for (i, k) in shown.split('+').enumerate() {
        if i > 0 {
            row = row.child(plus());
        }
        row = row.child(key_cap(k.to_string()));
    }
    row
}

/// Keys that each do the same (arrows, "Q / E").
fn keys_or(keys: &[&str]) -> Div {
    let mut row = div().flex().flex_none().items_center().gap(px(4.));
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            row = row.child(div().text_size(px(11.)).text_color(theme::text_dim()).child("/"));
        }
        row = row.child(key_cap(k.to_string()));
    }
    row
}

/// How a chip looks.
#[derive(Clone, Copy, Default)]
struct ChipState {
    /// Its popover is open.
    open: bool,
    /// The shortcut is off.
    off: bool,
    /// It waits for something (the overlay): drawn faded.
    waits: bool,
}

/// A shortcut that can change: its buttons or keys, a chevron, and a click
/// that opens `edit`'s popover (or closes it).
fn chip(id: &'static str, content: Div, st: ChipState, edit: Edit, cx: &mut Context<SettingsPage>) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(30.))
        .pl(px(7.))
        .pr(px(5.))
        .rounded(px(8.))
        .border_1()
        .cursor_pointer()
        .when(st.off, |c| c.child(div().text_size(px(11.5)).text_color(theme::text_muted()).mr(px(2.)).child(tr!("Off"))))
        .child(content.when(st.off, |c| c.opacity(0.45)))
        .child(icon(Icon::ExpandMore, px(14.), theme::text_muted()))
        .on_click(cx.listener(move |d, _, window, cx| open_edit(d, edit, window, cx)));
    let base = if st.open {
        base.bg(theme::accent_tint()).border_color(theme::accent())
    } else if st.off {
        base.border_dashed().border_color(theme::ink_a(0.22)).hover(|s| s.bg(theme::ink_a(0.05)))
    } else {
        base.bg(theme::ink_a(0.04))
            .border_color(theme::accent_alpha(0.28))
            .hover(|s| s.bg(theme::ink_a(0.08)).border_color(theme::accent_alpha(0.55)))
    };
    base.when(st.waits && !st.open, |c| c.opacity(0.55))
}

/// Opens `edit`'s popover, or closes it when it is the one open.
fn open_edit(d: &mut SettingsPage, edit: Edit, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let c = &mut d.controls;
    c.key_error = None;
    if c.edit == Some(edit) {
        c.edit = None;
    } else {
        c.edit = Some(edit);
        let focus = if matches!(edit, Edit::Key(_)) { &c.key_focus } else { &c.pop_focus };
        window.focus(focus, cx);
    }
    cx.notify();
}

fn close_edit(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.controls.edit = None;
    d.controls.key_error = None;
    cx.notify();
}

/// `chip` and, when it is open, its popover under it (kept in the window),
/// with a catcher behind that closes it.
fn with_popover(d: &SettingsPage, chip: Stateful<Div>, popover: Option<Div>, cx: &mut Context<SettingsPage>) -> AnyElement {
    let Some(popover) = popover else { return chip.into_any_element() };
    let focus = d.controls.pop_focus.clone();
    let catcher = deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id("controls-popover-catcher")
                .w(px(20000.))
                .h(px(20000.))
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|d, _, _, cx| close_edit(d, cx))),
        ),
    )
    .with_priority(39);
    let panel = deferred(
        anchored().snap_to_window_with_margin(px(8.)).child(
            div()
                .id("controls-popover")
                .track_focus(&focus)
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(|d, ev: &KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" {
                        cx.stop_propagation();
                        close_edit(d, cx);
                    }
                }))
                .child(popover),
        ),
    )
    .with_priority(40);
    div().relative().child(chip).child(div().absolute().top(px(36.)).left_0().child(catcher).child(panel)).into_any_element()
}

/// A popover's frame: the title, what it says, its body and a footer.
fn popover(title: &str, lead: Option<String>) -> Div {
    crate::ui::dropdown::menu_panel(POP_W)
        .w(px(POP_W))
        .p(px(14.))
        .gap(px(0.))
        .child(
            div()
                .font_family(theme::font_heading())
                .text_size(px(14.5))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(title.to_string()),
        )
        .children(lead.map(|l| div().mt(px(2.)).text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_muted()).child(l)))
}

/// A popover's footer: an action on the left, the on switch on the right.
fn footer(
    left: Option<AnyElement>,
    on: Option<(bool, &'static str, fn(&mut SettingsPage, bool, &mut Context<SettingsPage>))>,
    cx: &mut Context<SettingsPage>,
) -> Div {
    let switch = on.map(|(value, id, set)| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child(if value { tr!("On") } else { tr!("Off") })
            .child(mini_checkbox(id, value, false).on_click(cx.listener(move |d, _, _, cx| set(d, !value, cx))))
    });
    div()
        .flex()
        .items_center()
        .mt(px(12.))
        .pt(px(12.))
        .border_t_1()
        .border_color(theme::row_separator())
        .children(left)
        .child(div().flex_1())
        .children(switch)
}

fn line_el(i: usize, l: Line, picked: Option<Part>, cx: &mut Context<SettingsPage>) -> AnyElement {
    let on = picked.is_some() && (l.part == picked || l.also == picked);
    let label: SharedString = l.action.into();
    let action = div()
        .relative()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(crate::ui::find::flash(&label, 6.))
        .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(label))
        .children(l.note.map(|n| div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(n)))
        .children(l.extra);
    let part = l.part;
    // A cell's clicks stay in it (the row picks its button on the drawing).
    let cell = |id: String, w: f32| {
        div()
            .id(SharedString::from(id))
            .flex_none()
            .w(px(w))
            .flex()
            .items_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    let row = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(action)
        .child(cell(format!("controls-row-{i}-pad"), PAD_W).child(l.pad))
        .child(cell(format!("controls-row-{i}-key"), KEY_W).child(l.key));
    div()
        .id(SharedString::from(format!("controls-row-{i}")))
        .flex()
        .flex_col()
        .px(px(16.))
        .py(px(10.))
        .min_h(px(52.))
        .justify_center()
        .border_t_1()
        .border_color(theme::row_separator())
        .when(on, |r| r.bg(theme::accent_tint()))
        .when(part.is_some(), |r| r.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.03))))
        .on_click(cx.listener(move |d, _, _, cx| {
            if let Some(part) = part {
                toggle_selected(d, part, cx);
            }
        }))
        .child(row)
        .into_any_element()
}

/// The keyboard shortcut set for `key` (with the default when unset);
/// `None`: none (the replay's, until one is recorded).
fn accelerator(d: &SettingsPage, key: Key, cx: &App) -> Option<String> {
    match key {
        Key::Overlay => Some(d.overlay.prefs.keyboard_accelerator.clone()),
        Key::Shot => Some(screenshots::accelerator(d)),
        Key::Replay => replay::accelerator(d, cx),
    }
}

/// The key is on (the replay's: set).
fn key_on(d: &SettingsPage, key: Key, cx: &App) -> bool {
    match key {
        Key::Overlay => d.overlay.prefs.keyboard_shortcut,
        Key::Shot => d.settings.screenshots.capture_enabled,
        Key::Replay => accelerator(d, key, cx).is_some(),
    }
}

/// What else already uses `accelerator`.
fn key_taken(d: &SettingsPage, key: Key, pressed: &str, cx: &App) -> Option<String> {
    // The replay's key comes back as shown ("Ctrl"), the others as saved.
    let shown = pressed.replace("CommandOrControl", "Ctrl");
    let same = |other: Key| {
        other != key
            && key_on(d, other, cx)
            && accelerator(d, other, cx).is_some_and(|a| a.replace("CommandOrControl", "Ctrl").eq_ignore_ascii_case(&shown))
    };
    if same(Key::Overlay) && d.overlay.prefs.enabled {
        Some(tr!("{key} already opens the overlay.", key = &shown))
    } else if same(Key::Shot) {
        Some(tr!("{key} already takes a screenshot.", key = &shown))
    } else if same(Key::Replay) {
        Some(tr!("{key} already saves the replay.", key = &shown))
    } else {
        None
    }
}

/// Saves the key recorded for `key`.
fn set_key(d: &mut SettingsPage, key: Key, accelerator: String, cx: &mut Context<SettingsPage>) {
    match key {
        Key::Overlay => overlay::update(d, cx, |p| {
            p.keyboard_accelerator = accelerator;
            p.keyboard_shortcut = true;
        }),
        Key::Shot => screenshots::set_accelerator(d, accelerator, cx),
        Key::Replay => replay::set_accelerator(d, accelerator, cx),
    }
}

/// A key chip: the keys (or "None"), and its popover when open.
fn key_chip(d: &SettingsPage, id: &'static str, key: Key, waits: bool, cx: &mut Context<SettingsPage>) -> AnyElement {
    let edit = Edit::Key(key);
    let accel = accelerator(d, key, cx);
    let content = match &accel {
        Some(a) => key_caps(a),
        None => div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("None")),
    };
    let st = ChipState { open: d.controls.edit == Some(edit), off: accel.is_some() && !key_on(d, key, cx), waits };
    let c = chip(id, content, st, edit, cx);
    let pop = st.open.then(|| key_popover(d, key, cx));
    with_popover(d, c, pop, cx)
}

/// The popover of a keyboard shortcut: a box that records the next keys
/// (Esc cancels; Backspace removes the replay's), what clashes, and the
/// key's on switch or "Remove the key".
fn key_popover(d: &SettingsPage, key: Key, cx: &mut Context<SettingsPage>) -> Div {
    let title = match key {
        Key::Overlay => tr!("Open the overlay"),
        Key::Shot => tr!("Take a screenshot"),
        Key::Replay => tr!("Save the replay"),
    };
    let current = accelerator(d, key, cx);
    let focused = d.controls.recording;
    let shown = match (&current, focused) {
        (_, true) => tr!("Press the new keys…").to_string(),
        (Some(a), false) => a.replace("CommandOrControl", "Ctrl"),
        (None, false) => tr!("Click here, then press the keys").to_string(),
    };
    let recorder = div()
        .id("controls-key-recorder")
        .track_focus(&d.controls.key_focus)
        .mt(px(12.))
        .h(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .border_1()
        .text_size(px(13.))
        .cursor_pointer()
        .map(|b| {
            if focused {
                b.bg(theme::accent_tint()).border_color(theme::accent()).text_color(theme::accent())
            } else {
                b.bg(theme::ink_a(0.04)).border_color(theme::ink_a(0.14)).text_color(theme::text_primary())
            }
        })
        .child(shown)
        .on_click(cx.listener(|d, _, window, cx| {
            window.focus(&d.controls.key_focus, cx);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |d, ev: &KeyDownEvent, _, cx| {
            cx.stop_propagation();
            let k = &ev.keystroke;
            let plain = !(k.modifiers.control || k.modifiers.alt || k.modifiers.shift || k.modifiers.platform);
            if plain && k.key == "escape" {
                close_edit(d, cx);
                return;
            }
            if plain && key == Key::Replay && matches!(k.key.as_str(), "backspace" | "delete") {
                replay::set_accelerator(d, String::new(), cx);
                close_edit(d, cx);
                return;
            }
            let Some(a) = accelerator_from_key(ev) else { return };
            match key_taken(d, key, &a, cx) {
                Some(why) => {
                    d.controls.key_error = Some(why);
                    cx.notify();
                }
                None => {
                    set_key(d, key, a, cx);
                    close_edit(d, cx);
                }
            }
        }));
    let hint = if key == Key::Replay { tr!("Esc cancels. Backspace removes the key.") } else { tr!("Esc cancels.") };
    let error = d.controls.key_error.clone();
    let left: Option<AnyElement> = match key {
        Key::Overlay => (d.overlay.prefs.keyboard_accelerator != DEFAULT_KEYBOARD_ACCELERATOR)
            .then(|| reset_button("controls-key-reset", Key::Overlay, DEFAULT_KEYBOARD_ACCELERATOR, cx)),
        Key::Shot => (screenshots::accelerator(d) != DEFAULT_CAPTURE_ACCELERATOR)
            .then(|| reset_button("controls-key-reset", Key::Shot, DEFAULT_CAPTURE_ACCELERATOR, cx)),
        Key::Replay => current.is_some().then(|| {
            button_sm("controls-key-remove", ButtonKind::Glass, tr!("Remove the key"))
                .on_click(cx.listener(|d, _, _, cx| {
                    replay::set_accelerator(d, String::new(), cx);
                    close_edit(d, cx);
                }))
                .into_any_element()
        }),
    };
    let on: Option<(bool, &'static str, fn(&mut SettingsPage, bool, &mut Context<SettingsPage>))> = match key {
        Key::Overlay => {
            Some((d.overlay.prefs.keyboard_shortcut, "controls-key-on", |d, on, cx| overlay::update(d, cx, |p| p.keyboard_shortcut = on)))
        }
        Key::Shot => Some((d.settings.screenshots.capture_enabled, "controls-key-on", |d, on, cx| screenshots::set_capture(d, on, cx))),
        Key::Replay => None,
    };
    popover(title, Some(tr!("Keyboard shortcut").to_string()))
        .child(recorder)
        .child(div().mt(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).child(hint))
        .children(error.map(|e| div().mt(px(6.)).text_size(px(12.5)).line_height(px(17.)).text_color(theme::danger_text()).child(e)))
        .child(footer(left, on, cx))
}

/// "Reset to F10": puts `key`'s default back, unless another shortcut has it.
fn reset_button(id: &'static str, key: Key, default: &'static str, cx: &mut Context<SettingsPage>) -> AnyElement {
    let label = tr!("Reset to {key}", key = default.replace("CommandOrControl", "Ctrl"));
    button_sm(id, ButtonKind::Glass, label)
        .on_click(cx.listener(move |d, _, _, cx| {
            d.controls.key_error = key_taken(d, key, default, cx);
            if d.controls.key_error.is_none() {
                set_key(d, key, default.to_string(), cx);
            }
            cx.notify();
        }))
        .into_any_element()
}

/// The popover of a shortcut whose button cannot change: only its switch.
fn fixed_popover(d: &SettingsPage, fixed: Fixed, cx: &mut Context<SettingsPage>) -> Div {
    let p = &d.overlay.prefs;
    let (title, lead, on, set): (&str, &str, bool, fn(&mut SettingsPage, bool, &mut Context<SettingsPage>)) = match fixed {
        Fixed::Replay => {
            (tr!("Save the replay"), tr!("Its button is fixed. You can turn the shortcut off."), p.replay_shortcut, |d, on, cx| {
                overlay::update(d, cx, |p| p.replay_shortcut = on)
            })
        }
        Fixed::Moment => {
            (tr!("Mark a moment"), tr!("Its button is fixed. You can turn the shortcut off."), p.moment_shortcut, |d, on, cx| {
                overlay::update(d, cx, |p| p.moment_shortcut = on)
            })
        }
        Fixed::Volume => {
            (tr!("Volume up / down"), tr!("Its button is fixed. You can turn the shortcut off."), p.volume_shortcut, |d, on, cx| {
                overlay::update(d, cx, |p| p.volume_shortcut = on)
            })
        }
        Fixed::Anywhere => {
            (tr!("Open the overlay outside games"), crate::overlay::labels::without_game_short(), p.without_game, |d, on, cx| {
                overlay::update(d, cx, |p| p.without_game = on)
            })
        }
    };
    // Its button taken by the screenshot: it is off whatever its switch says.
    let shot = p.screenshot_shortcut.then(|| overlay::screenshot_button(p));
    let taken = match fixed {
        Fixed::Replay if shot == Some(ChordButton::X) => Some(if cfg!(windows) {
            tr!("Off while Back + Start + X takes the screenshot: pick another button for it above.")
        } else {
            tr!("Off while Guide + X takes the screenshot: pick another button for it above.")
        }),
        Fixed::Moment if shot == Some(ChordButton::Y) => Some(crate::overlay::labels::moment_taken_by_screenshot()),
        _ => None,
    };
    match taken {
        Some(why) => popover(title, Some(why.to_string())),
        None => popover(title, Some(lead.to_string())).child(footer(None, Some((on, "controls-fixed-on", set)), cx)),
    }
}

/// The popover of the screenshot's button: the buttons this system allows,
/// X and Y off limits while the replay and the moment use them (unless the
/// screenshot already has one), "Reset to RB" and the on switch.
fn shot_popover(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let p = &d.overlay.prefs;
    let current = overlay::screenshot_button(p);
    let replay_x = supported(Feature::Replay) && p.replay_shortcut && current != ChordButton::X;
    let moment_y = p.moment_shortcut && current != ChordButton::Y;
    let mut grid = div().mt(px(12.)).flex().flex_wrap().gap(px(6.));
    for b in overlay::screenshot_buttons() {
        let on = b == current;
        let blocked = (b == ChordButton::X && replay_x) || (b == ChordButton::Y && moment_y);
        let label = overlay::button_short(b);
        grid = grid.child(
            div()
                .id(SharedString::from(format!("controls-shot-pick-{}", b.as_str())))
                .p(px(4.))
                .rounded(px(14.))
                .border_1()
                .border_color(if on { theme::accent().into() } else { theme::ink_a(0.) })
                .when(on, |c| c.bg(theme::accent_tint()))
                .when(blocked, |c| c.opacity(0.35))
                .when(!blocked && !on, |c| c.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.08))))
                .tooltip(crate::ui::tooltip(overlay::button_label(b)))
                .child(glyph(&label))
                .when(!blocked, |c| c.on_click(cx.listener(move |d, _, _, cx| overlay::update(d, cx, |p| p.screenshot_button = b)))),
        );
    }
    let why = match (replay_x, moment_y) {
        (true, true) => Some(if cfg!(windows) {
            tr!("Back + Start + X saves the replay and Back + Start + Y marks a moment: turn one of them off to use its button here.")
        } else {
            tr!("Guide + X saves the replay and Guide + Y marks a moment: turn one of them off to use its button here.")
        }),
        (true, false) => Some(if cfg!(windows) {
            tr!("Back + Start + X saves the replay: turn it off to use X here.")
        } else {
            tr!("Guide + X saves the replay: turn it off to use X here.")
        }),
        (false, true) => Some(if cfg!(windows) {
            tr!("Back + Start + Y marks a moment: turn it off to use Y here.")
        } else {
            tr!("Guide + Y marks a moment: turn it off to use Y here.")
        }),
        (false, false) => None,
    };
    let lead = if cfg!(windows) { tr!("Hold Back + Start and press:") } else { tr!("Hold Guide and press:") };
    let reset = (current != ChordButton::RB).then(|| {
        button_sm("controls-shot-reset", ButtonKind::Glass, tr!("Reset to RB"))
            .on_click(cx.listener(|d, _, _, cx| overlay::update(d, cx, |p| p.screenshot_button = ChordButton::RB)))
            .into_any_element()
    });
    popover(tr!("Take a screenshot"), Some(lead.to_string()))
        .child(grid)
        .children(why.map(|w| div().mt(px(8.)).text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(w)))
        .child(footer(
            reset,
            Some((p.screenshot_shortcut, "controls-shot-on", |d, on, cx| overlay::update(d, cx, |p| p.screenshot_shortcut = on))),
            cx,
        ))
}

/// The menu of the button held to enter console mode.
fn console_menu(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let current = console_button(d);
    let mut menu = crate::ui::dropdown::menu_panel(230.);
    for (i, b) in ConsoleButton::CHOICES.iter().enumerate() {
        let b = *b;
        let on = b == current;
        menu = menu.child(
            crate::ui::dropdown::menu_item(SharedString::from(format!("controls-console-button-{i}")), on)
                .child(div().flex_1().child(translate(console_button_name(b))))
                .when(on, |c| c.child(icon(Icon::Check, px(14.), theme::text_primary())))
                .on_click(cx.listener(move |d, _, _, cx| {
                    crate::console::startup::set_console_button(&d.ctx.app, b);
                    crate::console::summon::options_changed(cx);
                    close_edit(d, cx);
                })),
        );
    }
    menu
}

/// The overlay's switch-on banner while it is off.
fn overlay_off_banner(cx: &mut Context<SettingsPage>) -> AnyElement {
    let warn: Hsla = theme::warning().into();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(10.))
        .px(px(16.))
        .py(px(10.))
        .border_t_1()
        .border_color(theme::row_separator())
        .bg(warn.opacity(0.07))
        .child(icon(Icon::Warning, px(14.), theme::warning()))
        .child(
            div()
                .flex_1()
                .min_w(px(220.))
                .text_size(px(12.5))
                .line_height(px(17.))
                .text_color(theme::text_primary())
                .child(tr!("The overlay is off: its controller shortcuts and its key wait for it.")),
        )
        .child(
            button_sm("controls-overlay-on", ButtonKind::Primary, tr!("Turn on the overlay"))
                .on_click(cx.listener(|d, _, _, cx| overlay::turn_on(d, cx))),
        )
        .into_any_element()
}

/// During a game: the overlay's Guide chords and the keys beside them.
fn in_game(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<Line> {
    let p = d.overlay.prefs.clone();
    let waits = !p.enabled;
    let open = d.controls.edit;
    let mut v = Vec::new();

    let mut overlay_line = Line::new(tr!("Open the overlay"), Some(if cfg!(windows) { Part::View } else { Part::Guide }));
    overlay_line.pad = gesture(tr!("Tap"), chord(GUIDE)).when(waits, |c| c.opacity(0.55)).into_any_element();
    overlay_line.key = key_chip(d, "controls-key-overlay", Key::Overlay, waits, cx);
    v.push(overlay_line);

    let shot = overlay::screenshot_button(&p);
    let mut take = Line::new(tr!("Take a screenshot"), Some(Part::of_chord(shot)));
    let st = ChipState { open: open == Some(Edit::ShotButton), off: !p.screenshot_shortcut, waits };
    let c = chip("controls-shot", guide_with(&overlay::button_short(shot)), st, Edit::ShotButton, cx);
    let pop = st.open.then(|| shot_popover(d, cx));
    take.pad = with_popover(d, c, pop, cx);
    if Section::Screenshots.available() {
        take.key = key_chip(d, "controls-key-shot", Key::Shot, false, cx);
    }
    v.push(take);

    let fixed_chip = |id: &'static str, keys: Div, on: bool, fixed: Fixed, cx: &mut Context<SettingsPage>| {
        let edit = Edit::Fixed(fixed);
        let st = ChipState { open: open == Some(edit), off: !on, waits };
        let c = chip(id, keys, st, edit, cx);
        let pop = st.open.then(|| fixed_popover(d, fixed, cx));
        with_popover(d, c, pop, cx)
    };
    if supported(Feature::Replay) {
        let mut save = Line::new(tr!("Save the replay"), Some(Part::X));
        let taken = p.screenshot_shortcut && shot == ChordButton::X;
        save.pad = fixed_chip("controls-replay", guide_with("X"), p.replay_shortcut && !taken, Fixed::Replay, cx);
        if taken && p.replay_shortcut {
            save.note = Some(
                if cfg!(windows) {
                    tr!("Off while Back + Start + X takes the screenshot: pick another button for it above.")
                } else {
                    tr!("Off while Guide + X takes the screenshot: pick another button for it above.")
                }
                .to_string(),
            );
        }
        save.key = key_chip(d, "controls-key-replay", Key::Replay, false, cx);
        v.push(save);
    }

    let mut moment = Line::new(tr!("Mark a moment"), Some(Part::Y));
    let taken = p.screenshot_shortcut && shot == ChordButton::Y;
    moment.pad = fixed_chip("controls-moment", guide_with("Y"), p.moment_shortcut && !taken, Fixed::Moment, cx);
    if taken && p.moment_shortcut {
        moment.note = Some(crate::overlay::labels::moment_taken_by_screenshot().to_string());
    }
    v.push(moment);

    let mut volume = Line::new(tr!("Volume up / down"), Some(Part::DPad));
    volume.pad = fixed_chip("controls-volume", guide_with("↑↓"), p.volume_shortcut, Fixed::Volume, cx);
    let active = p.volume_shortcut && p.enabled;
    let mut steps =
        div().flex().items_center().gap(px(2.)).p(px(2.)).rounded(px(8.)).bg(theme::ink_a(0.05)).when(!active, |c| c.opacity(0.4));
    for step in VOLUME_STEPS {
        let on = p.volume_step == step;
        steps = steps.child(
            div()
                .id(SharedString::from(format!("controls-volume-step-{step}")))
                .px(px(8.))
                .py(px(2.))
                .rounded(px(6.))
                .text_size(px(11.5))
                .cursor_pointer()
                .text_color(if on { theme::text_primary() } else { theme::text_muted() })
                .when(on, |c| c.bg(theme::ink_a(0.12)))
                .when(!on, |c| c.hover(|s| s.bg(theme::ink_a(0.07))))
                .child(overlay::step_label(step))
                .on_click(cx.listener(move |d, _, _, cx| overlay::update(d, cx, |p| p.volume_step = step))),
        );
    }
    volume.extra = Some(
        div()
            .id("controls-volume-steps")
            .mt(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("In steps of")))
            .child(steps)
            .into_any_element(),
    );
    v.push(volume);
    v
}

/// Outside games: the button held for console mode, and the overlay
/// without a game.
fn outside(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<Line> {
    let button = console_button(d);
    let keys: &[&str] = match button {
        ConsoleButton::BackStart => &["Back", "Start"],
        ConsoleButton::Bumpers => &["LB", "RB"],
        ConsoleButton::Off => &[],
        _ => &["Guide"],
    };
    let part = match button {
        ConsoleButton::BackStart => Some(Part::View),
        ConsoleButton::Bumpers => Some(Part::LB),
        ConsoleButton::Off => None,
        _ => Some(Part::Guide),
    };
    let open = d.controls.edit;
    let mut enter = Line::new(tr!("Enter console mode"), part);
    enter.note = Some(tr!("Not while a game runs").to_string());
    let st = ChipState { open: open == Some(Edit::ConsoleButton), off: keys.is_empty(), waits: false };
    let content = if keys.is_empty() { gesture(tr!("Hold"), glyph("Guide")) } else { gesture(tr!("Hold"), chord(keys)) };
    let c = chip("controls-console-button", content, st, Edit::ConsoleButton, cx);
    let pop = st.open.then(|| console_menu(d, cx));
    enter.pad = with_popover(d, c, pop, cx);
    let mut v = vec![enter];

    if supported(Feature::Overlay) {
        let p = &d.overlay.prefs;
        let mut anywhere = Line::new(tr!("Open the overlay"), Some(if cfg!(windows) { Part::View } else { Part::Guide }));
        let edit = Edit::Fixed(Fixed::Anywhere);
        let st = ChipState { open: open == Some(edit), off: !p.without_game, waits: !p.enabled };
        let c = chip("controls-anywhere", gesture(tr!("Tap"), chord(GUIDE)), st, edit, cx);
        let pop = st.open.then(|| fixed_popover(d, Fixed::Anywhere, cx));
        anywhere.pad = with_popover(d, c, pop, cx);
        if !p.enabled {
            anywhere.note = Some(tr!("Needs the overlay").to_string());
        }
        anywhere.key = if p.keyboard_shortcut {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .when(!(p.enabled && p.without_game), |c| c.opacity(0.5))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Same key as in a game")))
                .child(key_caps(&p.keyboard_accelerator))
                .into_any_element()
        } else {
            dash()
        };
        v.push(anywhere);
    }
    v
}

/// The other button of the pairs the read-only rows show (LB / RB, LT / RT,
/// L stick / D-pad).
fn pair(part: Part) -> Option<Part> {
    match part {
        Part::RB => Some(Part::LB),
        Part::RT => Some(Part::LT),
        Part::LStick => Some(Part::DPad),
        _ => None,
    }
}

/// While the overlay is open: fixed.
fn overlay_open(d: &SettingsPage) -> Vec<Line> {
    let read = |action: &'static str, part: Part, pad: Div, key: Option<Div>| {
        let mut l = Line::new(action, Some(part));
        l.also = pair(part);
        l.pad = pad.into_any_element();
        l.key = key.map(|k| k.into_any_element()).unwrap_or_else(dash);
        l
    };
    let p = &d.overlay.prefs;
    let close_key = p.keyboard_shortcut.then(|| key_caps(&p.keyboard_accelerator));
    vec![
        read(tr!("Move"), Part::LStick, glyphs(&[tr!("L stick"), tr!("D-pad")]), Some(key_cap(tr!("Arrows")))),
        read(tr!("Select"), Part::A, glyph("A"), Some(key_cap("Enter"))),
        read(tr!("Back · close"), Part::B, glyph("B"), Some(key_cap("Esc"))),
        read(tr!("Previous / next tab"), Part::RB, glyphs(&["LB", "RB"]), Some(keys_or(&["Q", "E"]))),
        read(tr!("Mark a moment in Notes"), Part::Y, glyph("Y"), None),
        read(tr!("Close the overlay"), if cfg!(windows) { Part::View } else { Part::Guide }, chord(GUIDE), close_key),
    ]
}

/// Console mode, as its theme has it (read-only here).
fn console() -> Vec<Line> {
    let theme = crate::console::look::theme();
    let swap = theme.controls.swap_ab;
    let details = theme.nav.a_button == AButton::Details;
    let read = |action: &'static str, part: Part, pad: Div, key: Option<Div>| {
        let mut l = Line::new(action, Some(part));
        l.also = pair(part);
        l.pad = pad.into_any_element();
        l.key = key.map(|k| k.into_any_element()).unwrap_or_else(dash);
        l
    };
    let (confirm, back) = if swap { (("B", Part::B), ("A", Part::A)) } else { (("A", Part::A), ("B", Part::B)) };
    let (opens, other) = if details { (tr!("Open the game page"), tr!("Play")) } else { (tr!("Play"), tr!("Open the game page")) };
    let mut first = read(opens, confirm.1, glyph(confirm.0), Some(key_cap("Enter")));
    if swap {
        first.note = Some(tr!("Confirm with the east button is on").to_string());
    }
    vec![
        first,
        read(other, Part::X, glyph("X"), Some(key_cap("X"))),
        read(tr!("Back · sidebar"), back.1, glyph(back.0), Some(key_cap("Esc"))),
        read(tr!("Search and filters"), Part::Y, glyph("Y"), Some(key_cap("Y"))),
        read(tr!("Previous / next tab"), Part::RB, glyphs(&["LB", "RB"]), Some(keys_or(&["Q", "E"]))),
        read(tr!("Jump by letter"), Part::RT, glyphs(&["LT", "RT"]), Some(keys_or(&["Page Up", "Page Down"]))),
        read(tr!("Open the menu"), Part::Menu, glyph(tr!("Menu")), Some(key_cap("M"))),
        read(tr!("Move"), Part::LStick, glyphs(&[tr!("L stick"), tr!("D-pad")]), Some(key_cap(tr!("Arrows")))),
    ]
}

/// The tab that shows `label` (a row's action, translated), for the
/// Settings search.
pub(super) fn tab_of(label: &str) -> Option<Show> {
    const TABS: &[(Show, &[&str])] = &[
        (Show::InGame, &["Open the overlay", "Take a screenshot", "Save the replay", "Mark a moment", "Volume up / down"]),
        (Show::Outside, &["Enter console mode"]),
        (Show::Console, &["Search and filters", "Jump by letter", "Open the game page", "Play", "Back · sidebar"]),
    ];
    TABS.iter().find(|(_, labels)| labels.iter().any(|l| translate(l) == label)).map(|(s, _)| *s)
}

/// The table of the tab shown.
pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    d.controls.recording = d.controls.key_focus.is_focused(window);
    let show = d.controls.show;
    let picked = d.controls.selected;
    let head = |t: &'static str, w: Option<f32>| {
        let c = div().text_size(px(12.)).text_color(theme::text_dim()).child(t);
        match w {
            Some(w) => c.flex_none().w(px(w)),
            None => c.flex_1(),
        }
    };
    let mut table = div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(theme::R_CARD))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(10.))
                .child(head(tr!("Action"), None))
                .child(head(tr!("Controller"), Some(PAD_W)))
                .child(head(tr!("Keyboard"), Some(KEY_W))),
        );
    let lines = match show {
        Show::InGame => {
            if !d.overlay.prefs.enabled {
                table = table.child(overlay_off_banner(cx));
            }
            in_game(d, cx)
        }
        Show::Outside => outside(d, cx),
        Show::Overlay => overlay_open(d),
        Show::Console => console(),
    };
    for (i, l) in lines.into_iter().enumerate() {
        table = table.child(line_el(i, l, picked, cx));
    }
    if show == Show::InGame && supported(Feature::Overlay) {
        let note = if cfg!(windows) {
            tr!("Game Bar may take the Guide button, so Back + Start stands in for it. The game also gets those buttons.")
        } else {
            tr!("With Steam's \"Guide Button Focuses Steam\" on, Guide opens Steam instead of the overlay. Turn it off in Steam → Settings → Controller.")
        };
        table = table.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .px(px(16.))
                .py(px(10.))
                .border_t_1()
                .border_color(theme::row_separator())
                .child(div().pt(px(1.)).child(icon(Icon::InfoOutlined, px(14.), theme::text_dim())))
                .child(div().flex_1().text_size(px(12.)).line_height(px(17.)).text_color(theme::text_muted()).child(note)),
        );
    }
    table
}
