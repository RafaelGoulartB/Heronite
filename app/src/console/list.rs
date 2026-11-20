//! Focusable rows for console screens: one focused row at a time, values
//! changed with ◀ ▶ (or A), the list scrolled to keep the focus visible.
//!
//! The owner builds `Vec<Row>` from its state on every render and keeps a
//! [`ListNav`] (focus + scroll). [`ListNav::on_pad`] answers what happened
//! (moved, a value changed, a row was activated); the owner applies it and
//! the next render shows it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::kit::{self, u};
use super::motion::{self, Axis, Glide, Goal};
use super::pointer::Clickable;
use super::{hint, Btn, Hint, Pad};
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
    /// What A does on an action row, written at its end in card lists
    /// ("Open", "Refresh"); without it a card shows a chevron.
    pub verb: Option<SharedString>,
    /// The row this one is a sub-setting of: the rows of a group come right
    /// after their parent, show only while it is open and draw inside its
    /// card ([`Row::child_of`]).
    pub parent: Option<SharedString>,
    /// What a closed group says under its parent's label; without it, its
    /// sub-settings' values ([`group_summary`]).
    pub summary: Option<SharedString>,
    /// Its group folds even with a single sub-setting ([`Row::always_folds`]).
    pub folding: bool,
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
            verb: None,
            parent: None,
            summary: None,
            folding: false,
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
    /// A row that only gathers the sub-settings after it (A opens it).
    pub fn group(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self::new(id, label, Value::Action)
    }
    /// Makes this row a sub-setting of the row `parent` (it must come right
    /// after the parent or its other sub-settings).
    pub fn child_of(mut self, parent: impl Into<SharedString>) -> Self {
        self.parent = Some(parent.into());
        self
    }
    /// What the group says while closed (else its sub-settings' values).
    pub fn summary(mut self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.summary = (!text.is_empty()).then_some(text);
        self
    }
    /// Its group folds even when it holds a single sub-setting (a parent
    /// whose sub-settings come and go, so A keeps one meaning).
    pub fn always_folds(mut self) -> Self {
        self.folding = true;
        self
    }
    /// The word a card list writes at the end of an action row.
    pub fn verb(mut self, v: impl Into<SharedString>) -> Self {
        self.verb = Some(v.into());
        self
    }
    fn focusable(&self) -> bool {
        self.value != Value::Header
    }
}

/// Whether row `i` opens a group: the row after it is one of its sub-settings.
pub fn has_children(rows: &[Row], i: usize) -> bool {
    match (rows.get(i), rows.get(i + 1)) {
        (Some(r), Some(next)) => next.parent.as_ref() == Some(&r.id),
        _ => false,
    }
}

/// The parent of row `i`, by index (the nearest row before it with its id).
fn parent_index(rows: &[Row], i: usize) -> Option<usize> {
    let p = rows.get(i)?.parent.as_ref()?;
    (0..i).rev().find(|&j| &rows[j].id == p)
}

/// How many sub-settings row `i` has (they come right after it).
fn kid_count(rows: &[Row], i: usize) -> usize {
    let Some(id) = rows.get(i).map(|r| &r.id) else { return 0 };
    rows[i + 1..].iter().take_while(|r| r.parent.as_ref() == Some(id)).count()
}

/// Whether row `i` opens a group that folds: two sub-settings or more (a
/// single one always shows under its parent).
pub fn folds(rows: &[Row], i: usize) -> bool {
    let n = kid_count(rows, i);
    n >= 2 || (n == 1 && rows[i].folding)
}

/// A child's value in a line ("Region: Europe", "Find by name: On").
/// Texts never show: they may be names, keys or secrets.
fn value_note(r: &Row) -> Option<String> {
    match &r.value {
        Value::Toggle(on) => Some(format!("{}: {}", r.label, if *on { tr!("On") } else { tr!("Off") })),
        Value::Choice { options, index } => options.get(*index).map(|o| format!("{}: {o}", r.label)),
        Value::Number { value, suffix, .. } => Some(format!("{}: {value}{suffix}", r.label)),
        _ => None,
    }
}

/// Whether a sub-setting of row `i` warns (its summary then does too).
pub fn group_danger(rows: &[Row], i: usize) -> bool {
    let Some(id) = rows.get(i).map(|r| &r.id) else { return false };
    // A destructive action ("Clear favourites") is not a warning.
    rows[i + 1..].iter().take_while(|r| r.parent.as_ref() == Some(id)).any(|r| r.danger && r.value != Value::Action)
}

/// What the closed group of row `i` says: its own summary, else its
/// sub-settings' values in one line, else how many there are.
pub fn group_summary(rows: &[Row], i: usize) -> SharedString {
    if let Some(s) = rows.get(i).and_then(|r| r.summary.clone()) {
        return s;
    }
    let id = &rows[i].id;
    let kids: Vec<&Row> = rows[i + 1..].iter().take_while(|r| r.parent.as_ref() == Some(id)).collect();
    let notes: Vec<String> = kids.iter().filter_map(|r| value_note(r)).collect();
    if notes.is_empty() {
        heronite_i18n::trn!(kids.len(), "{n} more setting", "{n} more settings").into()
    } else {
        notes.join(" \u{b7} ").into()
    }
}

