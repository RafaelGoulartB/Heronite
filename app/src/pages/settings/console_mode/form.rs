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
use crate::theme;
use crate::ui::controls::{button_sm, row, toggle_row, ButtonKind};
use crate::ui::fields::{input_box, new_input, on_commit, select_field};

/// A text input kept between frames: `shown` is the text last put in it.
pub(in crate::pages::settings) struct Field {
    input: Entity<InputState>,
    shown: String,
}

/// More choices than this become a select instead of segments.
const SEG_MAX: usize = 4;

/// Segmented pills like `controls::tabs`, with runtime labels.
pub(super) fn seg(id: &str, options: &[SharedString], active: usize, enabled: bool, on_pick: impl Fn(usize, &mut Window, &mut App) + 'static) -> Div {
    let on_pick = std::rc::Rc::new(on_pick);
    let mut track = div()
        .id(SharedString::from(format!("{id}-seg")))
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(4.))
        .flex_wrap()
        .max_w_full()
        .rounded(px(18.))
        .bg(theme::seg_track())
        .when(!enabled, |d| d.opacity(0.45));
    for (i, label) in options.iter().enumerate() {
        let on = i == active;
        let cb = on_pick.clone();
        track = track.child(
            div()
                .id(SharedString::from(format!("{id}-tab-{i}")))
                .flex_none()
                .px(px(12.))
                .py(px(6.))
                .rounded_full()
                .text_size(px(13.))
                .whitespace_nowrap()
                .text_color(if on { theme::text_bright() } else { theme::seg_text().into() })
                .when(on, |d| d.bg(theme::seg_active()))
                .when(enabled && !on, |d| d.cursor_pointer().hover(|s| s.text_color(theme::text_bright())))
                .when(enabled, |d| d.on_click(move |_, w, cx| cb(i, w, cx)))
                .child(label.clone()),
        );
    }
    div().flex().min_w_0().child(track)
}

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

/// One option as a row of a card.
pub(super) fn option_row(d: &mut SettingsPage, r: &Row, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let desc = r.desc.clone();
    let id = r.id.clone();
    let enabled = r.enabled;
    let label = r.label.clone();
    let desc_str = desc.as_deref();
    match &r.value {
        Value::Toggle(on) => {
            let on = *on;
            let sw = toggle_row(SharedString::from(format!("cm-{id}")), label, desc_str, on).when(!enabled, |d| d.opacity(0.45));
            if !enabled {
                return sw.into_any_element();
            }
            sw.on_click(cx.listener(move |d, _, window, cx| send(d, Outcome::Changed(id.clone(), Value::Toggle(!on)), window, cx))).into_any_element()
        }
        Value::Choice { options, index } => {
            let options = options.clone();
            let control = if options.len() <= SEG_MAX {
                let (id2, opts) = (id.clone(), options.clone());
                let entity = cx.entity().downgrade();
                seg(&format!("cm-{id}"), &options, *index, enabled, move |i, window, cx| {
                    let v = Value::Choice { options: opts.clone(), index: i };
                    let o = Outcome::Changed(id2.clone(), v);
                    entity.update(cx, |d, cx| send(d, o, window, cx)).ok();
                })
            } else {
                choice_select(d, &id, &options, *index, enabled, cx)
            };
            row(label, desc_str, div().max_w(px(520.)).child(control)).when(!enabled, |d| d.opacity(0.45)).into_any_element()
        }
        Value::Number { value, min, max, step, suffix } => {
            let (value, min, max, step) = (*value, *min, *max, *step);
            let make = |id: SharedString, delta: i64, text: &'static str| {
                let go = (value + delta).clamp(min, max) != value;
                let (id2, suffix2) = (id.clone(), *suffix);
                button_sm(SharedString::from(format!("cm-{id}-{text}")), ButtonKind::Glass, text).when(!go || !enabled, |d| d.opacity(0.4)).when(go && enabled, |d| {
                    d.on_click(cx.listener(move |d, _, window, cx| {
                        let v = Value::Number { value: (value + delta).clamp(min, max), min, max, step, suffix: suffix2 };
                        send(d, Outcome::Changed(id2.clone(), v), window, cx);
                    }))
                })
            };
            let control = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(make(id.clone(), -step, "\u{2212}"))
                .child(div().w(px(64.)).flex().justify_center().text_size(px(14.)).text_color(theme::text_primary()).child(format!("{value}{suffix}")))
                .child(make(id.clone(), step, "+"));
            row(label, desc_str, control).when(!enabled, |d| d.opacity(0.45)).into_any_element()
        }
        Value::Text(text) => {
            let key = id.to_string();
            let input = text_input(d, &key, &id, text, "", window, cx).w(px(260.));
            let control = if id.as_ref() == "background.image" {
                let browse = button_sm("cm-bg-browse", ButtonKind::Glass, tr!("Browse")).on_click(cx.listener(|_, _, window, cx| {
                    crate::ui::fields::choose_path(window, cx, false, tr!("Select"), |d, path, window, cx| {
                        crate::console::theme_rows::text_done(d, "background.image", path, window, cx);
                    });
                }));
                div().flex().items_center().gap(px(8.)).child(input).child(browse)
            } else if id.starts_with("colors.") {
                let id2 = id.to_string();
                let reset = button_sm(SharedString::from(format!("cm-{id}-reset")), ButtonKind::Ghost, tr!("Reset"))
                    .on_click(cx.listener(move |d, _, window, cx| crate::console::theme_rows::text_done(d, &id2, String::new(), window, cx)));
                div().flex().items_center().gap(px(8.)).child(input).child(reset)
            } else {
                div().child(input)
            };
            row(label, desc_str, control).when(!enabled, |d| d.opacity(0.45)).into_any_element()
        }
        Value::Action | Value::Info(_) | Value::Header => div().into_any_element(),
    }
}

pub(super) fn choice_select(d: &mut SettingsPage, id: &SharedString, options: &[SharedString], index: usize, enabled: bool, cx: &mut Context<SettingsPage>) -> Div {
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
