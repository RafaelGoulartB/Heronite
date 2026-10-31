//! The section list without the mouse, as the Collection's search and list:
//! ↓ in the search moves the keyboard to the list (the current section, or
//! the first result), ↑ ↓ Home End move its cursor, ↑ on the first item goes
//! back to the search, Enter or Space opens the item, a letter or `/` types
//! in the search again. Esc is left to the page (it empties the search, then
//! closes the page).

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::search::{self, Pick};
use super::{Section, SettingsPage};

/// Bounds of the list's items in the last frame, by item index (to keep the
/// cursor in view).
pub(super) type ItemBounds = Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>;

/// Room kept above and below the cursor when the list scrolls to it.
const MARGIN: f32 = 12.;

impl SettingsPage {
    /// The list's items in drawing order: every section, or the results
    /// (each section, then the settings found in it).
    pub(super) fn nav_items(&self) -> Vec<Pick> {
        if self.nav_query.is_empty() {
            return Section::ALL.iter().filter(|s| s.available()).map(|&s| Pick::Section(s)).collect();
        }
        search::search(&self.nav_query)
            .into_iter()
            .flat_map(|h| std::iter::once(Pick::Section(h.section)).chain(h.settings.into_iter().map(move |s| Pick::Setting(h.section, s))))
            .collect()
    }

    /// The section list has the keyboard (not the search field).
    pub fn nav_has_keyboard(&self, window: &Window) -> bool {
        self.nav_focus.is_focused(window)
    }

    /// The item the cursor is on while the list has the keyboard.
    pub(super) fn nav_cursor_shown(&self, window: &Window) -> Option<usize> {
        self.nav_cursor.filter(|_| self.nav_focus.is_focused(window))
    }

    /// ↓ in the search: the keyboard on the list, on the current section (or
    /// the first result).
    pub(super) fn nav_enter_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.nav_items();
        if items.is_empty() {
            return;
        }
        let ix = if self.nav_query.is_empty() { items.iter().position(|p| *p == Pick::Section(self.section)).unwrap_or(0) } else { 0 };
        self.nav_cursor = Some(ix);
        window.focus(&self.nav_focus, cx);
        self.nav_reveal(ix);
        cx.notify();
    }

    /// A click on an item: the cursor (and the keyboard) on it.
    pub(super) fn nav_clicked(&mut self, ix: usize, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        self.nav_cursor = Some(ix);
        window.focus(&self.nav_focus, cx);
        self.pick(pick, window, cx);
    }

    /// The search field with the keyboard again; `typed` is added to its text.
    fn nav_back_to_search(&mut self, typed: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.nav_cursor = None;
        self.nav_scroll.set_offset(point(px(0.), px(0.)));
        match typed {
            Some(typed) => {
                let text = format!("{}{typed}", self.nav_search.read(cx).value());
                // `set_value` sends no change event.
                self.nav_search.update(cx, |s, cx| {
                    s.set_value(text.clone(), window, cx);
                    s.focus(window, cx);
                });
                self.nav_query = text.trim().to_string();
            }
            None => self.focus_search(window, cx),
        }
        cx.notify();
    }

    /// The list's keys while it has the keyboard (the search field has its own).
    pub(super) fn nav_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.nav_focus.is_focused(window) {
            return;
        }
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        let items = self.nav_items();
        let last = items.len().saturating_sub(1);
        let cursor = self.nav_cursor.filter(|&c| c <= last);
        let to = match ks.key.as_str() {
            "down" => Some(cursor.map_or(0, |c| (c + 1).min(last))),
            "up" => match cursor {
                None | Some(0) => {
                    cx.stop_propagation();
                    self.nav_back_to_search(None, window, cx);
                    return;
                }
                Some(c) => Some(c - 1),
            },
            "home" => Some(0),
            "end" => Some(last),
            "enter" | "space" => {
                if let Some(&pick) = cursor.and_then(|c| items.get(c)) {
                    cx.stop_propagation();
                    self.pick(pick, window, cx);
                }
                return;
            }
            _ => {
                // A letter or `/`: back to the search, as on the Collection's list.
                let Some(text) = ks.key_char.as_deref().filter(|t| t.chars().count() == 1) else { return };
                let c = text.chars().next().unwrap_or(' ');
                if c == '/' {
                    cx.stop_propagation();
                    self.nav_back_to_search(None, window, cx);
                } else if c.is_alphanumeric() {
                    cx.stop_propagation();
                    self.nav_back_to_search(Some(text), window, cx);
                }
                return;
            }
        };
        let Some(to) = to.filter(|_| !items.is_empty()) else { return };
        cx.stop_propagation();
        self.nav_cursor = Some(to);
        self.nav_reveal(to);
        cx.notify();
    }

    /// Scrolls the list so item `ix` is in view (its bounds from the last frame).
    pub(super) fn nav_reveal(&self, ix: usize) {
        let Some(item) = self.nav_bounds.borrow().get(ix).copied().flatten() else { return };
        let view = self.nav_scroll.bounds();
        let offset = f32::from(self.nav_scroll.offset().y);
        let max = f32::from(self.nav_scroll.max_offset().y).max(0.);
        let above = f32::from(view.top() - item.top()) + MARGIN;
        let below = f32::from(item.bottom() - view.bottom()) + MARGIN;
        let y = if above > 0. {
            offset + above
        } else if below > 0. {
            offset - below
        } else {
            return;
        };
        self.nav_scroll.set_offset(point(px(0.), px(y.clamp(-max, 0.))));
    }
}

/// An invisible layer over item `ix` that records where it is drawn, with
/// the keyboard cursor's ring when it is on it. The item must be `relative()`.
pub(super) fn item_layer(bounds: &ItemBounds, ix: usize, cursor: bool, radius: Pixels) -> AnyElement {
    let bounds = bounds.clone();
    div()
        .absolute()
        .inset_0()
        .rounded(radius)
        .when(cursor, |d| d.border_1().border_color(crate::theme::accent_alpha(0.6)))
        .child(
            canvas(
                move |b, _, _| {
                    let mut all = bounds.borrow_mut();
                    if all.len() <= ix {
                        all.resize(ix + 1, None);
                    }
                    all[ix] = Some(b);
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
        .into_any_element()
}
