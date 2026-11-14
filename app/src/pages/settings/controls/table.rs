//! The bindings of Settings → Controls → Controller as a table, grouped by
//! where they work: the action, the controller's buttons (with their own
//! switch, or "waits for the overlay" while it is off), the keyboard's key
//! (it opens the Keyboard page to change it, with its own switch where the
//! app has one) and what can change: the screenshot's button ("Change"), the
//! button that enters console mode, a padlock on fixed ones. Console mode's
//! rows are its theme's: read here, changed in Console mode.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{AButton, ConsoleButton, Hints};
use heronite_data::overlay_prefs::{ChordButton, VOLUME_STEPS};
use heronite_i18n::{tr, tr as translate};
use heronite_platform::{supported, Feature};

use super::pad::Part;
use super::{console_button, shares_guide, toggle_selected, Show, GUIDE};
use crate::console::theme_rows::console_button_name;
use crate::icons::Icon;
use crate::pages::settings::{overlay, replay, screenshots, Section, SettingsPage};
use crate::theme;
use crate::ui::controls::{button_sm, mini_checkbox, tabs, ButtonKind};
use crate::ui::fields::select_field;
use crate::ui::icon;

/// Widths of the controller, keyboard and button columns.
const PAD_W: f32 = 210.;
const KEY_W: f32 = 190.;
const BUTTON_W: f32 = 170.;

/// A row of the table.
struct Line {
    /// The button it lights on the drawing.
    part: Option<Part>,
    action: &'static str,
    /// Dim text after the action ("· 5% steps").
    extra: Option<String>,
    note: Option<String>,
    pad: AnyElement,
    key: AnyElement,
    button: AnyElement,
}

impl Line {
    fn new(action: &'static str, part: Option<Part>) -> Self {
        Self { part, action, extra: None, note: None, pad: dash(), key: dash(), button: div().into_any_element() }
    }
}

/// "–": nothing in this column.
fn dash() -> AnyElement {
    div().text_size(px(13.)).text_color(theme::text_dim()).child("–").into_any_element()
}

/// A controller button as printed on it: A, B, X and Y round in their
/// colours, the others in a pill.
fn glyph(name: &str) -> Div {
    let face = match name {
        "A" => Some(theme::console_btn_a()),
        "B" => Some(theme::console_btn_b()),
        "X" => Some(theme::console_btn_x()),
        "Y" => Some(theme::console_btn_y()),
        _ => None,
    };
    let g = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.))
        .rounded_full()
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .whitespace_nowrap();
    match face {
        Some(c) => g.w(px(22.)).bg(c).text_color(theme::console_on_focus()),
        None => g.min_w(px(22.)).px(px(7.)).bg(theme::ink_a(0.12)).text_color(theme::text_primary()),
    }
    .child(name.to_string())
}

/// Buttons pressed together, with a "+" between them.
fn chord(keys: &[&str]) -> Div {
    let mut row = div().flex().flex_none().items_center().gap(px(4.));
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            row = row.child(div().text_size(px(11.)).text_color(theme::text_dim()).child("+"));
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

fn waits() -> Div {
    div().text_size(px(11.5)).text_color(theme::text_dim()).whitespace_nowrap().child(tr!("waits for the overlay"))
}

/// A key, as the keyboard says it.
fn key_cap(text: impl Into<SharedString>) -> Div {
    crate::ui::controls::kbd(text)
}

/// A key that the Keyboard page changes: the key and a pencil.
fn key_edit(id: &'static str, text: String, to: Section, cx: &mut Context<SettingsPage>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(6.))
        .bg(theme::ink_a(0.08))
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(theme::text_muted())
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.14)))
        .tooltip(crate::ui::tooltip(tr!("Change the key")))
        .child(text)
        .child(icon(Icon::Edit, px(11.), theme::text_dim()))
        .on_click(cx.listener(move |d, _, window, cx| d.select(to, window, cx)))
}

