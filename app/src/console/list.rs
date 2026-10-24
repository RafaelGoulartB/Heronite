//! Focusable rows for console screens: one focused row at a time, values
//! changed with ◀ ▶ (or A), the list scrolled to keep the focus visible.
//!
//! The owner builds `Vec<Row>` from its state on every render and keeps a
//! [`ListNav`] (focus + scroll). [`ListNav::on_pad`] answers what happened
//! (moved, a value changed, a row was activated); the owner applies it and
//! the next render shows it.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::kit::{self, u};
use super::motion::{Axis, Glide, Goal};
use super::Pad;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Plain action ("›" on the right).
    Action,
    Toggle(bool),
    Choice {
        options: Vec<SharedString>,
        index: usize,
    },
    Number {
        value: i64,
        min: i64,
        max: i64,
        step: i64,
        suffix: &'static str,
    },
    /// Editable text shown on the right; A asks the owner to edit it.
    Text(SharedString),
    /// Read-only text on the right.
    Info(SharedString),
    /// Caption between groups (not focusable).
    Header,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: SharedString,
    pub label: SharedString,
    pub desc: Option<SharedString>,
    pub value: Value,
    pub enabled: bool,
    pub danger: bool,
    pub icon: Option<Icon>,
    /// Differs from the defaults: a small dot after the label.
    pub changed: bool,
    /// A short value: its ‹ › take less width.
    pub compact: bool,
    /// A picture drawn in place of the value (a capture's preview).
    pub image: Option<std::path::PathBuf>,
}