/// Where a row sits in a list of groups, for drawing it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// It opens a group that folds: `Some(open)`.
    pub group: Option<bool>,
    /// Its sub-settings show under it (open, or a group of one): its card
    /// goes on under it.
    pub body: bool,
    /// A sub-setting: drawn inside its parent's card.
    pub child: bool,
    /// The last sub-setting of its group (closes the card).
    pub last: bool,
    /// Its parent is a switch that is off: shown, but it does nothing.
    pub off: bool,
    /// Why it does nothing, said while it has the focus ("Turn on … first").
    pub why: Option<SharedString>,
    /// Its closed group holds a warning: the summary says it in red.
    pub warn: bool,
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

/// How a list draws its rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowStyle {
    /// Bare rows; the focused one on the light focus pill.
    #[default]
    Pill,
    /// Each row a rounded card with its description always shown, a switch
    /// for toggles and `‹ chip ›` for choices; the focused one ringed.
    Card,
}

pub struct ListNav {
    pub focus: usize,
    glide: Glide,
    reveal: bool,
    /// Drawn once (the first reveal jumps instead of gliding).
    shown: bool,
    /// Renders left that bring the focus into view again, once the rows a
    /// group just opened have their layout.
    settle: u8,
    style: RowStyle,
    /// The groups open (by their parent's id); kept across [`Self::reset`],
    /// so a section shows its groups as they were left.
    open: std::collections::HashSet<SharedString>,
}

impl Default for ListNav {
    fn default() -> Self {
        Self {
            focus: 0,
            glide: Glide::new(Axis::Y),
            reveal: true,
            shown: false,
            settle: 0,
            style: RowStyle::Pill,
            open: Default::default(),
        }
    }
}

impl ListNav {
    /// Draws the rows as `style` asks (from the next render).
    pub fn set_style(&mut self, style: RowStyle) {
        self.style = style;
    }

    /// Brings the focused row into view on the next render (the list got
    /// the focus back).
    pub fn reveal_focus(&mut self) {
        self.reveal = true;
    }

    /// Makes `id` the focused row when the list has it (a search result
    /// picked), opening its group; the next render brings it into view.
    pub fn focus_id(&mut self, rows: &[Row], id: &str) -> bool {
        match rows.iter().position(|r| r.id.as_ref() == id && r.focusable()) {
            Some(i) => {
                if let Some(p) = rows[i].parent.clone() {
                    self.open.insert(p);
                }
                self.focus = i;
                self.reveal = true;
                true
            }
            None => false,
        }
    }

    /// The groups open now (to keep them while another list shows).
    pub fn open_groups(&mut self) -> std::collections::HashSet<SharedString> {
        std::mem::take(&mut self.open)
    }

    /// Opens the groups `open` (kept by [`Self::open_groups`]).
    pub fn set_open_groups(&mut self, open: std::collections::HashSet<SharedString>) {
        self.open = open;
    }

    /// Whether row `i` is on screen (not inside a closed group).
    pub fn shows(&self, rows: &[Row], i: usize) -> bool {
        let Some(r) = rows.get(i) else { return false };
        match &r.parent {
            None => true,
            Some(p) => self.open.contains(p) || parent_index(rows, i).is_none_or(|at| !folds(rows, at)),
        }
    }

    /// Whether row `i` can take the focus.
    fn can_focus(&self, rows: &[Row], i: usize) -> bool {
        rows.get(i).is_some_and(Row::focusable) && self.shows(rows, i)
    }

    /// Whether row `i` takes its presses (not disabled, not under a switch
    /// that is off).
    fn usable(&self, rows: &[Row], i: usize) -> bool {
        rows.get(i).is_some_and(|r| r.enabled) && !self.parent_off(rows, i)
    }

    /// The press that opens the group of row `r` when it is closed, or
    /// closes it when open: ▶ / ◀ beside a switch or a plain row, A beside
    /// a value that ◀ ▶ change.
    pub fn group_pad(r: &Row, open: bool) -> Pad {
        match r.value {
            Value::Choice { .. } | Value::Number { .. } => Pad::A,
            _ if open => Pad::Left,
            _ => Pad::Right,
        }
    }

    /// Whether row `i` is a sub-setting of a switch that is off.
    fn parent_off(&self, rows: &[Row], i: usize) -> bool {
        parent_index(rows, i).is_some_and(|p| rows[p].value == Value::Toggle(false))
    }

    /// How row `i` is drawn among the groups.
    pub fn place(&self, rows: &[Row], i: usize) -> Place {
        let Some(r) = rows.get(i) else { return Place::default() };
        let parent = parent_index(rows, i).filter(|_| r.parent.is_some());
        let child = parent.is_some();
        let off = child && self.parent_off(rows, i);
        let group = folds(rows, i).then(|| self.open.contains(&r.id));
        Place {
            group,
            body: has_children(rows, i) && group != Some(false),
            child,
            last: child && rows.get(i + 1).is_none_or(|n| n.parent != r.parent),
            off,
            why: parent.filter(|_| off).map(|p| tr!("Turn on “{parent}” first", parent = rows[p].label).into()),
            warn: group == Some(false) && group_danger(rows, i),
        }
    }

    /// Opens or closes the group of row `i`; the focus leaves a closed one
    /// for its parent.
    fn set_open(&mut self, rows: &[Row], i: usize, open: bool) {
        let id = rows[i].id.clone();
        if open {
            self.open.insert(id);
        } else {
            self.open.remove(&id);
            if self.focus > i && rows.get(self.focus).is_some_and(|r| r.parent.as_ref() == Some(&id)) {
                self.focus = i;
            }
        }
        motion::press();
        self.reveal = true;
    }