/// The padlock of a binding that cannot change.
fn lock() -> AnyElement {
    div()
        .id("controls-lock")
        .flex_none()
        .tooltip(crate::ui::tooltip(tr!("Fixed")))
        .child(icon(Icon::Lock, px(13.), theme::text_dim()))
        .into_any_element()
}

/// A console theme's value, read-only, with its padlock.
fn theme_value(text: &'static str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(text))
        .child(lock())
        .into_any_element()
}

/// A column cell: its clicks stay in it (the row picks its button on the
/// drawing).
fn cell(id: impl Into<ElementId>, w: Option<f32>) -> Stateful<Div> {
    let c = div().id(id).flex().items_center().gap(px(6.)).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    match w {
        Some(w) => c.flex_none().w(px(w)),
        None => c,
    }
}

/// A compact switch whose click changes the overlay's options.
fn overlay_switch(
    id: &'static str,
    on: bool,
    flip: fn(&mut heronite_data::overlay_prefs::OverlayPrefs),
    cx: &mut Context<SettingsPage>,
) -> AnyElement {
    mini_checkbox(id, on, false).on_click(cx.listener(move |d, _, _, cx| overlay::update(d, cx, flip))).into_any_element()
}

/// A controller cell of the in-game group: the buttons, then their switch,
/// or "waits for the overlay" while it is off.
fn pad_cell(keys: Div, switch: Option<AnyElement>, enabled: bool) -> AnyElement {
    let row = div().flex().items_center().gap(px(8.));
    if enabled {
        row.child(keys).children(switch).into_any_element()
    } else {
        row.child(keys.opacity(0.45)).child(waits()).into_any_element()
    }
}

/// A group's caption across the table, with an action.
fn group(text: String, action: Option<AnyElement>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(7.))
        .bg(theme::ink_a(0.03))
        .border_t_1()
        .border_color(theme::row_separator())
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(text))
        .children(action)
        .into_any_element()
}

fn line_el(i: usize, l: Line, picked: Option<Part>, cx: &mut Context<SettingsPage>) -> AnyElement {
    let on = l.part.is_some() && l.part == picked;
    let label: SharedString = l.action.into();
    let action = div()
        .relative()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(1.))
        .children(crate::ui::find::flash(&label, 6.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .text_size(px(13.5))
                .text_color(theme::text_primary())
                .child(label)
                .children(l.extra.map(|e| div().text_size(px(12.5)).text_color(theme::text_dim()).child(e))),
        )
        .children(l.note.map(|n| div().text_size(px(11.5)).line_height(px(15.)).text_color(theme::text_dim()).child(n)));
    let part = l.part;
    div()
        .id(SharedString::from(format!("controls-row-{i}")))
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(9.))
        .min_h(px(44.))
        .border_t_1()
        .border_color(theme::row_separator())
        .when(on, |r| r.bg(theme::accent_tint()))
        .when(part.is_some(), |r| r.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.03))))
        .on_click(cx.listener(move |d, _, _, cx| {
            if let Some(part) = part {
                toggle_selected(d, part, cx);
            }
        }))
        .child(action)
        .child(cell(SharedString::from(format!("controls-row-{i}-pad")), Some(PAD_W)).child(l.pad))
        .child(cell(SharedString::from(format!("controls-row-{i}-key")), Some(KEY_W)).child(l.key))
        .child(cell(SharedString::from(format!("controls-row-{i}-button")), Some(BUTTON_W)).justify_end().child(l.button))
        .into_any_element()
}

