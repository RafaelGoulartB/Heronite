//! Form widgets for the metadata / game-settings dialogs: Heroic's
//! `TextInputField`, textarea, `SelectField` menu and the Collection tag
//! field (`CollectionTagField`, a MUI Autocomplete with chips).

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::icon;

/// `--input-background` look shared by inputs, textareas and tag fields.
fn input_surface() -> Div {
    div().rounded(px(4.)).bg(theme::navbar_background()).shadow(vec![BoxShadow {
        color: theme::black_a(0.25),
        offset: point(px(0.), px(4.)),
        blur_radius: px(4.),
        spread_radius: px(0.),
        inset: false,
    }])
}

pub fn text_state(window: &mut Window, cx: &mut App, value: &str, placeholder: &str, masked: bool) -> Entity<InputState> {
    let (value, placeholder) = (value.to_string(), placeholder.to_string());
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).masked(masked);
        s.set_placeholder(placeholder, window, cx);
        s.set_value(value, window, cx);
        s
    })
}

pub fn textarea_state(window: &mut Window, cx: &mut App, value: &str, rows: usize) -> Entity<TextareaState> {
    let value = value.to_string();
    cx.new(|cx| {
        let mut s = TextareaState::new(window, cx).rows(rows);
        s.set_value(value, window, cx);
        s
    })
}

/// Label above a control (`label` = `--text-default`).
pub fn labeled(label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(5.6))
        .child(div().text_size(px(16.)).text_color(rgb(0xcaf3fd)).child(label.into()))
        .child(control)
}

/// 40px single-line field.
pub fn input_box(state: &Entity<InputState>) -> Div {
    input_surface().h(px(40.)).w_full().px(px(12.)).flex().items_center().text_size(px(16.)).text_color(white()).child(
        div().w_full().child(Input::new(state).appearance(false)),
    )
}

pub fn textarea_box(state: &Entity<TextareaState>, rows: usize) -> Div {
    input_surface()
        .w_full()
        .h(px(rows as f32 * 22. + 20.))
        .px(px(12.))
        .py(px(8.))
        .text_size(px(16.))
        .text_color(white())
        .child(Textarea::new(state).appearance(false).h_full())
}

/// Dropdown menu of a select: `items` are `(value, label)`.
pub fn select_menu<F>(id: &str, items: &[(String, String)], current: &str, on_pick: F) -> Div
where
    F: Fn(String, &mut Window, &mut App) + 'static,
{
    let on_pick = Rc::new(on_pick);
    let mut menu = menu_panel(240.).bg(theme::navbar_background());
    for (i, (value, label)) in items.iter().enumerate() {
        let (v, cb) = (value.clone(), on_pick.clone());
        menu = menu.child(
            menu_item(SharedString::from(format!("{id}-{i}")), value == current)
                .child(label.clone())
                .on_click(move |_, w, cx| cb(v.clone(), w, cx)),
        );
    }
    menu
}

/// Wraps `select` (a `select_box`) and, when `open`, anchors `menu` below it.
pub fn select_with_menu(select: impl IntoElement, menu: Option<Div>) -> Div {
    let mut wrap = div().relative().w_full().child(select);
    if let Some(menu) = menu {
        wrap = wrap.child(deferred(div().absolute().top(px(44.)).left_0().occlude().child(menu)).with_priority(40));
    }
    wrap
}

pub fn normalize_tag(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tag_key(value: &str) -> String {
    heronite_data::text::facet_key(value)
}

pub fn unique_tags<I: IntoIterator<Item = String>>(values: I) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in values {
        let v = normalize_tag(&raw);
        if v.is_empty() || !seen.insert(tag_key(&v)) {
            continue;
        }
        out.push(v);
    }
    out
}

fn split_typed(value: &str) -> Vec<String> {
    unique_tags(value.split([',', ';', '\n']).map(str::to_string))
}

pub enum TagEvent {
    Changed,
}

/// Tag editor: chips + free text with autocomplete (multiple), or a single
/// free-text value with suggestions.
pub struct TagField {
    id: SharedString,
    label: SharedString,
    multiple: bool,
    values: Vec<String>,
    options: Vec<String>,
    input: Entity<InputState>,
    focused: bool,
    /// Single mode: the value was just set from a suggestion.
    _subs: Vec<Subscription>,
}

impl EventEmitter<TagEvent> for TagField {}