    /// Keeps the focus on a focusable row inside `rows`.
    pub fn clamp(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            self.focus = 0;
            return;
        }
        self.focus = self.focus.min(rows.len() - 1);
        if !self.can_focus(rows, self.focus) {
            // A hidden sub-setting gives the focus to its parent.
            if let Some(p) = parent_index(rows, self.focus).filter(|&p| self.can_focus(rows, p)) {
                self.focus = p;
            } else if let Some(i) = (self.focus..rows.len()).chain((0..self.focus).rev()).find(|&i| self.can_focus(rows, i)) {
                self.focus = i;
            }
        }
    }

    pub fn focused<'a>(&self, rows: &'a [Row]) -> Option<&'a Row> {
        rows.get(self.focus).filter(|_| self.can_focus(rows, self.focus))
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
            if self.can_focus(rows, i) {
                self.focus = i;
                moved += 1;
                if moved == by {
                    break;
                }
            }
        }
        // A hover moves the focus without scrolling the rows under the pointer.
        if moved > 0 && !super::pointer::hovering() {
            self.reveal = true;
        }
        moved > 0
    }

    pub fn on_pad(&mut self, pad: Pad, rows: &[Row]) -> Outcome {
        self.clamp(rows);
        let i = self.focus;
        let Some(row) = rows.get(i) else { return Outcome::Ignored };
        if let Some(o) = self.group_pad_press(pad, rows, i) {
            return o;
        }
        let usable = self.usable(rows, i);
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
            Pad::Left | Pad::Right | Pad::A if usable => {
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

    /// What a press does to the groups, when it does something: on a parent,
    /// its open / close press ([`Self::group_pad`]; ▶ on an open one goes in);
    /// on a sub-setting, B (or ◀ where ◀ changes nothing) goes back to the
    /// parent and closes the group.
    fn group_pad_press(&mut self, pad: Pad, rows: &[Row], i: usize) -> Option<Outcome> {
        let row = &rows[i];
        // A group of one is always open: its parent works as any row.
        if folds(rows, i) {
            let open = self.open.contains(&row.id);
            let choice = Self::group_pad(row, false) == Pad::A;
            match pad {
                // B closes the group first; a second B leaves.
                Pad::B if open => self.set_open(rows, i, false),
                Pad::A if choice || matches!(row.value, Value::Action | Value::Info(_)) => self.set_open(rows, i, !open),
                Pad::Right if !choice && !open => self.set_open(rows, i, true),
                Pad::Right if !choice => {
                    self.step(rows, true, 1);
                }
                Pad::Left if !choice && open => self.set_open(rows, i, false),
                // ◀ on a closed group never changes its switch: it goes back.
                Pad::Left if !choice => return Some(Outcome::Edge(Pad::Left)),
                _ => return None,
            }
            return Some(Outcome::Moved);
        }
        let parent = parent_index(rows, i)?;
        let back = match pad {
            Pad::B => true,
            Pad::Left => matches!(row.value, Value::Action | Value::Text(_) | Value::Info(_)),
            _ => false,
        };
        if !back {
            return None;
        }
        self.focus = parent;
        self.set_open(rows, parent, false);
        Some(Outcome::Moved)
    }

    /// The hints of the focused row: what A and ◀ ▶ do on it, the group's
    /// open / close among them.
    pub fn hints(&self, rows: &[Row]) -> Vec<Hint> {
        let Some(r) = self.focused(rows) else { return Vec::new() };
        let i = self.focus;
        if !self.usable(rows, i) && !folds(rows, i) {
            return Vec::new();
        }
        let mut h: Vec<Hint> = Vec::new();
        if folds(rows, i) {
            let open = self.open.contains(&r.id);
            let word = if open { tr!("Collapse") } else { tr!("Expand") };
            match Self::group_pad(r, open) {
                Pad::A => {
                    h.extend(hint_for(r).filter(|x| x.btn != Btn::A));
                    h.push(hint(Btn::A, word));
                }
                _ if matches!(r.value, Value::Action | Value::Info(_)) => h.push(hint(Btn::A, word)),
                // Only the arrow that works: ▶ opens, ◀ closes.
                pad => {
                    h.extend(hint_for(r));
                    h.push(hint(if pad == Pad::Left { Btn::Left } else { Btn::Right }, word));
                }
            }
            return h;
        }
        h.extend(hint_for(r));
        h
    }

    /// The presses that move the focus to row `to` (↑ ↓ over the focusable
    /// rows between; nothing when it has it or cannot take it).
    pub fn route(&self, rows: &[Row], to: usize) -> Vec<Pad> {
        if !self.can_focus(rows, to) || self.focus >= rows.len() {
            return Vec::new();
        }
        let between = |a: usize, b: usize| (a..b).filter(|&i| self.can_focus(rows, i)).count();
        if to > self.focus {
            super::pointer::repeat(Pad::Down, between(self.focus + 1, to + 1)).collect()
        } else {
            super::pointer::repeat(Pad::Up, between(to, self.focus)).collect()
        }
    }

    /// The presses a click on row `to` makes: the focus goes there, and the
    /// row is used as A uses it (an action runs, a switch flips, a choice
    /// takes its next value; a number only takes the focus: its arrows change it).
    pub fn click(&self, rows: &[Row], to: usize) -> Vec<Pad> {
        let Some(row) = rows.get(to).filter(|_| self.can_focus(rows, to)) else { return Vec::new() };
        let mut pads = self.route(rows, to);
        if self.usable(rows, to) && !matches!(row.value, Value::Number { .. }) {
            pads.push(Pad::A);
        }
        pads
    }

    /// The rows in a vertical scroll area (`id` must be unique on screen).
    /// `active` = the list has the focus (else no pill is drawn).
    pub fn render(&mut self, id: impl Into<ElementId>, rows: &[Row], active: bool) -> Stateful<Div> {
        self.render_entered(id, rows, active, None)
    }

    /// The rows as settings cards in a [`kit::settings_column`]: how a
    /// full-screen console config screen draws its list (one call).
    pub fn render_cards(&mut self, id: impl Into<ElementId>, rows: &[Row], active: bool) -> Div {
        self.style = RowStyle::Card;
        kit::settings_column(self.render(id, rows, active).flex_1().min_h_0())
    }

    /// [`render`](Self::render) for a list that `entry` gives the focus to
    /// while it does not have it (a column beside the owner's own): a click
    /// on one of its rows sends `entry` first.
    pub fn render_entered(&mut self, id: impl Into<ElementId>, rows: &[Row], active: bool, entry: Option<&[Pad]>) -> Stateful<Div> {
        self.clamp(rows);
        let lead: Option<Vec<Pad>> = if active { Some(Vec::new()) } else { entry.map(<[Pad]>::to_vec) };
        let focus = self.focus;
        let style = self.style;
        let places: Vec<Place> = (0..rows.len()).map(|i| self.place(rows, i)).collect();
        // A click on a group's chevron: the presses to its row, then its open / close press.
        let chevrons: Vec<Vec<Pad>> = (0..rows.len())
            .map(|i| match (places[i].group, &lead) {
                (Some(open), Some(lead)) => {
                    let to = if i == focus { Vec::new() } else { self.route(rows, i) };
                    lead.iter().copied().chain(to).chain([Self::group_pad(&rows[i], open)]).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        self.render_clicks(id, rows, active, lead.clone(), |i, r, focused| {
            // The ‹ › of the row the focus is on (or comes back to).
            let arrows = lead.as_deref().filter(|_| i == focus);
            let place = places[i].clone();
            let summary = (place.group == Some(false)).then(|| group_summary(rows, i));
            match style {
                RowStyle::Pill => render_row_in(r, focused, arrows, place, summary, chevrons[i].clone()).into_any_element(),
                RowStyle::Card => render_card_in(r, focused, arrows, place, summary, chevrons[i].clone()).into_any_element(),
            }
        })
    }

    /// [`render`](Self::render) with rows drawn by `draw(index, row, focused)`
    /// (a tab whose rows carry more than a label and a value).
    pub fn render_with(
        &mut self,
        id: impl Into<ElementId>,
        rows: &[Row],
        active: bool,
        draw: impl FnMut(usize, &Row, bool) -> AnyElement,
    ) -> Stateful<Div> {
        self.clamp(rows);
        // A click goes to a row of the list that has the focus.
        self.render_clicks(id, rows, active, active.then(Vec::new), draw)
    }

    /// The rows; with `lead` (the presses that give the list the focus),
    /// each takes clicks ([`Self::click`]).
    fn render_clicks(
        &mut self,
        id: impl Into<ElementId>,
        rows: &[Row],
        active: bool,
        lead: Option<Vec<Pad>>,
        mut draw: impl FnMut(usize, &Row, bool) -> AnyElement,
    ) -> Stateful<Div> {
        // A little of the rows around the focus stays in sight.
        let goal = Goal::Reveal { ix: self.focus, margin: 34. };
        if self.reveal && active {
            self.reveal = false;
            self.settle = 3;
            if self.shown {
                self.glide.go(goal);
            } else {
                self.glide.jump(goal);
            }
        } else if self.settle > 0 && active {
            // The rows measured last frame may predate a group opening.
            self.settle -= 1;
            self.glide.go(goal);
        }
        self.shown = true;
        let can: Vec<bool> = (0..rows.len()).map(|i| self.can_focus(rows, i)).collect();
        let usable: Vec<bool> = (0..rows.len()).map(|i| self.usable(rows, i)).collect();
        let clicks = lead.map(|lead| Clicks::new(&can, self.focus, lead));
        let gap = if self.style == RowStyle::Card { 8. } else { 4. };
        let mut list = div().id(id).flex().flex_col().overflow_y_scroll().track_scroll(self.glide.handle());
        let mut first = true;
        for (i, r) in rows.iter().enumerate() {
            // A row inside a closed group keeps its place (the scroll counts
            // the children) but draws nothing.
            if !self.shows(rows, i) {
                list = list.child(div());
                continue;
            }
            // A sub-setting joins its parent's card: no gap above it.
            let joined = r.parent.is_some() && parent_index(rows, i).is_some();
            let mut row = div().when(!first && !joined, |d| d.mt(u(gap))).child(draw(i, r, active && i == self.focus));
            first = false;
            if let Some(click) = clicks.as_ref().and_then(|c| c.at(&can, usable[i] && !matches!(r.value, Value::Number { .. }), i)) {
                // The mouse over a row moves the focus there.
                if click.moves() {
                    let route = click.route();
                    row = row.on_mouse_move(move |_, window, cx| {
                        if super::pointer::take_hover(window) {
                            super::pointer::press_hover(route.clone(), window, cx);
                        }
                    });
                }
                row = row.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    super::pointer::press(click.pads(), window, cx);
                });
            }
            list = list.child(row);
        }
        list.child(self.glide.driver())
    }
}

/// The clicks of a list's rows, worked out in one pass (a long list, like a
/// folder, has thousands of rows): how many focusable rows lie before each.
struct Clicks {
    lead: std::rc::Rc<[Pad]>,
    focus: usize,
    before: Vec<usize>,
}

/// One row's click, small until it is used: `lead`, `n` × `step`, then `then`.
struct Click {
    lead: std::rc::Rc<[Pad]>,
    step: Pad,
    n: usize,
    then: Option<Pad>,
}

impl Click {
    fn pads(&self) -> Vec<Pad> {
        self.route().into_iter().chain(self.then).collect()
    }

    /// The presses that bring the focus to the row (a hover).
    fn route(&self) -> Vec<Pad> {
        self.lead.iter().copied().chain(std::iter::repeat_n(self.step, self.n)).collect()
    }

    /// Whether the row needs presses to get the focus.
    fn moves(&self) -> bool {
        !self.lead.is_empty() || self.n > 0
    }
}

impl Clicks {
    /// `can`: which rows take the focus.
    fn new(can: &[bool], focus: usize, lead: Vec<Pad>) -> Self {
        let mut before = Vec::with_capacity(can.len() + 1);
        let mut n = 0;
        before.push(0);
        for c in can {
            n += usize::from(*c);
            before.push(n);
        }
        Self { lead: lead.into(), focus, before }
    }

    /// The click of row `to` (as [`ListNav::click`]); `None` for a header
    /// or a hidden row. `press`: the row takes A once it has the focus.
    fn at(&self, can: &[bool], press: bool, to: usize) -> Option<Click> {
        if !can.get(to).copied().unwrap_or(false) {
            return None;
        }
        let len = can.len();
        let (step, n) = if to > self.focus {
            (Pad::Down, self.before[to + 1] - self.before[(self.focus + 1).min(len)])
        } else {
            (Pad::Up, self.before[self.focus.min(len)] - self.before[to])
        };
        Some(Click { lead: self.lead.clone(), step, n, then: press.then_some(Pad::A) })
    }
}

/// One row (also used outside lists).
pub fn render_row(r: &Row, focused: bool) -> Div {
    render_row_at(r, focused, None)
}

/// [`render_row_at`] at its `place` among the groups: a sub-setting
/// indented under its parent beside a guide line, a parent with its chevron
/// and, closed, its `summary` under its label (`chevron`: the presses a
/// click on the chevron makes).
pub fn render_row_in(
    r: &Row,
    focused: bool,
    route: Option<&[Pad]>,
    place: Place,
    summary: Option<SharedString>,
    chevron_pads: Vec<Pad>,
) -> Div {
    if place.child {
        let row = render_row_full(r, focused, route, None, place.why.clone().filter(|_| focused), place.off);
        return div()
            .relative()
            .pl(u(34.))
            .child(div().absolute().top_0().left(u(20.)).w(px(2.)).bottom(if place.last { u(10.) } else { px(0.) }).bg(kit::ink(0.16)))
            .child(row);
    }
    let note = summary.map(|s| (s, place.warn));
    let row = render_row_full(r, focused, route, note, None, false);
    match place.group {
        Some(open) => row.child(chevron(&r.id, open, focused, chevron_pads)),
        None => row,
    }
}

/// [`render_row`] for a row the focus gets to with `route`: the ‹ › of its
/// value take clicks (the route, then ◀ or ▶).
pub fn render_row_at(r: &Row, focused: bool, route: Option<&[Pad]>) -> Div {
    render_row_full(r, focused, route, None, None, false)
}

/// [`render_row_at`] with a group's `summary` (red when it warns) under
/// the label, `why` in place of the description, its text and value faded
/// when `dim` (the focus pill keeps its strength).
fn render_row_full(
    r: &Row,
    focused: bool,
    route: Option<&[Pad]>,
    summary: Option<(SharedString, bool)>,
    why: Option<SharedString>,
    dim: bool,
) -> Div {
    let usable = r.enabled && !dim;
    let arrow = |pad: Pad| -> Vec<Pad> {
        match route {
            Some(route) if usable => route.iter().copied().chain([pad]).collect(),
            _ => Vec::new(),
        }
    };
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
            kit::choice_clickable(text, focused, *index > 0, index + 1 < options.len(), min_w, arrow(Pad::Left), arrow(Pad::Right))
                .into_any_element()
        }
        Value::Number { value, min, max, suffix, .. } => {
            kit::choice_clickable(format!("{value}{suffix}"), focused, value > min, value < max, 150., arrow(Pad::Left), arrow(Pad::Right))
                .into_any_element()
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
    // Under a switch that is off the text and value fade; the pill does not.
    let fade = if dim { 0.45 } else { 1. };
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
                .opacity(fade)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(u(8.))
                        .child(div().text_size(u(18.)).text_color(label_color).truncate().child(r.label.clone()))
                        .when(r.changed, |d| d.child(div().flex_none().size(u(7.)).rounded_full().bg(theme::accent()))),
                )
                .children(why.or_else(|| r.desc.clone()).filter(|_| focused).map(|d| {
                    div()
                        .text_size(u(13.))
                        .line_height(u(18.))
                        // One line: a longer one ends in "…" instead of growing the row.
                        .truncate()
                        .text_color(Hsla { a: 0.7, ..label_color })
                        .child(d)
                }))
                .children(summary.map(|(s, warn)| {
                    let c: Hsla = if warn { theme::danger_text().into() } else { Hsla { a: 0.7, ..label_color } };
                    div().text_size(u(13.)).line_height(u(18.)).truncate().text_color(c).child(s)
                })),
        )
        .child(div().flex_none().opacity(fade).child(value))
}