impl Row {
    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, value: Value) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            desc: None,
            value,
            enabled: true,
            danger: false,
            icon: None,
            changed: false,
            compact: false,
            image: None,
        }
    }
    pub fn action(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, Value::Action)
    }
    pub fn toggle(id: impl Into<SharedString>, label: impl Into<SharedString>, on: bool) -> Self {
        Self::new(id, label, Value::Toggle(on))
    }
    pub fn choice(id: impl Into<SharedString>, label: impl Into<SharedString>, options: Vec<SharedString>, index: usize) -> Self {
        let index = index.min(options.len().saturating_sub(1));
        Self::new(id, label, Value::Choice { options, index })
    }
    pub fn number(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: i64,
        min: i64,
        max: i64,
        step: i64,
        suffix: &'static str,
    ) -> Self {
        Self::new(id, label, Value::Number { value: value.clamp(min, max), min, max, step, suffix })
    }
    pub fn text(id: impl Into<SharedString>, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self::new(id, label, Value::Text(value.into()))
    }
    pub fn info(id: impl Into<SharedString>, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self::new(id, label, Value::Info(value.into()))
    }
    pub fn header(label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self::new(SharedString::from(format!("h:{label}")), label, Value::Header)
    }
    pub fn desc(mut self, d: impl Into<SharedString>) -> Self {
        let d = d.into();
        self.desc = (!d.is_empty()).then_some(d);
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn icon(mut self, ic: Icon) -> Self {
        self.icon = Some(ic);
        self
    }
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn image(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.image = path;
        self
    }
    fn focusable(&self) -> bool {
        self.value != Value::Header
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Not for the list (the owner or the shell may use it).
    Ignored,
    Moved,
    /// A on an action / text / info row.
    Activate(SharedString),
    /// The new value of a toggle / choice / number.
    Changed(SharedString, Value),
    /// Up on the first row or Down on the last: the owner may move the focus
    /// to another block.
    Edge(Pad),
}

pub struct ListNav {
    pub focus: usize,
    glide: Glide,
    reveal: bool,
    /// Drawn once (the first reveal jumps instead of gliding).
    shown: bool,
}

impl Default for ListNav {
    fn default() -> Self {
        Self { focus: 0, glide: Glide::new(Axis::Y), reveal: true, shown: false }
    }
}

impl ListNav {
    /// Keeps the focus on a focusable row inside `rows`.
    pub fn clamp(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            self.focus = 0;
            return;
        }
        self.focus = self.focus.min(rows.len() - 1);
        if !rows[self.focus].focusable() {
            if let Some(i) = (self.focus..rows.len()).chain((0..self.focus).rev()).find(|&i| rows[i].focusable()) {
                self.focus = i;
            }
        }
    }

    pub fn focused<'a>(&self, rows: &'a [Row]) -> Option<&'a Row> {
        rows.get(self.focus).filter(|r| r.focusable())
    }

    pub fn reset(&mut self) {
        self.focus = 0;
        self.reveal = true;
        self.glide.reset();
    }

    fn step(&mut self, rows: &[Row], down: bool, by: usize) -> bool {
        let mut moved = 0;
        let mut i = self.focus;
        loop {
            let next = if down { i + 1 } else { i.wrapping_sub(1) };
            if next >= rows.len() {
                break;
            }
            i = next;
            if rows[i].focusable() {
                self.focus = i;
                moved += 1;
                if moved == by {
                    break;
                }
            }
        }
        if moved > 0 {
            self.reveal = true;
        }
        moved > 0
    }

    pub fn on_pad(&mut self, pad: Pad, rows: &[Row]) -> Outcome {
        self.clamp(rows);
        let Some(row) = rows.get(self.focus) else { return Outcome::Ignored };
        match pad {
            Pad::Up | Pad::Down => {
                if self.step(rows, pad == Pad::Down, 1) {
                    Outcome::Moved
                } else {
                    Outcome::Edge(pad)
                }
            }
            Pad::ScrollUp | Pad::ScrollDown => {
                if self.step(rows, pad == Pad::ScrollDown, 5) {
                    Outcome::Moved
                } else {
                    Outcome::Ignored
                }
            }
            Pad::Left | Pad::Right | Pad::A if row.enabled => {
                let right = pad != Pad::Left;
                let changed = match &row.value {
                    Value::Toggle(on) => Some(Value::Toggle(!on)),
                    Value::Choice { options, index } => {
                        let n = options.len();
                        let next = match pad {
                            Pad::A if n > 0 => (index + 1) % n,
                            Pad::Right => (index + 1).min(n.saturating_sub(1)),
                            Pad::Left => index.saturating_sub(1),
                            _ => *index,
                        };
                        (next != *index).then(|| Value::Choice { options: options.clone(), index: next })
                    }
                    Value::Number { value, min, max, step, suffix } if pad != Pad::A => {
                        let next = if right { value + step } else { value - step }.clamp(*min, *max);
                        (next != *value).then_some(Value::Number { value: next, min: *min, max: *max, step: *step, suffix })
                    }
                    Value::Action | Value::Text(_) | Value::Info(_) if pad == Pad::A => {
                        return Outcome::Activate(row.id.clone());
                    }
                    _ => None,
                };
                match changed {
                    Some(v) => Outcome::Changed(row.id.clone(), v),
                    None if pad == Pad::A => Outcome::Ignored,
                    // ◀ ▶ on a row without a value: let the owner move sideways.
                    None if !matches!(row.value, Value::Toggle(_) | Value::Choice { .. } | Value::Number { .. }) => Outcome::Edge(pad),
                    None => Outcome::Moved,
                }
            }
            _ => Outcome::Ignored,
        }
    }

    /// The rows in a vertical scroll area (`id` must be unique on screen).
    /// `active` = the list has the focus (else no pill is drawn).
    pub fn render(&mut self, id: impl Into<ElementId>, rows: &[Row], active: bool) -> Stateful<Div> {
        self.clamp(rows);
        if self.reveal && active {
            self.reveal = false;
            // A little of the rows around the focus stays in sight.
            let goal = Goal::Reveal { ix: self.focus, margin: 34. };
            if self.shown {
                self.glide.go(goal);
            } else {
                self.glide.jump(goal);
            }
        }
        self.shown = true;
        let mut list = div().id(id).flex().flex_col().gap(u(4.)).overflow_y_scroll().track_scroll(self.glide.handle());
        for (i, r) in rows.iter().enumerate() {
            list = list.child(render_row(r, active && i == self.focus));
        }
        list.child(self.glide.driver())
    }
}