impl TagField {
    pub fn new(
        id: &str,
        label: &str,
        multiple: bool,
        options: Vec<String>,
        value: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let values = unique_tags(value);
        let initial = if multiple { String::new() } else { values.first().cloned().unwrap_or_default() };
        let input = text_state(window, cx, &initial, "Select or type to add", false);
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Focus => {
                this.focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                this.focused = false;
                if this.multiple {
                    this.commit_typed(window, cx);
                }
                cx.notify();
            }
            InputEvent::Change => {
                if !this.multiple {
                    let text = normalize_tag(input.read(cx).value().as_ref());
                    let next = if text.is_empty() { Vec::new() } else { vec![text] };
                    if next != this.values {
                        this.values = next;
                        cx.emit(TagEvent::Changed);
                    }
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if this.multiple {
                    this.commit_typed(window, cx);
                } else {
                    this.focused = false;
                    cx.notify();
                }
            }
        });
        Self {
            id: id.to_string().into(),
            label: label.to_string().into(),
            multiple,
            values,
            options,
            input,
            focused: false,
            _subs: vec![sub],
        }
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// Single-value fields.
    pub fn value(&self) -> String {
        self.values.first().cloned().unwrap_or_default()
    }

    pub fn set_values(&mut self, values: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.values = unique_tags(values);
        if !self.multiple {
            let text = self.values.first().cloned().unwrap_or_default();
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        cx.notify();
    }

    fn add_all(&mut self, items: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiple {
            let mut next = self.values.clone();
            next.extend(items);
            let next = unique_tags(next);
            if next != self.values {
                self.values = next;
                cx.emit(TagEvent::Changed);
            }
        } else if let Some(first) = items.into_iter().next() {
            self.values = vec![first.clone()];
            self.input.update(cx, |s, cx| s.set_value(first, window, cx));
            self.focused = false;
            cx.emit(TagEvent::Changed);
        }
        cx.notify();
    }

    fn commit_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        if typed.trim().is_empty() {
            return;
        }
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.add_all(split_typed(&typed), window, cx);
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.values.len() {
            self.values.remove(index);
            cx.emit(TagEvent::Changed);
            cx.notify();
        }
    }

    fn suggestions(&self, cx: &App) -> Vec<String> {
        let typed = self.input.read(cx).value().to_string();
        let needle = typed.trim().to_lowercase();
        let selected: std::collections::HashSet<String> = self.values.iter().map(|v| tag_key(v)).collect();
        let mut out: Vec<String> = Vec::new();
        for o in &self.options {
            let v = normalize_tag(o);
            if v.is_empty() || (self.multiple && selected.contains(&tag_key(&v))) {
                continue;
            }
            if needle.is_empty() || v.to_lowercase().contains(&needle) {
                out.push(v);
            }
            if out.len() >= 60 {
                break;
            }
        }
        if self.multiple {
            for t in split_typed(&typed) {
                if !out.iter().any(|o| tag_key(o) == tag_key(&t)) && !selected.contains(&tag_key(&t)) {
                    out.push(t);
                }
            }
        }
        out
    }
}

impl Render for TagField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut boxed = input_surface()
            .min_h(px(40.))
            .w_full()
            .px(px(8.))
            .py(px(4.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .text_size(px(16.))
            .text_color(white());
        if self.multiple {
            for (i, v) in self.values.iter().enumerate() {
                boxed = boxed.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .h(px(24.))
                        .pl(px(10.))
                        .pr(px(4.))
                        .rounded_full()
                        .bg(theme::white_a(0.08))
                        .text_size(px(13.))
                        .text_color(white())
                        .child(v.clone())
                        .child(
                            div()
                                .id(SharedString::from(format!("{}-rm-{i}", self.id)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(16.))
                                .rounded_full()
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::white_a(0.2)))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| this.remove(i, cx)))
                                .child(icon(Icon::Cancel, px(14.), theme::white_a(0.6))),
                        ),
                );
            }
        }
        boxed = boxed.child(div().flex_1().min_w(px(96.)).child(Input::new(&self.input).appearance(false)));

        let mut wrap = div().relative().w_full().child(boxed);
        if self.focused {
            let items = self.suggestions(cx);
            if !items.is_empty() {
                let mut list = div()
                    .id(SharedString::from(format!("{}-menu", self.id)))
                    .flex()
                    .flex_col()
                    .max_h(px(240.))
                    .overflow_y_scroll();
                for (i, item) in items.into_iter().enumerate() {
                    let value = item.clone();
                    list = list.child(
                        menu_item(SharedString::from(format!("{}-opt-{i}", self.id)), false)
                            .child(item)
                            // Mouse down (not click): the input must not lose focus first.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    if this.multiple {
                                        this.input.update(cx, |s, cx| s.set_value("", window, cx));
                                    }
                                    this.add_all(vec![value.clone()], window, cx);
                                }),
                            ),
                    );
                }
                wrap = wrap.child(
                    deferred(
                        div()
                            .absolute()
                            .top(relative(1.))
                            .left_0()
                            .w_full()
                            .pt(px(4.))
                            .occlude()
                            .child(menu_panel(120.).bg(theme::navbar_background()).child(list)),
                    )
                    .with_priority(40),
                );
            }
        }
        labeled(self.label.clone(), wrap)
    }
}