/// During a game: the overlay's Guide chords and the keys beside them.
fn in_game(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<Line> {
    let p = d.overlay.prefs.clone();
    let enabled = p.enabled;
    let mut v = Vec::new();

    let mut open = Line::new(tr!("Open the overlay"), Some(if cfg!(windows) { Part::View } else { Part::Guide }));
    open.pad = pad_cell(chord(GUIDE), None, enabled);
    let accel = p.keyboard_accelerator.replace("CommandOrControl", "Ctrl");
    let keyboard = key_edit("controls-key-overlay", accel, Section::Overlay, cx);
    open.key = if enabled {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(keyboard)
            .child(overlay_switch("controls-sw-overlay-key", p.keyboard_shortcut, |p| p.keyboard_shortcut = !p.keyboard_shortcut, cx))
            .into_any_element()
    } else {
        div().flex().items_center().gap(px(8.)).child(keyboard.opacity(0.45)).child(waits()).into_any_element()
    };
    v.push(open);

    let shot = overlay::screenshot_button(&p);
    let mut take = Line::new(tr!("Take a screenshot"), Some(Part::of_chord(shot)));
    take.pad = pad_cell(
        guide_with(&overlay::button_short(shot)),
        Some(overlay_switch("controls-sw-shot", p.screenshot_shortcut, |p| p.screenshot_shortcut = !p.screenshot_shortcut, cx)),
        enabled,
    );
    if Section::Screenshots.available() {
        let key = screenshots::accelerator(d).replace("CommandOrControl", "Ctrl");
        let capture = d.settings.screenshots.capture_enabled;
        if !enabled {
            take.note = Some(tr!("{key} works without the overlay", key = &key));
        }
        take.key = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(key_edit("controls-key-shot", key, Section::Screenshots, cx))
            .child(mini_checkbox("controls-sw-shot-key", capture, false).on_click(cx.listener(move |d, _, _, cx| {
                screenshots::set_capture(d, !capture, cx);
            })))
            .into_any_element();
    }
    take.button = change_button(d, cx);
    v.push(take);

    let mut save = Line::new(tr!("Save the replay"), Some(Part::X));
    if supported(Feature::Replay) {
        save.pad = pad_cell(
            guide_with("X"),
            Some(overlay_switch("controls-sw-replay", p.replay_shortcut, |p| p.replay_shortcut = !p.replay_shortcut, cx)),
            enabled,
        );
        if p.screenshot_shortcut && shot == ChordButton::X {
            save.note = Some(tr!("Off while Guide + X takes the screenshot: pick another button for it.").to_string());
        }
        let key = replay::accelerator(d, cx).unwrap_or_else(|| tr!("None").to_string());
        save.key = key_edit("controls-key-replay", key, Section::Replay, cx).into_any_element();
    } else {
        save.part = None;
        save.pad = div().text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Not on this system")).into_any_element();
    }
    v.push(save);

    let mut moment = Line::new(tr!("Mark a moment"), Some(Part::Y));
    moment.pad = pad_cell(
        guide_with("Y"),
        Some(overlay_switch("controls-sw-moment", p.moment_shortcut, |p| p.moment_shortcut = !p.moment_shortcut, cx)),
        enabled,
    );
    if p.screenshot_shortcut && shot == ChordButton::Y {
        moment.note = Some(crate::overlay::labels::moment_taken_by_screenshot().to_string());
    }
    v.push(moment);

    let mut volume = Line::new(tr!("Volume up / down"), Some(Part::DPad));
    volume.extra = Some(format!("· {}", tr!("{step} steps", step = overlay::step_label(p.volume_step))));
    volume.pad = pad_cell(
        guide_with("↑↓"),
        Some(overlay_switch("controls-sw-volume", p.volume_shortcut, |p| p.volume_shortcut = !p.volume_shortcut, cx)),
        enabled,
    );
    let steps: Vec<(u8, &'static str)> = VOLUME_STEPS.iter().map(|s| (*s, overlay::step_label(*s))).collect();
    let page = cx.entity().downgrade();
    volume.button = tabs("controls-volume-step", &steps, p.volume_step, move |v, _, cx| {
        page.update(cx, |d, cx| overlay::update(d, cx, |p| p.volume_step = v)).ok();
    })
    .into_any_element();
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
    let mut enter = Line::new(tr!("Enter console mode"), part);
    enter.note = Some(tr!("Not while a game runs").to_string());
    if !keys.is_empty() {
        enter.pad = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(tr!("hold")))
            .child(chord(keys))
            .into_any_element();
    }
    let options: Vec<(SharedString, bool)> =
        ConsoleButton::CHOICES.iter().map(|b| (SharedString::from(translate(console_button_name(*b))), *b == button)).collect();
    let select = select_field(
        "controls-console-button",
        None,
        translate(console_button_name(button)).to_string(),
        false,
        d.controls.console_button_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.controls.console_button_open = !d.controls.console_button_open;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.controls.console_button_open = false;
            crate::console::startup::set_console_button(&d.ctx.app, ConsoleButton::CHOICES[i]);
            crate::console::summon::options_changed(cx);
            cx.notify();
        },
    );
    enter.button = div().w(px(BUTTON_W)).child(select).into_any_element();
    let mut v = vec![enter];

    if supported(Feature::Overlay) {
        let p = &d.overlay.prefs;
        let mut anywhere = Line::new(tr!("Open the overlay without a game too"), None);
        anywhere.pad = if p.enabled {
            overlay_switch("controls-sw-without-game", p.without_game, |p| p.without_game = !p.without_game, cx)
        } else {
            waits().into_any_element()
        };
        anywhere.note = Some(
            if !p.enabled {
                tr!("Needs the overlay")
            } else if shares_guide(d) && cfg!(windows) {
                tr!("Then a tap and a hold of Back + Start do different things")
            } else if shares_guide(d) {
                tr!("Then a tap and a hold of Guide do different things")
            } else {
                crate::overlay::labels::without_game_short()
            }
            .to_string(),
        );
        anywhere.key = div().into_any_element();
        v.push(anywhere);
    }
    v
}

