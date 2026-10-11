//! Collection tag field (`CollectionTagField`, a MUI Autocomplete with chips)
//! and the helpers that normalise tag values.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::input::{input_surface, new_input};
use super::text::labeled;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::icon;

fn normalize_tag(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tag_key(value: &str) -> String {
    heronite_data::text::facet_key(value)
}

fn unique_tags<I: IntoIterator<Item = String>>(values: I) -> Vec<String> {
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
        let input = new_input(window, cx, &initial, "Select or type to add");
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

    /// Replaces the values (console mode edits a field as one list).
    pub fn set_values(&mut self, values: Vec<String>, cx: &mut Context<Self>) {
        let mut v = unique_tags(values);
        if !self.multiple {
            v.truncate(1);
        }
        self.values = v;
        cx.emit(TagEvent::Changed);
        cx.notify();
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// Single-value fields.
    pub fn value(&self) -> String {
        self.values.first().cloned().unwrap_or_default()
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
        let mut boxed =
            input_surface().min_h(px(theme::INPUT_H)).w_full().px(px(10.)).py(px(4.)).flex().flex_wrap().items_center().gap(px(4.));
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
                        .bg(theme::accent_tint())
                        .text_size(px(13.))
                        .text_color(theme::accent())
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
                let mut list =
                    div().id(SharedString::from(format!("{}-menu", self.id))).flex().flex_col().max_h(px(240.)).overflow_y_scroll();
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
                    deferred(div().absolute().top(relative(1.)).left_0().w_full().pt(px(4.)).occlude().child(menu_panel(120.).child(list)))
                        .with_priority(40),
                );
            }
        }
        labeled(self.label.clone(), wrap)
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_tag, split_typed, unique_tags};

    #[test]
    fn tags_are_trimmed_and_collapsed() {
        assert_eq!(normalize_tag("  Open   world "), "Open world");
    }

    #[test]
    fn unique_tags_drop_empty_and_duplicates_keeping_the_first_spelling() {
        let out = unique_tags(["Action", " action ", "", "Open  world", "RPG"].map(String::from));
        assert_eq!(out, vec!["Action", "Open world", "RPG"]);
    }

    #[test]
    fn typed_text_splits_on_commas_semicolons_and_newlines() {
        assert_eq!(split_typed("a, b;c\nA"), vec!["a", "b", "c"]);
    }
}
