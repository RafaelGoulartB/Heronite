//! Text inputs: single line, multi-line, path (with clear / browse icon) and
//! the platform file chooser.

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::text::{field_label, labeled, text_default};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The `--input-background` surface shared by inputs, textareas, selects and
/// the tag field: radius 4, `0 4px 4px` shadow, `Rubik` 16px white text.
pub(super) fn input_surface() -> Div {
    div()
        .rounded(px(4.))
        .bg(theme::navbar_background())
        .shadow(vec![BoxShadow {
            color: theme::black_a(0.25),
            offset: point(px(0.), px(4.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_family(theme::FONT_HEADING)
        .text_size(px(16.))
        .text_color(white())
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

/// 40px input on `--input-background` (text indent 22px in Heroic).
pub fn input_box(input: &Entity<InputState>, disabled: bool) -> Div {
    input_surface()
        .relative()
        .flex()
        .items_center()
        .w_full()
        .h(px(40.))
        .px(px(10.))
        .when(disabled, |d| d.opacity(0.7))
        .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).disabled(disabled)))
}

/// Multi-line input of `rows` rows. Long prose reads better in the body
/// font than in the `Rubik` of the single-line inputs.
pub fn textarea_box(state: &Entity<TextareaState>, rows: usize) -> Div {
    input_surface()
        .font_family(theme::FONT_BODY)
        .w_full()
        .h(px(rows as f32 * 22. + 20.))
        .px(px(10.))
        .py(px(8.))
        .child(Textarea::new(state).appearance(false).h_full())
}

/// Search bar of the artwork pickers: magnifier + borderless input on a dark
/// translucent pill. Grows to fill the row (`flex_1`).
pub fn search_box(input: &Entity<InputState>) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w(px(192.))
        .items_center()
        .gap(px(8.))
        .px(px(11.))
        .py(px(5.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::white_a(0.12))
        .bg(theme::black_a(0.28))
        .child(icon(Icon::FaSearch, px(13.), theme::white_a(0.6)))
        .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false)))
}

/// Labelled text input.
pub fn text_field(label: &str, input: &Entity<InputState>) -> Div {
    labeled(label.to_string(), input_box(input, false))
}

/// `PathSelectionBox`: an input with a Backspace (clear) / Folder (browse)
/// icon at the right. `on_icon` receives the click.
pub fn path_field(
    id: impl Into<ElementId>,
    label: Option<&str>,
    input: &Entity<InputState>,
    has_value: bool,
    disabled: bool,
    on_icon: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let btn = div()
        .id(id)
        .absolute()
        .right(px(16.))
        .top(px(8.))
        .px(px(8.))
        .py(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .when(!disabled, |d| d.cursor_pointer().on_click(on_icon))
        .child(icon(if has_value { Icon::Backspace } else { Icon::Folder }, px(22.), text_default()));
    div()
        .flex()
        .flex_col()
        .children(label.map(|l| field_label(l.to_string())))
        .child(input_box(input, disabled).pr(px(50.)).child(btn))
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
