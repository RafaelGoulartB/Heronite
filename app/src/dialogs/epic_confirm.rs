//! Generic confirmation dialog of the Epic features: a message, optional
//! checkboxes and an optional folder field (uninstall with "remove prefix",
//! move install, verify and repair, update prompt) - or just a text to read
//! (cloud saves output).

mod console;

use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, new_input, paragraph, path_field};

pub struct Toggle {
    pub label: String,
    pub value: bool,
    pub disabled: bool,
    /// Shown under the label when set (why it is disabled).
    pub note: Option<String>,
}

pub struct Choice {
    pub toggles: Vec<bool>,
    pub path: String,
}

/// What the dialog shows besides the message.
pub struct Confirm {
    pub id: String,
    pub title: String,
    pub message: String,
    pub toggles: Vec<Toggle>,
    /// `(label, initial value)` of a folder field.
    pub path: Option<(String, String, String)>,
    /// Confirm button label; `None` makes it a plain message with Close.
    pub confirm: Option<String>,
    /// Extra secondary button (label, action).
    pub secondary: Option<(String, Rc<dyn Fn(&mut App)>)>,
    pub danger: bool,
    /// Monospace text block (command output).
    pub text: Option<String>,
    pub width: f32,
}

impl Confirm {
    pub fn new(id: &str, title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            message: message.into(),
            toggles: Vec::new(),
            path: None,
            confirm: None,
            secondary: None,
            danger: false,
            text: None,
            width: 520.,
        }
    }
}

pub struct EpicConfirmDialog {
    spec: Confirm,
    toggles: Vec<bool>,
    path_input: Option<Entity<InputState>>,
    on_confirm: Rc<dyn Fn(Choice, &mut App)>,
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for EpicConfirmDialog {}

impl EpicConfirmDialog {
    pub fn new(spec: Confirm, on_confirm: impl Fn(Choice, &mut App) + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let toggles = spec.toggles.iter().map(|t| t.value).collect();
        let path_input = spec.path.as_ref().map(|(_, initial, _)| new_input(window, cx, initial, ""));
        Self { spec, toggles, path_input, on_confirm: Rc::new(on_confirm), console_list: Default::default(), layer: Default::default() }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let path = self.path_input.as_ref().map(|i| i.read(cx).value().trim().to_string()).unwrap_or_default();
        if self.spec.path.is_some() && path.is_empty() {
            return;
        }
        (self.on_confirm)(Choice { toggles: self.toggles.clone(), path }, cx);
        cx.emit(DialogEvent::Close);
    }
}

impl Render for EpicConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let id = self.spec.id.clone();
        let mut body = div().flex().flex_col().gap(px(14.));
        for line in self.spec.message.lines() {
            body = body.child(paragraph(line.to_string()));
        }
        if let Some(text) = &self.spec.text {
            body = body.child(
                div()
                    .max_h(px(320.))
                    .overflow_hidden()
                    .p(px(10.))
                    .rounded(px(theme::R_INPUT))
                    .bg(theme::input_fill())
                    .border_1()
                    .border_color(theme::input_border())
                    .font_family(theme::FONT_MONO)
                    .text_size(px(12.))
                    .text_color(theme::text_primary())
                    .child(text.clone()),
            );
        }
        if !self.spec.toggles.is_empty() {
            let mut rows: Vec<AnyElement> = Vec::new();
            for (i, t) in self.spec.toggles.iter().enumerate() {
                let on = self.toggles[i];
                let row =
                    crate::ui::controls::toggle_row(SharedString::from(format!("{id}-toggle-{i}")), t.label.clone(), t.note.as_deref(), on);
                rows.push(
                    if t.disabled {
                        row.opacity(0.55).cursor_not_allowed()
                    } else {
                        row.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggles[i] = !this.toggles[i];
                            cx.notify();
                        }))
                    }
                    .into_any_element(),
                );
            }
            body = body.child(crate::ui::controls::card(rows));
        }
        if let (Some((label, _, prompt)), Some(input)) = (&self.spec.path, &self.path_input) {
            let has = !input.read(cx).value().trim().is_empty();
            let prompt = prompt.clone();
            body = body.child(path_field(
                SharedString::from(format!("{id}-path")),
                Some(label.as_str()),
                input,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    let Some(input) = this.path_input.clone() else { return };
                    if has {
                        input.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    } else {
                        choose_path(window, cx, true, &prompt, move |_: &mut Self, p, window, cx| {
                            input.update(cx, |s, cx| s.set_value(p, window, cx));
                            cx.notify();
                        });
                    }
                }),
            ));
        }

        let mut footer = div().flex().gap(px(10.));
        match &self.spec.confirm {
            Some(label) => {
                footer = footer.child(
                    button(SharedString::from(format!("{id}-cancel")), ButtonKind::Ghost, "Cancel")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
                );
                if let Some((label, action)) = self.spec.secondary.clone() {
                    footer = footer.child(button(SharedString::from(format!("{id}-secondary")), ButtonKind::Glass, label).on_click(
                        cx.listener(move |_, _, _, cx| {
                            action(cx);
                            cx.emit(DialogEvent::Close);
                        }),
                    ));
                }
                let ready = self.spec.path.is_none() || self.path_input.as_ref().is_some_and(|i| !i.read(cx).value().trim().is_empty());
                let kind = if self.spec.danger { ButtonKind::Danger } else { ButtonKind::Primary };
                footer = footer.child(action_button(
                    SharedString::from(format!("{id}-confirm")),
                    kind,
                    label.clone(),
                    ready,
                    cx.listener(|this, _, _, cx| this.confirm(cx)),
                ));
            }
            None => {
                footer = footer.child(
                    button(SharedString::from(format!("{id}-close")), ButtonKind::Primary, "Close")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
                );
            }
        }
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: id.clone().into(), title: self.spec.title.clone().into(), width: self.spec.width, height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}