/// While the overlay is open: fixed.
fn overlay_open() -> Vec<Line> {
    let mut nav = Line::new(tr!("Move · select · back"), Some(Part::A));
    nav.pad = glyphs(&["D-pad", "A", "B"]).into_any_element();
    nav.key = key_cap(tr!("Arrows · Enter · Esc")).into_any_element();
    nav.button = lock();
    let mut tabs_line = Line::new(tr!("Overlay tabs"), Some(Part::RB));
    tabs_line.pad = glyphs(&["LB", "RB"]).into_any_element();
    tabs_line.key = key_cap("Q / E").into_any_element();
    tabs_line.button = lock();
    vec![nav, tabs_line]
}

/// Console mode, as its theme has it (read-only here).
fn console() -> Vec<Line> {
    let theme = crate::console::look::theme();
    let swap = theme.controls.swap_ab;
    let on_off = |b: bool| if b { tr!("On") } else { tr!("Off") };
    let fixed = |action: &'static str, part: Part, pad: &[&str], key: &'static str| {
        let mut l = Line::new(action, Some(part));
        l.pad = glyphs(pad).into_any_element();
        l.key = key_cap(key).into_any_element();
        l.button = lock();
        l
    };
    let mut opens = Line::new(tr!("A opens"), Some(if swap { Part::B } else { Part::A }));
    opens.pad = glyph(if swap { "B" } else { "A" }).into_any_element();
    opens.key = key_cap("Enter").into_any_element();
    opens.note = Some(tr!("X does the other one").to_string());
    opens.button = theme_value(if theme.nav.a_button == AButton::Details { tr!("Game page") } else { tr!("Play") });
    let mut east = Line::new(tr!("Confirm with the east button"), Some(Part::B));
    east.pad = glyph("B").into_any_element();
    east.note = Some(tr!("The Nintendo way: east confirms, south goes back.").to_string());
    east.button = theme_value(on_off(swap));
    let mut hints = Line::new(tr!("Hint bar"), None);
    hints.button = theme_value(match theme.controls.hints {
        Hints::Always => tr!("Always"),
        Hints::Auto => tr!("Fades by itself"),
        Hints::Off => tr!("Hidden"),
    });
    let mut sounds = Line::new(tr!("Navigation sounds"), None);
    sounds.button = theme_value(on_off(theme.controls.sounds));
    let mut wrap = Line::new(tr!("Wrap around at the ends"), None);
    wrap.button = theme_value(on_off(theme.nav.wrap));
    vec![
        opens,
        east,
        fixed(tr!("Search and filters"), Part::Y, &["Y"], "Y"),
        fixed(tr!("Previous or next tab"), Part::RB, &["LB", "RB"], "Q / E"),
        fixed(tr!("Jump by letter or by page"), Part::RT, &["LT", "RT"], "Page Up / Page Down"),
        fixed(tr!("Menu"), Part::Menu, &["Menu"], "M"),
        hints,
        sounds,
        wrap,
    ]
}

