//! Row widgets shared by the tabs.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::SettingsPanel;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, field_label, input_box, path_field, select_field, select_menu_field, SelectOption};

/// Menus longer than this get a search box.
const SEARCH_FROM: usize = 8;

/// Info callout (`.defaults-hint`): a quiet glass box.
pub fn hint(text: &str) -> Div {
    div()
        .px(px(14.))
        .py(px(10.))
        .rounded(px(theme::R_INPUT))
        .bg(theme::accent_tint())
        .border_1()
        .border_color(theme::white_a(0.08))
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(theme::text_primary())
        .child(text.to_string())
}

/// Small muted explanation (Heroic's `InfoIcon` tooltip / `InfoBox`).
pub fn help(text: &str) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text.to_string())
}

pub fn danger_text(text: &str) -> Div {
    div().text_size(px(13.)).text_color(theme::danger_text()).child(text.to_string())
}

/// Caption above a card (sentence case, muted).
pub fn heading(text: &str) -> Div {
    caption(text.to_string())
}

/// Vertical stack of captions and cards.
pub fn column() -> Div {
    div().flex().flex_col().gap(px(6.))
}

/// The rows of one card: toggle rows, field blocks, custom rows.
#[derive(Default)]
pub struct Rows(Vec<AnyElement>);

pub fn rows() -> Rows {
    Rows::default()
}

impl Rows {
    pub fn child(mut self, el: impl IntoElement) -> Self {
        self.0.push(el.into_any_element());
        self
    }

    /// A form field (select, input, path...) with its explanation under it.
    pub fn field(self, el: impl IntoElement) -> Self {
        self.child(div().py(px(12.)).child(el))
    }

    /// A form field followed by one or more help lines.
    pub fn field_help(self, el: impl IntoElement, notes: Vec<Div>) -> Self {
        let mut col = div().flex().flex_col().gap(px(5.)).py(px(12.)).child(el);
        for n in notes {
            col = col.child(n);
        }
        self.child(col)
    }

    /// The card holding the rows (nothing when there are none).
    pub fn card(self) -> Div {
        if self.0.is_empty() {
            return div();
        }
        card(self.0)
    }
}

impl SettingsPanel {
    /// A settings row: label, explanation and a switch. `disabled` shows it
    /// dimmed and inert.
    pub(super) fn toggle_row(
        &self,
        id: &'static str,
        key: &'static str,
        default: bool,
        label: &str,
        help_text: Option<&str>,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.flag(key, default);
        let mut row = toggle_row(id, label.to_string(), help_text, on);
        if disabled {
            row = row.opacity(0.4).cursor_not_allowed();
        } else {
            row = row.on_click(cx.listener(move |this, _, _, cx| this.toggle(key, default, cx)));
        }
        row.into_any_element()
    }

    /// Labelled text input bound to input `id`.
    pub(super) fn text_row(&self, id: &'static str, label: Option<&str>, help_text: Option<&str>) -> Div {
        div()
            .flex()
            .flex_col()
            .children(label.map(|l| field_label(l.to_string())))
            .child(input_box(&self.input(id), false))
            .children(help_text.map(|t| help(t).pt(px(5.))))
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

    /// [`select_row`](Self::select_row) with groups and notes; long lists
    /// get a search box (focused, and emptied, when the menu opens).
    pub(super) fn select_menu_row(
        &self,
        id: &'static str,
        label: Option<&str>,
        current: String,
        options: Vec<SelectOption>,
        disabled: bool,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static,
    ) -> Div {
        let searchable = options.len() > SEARCH_FROM;
        let filter = searchable.then(|| self.input("select.filter"));
        select_menu_field(
            id,
            label,
            current,
            disabled,
            self.open_select == Some(id),
            options,
            filter.as_ref(),
            cx,
            move |this: &mut Self, window, cx| {
                let opening = this.open_select != Some(id);
                this.open_select = if opening { Some(id) } else { None };
                if opening && searchable {
                    this.set_input("select.filter", "", window, cx);
                    this.input("select.filter").update(cx, |s, cx| s.focus(window, cx));
                }
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
        action_button(id, ButtonKind::Glass, label.to_string(), enabled, cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}
