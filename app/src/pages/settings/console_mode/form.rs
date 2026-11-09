//! The generic part of the desktop page: a console [`Row`] drawn as a
//! settings row (switch, segmented choice, select, stepper, text input), so
//! every option is written once (`console::theme_rows`) and shows here and
//! on the TV.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{send, SettingsPage};
use crate::console::list::{Outcome, Row, Value};
use crate::console::theme_rows;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, row, segmented, toggle, ButtonKind};
use crate::ui::fields::{input_box, new_input, on_commit, select_field};

/// A text input kept between frames: `shown` is the text last put in it.
pub(in crate::pages::settings) struct Field {
    input: Entity<InputState>,
    shown: String,
}

/// More choices than this, or longer names than [`SEG_CHARS`] together,
/// become a select instead of segments.
const SEG_MAX: usize = 4;
const SEG_CHARS: usize = 44;

/// The input kept under `key`, created on first use; it shows `current`
/// unless it is being typed in. Pressing Enter or leaving it sends the text
/// as the command `id`.
pub(super) fn text_input(
    d: &mut SettingsPage,
    key: &str,
    id: &str,
    current: &str,
    placeholder: &str,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> Div {
    let exists = d.console_mode.as_ref().is_some_and(|s| s.fields.contains_key(key));
    if !exists {
        let input = new_input(window, cx, current, placeholder);
        let key_ = key.to_string();
        let id_ = id.to_string();
        on_commit(&input, window, cx, move |d, text, window, cx| commit(d, &key_, &id_, text, window, cx));
        if let Some(s) = d.console_mode.as_mut() {
            s.fields.insert(key.to_string(), Field { input, shown: current.to_string() });
        }
    }
    let Some(f) = d.console_mode.as_mut().and_then(|s| s.fields.get_mut(key)) else { return div() };
    if f.shown != current && !f.input.focus_handle(cx).is_focused(window) {
        f.shown = current.to_string();
        let text = current.to_string();
        f.input.update(cx, |s, cx| s.set_value(text, window, cx));
    }
    input_box(&f.input, false)
}

fn commit(d: &mut SettingsPage, key: &str, id: &str, text: String, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(f) = d.console_mode.as_mut().and_then(|s| s.fields.get_mut(key)) else { return };
    if f.shown == text {
        return;
    }
    // Whatever came of it, the next frame shows the real value again.
    f.shown = String::from("\u{0}");
    crate::console::theme_rows::text_done(d, id, text, window, cx);
}

/// A bare ↺ (accent under the pointer) that puts the option `id` back as
/// the theme the one in use started from has it; `None` while it is the same.
pub(super) fn reset_button(d: &SettingsPage, id: &str, cx: &mut Context<SettingsPage>) -> Option<Stateful<Div>> {
    let s = d.console_mode.as_ref()?;
    let base = theme_rows::base_theme(&s.theme.prefs, &s.theme.data.saved)?;
    let cmd = theme_rows::reset_command(id, &s.theme.prefs, &base).filter(|_| theme_rows::differs(id, &s.theme.prefs, &base))?;
    let hover = SharedString::from(format!("cm-reset-{id}"));
    let name = theme_rows_name(&s.theme.prefs, &s.theme.data.saved);
    Some(
        div()
            .id(hover.clone())
            .group(hover.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .tooltip(crate::ui::tooltip(tr!("Back to how {name} has it", name = name)))
            .child(crate::ui::icon(Icon::Replay, px(15.), theme::text_dim()).group_hover(hover, |s| s.text_color(theme::accent())))
            .on_click(cx.listener(move |d, _, _, cx| {
                cx.stop_propagation();
                if let Some(s) = d.console_mode.as_mut() {
                    s.theme.end_step();
                }
                theme_rows::change(d, &cmd, cx);
            })),
    )
}

/// The shown name of the theme the one in use started from.
fn theme_rows_name(p: &heronite_data::console_theme::ConsolePrefs, saved: &[heronite_data::console_theme::ConsoleTheme]) -> String {
    theme_rows::base_theme(p, saved).map(|b| super::home::shown_name(&b)).unwrap_or_default()
}

/// One option as a row of a card, with its ↺ while it differs from the
/// theme it started from.
pub(super) fn option_row(d: &mut SettingsPage, r: &Row, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    if r.id.as_ref() == "interface.accent" {
        return super::colours::accent_row(d, r, window, cx);
    }
    let desc = r.desc.as_deref();
    let reset = if r.enabled { reset_button(d, &r.id, cx) } else { None };
    if let Value::Toggle(on) = r.value {
        let id = r.id.clone();
        let sw = toggle(SharedString::from(format!("cm-{id}-switch")), on, !r.enabled);
        let line = row(r.label.clone(), desc, div().flex().items_center().gap(px(8.)).children(reset).child(sw));
        if !r.enabled {
            return line.opacity(0.45).into_any_element();
        }
        return line
            .id(SharedString::from(format!("cm-{id}")))
            .cursor_pointer()
            .on_click(cx.listener(move |d, _, window, cx| send(d, Outcome::Changed(id.clone(), Value::Toggle(!on)), window, cx)))
            .into_any_element();
    }
    let Some(control) = control(d, r, window, cx) else { return div().into_any_element() };
    let control = div().flex().items_center().gap(px(8.)).children(reset).child(div().max_w(px(520.)).child(control));
    row(r.label.clone(), desc, control).when(!r.enabled, |d| d.opacity(0.45)).into_any_element()
}

/// The control of an option alone (segments, a select, a stepper, a text
/// input, a switch), for rows laid out another way.
pub(super) fn control(d: &mut SettingsPage, r: &Row, window: &mut Window, cx: &mut Context<SettingsPage>) -> Option<AnyElement> {
    control_in(d, r, SEG_CHARS, window, cx)
}

/// [`control`] for a narrow place: choices whose names take more than
/// `seg_chars` together become a select.
pub(super) fn control_in(
    d: &mut SettingsPage,
    r: &Row,
    seg_chars: usize,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> Option<AnyElement> {
    let id = r.id.clone();
    let enabled = r.enabled;
    Some(match &r.value {
        Value::Toggle(on) => {
            let on = *on;
            toggle(SharedString::from(format!("cm-{id}-switch")), on, !enabled)
                .when(enabled, |t| {
                    t.on_click(cx.listener(move |d, _, window, cx| send(d, Outcome::Changed(id.clone(), Value::Toggle(!on)), window, cx)))
                })
                .into_any_element()
        }
        Value::Choice { options, index } => {
            let options = options.clone();
            if options.len() <= SEG_MAX && options.iter().map(|o| o.chars().count()).sum::<usize>() <= seg_chars {
                let (id2, opts) = (id.clone(), options.clone());
                let entity = cx.entity().downgrade();
                let items: Vec<(usize, SharedString)> = options.iter().cloned().enumerate().collect();
                segmented(&format!("cm-{id}"), &items, *index, enabled, move |i, window, cx| {
                    let v = Value::Choice { options: opts.clone(), index: i };
                    let o = Outcome::Changed(id2.clone(), v);
                    entity.update(cx, |d, cx| send(d, o, window, cx)).ok();
                })
                .into_any_element()
            } else {
                choice_select(d, &id, &options, *index, enabled, cx).into_any_element()
            }
        }
        Value::Number { value, min, max, step, suffix } => {
            let (value, min, max, step) = (*value, *min, *max, *step);
            let make = |id: SharedString, delta: i64, text: &'static str| {
                let go = (value + delta).clamp(min, max) != value;
                let (id2, suffix2) = (id.clone(), *suffix);
                button_sm(SharedString::from(format!("cm-{id}-{text}")), ButtonKind::Glass, text)
                    .when(!go || !enabled, |d| d.opacity(0.4))
                    .when(go && enabled, |d| {
                        d.on_click(cx.listener(move |d, _, window, cx| {
                            let v = Value::Number { value: (value + delta).clamp(min, max), min, max, step, suffix: suffix2 };
                            send(d, Outcome::Changed(id2.clone(), v), window, cx);
                        }))
                    })
            };
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(make(id.clone(), -step, "\u{2212}"))
                .child(
                    div()
                        .w(px(64.))
                        .flex()
                        .justify_center()
                        .text_size(px(14.))
                        .text_color(theme::text_primary())
                        .child(format!("{value}{suffix}")),
                )
                .child(make(id.clone(), step, "+"))
                .into_any_element()
        }
        Value::Text(text) => {
            let key = id.to_string();
            if id.as_ref() == "background.image" {
                return Some(picture_control(d, text, window, cx).into_any_element());
            }
            let input = text_input(d, &key, &id, text, "", window, cx).w(px(260.));
            if id.starts_with("colors.") {
                let id2 = id.to_string();
                let reset = button_sm(SharedString::from(format!("cm-{id}-reset")), ButtonKind::Ghost, tr!("Reset")).on_click(
                    cx.listener(move |d, _, window, cx| crate::console::theme_rows::text_done(d, &id2, String::new(), window, cx)),
                );
                div().flex().items_center().gap(px(8.)).child(input).child(reset).into_any_element()
            } else {
                div().child(input).into_any_element()
            }
        }
        Value::Action | Value::Info(_) | Value::Header => return None,
    })
}

/// The theme's own picture: a thumbnail, a file or web address to type,
/// Browse and Remove.
fn picture_control(d: &mut SettingsPage, current: &str, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let input = text_input(d, "background.image", "background.image", current, tr!("A file, or https://…"), window, cx).w(px(220.));
    let browse = button_sm("cm-bg-browse", ButtonKind::Glass, tr!("Browse")).on_click(cx.listener(|_, _, window, cx| {
        crate::ui::fields::choose_path(window, cx, false, tr!("Select"), |d, path, window, cx| {
            crate::console::theme_rows::text_done(d, "background.image", path, window, cx);
        });
    }));
    let remove = (!current.is_empty()).then(|| {
        button_sm("cm-bg-remove", ButtonKind::Ghost, tr!("Remove")).on_click(
            cx.listener(|d, _, window, cx| crate::console::theme_rows::text_done(d, "background.image", String::new(), window, cx)),
        )
    });
    let thumb = super::home::file_thumb(d, current, 64., 36., window);
    div().flex().items_center().gap(px(8.)).child(thumb).child(input).child(browse).children(remove)
}

pub(super) fn choice_select(
    d: &mut SettingsPage,
    id: &SharedString,
    options: &[SharedString],
    index: usize,
    enabled: bool,
    cx: &mut Context<SettingsPage>,
) -> Div {
    let open = d.console_mode.as_ref().is_some_and(|s| s.open.as_deref() == Some(id.as_ref()));
    let list = options.iter().enumerate().map(|(i, o)| (o.clone(), i == index)).collect();
    let (toggle_id, pick_id, opts) = (id.clone(), id.clone(), options.to_vec());
    div().w(px(260.)).child(select_field(
        &format!("cm-{id}"),
        None,
        options.get(index).cloned().unwrap_or_default(),
        !enabled,
        open,
        list,
        cx,
        move |d: &mut SettingsPage, _, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.open = if s.open.as_deref() == Some(toggle_id.as_ref()) { None } else { Some(toggle_id.to_string()) };
            }
            cx.notify();
        },
        move |d: &mut SettingsPage, i, window, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.open = None;
            }
            send(d, Outcome::Changed(pick_id.clone(), Value::Choice { options: opts.clone(), index: i }), window, cx);
        },
    ))
}