/// The value part of a card: the switch, `‹ chip ›`, a text action or a
/// muted value; `route` as in [`render_row_at`].
fn card_value(r: &Row, route: Option<&[Pad]>, usable: bool) -> AnyElement {
    let arrow = |pad: Pad| -> Vec<Pad> {
        match route {
            Some(route) if usable => route.iter().copied().chain([pad]).collect(),
            _ => Vec::new(),
        }
    };
    let muted = |t: &SharedString| {
        div().min_w_0().max_w(u(380.)).truncate().text_size(u(14.)).text_color(theme::text_muted()).child(if t.is_empty() {
            SharedString::from("\u{2014}")
        } else {
            t.clone()
        })
    };
    let value: AnyElement = match &r.value {
        Value::Action => match &r.verb {
            Some(v) => kit::text_action(v.clone()).into_any_element(),
            None => icon(Icon::ChevronRight, u(20.), theme::text_muted()).into_any_element(),
        },
        Value::Toggle(on) => kit::switch(SharedString::from(format!("switch-{}", r.id)), *on),
        Value::Choice { options, index } => {
            let text = options.get(*index).cloned().unwrap_or_default();
            kit::choice_chip(text, *index > 0, index + 1 < options.len(), arrow(Pad::Left), arrow(Pad::Right)).into_any_element()
        }
        Value::Number { value, min, max, suffix, .. } => {
            kit::choice_chip(format!("{value}{suffix}"), value > min, value < max, arrow(Pad::Left), arrow(Pad::Right)).into_any_element()
        }
        Value::Text(t) => div()
            .flex()
            .min_w_0()
            .items_center()
            .gap(u(14.))
            .child(muted(t))
            .when(usable, |d| d.child(kit::text_action(r.verb.clone().unwrap_or_else(|| tr!("Change").into()))))
            .into_any_element(),
        // A line with no value (a note) shows nothing at its end.
        Value::Info(t) if t.is_empty() => div().into_any_element(),
        Value::Info(t) => muted(t).into_any_element(),
        Value::Header => div().into_any_element(),
    };
    match &r.image {
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
    }
}

