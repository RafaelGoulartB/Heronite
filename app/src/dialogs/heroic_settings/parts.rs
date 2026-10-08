//! Row widgets shared by the tabs.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::HeroicSettingsPanel;
use crate::theme;
use crate::ui::controls::{action_button, checkbox, ButtonKind};
use crate::ui::fields::{choose_path, field_label, input_box, path_field, select_field};

/// Info callout (`.defaults-hint`).
pub fn hint(text: &str) -> Div {
    div()
        .p(px(12.))
        .rounded(px(4.))
        .bg(theme::white_a(0.04))
        .border_1()
        .border_color(theme::white_a(0.08))
        .text_size(px(14.))
        .line_height(px(20.))
        .text_color(white())
        .child(text.to_string())
}

/// Small muted explanation (Heroic's `InfoIcon` tooltip / `InfoBox`).
pub fn help(text: &str) -> Div {
    div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::white_a(0.62)).child(text.to_string())
}

pub fn danger_text(text: &str) -> Div {
    div().text_size(px(13.33)).text_color(theme::danger()).child(text.to_string())
}

pub fn heading(text: &str) -> Div {
    div().pt(px(8.)).text_size(px(16.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child(text.to_string())
}

pub fn column() -> Div {
    div().flex().flex_col().gap(px(14.))
}

impl HeroicSettingsPanel {
    /// `ToggleSwitch` + help text. `disabled` shows it dimmed and inert.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn toggle_row(
        &self,
        id: &'static str,
        key: &'static str,
        default: bool,
        label: &str,
        help_text: Option<&str>,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let on = self.flag(key, default);
        let mut cb = checkbox(id, on, label.to_string());
        if !disabled {
            cb = cb.on_click(cx.listener(move |this, _, _, cx| this.toggle(key, default, cx)));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(disabled, |d| d.opacity(0.5))
            .child(cb)
            .children(help_text.map(|t| help(t).pl(px(41.))))
    }

    /// Labelled text input bound to input `id`.
    pub(super) fn text_row(&self, id: &'static str, label: Option<&str>, help_text: Option<&str>) -> Div {
        div()
            .flex()
            .flex_col()
            .children(label.map(|l| field_label(l.to_string())))
            .child(input_box(&self.input(id), false))
            .children(help_text.map(|t| help(t).pt(px(4.))))
    }

    /// `PathSelectionBox`: input + folder/clear icon. `deletable`: the icon
    /// clears a non-empty value.
    pub(super) fn path_row(
        &self,
        id: &'static str,
        label: Option<&str>,
        directory: bool,
        prompt: &'static str,
        deletable: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let input = self.input(id);
        let has = deletable && !input.read(cx).value().is_empty();
        path_field(
            SharedString::from(format!("{id}-pick")),
            label,
            &input,
            has,
            false,
            cx.listener(move |this, _, window, cx| {
                if has {
                    this.set_input(id, "", window, cx);
                    this.commit(id, cx);
                } else {
                    choose_path(window, cx, directory, prompt, move |this: &mut Self, path, window, cx| {
                        this.set_input(id, &path, window, cx);
                        this.commit(id, cx);
                    });
                }
            }),
        )
    }

    /// `SelectField`; `options` are `(label, selected)`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_row(
        &self,
        id: &'static str,
        label: Option<&str>,
        current: String,
        options: Vec<(SharedString, bool)>,
        disabled: bool,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static,
    ) -> Div {
        select_field(
            id,
            label,
            current,
            disabled,
            self.open_select == Some(id),
            options,
            cx,
            move |this: &mut Self, _, cx| {
                this.open_select = if this.open_select == Some(id) { None } else { Some(id) };
                cx.notify();
            },
            move |this: &mut Self, i, window, cx| {
                this.open_select = None;
                on_pick(this, i, window, cx);
                cx.notify();
            },
        )
    }

    pub(super) fn outline_button(
        &self,
        id: &'static str,
        label: &str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        action_button(id, ButtonKind::Outline, label.to_string(), enabled, cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}