/// The table, the groups "Show" asks for.
pub(super) fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let show = d.controls.show;
    let picked = d.controls.selected;
    let head = |t: &'static str, w: Option<f32>, right: bool| {
        let c = div().text_size(px(12.)).text_color(theme::text_dim()).child(t);
        let c = match w {
            Some(w) => c.flex_none().w(px(w)),
            None => c.flex_1(),
        };
        if right {
            c.flex().justify_end()
        } else {
            c
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
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(9.))
                .child(head(tr!("Action"), None, false))
                .child(head(tr!("Controller"), Some(PAD_W), false))
                .child(head(tr!("Keyboard"), Some(KEY_W), false))
                .child(head(tr!("Button"), Some(BUTTON_W), true)),
        );
    let mut i = 0;
    let mut add = |table: Div, lines: Vec<Line>, cx: &mut Context<SettingsPage>| {
        let mut t = table;
        for l in lines {
            t = t.child(line_el(i, l, picked, cx));
            i += 1;
        }
        t
    };
    if supported(Feature::Overlay) && show.has(Show::InGame) {
        let enabled = d.overlay.prefs.enabled;
        let caption = match (enabled, cfg!(windows)) {
            (true, false) => tr!("In a game · overlay on"),
            (true, true) => tr!("In a game · the game also gets these buttons · overlay on"),
            (false, false) => tr!("In a game · the controller shortcuts wait for the overlay (off)"),
            (false, true) => tr!("In a game · the game also gets these buttons · the controller shortcuts wait for the overlay (off)"),
        };
        let turn_on = (!enabled).then(|| {
            button_sm("controls-overlay-on", ButtonKind::Primary, tr!("Turn on the overlay"))
                .on_click(cx.listener(|d, _, _, cx| overlay::turn_on(d, cx)))
                .into_any_element()
        });
        table = table.child(group(caption.to_string(), turn_on));
        let lines = in_game(d, cx);
        table = add(table, lines, cx);
    }
    if show.has(Show::Outside) {
        table = table.child(group(tr!("Outside games · this PC").to_string(), None));
        let lines = outside(d, cx);
        table = add(table, lines, cx);
    }
    if supported(Feature::Overlay) && show.has(Show::Overlay) {
        table = table.child(group(tr!("Overlay open").to_string(), None));
        table = add(table, overlay_open(), cx);
    }
    if show.has(Show::Console) {
        let name = crate::console::look::theme().name.clone();
        let caption = if name.trim().is_empty() {
            tr!("Console mode · from its theme").to_string()
        } else {
            tr!("Console mode · from its theme ({name})", name = name.trim())
        };
        let change = button_sm("controls-console-look", ButtonKind::Glass, tr!("Change in Console mode"))
            .on_click(cx.listener(|d, _, window, cx| d.select(Section::ConsoleMode, window, cx)));
        table = table.child(group(caption, Some(change.into_any_element())));
        table = add(table, console(), cx);
    }
    table
}

/// "Change" next to the screenshot's chord, and the sheet it opens.
fn change_button(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let open = d.controls.shot_pick;
    let button = button_sm("controls-shot-change", ButtonKind::Glass, tr!("Change")).on_click(cx.listener(|d, _, _, cx| {
        d.controls.shot_pick = match d.controls.shot_pick {
            Some(_) => None,
            None => Some(overlay::screenshot_button(&d.overlay.prefs)),
        };
        cx.notify();
    }));
    let mut wrap = div().relative().child(button);
    if let Some(pick) = open {
        let sheet = shot_sheet(d, pick, cx);
        wrap = wrap.child(deferred(div().absolute().right_0().top(px(36.)).occlude().child(sheet)).with_priority(40));
    }
    wrap.into_any_element()
}