/// The open / closed mark of a group: ⌄ closed, ⌃ open (crossing between
/// them); a click sends `pads`. An action's "›" stays for what opens
/// something else.
fn chevron(id: &SharedString, open: bool, on_pill: bool, pads: Vec<Pad>) -> AnyElement {
    // On the light focus pill of a bare row it takes the pill's text colour.
    let (ink, fill): (Hsla, Hsla) = if on_pill {
        let c: Hsla = theme::console_on_focus().into();
        (c, Hsla { a: 0.12, ..c })
    } else {
        (theme::text_muted().into(), kit::ink(0.08))
    };
    let mark = div().flex_none().size(u(28.)).flex().items_center().justify_center().rounded_full().bg(fill);
    let mark =
        motion::spring(SharedString::from(format!("chev-{id}")), mark, motion::SOFT, open, move |d, k| {
            let k = k.clamp(0., 1.);
            // Two icons crossing over: the fonts have no rotation.
            d.child(div().absolute().opacity(1. - k).child(icon(Icon::ExpandMore, u(20.), ink)))
                .child(div().absolute().opacity(k).child(icon(Icon::ExpandLess, u(20.), ink)))
        });
    div().flex_none().relative().p(u(4.)).m(u(-4.)).child(mark).on_pads(pads).into_any_element()
}