/// One row (also used outside lists).
pub fn render_row(r: &Row, focused: bool) -> Div {
    if r.value == Value::Header {
        return kit::caption(r.label.clone());
    }
    let label_color: Hsla = if focused {
        theme::console_on_focus().into()
    } else if r.danger {
        theme::danger_text().into()
    } else {
        theme::text_primary().into()
    };
    let value: AnyElement = match &r.value {
        Value::Action => icon(Icon::ChevronRight, u(18.), if focused { label_color } else { theme::text_dim().into() }).into_any_element(),
        Value::Toggle(on) => kit::toggle_pill(*on, focused).into_any_element(),
        Value::Choice { options, index } => {
            let text = options.get(*index).cloned().unwrap_or_default();
            let min_w = if r.compact { 84. } else { 150. };
            kit::choice_sized(text, focused, *index > 0, index + 1 < options.len(), min_w).into_any_element()
        }
        Value::Number { value, min, max, suffix, .. } => {
            kit::choice(format!("{value}{suffix}"), focused, value > min, value < max).into_any_element()
        }
        Value::Text(t) | Value::Info(t) => div()
            .max_w(u(420.))
            .truncate()
            .text_size(u(15.))
            .text_color(if focused { theme::console_on_focus() } else { theme::text_muted() })
            .child(if t.is_empty() { SharedString::from("\u{2014}") } else { t.clone() })
            .into_any_element(),
        Value::Header => div().into_any_element(),
    };
    let value = match &r.image {
        Some(path) => div()
            .flex_none()
            .w(u(72.))
            .h(u(40.))
            .rounded(u(8.))
            .overflow_hidden()
            .bg(kit::ink(0.06))
            .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover))
            .into_any_element(),
        None => value,
    };
    kit::row_base(focused)
        .when(!r.enabled, |d| d.opacity(0.4))
        .children(r.icon.map(|ic| icon(ic, u(22.), label_color)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .py(u(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(u(8.))
                        .child(div().text_size(u(18.)).text_color(label_color).truncate().child(r.label.clone()))
                        .when(r.changed, |d| d.child(div().flex_none().size(u(7.)).rounded_full().bg(theme::accent()))),
                )
                .children(r.desc.clone().filter(|_| focused).map(|d| {
                    div()
                        .text_size(u(13.))
                        .line_height(u(18.))
                        // One line: a longer one ends in "…" instead of growing the row.
                        .truncate()
                        .text_color(Hsla { a: 0.7, ..label_color })
                        .child(d)
                })),
        )
        .child(value)
}

/// A small console dialog: title, message lines, an optional error and the
/// rows (confirmations).
pub fn simple_dialog(
    nav: &mut ListNav,
    id: &'static str,
    title: impl Into<SharedString>,
    message: &str,
    error: Option<SharedString>,
    rows: &[Row],
) -> AnyElement {
    let mut body = div().flex().flex_col().gap(u(12.));
    for line in message.lines().filter(|l| !l.trim().is_empty()) {
        body = body.child(kit::body_text(line.to_string()));
    }
    if let Some(e) = error {
        body = body.child(kit::body_text(e).text_color(theme::danger_text()));
    }
    kit::modal(title, 680., body.child(nav.render(id, rows, true).min_h_0())).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{ListNav, Outcome, Row, Value};
    use crate::console::Pad;

    fn rows() -> Vec<Row> {
        vec![
            Row::header("General"),
            Row::toggle("tray", "Exit to tray", true),
            Row::choice("lang", "Language", vec!["English".into(), "Português".into()], 0),
            Row::number("vol", "Volume", 50, 0, 100, 10, "%"),
            Row::action("logs", "Open logs").enabled(false),
            Row::action("about", "About"),
        ]
    }

    #[test]
    fn skips_headers_and_reports_edges() {
        let r = rows();
        let mut nav = ListNav::default();
        nav.clamp(&r);
        assert_eq!(nav.focus, 1);
        assert_eq!(nav.on_pad(Pad::Up, &r), Outcome::Edge(Pad::Up));
        for _ in 0..4 {
            assert_eq!(nav.on_pad(Pad::Down, &r), Outcome::Moved);
        }
        assert_eq!(nav.focus, 5);
        assert_eq!(nav.on_pad(Pad::Down, &r), Outcome::Edge(Pad::Down));
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Activate("about".into()));
        nav.focus = 4;
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Ignored, "disabled");
    }

    #[test]
    fn values_change_with_left_right_and_a() {
        let r = rows();
        let mut nav = ListNav { focus: 1, ..Default::default() };
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Changed("tray".into(), Value::Toggle(false)));
        nav.focus = 2;
        assert_eq!(nav.on_pad(Pad::Left, &r), Outcome::Moved, "already first");
        match nav.on_pad(Pad::Right, &r) {
            Outcome::Changed(id, Value::Choice { index, .. }) => assert_eq!((id.as_ref(), index), ("lang", 1)),
            o => panic!("{o:?}"),
        }
        nav.focus = 3;
        match nav.on_pad(Pad::Left, &r) {
            Outcome::Changed(_, Value::Number { value, .. }) => assert_eq!(value, 40),
            o => panic!("{o:?}"),
        }
        nav.focus = 5;
        assert_eq!(nav.on_pad(Pad::Right, &r), Outcome::Edge(Pad::Right));
    }
}
