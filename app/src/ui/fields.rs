//! Heroic form fields used by the emulation / backup dialogs: labelled text
//! and path inputs (`TextInputField`, `PathSelectionBox`), selects
//! (`SelectField`), the dialog paragraph and the grey "panel" boxes.
//! Values come from `TextInputField/index.css`, `SelectField/index.css` and
//! the captures in `docs/reference/dialogs/`.

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::icon;

/// `--text-default` on nord-dark.
pub fn text_default() -> Rgba {
    rgb(0xcaf3fd)
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

/// Dialog paragraph (`p`, 16px, `--text-default`).
pub fn paragraph(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(text_default()).child(text.into())
}

/// Same, but `--text-secondary` (white on nord-dark), as the settings tabs use.
pub fn paragraph_secondary(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(white()).child(text.into())
}

/// Small meta line (`--text-sm`, 13.33px).
pub fn meta_text(text: impl Into<SharedString>) -> Div {
    div().text_size(px(13.33)).line_height(px(16.).max(px(13.33))).text_color(white()).child(text.into())
}

/// Field label (`label`, 16px, `--text-default`).
pub fn field_label(text: impl Into<SharedString>) -> Div {
    div().text_size(px(16.)).line_height(px(19.2)).text_color(text_default()).child(text.into())
}

/// Grey bordered box (`rgba(255,255,255,.04)`, 1px `.08`, radius 4, padding 12).
pub fn panel() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(4.))
        .bg(theme::white_a(0.04))
        .border_1()
        .border_color(theme::white_a(0.08))
}

fn box_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme::black_a(0.25),
        offset: point(px(0.), px(4.)),
        blur_radius: px(4.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// 40px input on `--input-background` (text indent 22px in Heroic).
pub fn input_box(input: &Entity<InputState>, disabled: bool) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .w_full()
        .h(px(40.))
        .px(px(10.))
        .rounded(px(4.))
        .bg(theme::navbar_background())
        .shadow(box_shadow())
        .font_family(theme::FONT_HEADING)
        .text_size(px(16.))
        .text_color(white())
        .when(disabled, |d| d.opacity(0.7))
        .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).disabled(disabled)))
}

/// Labelled text input.
pub fn text_field(label: &str, input: &Entity<InputState>) -> Div {
    div().flex().flex_col().child(field_label(label.to_string())).child(input_box(input, false))
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

/// `SelectField`: label, 40px select and, when `open`, its menu of `options`
/// `(label, selected)`. `on_toggle` opens/closes it, `on_pick(index)` picks.
#[allow(clippy::too_many_arguments)]
pub fn select_field<V: 'static>(
    id: &str,
    label: Option<&str>,
    value: impl Into<SharedString>,
    disabled: bool,
    open: bool,
    options: Vec<(SharedString, bool)>,
    cx: &mut Context<V>,
    on_toggle: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
) -> Div {
    let select = div()
        .id(SharedString::from(format!("{id}-select")))
        .relative()
        .flex()
        .items_center()
        .h(px(40.))
        .w_full()
        .pl(px(22.))
        .pr(px(44.))
        .rounded(px(4.))
        .bg(theme::navbar_background())
        .shadow(box_shadow())
        .font_family(theme::FONT_HEADING)
        .text_size(px(16.))
        .text_color(white())
        .when(disabled, |d| d.opacity(0.7).cursor_not_allowed())
        .when(!disabled, |d| d.cursor_pointer().on_click(cx.listener(move |this, _, w, cx| on_toggle(this, w, cx))))
        .child(div().min_w_0().truncate().child(value.into()))
        .child(
            div()
                .absolute()
                .right(px(0.))
                .top(px(10.))
                .bottom(px(10.))
                .w(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(rgb(0xcccccc))
                .child(icon(Icon::ExpandMore, px(18.), white())),
        );
    let mut wrap = div()
        .relative()
        .flex()
        .flex_col()
        .children(label.map(|l| field_label(l.to_string())))
        .child(select);
    if open && !disabled {
        let on_pick = Rc::new(on_pick);
        let mut menu = menu_panel(100.).w_full().max_h(px(280.)).overflow_hidden();
        for (i, (text, selected)) in options.into_iter().enumerate() {
            let on_pick = on_pick.clone();
            menu = menu.child(
                menu_item(SharedString::from(format!("{id}-opt-{i}")), selected)
                    .text_size(px(16.))
                    .child(div().min_w_0().truncate().child(text))
                    .on_click(cx.listener(move |this, _, w, cx| on_pick(this, i, w, cx))),
            );
        }
        let top = if label.is_some() { 19.2 + 44. } else { 44. };
        wrap = wrap.child(deferred(div().absolute().top(px(top)).left_0().right_0().occlude().child(menu)).with_priority(40));
    }
    wrap
}