/// One row as a card ([`RowStyle::Card`]) at its `place` among the groups:
/// a folding group's parent shows its chevron (and `summary` while closed);
/// shown sub-settings draw inside the same card under it, indented beside a
/// guide line. `chevron`: the presses a click on the chevron makes.
pub fn render_card_in(
    r: &Row,
    focused: bool,
    route: Option<&[Pad]>,
    place: Place,
    summary: Option<SharedString>,
    chevron_pads: Vec<Pad>,
) -> Div {
    if r.value == Value::Header {
        return kit::card_heading(r.label.clone());
    }
    let usable = r.enabled && !place.off;
    let label_color: Hsla = if r.danger { theme::danger_text().into() } else { theme::text_primary().into() };
    // A long read-only value (a status line) reads as the description,
    // under the label, instead of a cut line at the right.
    let long = match &r.value {
        Value::Info(t) if t.chars().count() > 32 => Some(t.clone()),
        _ => None,
    };
    // A row that only gathers its group shows the group's chevron alone.
    let value = if (place.group.is_some() && r.value == Value::Action && r.verb.is_none()) || long.is_some() {
        div().into_any_element()
    } else {
        card_value(r, route, usable)
    };
    let r = &Row {
        desc: long.map(|l| SharedString::from(r.desc.as_ref().map_or(l.to_string(), |d| format!("{l} {d}")))).or_else(|| r.desc.clone()),
        ..r.clone()
    };
    if place.child {
        return render_sub_card(r, focused, place, label_color, value);
    }
    let warn = place.warn;
    let text =
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(8.))
                    .child(div().text_size(u(17.)).text_color(label_color).truncate().child(r.label.clone()))
                    .when(r.changed, |d| d.child(div().flex_none().size(u(7.)).rounded_full().bg(kit::accent_ink()))),
            )
            .children(r.desc.clone().map(|d| {
                div().text_size(u(14.)).line_height(u(19.)).line_clamp(2).text_ellipsis().text_color(theme::text_muted()).child(d)
            }))
            .children(summary.map(|s| {
                let c: Hsla = if warn { theme::danger_text().into() } else { theme::text_muted().into() };
                div()
                    .flex()
                    .items_center()
                    .gap(u(6.))
                    .pt(u(2.))
                    .child(icon(Icon::Tune, u(14.), if warn { c } else { kit::accent_ink().into() }))
                    .child(div().min_w_0().truncate().text_size(u(14.)).text_color(c).child(s))
            }));
    // A disabled row fades its text and value; the ring keeps its strength.
    let fade = if r.enabled { 1. } else { 0.45 };
    kit::card_base(focused)
        .when(place.body, |d| d.rounded_bl(px(0.)).rounded_br(px(0.)))
        .child(text.opacity(fade))
        .child(div().flex().flex_none().max_w(relative(0.55)).justify_end().opacity(fade).child(value))
        .children(place.group.map(|open| chevron(&r.id, open, false, chevron_pads)))
}