/// The sheet that picks the button held with Guide for the screenshot: the
/// buttons this system allows, what a taken one turns off, and Save.
fn shot_sheet(d: &SettingsPage, pick: ChordButton, cx: &mut Context<SettingsPage>) -> Div {
    let p = &d.overlay.prefs;
    let how = if cfg!(windows) { tr!("Pick the button held with Back + Start") } else { tr!("Pick the button held with Guide") };
    let keep = crate::overlay::labels::screenshot_button_desc();
    let combo = crate::overlay::labels::chord_text("").trim_end_matches(" + ").to_string();
    let combo = format!("{combo} + {}", overlay::button_short(pick));
    let mut options = div().flex().flex_wrap().justify_center().gap(px(6.)).mt(px(10.));
    for b in overlay::screenshot_buttons() {
        let on = b == pick;
        let label = overlay::button_short(b);
        options = options.child(
            div()
                .id(SharedString::from(format!("controls-shot-pick-{}", b.as_str())))
                .rounded_full()
                .p(px(2.))
                .when(on, |d| d.bg(theme::accent()))
                .cursor_pointer()
                .child(glyph(&label))
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.controls.shot_pick = Some(b);
                    cx.notify();
                })),
        );
    }
    let conflict = match pick {
        ChordButton::Y if p.moment_shortcut => Some(if cfg!(windows) {
            tr!("Back + Start + Y marks a moment. With Y here, Mark a moment is off while the screenshot uses it.")
        } else {
            tr!("Guide + Y marks a moment. With Y here, Mark a moment is off while the screenshot uses it.")
        }),
        ChordButton::X if supported(Feature::Replay) && p.replay_shortcut => {
            Some(tr!("Guide + X saves the replay. With X here, Guide + X is off for the replay while the screenshot uses it."))
        }
        _ => None,
    };
    let warn: Hsla = theme::warning().into();
    let title = div()
        .flex()
        .items_start()
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Change the button")))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .text_size(px(17.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(tr!("Take a screenshot")),
                ),
        )
        .child(button_sm("controls-shot-cancel", ButtonKind::Glass, tr!("Cancel")).on_click(cx.listener(|d, _, _, cx| {
            d.controls.shot_pick = None;
            cx.notify();
        })));
    let capture = div()
        .mt(px(14.))
        .p(px(16.))
        .rounded(px(14.))
        .border_1()
        .border_dashed()
        .border_color(theme::accent_alpha(0.5))
        .flex()
        .flex_col()
        .items_center()
        .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(how))
        .child(div().mt(px(4.)).text_size(px(11.5)).text_color(theme::text_dim()).text_center().child(keep))
        .child(
            div()
                .mt(px(8.))
                .font_family(theme::font_heading())
                .text_size(px(20.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(combo),
        )
        .child(options);
    let footer = div()
        .flex()
        .items_center()
        .mt(px(12.))
        .child(button_sm("controls-shot-default", ButtonKind::Glass, tr!("Back to RB")).on_click(cx.listener(|d, _, _, cx| {
            d.controls.shot_pick = Some(ChordButton::RB);
            cx.notify();
        })))
        .child(div().flex_1())
        .child(button_sm("controls-shot-save", ButtonKind::Primary, tr!("Save")).on_click(cx.listener(move |d, _, _, cx| {
            d.controls.shot_pick = None;
            overlay::update(d, cx, |p| p.screenshot_button = pick);
        })));
    crate::ui::dropdown::menu_panel(370.)
        .w(px(370.))
        .p(px(16.))
        .child(title)
        .child(capture)
        .children(
            conflict.map(|text| {
                div().mt(px(10.)).px(px(12.)).py(px(10.)).rounded(px(12.)).bg(warn.opacity(0.14)).text_size(px(12.5)).child(text)
            }),
        )
        .child(footer)
}
