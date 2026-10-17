//! Text inputs: single line, multi-line, path (with clear / browse icon) and
//! the platform file chooser.

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::text::{field_label, labeled};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

/// The glass input surface shared by inputs, textareas, selects and the tag
/// field: height by the caller, radius 12, dark fill, 1px border, 14px text.
pub(super) fn input_surface() -> Div {
    div()
        .rounded(px(theme::R_INPUT))
        .bg(theme::input_fill())
        .border_1()
        .border_color(theme::input_border())
        .font_family(theme::FONT_BODY)
        .text_size(px(14.))
        .text_color(theme::text_primary())
}

/// Accent ring (60%) painted over an input surface while `input` has focus.
fn focus_ring<T: 'static + Focusable>(input: &Entity<T>) -> Canvas<()> {
    let input = input.clone();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            if input.focus_handle(cx).is_focused(window) {
                window.paint_quad(quad(
                    bounds,
                    Corners::all(px(theme::R_INPUT)),
                    transparent_black(),
                    Edges::all(px(1.)),
                    theme::input_border_focus(),
                    BorderStyle::Solid,
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

/// A new single-line input holding `value`.
pub fn new_input(window: &mut Window, cx: &mut App, value: &str, placeholder: &str) -> Entity<InputState> {
    let (value, placeholder) = (value.to_string(), placeholder.to_string());
    cx.new(|cx| {
        let mut s = InputState::new(window, cx);
        s.set_placeholder(placeholder, window, cx);
        s.set_value(value, window, cx);
        s
    })
}

/// A new multi-line input of `rows` rows holding `value`.
pub fn new_textarea(window: &mut Window, cx: &mut App, value: &str, rows: usize) -> Entity<TextareaState> {
    let value = value.to_string();
    cx.new(|cx| {
        let mut s = TextareaState::new(window, cx).rows(rows);
        s.set_value(value, window, cx);
        s
    })
}

/// Calls `on_commit` with the text when the input loses focus or Enter is
/// pressed (Heroic's `onBlur` commit of `PathSelectionBox`).
pub fn on_commit<V: 'static>(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<V>,
    on_commit: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static,
) {
    cx.subscribe_in(input, window, move |this, state, ev: &InputEvent, window, cx| {
        if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
            let value = state.read(cx).value().trim().to_string();
            on_commit(this, value, window, cx);
        }
    })
    .detach();
}

/// 40px glass input.
pub fn input_box(input: &Entity<InputState>, disabled: bool) -> Div {
    field_row(input, disabled, None, None)
}

/// Input row: optional leading element, the text input, optional trailing one.
fn field_row(input: &Entity<InputState>, disabled: bool, leading: Option<AnyElement>, trailing: Option<AnyElement>) -> Div {
    input_surface()
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .h(px(theme::INPUT_H))
        .px(px(14.))
        .when(disabled, |d| d.opacity(0.5))
        .children(leading)
        .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).disabled(disabled)))
        .children(trailing)
        .child(focus_ring(input))
}

/// A duration as one field: a clock, the hours ("h") and the minutes ("min")
/// typed in place on a single input surface (the time played of a game).
pub fn duration_field(hours: &Entity<InputState>, minutes: &Entity<InputState>) -> Div {
    let unit = |text: SharedString| div().flex_none().text_size(px(13.)).text_color(theme::text_muted()).child(text);
    // Right-aligned, so the number sits against its unit.
    let number = |input: &Entity<InputState>, w: f32| div().w(px(w)).flex_none().child(Input::new(input).appearance(false).text_right());
    input_surface()
        .relative()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(theme::INPUT_H))
        .pl(px(12.))
        .pr(px(14.))
        .child(icon(Icon::AccessTime, px(18.), theme::text_muted()).mr(px(4.)))
        .child(number(hours, 52.))
        .child(unit(tr!("h").into()))
        .child(div().w(px(4.)).flex_none())
        .child(number(minutes, 36.))
        .child(unit(tr!("min").into()))
        .child(focus_ring(hours))
        .child(focus_ring(minutes))
}

/// Multi-line input of `rows` rows (body font).
pub fn textarea_box(state: &Entity<TextareaState>, rows: usize) -> Div {
    input_surface()
        .relative()
        .w_full()
        .h(px(rows as f32 * 20. + 22.))
        .px(px(12.))
        .py(px(10.))
        .child(Textarea::new(state).appearance(false).h_full())
        .child(focus_ring(state))
}

/// Search bar of the artwork pickers: magnifier + borderless input on the
/// glass input surface. Grows to fill the row (`flex_1`).
pub fn search_box(input: &Entity<InputState>) -> Div {
    search_row(input, None)
}

/// [`search_box`] with `trailing` after the text (a clear button).
pub fn search_box_with(input: &Entity<InputState>, trailing: AnyElement) -> Div {
    search_row(input, Some(trailing))
}

fn search_row(input: &Entity<InputState>, trailing: Option<AnyElement>) -> Div {
    field_row(input, false, Some(icon(Icon::FaSearch, px(14.), theme::text_muted()).into_any_element()), trailing)
        .flex_1()
        .w_auto()
        .min_w(px(192.))
}

/// Labelled text input.
pub fn text_field(label: &str, input: &Entity<InputState>) -> Div {
    labeled(label.to_string(), input_box(input, false))
}

/// `PathSelectionBox`: a folder icon, the input and a text action at the
/// right (accent): "Clear" when there is a value, "Browse" otherwise.
/// `on_icon` receives the click.
pub fn path_field(
    id: impl Into<ElementId>,
    label: Option<&str>,
    input: &Entity<InputState>,
    has_value: bool,
    disabled: bool,
    on_icon: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let action = div()
        .id(id)
        .flex_none()
        .px(px(4.))
        .text_size(px(13.))
        .text_color(if disabled { theme::text_dim() } else { theme::accent() })
        .when(!disabled, |d| d.cursor_pointer().hover(|s| s.text_color(theme::accent_overlay())).on_click(on_icon))
        .child(if has_value { tr!("Clear") } else { tr!("Browse") })
        .into_any_element();
    div().flex().flex_col().children(label.map(|l| field_label(l.to_string()))).child(field_row(
        input,
        disabled,
        Some(icon(Icon::Folder, px(16.), theme::text_muted()).into_any_element()),
        Some(action),
    ))
}

/// Opens the platform file chooser and reports the choice.
pub fn choose_path<V: 'static>(
    window: &mut Window,
    cx: &mut Context<V>,
    directory: bool,
    prompt: &str,
    done: impl FnOnce(&mut V, String, &mut Window, &mut Context<V>) + 'static,
) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: !directory,
        directories: directory,
        multiple: false,
        prompt: Some(prompt.to_string().into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        if let Ok(Ok(Some(paths))) = rx.await {
            if let Some(p) = paths.first() {
                let p = p.to_string_lossy().to_string();
                this.update_in(cx, |this, window, cx| done(this, p, window, cx)).ok();
            }
        }
    })
    .detach();
}