/// A sub-setting inside its parent's card.
fn render_sub_card(r: &Row, focused: bool, place: Place, label_color: Hsla, value: AnyElement) -> Div {
    let usable = r.enabled && !place.off;
    let fade = if usable { 1. } else { 0.45 };
    let radius = u(14.);
    // Under a switch that is off, the focused one says why it does nothing.
    let desc = place.why.clone().filter(|_| focused).or_else(|| r.desc.clone());
    let content = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(u(14.))
        .opacity(fade)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(u(1.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(u(8.))
                        .child(div().text_size(u(16.)).text_color(label_color).truncate().child(r.label.clone()))
                        .when(r.changed, |d| d.child(div().flex_none().size(u(6.)).rounded_full().bg(kit::accent_ink()))),
                )
                .children(desc.map(|d| {
                    div().text_size(u(13.5)).line_height(u(18.)).line_clamp(2).text_ellipsis().text_color(theme::text_muted()).child(d)
                })),
        )
        .child(div().flex().flex_none().max_w(relative(0.55)).justify_end().child(value));
    // The ring sits outside the faded part, at full strength.
    let inner = div()
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .min_h(u(46.))
        .px(u(12.))
        .py(u(6.))
        .rounded(u(10.))
        .when(focused, |d| d.child(kit::focus_ring(10.)))
        .child(content);
    let row = div()
        .relative()
        .flex()
        .pl(u(34.))
        .pr(u(8.))
        .pt(u(3.))
        .pb(if place.last { u(9.) } else { u(3.) })
        .bg(kit::card_fill())
        .when(place.last, |d| d.rounded_bl(radius).rounded_br(radius))
        // The nested panel: a shade darker than the card.
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .when(place.last, |d| d.rounded_bl(radius).rounded_br(radius))
                .bg(kit::well(0.22)),
        )
        // The guide that ties the sub-settings to their parent.
        .child(div().absolute().top_0().left(u(22.)).w(px(2.)).bottom(if place.last { u(16.) } else { px(0.) }).bg(kit::ink(0.16)))
        .child(inner);
    let id = SharedString::from(format!("sub-in-{}", r.id));
    div().child(motion::enter_from(id, row, 220, 0., -6., 0.))
}

/// What A (or ◀ ▶) does on row `r`, as a hint (none for a row that does
/// nothing).
pub fn hint_for(r: &Row) -> Option<Hint> {
    if !r.enabled {
        return None;
    }
    Some(match &r.value {
        Value::Toggle(on) => hint(Btn::A, if *on { tr!("Turn off") } else { tr!("Turn on") }),
        Value::Choice { .. } | Value::Number { .. } => hint(Btn::LeftRight, tr!("Change")),
        Value::Text(_) => hint(Btn::A, r.verb.clone().unwrap_or_else(|| tr!("Change").into())),
        Value::Action => hint(Btn::A, r.verb.clone().unwrap_or_else(|| tr!("Select").into())),
        Value::Info(_) | Value::Header => return None,
    })
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
    fn a_click_moves_over_the_focusable_rows_then_uses_the_row() {
        let r = rows();
        let nav = ListNav { focus: 1, ..Default::default() };
        assert_eq!(nav.click(&r, 1), [Pad::A], "the focused row");
        assert_eq!(nav.click(&r, 5), [Pad::Down, Pad::Down, Pad::Down, Pad::Down, Pad::A]);
        assert_eq!(nav.click(&r, 3), [Pad::Down, Pad::Down], "a number only takes the focus");
        assert_eq!(nav.click(&r, 4), [Pad::Down, Pad::Down, Pad::Down], "disabled");
        assert!(nav.click(&r, 0).is_empty(), "a header");
        let nav = ListNav { focus: 5, ..Default::default() };
        // The header is skipped going up.
        assert_eq!(nav.route(&r, 1), [Pad::Up; 4]);
    }

    #[test]
    fn the_clicks_of_a_render_match_one_click_at_a_time() {
        use super::Clicks;
        let r = rows();
        for focus in 0..r.len() {
            let nav = ListNav { focus, ..Default::default() };
            let can: Vec<bool> = (0..r.len()).map(|i| nav.can_focus(&r, i)).collect();
            let clicks = Clicks::new(&can, focus, vec![Pad::Right]);
            for to in 0..r.len() {
                let mut want = nav.click(&r, to);
                if !r[to].focusable() {
                    assert!(clicks.at(&can, true, to).is_none());
                    continue;
                }
                want.insert(0, Pad::Right);
                let press = r[to].enabled && !matches!(r[to].value, Value::Number { .. });
                assert_eq!(clicks.at(&can, press, to).map(|c| c.pads()), Some(want), "{focus} -> {to}");
            }
        }
    }

    fn grouped() -> Vec<Row> {
        vec![
            Row::toggle("src", "Source", true),
            Row::toggle("opt", "Option", false).child_of("src"),
            Row::number("place", "Place", 2, 1, 5, 1, "").child_of("src"),
            Row::number("limit", "Limit", 30, 0, 60, 10, ""),
            Row::choice("how", "How", vec!["A".into(), "B".into()], 0).child_of("limit"),
            Row::toggle("smooth", "Smooth", false).child_of("limit"),
            Row::toggle("alone", "Alone", true),
            Row::toggle("only", "Only child", false).child_of("alone"),
            Row::action("after", "After"),
        ]
    }

    #[test]
    fn groups_open_close_and_hide_their_rows() {
        let r = grouped();
        let mut nav = ListNav::default();
        // Closed: the sub-settings are skipped.
        assert_eq!(nav.on_pad(Pad::Down, &r), Outcome::Moved);
        assert_eq!(nav.focus, 3);
        // A opens a group whose parent's ◀ ▶ change its value; ↓ goes in.
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Moved);
        assert_eq!(nav.on_pad(Pad::Down, &r), Outcome::Moved);
        assert_eq!(nav.focus, 4);
        // B goes back to the parent and closes the group.
        assert_eq!(nav.on_pad(Pad::B, &r), Outcome::Moved);
        assert_eq!(nav.focus, 3);
        assert!(!nav.shows(&r, 4));
        // ▶ opens a switch's group, A still flips the switch.
        nav.focus = 0;
        assert_eq!(nav.on_pad(Pad::Right, &r), Outcome::Moved);
        assert!(nav.shows(&r, 1));
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Changed("src".into(), Value::Toggle(false)));
        // ◀ closes it.
        assert_eq!(nav.on_pad(Pad::Left, &r), Outcome::Moved);
        assert!(!nav.shows(&r, 1));
        // A search result inside a closed group opens it.
        assert!(nav.focus_id(&r, "how"));
        assert!(nav.shows(&r, 4));
        // B on an open parent closes it first.
        nav.focus = 3;
        assert_eq!(nav.on_pad(Pad::B, &r), Outcome::Moved);
        assert!(!nav.shows(&r, 4));
        assert_eq!(nav.on_pad(Pad::B, &r), Outcome::Ignored, "a second B leaves");
    }

    #[test]
    fn left_on_a_switch_that_opens_a_group_never_flips_it() {
        let r = grouped();
        let mut nav = ListNav::default();
        // Closed: ◀ goes back (to the sidebar), the switch stays.
        let out = nav.on_pad(Pad::Left, &r);
        assert!(!matches!(out, Outcome::Changed(..)), "{out:?}");
        assert_eq!(out, Outcome::Edge(Pad::Left));
        // Open: ◀ closes, a second ◀ goes back; neither flips it.
        nav.on_pad(Pad::Right, &r);
        assert!(nav.shows(&r, 1));
        for _ in 0..2 {
            assert!(!matches!(nav.on_pad(Pad::Left, &r), Outcome::Changed(..)));
        }
        assert!(!nav.shows(&r, 1));
    }

    #[test]
    fn a_parent_can_fold_a_single_sub_setting_and_actions_do_not_warn() {
        use super::{folds, group_danger};
        let mut r = vec![
            Row::number("limit", "Limit", 30, 0, 60, 10, "").always_folds(),
            Row::choice("how", "How", vec!["A".into()], 0).child_of("limit"),
            Row::group("fav", "Favourites"),
            Row::info("f1", "Game", "").child_of("fav"),
            Row::action("clear", "Clear").danger().child_of("fav"),
        ];
        assert!(folds(&r, 0), "folds with one sub-setting");
        assert!(!group_danger(&r, 2), "a destructive action is not a warning");
        r[3].danger = true;
        assert!(group_danger(&r, 2));
    }

    #[test]
    fn a_group_of_one_is_always_open() {
        let r = grouped();
        let mut nav = ListNav::default();
        assert!(nav.shows(&r, 7), "its only sub-setting shows");
        assert_eq!(nav.place(&r, 6).group, None, "no chevron");
        assert!(nav.place(&r, 6).body);
        nav.focus = 6;
        // Its switch works as any switch.
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Changed("alone".into(), Value::Toggle(false)));
        assert_eq!(nav.on_pad(Pad::Down, &r), Outcome::Moved);
        assert_eq!(nav.focus, 7);
    }

    #[test]
    fn a_sub_setting_of_a_switch_that_is_off_does_nothing() {
        let mut r = grouped();
        r[0].value = Value::Toggle(false);
        let mut nav = ListNav::default();
        nav.on_pad(Pad::Right, &r);
        nav.on_pad(Pad::Down, &r);
        assert_eq!(nav.focus, 1);
        assert_eq!(nav.on_pad(Pad::A, &r), Outcome::Ignored);
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
