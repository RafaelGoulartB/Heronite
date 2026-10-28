//! The Collection without the mouse: the search shortcuts (Ctrl F, `/`,
//! typing a letter on the list), the search field's own keys (Esc, ↓, Enter)
//! and the list's moves (arrows, Home / End, Page Up / Down), with Space and
//! Shift picking several games.

use gpui_kit::component::input::{Enter, Escape, MoveDown};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::vm::ListItem;
use super::CollectionView;

/// Rows a Page Up / Down moves when the list has not been laid out yet.
const PAGE_ROWS: i32 = 4;

/// A move of the list's keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Nav {
    /// Cards across (wrapping to the next row) and rows down.
    Step(i32, i32),
    First,
    Last,
    PageUp,
    PageDown,
}

fn nav_of(key: &str) -> Option<Nav> {
    Some(match key {
        "left" => Nav::Step(-1, 0),
        "right" => Nav::Step(1, 0),
        "up" => Nav::Step(0, -1),
        "down" => Nav::Step(0, 1),
        "home" => Nav::First,
        "end" => Nav::Last,
        "pageup" => Nav::PageUp,
        "pagedown" => Nav::PageDown,
        _ => return None,
    })
}

/// The text a key types to start a search from the list: one letter or
/// digit, without Ctrl, Alt or Super (Space and punctuation have other uses).
fn search_char(ks: &Keystroke) -> Option<&str> {
    let m = &ks.modifiers;
    if m.control || m.alt || m.platform || m.function {
        return None;
    }
    let text = ks.key_char.as_deref()?;
    let mut chars = text.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_alphanumeric()).then_some(text)
}

impl CollectionView {
    /// Nothing floats over the Collection's list (dialog, menu, palette,
    /// screenshots) and it is on screen.
    pub(super) fn list_free(&self) -> bool {
        self.page == crate::pages::Page::Collection
            && !crate::console::active()
            && self.dialog.is_none()
            && self.context_menu.is_none()
            && self.open_menu.is_none()
            && !self.palette_open()
            && !self.shots.is_open()
    }

    pub(super) fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search_input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Puts the keyboard in the search field; `text` replaces what is typed,
    /// otherwise the text is selected so typing replaces it.
    fn focus_search(&mut self, text: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.update(cx, |s, cx| {
            match text {
                Some(text) => s.set_value(text.to_string(), window, cx),
                None => s.select_all(window, cx),
            }
            s.focus(window, cx);
        });
        if let Some(text) = text {
            self.set_search(text.to_string());
        }
    }

    /// `set_value` sends no change event: the list follows by hand.
    fn set_search(&mut self, text: String) {
        if self.search != text {
            self.search = text;
            self.refresh_groups();
            self.scroll_list_to_top();
        }
    }

    /// Ctrl F anywhere in the Collection, `/` or a letter typed on the list:
    /// the search field. True when the key was used.
    pub(super) fn search_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let collection = self.page == crate::pages::Page::Collection
            && !crate::console::active()
            && self.dialog.is_none()
            && !self.palette_open()
            && !self.shots.is_open();
        if !collection {
            return false;
        }
        if ks.key == "f" && (m.control || m.platform) && !m.alt && !m.shift {
            self.context_menu = None;
            self.open_menu = None;
            self.focus_search(None, window, cx);
            return true;
        }
        // The other keys are the list's: a field elsewhere keeps its typing.
        if !self.list_free() || !self.focus_handle.is_focused(window) {
            return false;
        }
        if ks.key_char.as_deref() == Some("/") && !m.control && !m.alt && !m.platform {
            self.focus_search(None, window, cx);
            return true;
        }
        let Some(text) = search_char(ks) else { return false };
        let text = text.to_string();
        self.focus_search(Some(&text), window, cx);
        true
    }

    /// Esc in the search field: the first empties it, the next one gives the
    /// keyboard back to the list (where a third clears the selected game).
    pub(super) fn search_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_focused(window, cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        if self.search.is_empty() {
            window.focus(&self.focus_handle, cx);
        } else {
            self.search_input.update(cx, |s, cx| s.set_value("", window, cx));
            self.set_search(String::new());
            self.reveal_selected();
        }
        cx.notify();
    }

    /// ↓ in the search field: the first result, with the keyboard on the list.
    pub(super) fn search_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_focused(window, cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        if let Some(first) = self.shown_keys().into_iter().next() {
            self.select(first, true);
            self.reveal_selected();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Enter in the search field: plays the selected game when it is among
    /// the results (else the first result) if it is installed, and shows it
    /// otherwise.
    pub(super) fn search_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_focused(window, cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        let shown = self.shown_keys();
        let target = self.selected.clone().filter(|k| shown.contains(k)).or_else(|| shown.into_iter().next());
        let Some(key) = target else { return };
        self.select(key.clone(), true);
        self.reveal_selected();
        window.focus(&self.focus_handle, cx);
        if self.focus.as_ref().is_some_and(|f| f.installed && !f.playing) {
            self.install_or_play(&key, window, cx);
        }
        cx.notify();
    }

    /// The list's moves: arrows, Home / End and Page Up / Down move the
    /// selected game; with Shift they pick every game from where the
    /// selection started. Space picks or drops the selected game. True when
    /// the key was used.
    pub(super) fn list_key(&mut self, ev: &KeyDownEvent, window: &mut Window) -> bool {
        if !self.list_free() || !self.focus_handle.is_focused(window) {
            return false;
        }
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return false;
        }
        let key = ev.keystroke.key.as_str();
        if key == "space" && !m.shift {
            let Some(sel) = self.selected.clone().filter(|k| self.shown_keys().contains(k)) else { return false };
            self.toggle_selected(sel);
            return true;
        }
        let Some(nav) = nav_of(key) else { return false };
        let from = self.selected.clone();
        let Some(to) = self.nav_target(nav) else { return true };
        if m.shift {
            self.extend_selection(from, to.clone());
        }
        self.select(to, true);
        self.reveal_selected();
        true
    }

    fn nav_target(&self, nav: Nav) -> Option<SharedString> {
        match nav {
            Nav::Step(dx, dy) => self.grid_neighbour(dx, dy),
            Nav::First => self.shown_keys().into_iter().next(),
            Nav::Last => self.shown_keys().pop(),
            Nav::PageUp => self.grid_neighbour(0, -self.page_rows()),
            Nav::PageDown => self.grid_neighbour(0, self.page_rows()),
        }
    }

    /// Card rows that fit in the list's viewport (at least one).
    fn page_rows(&self) -> i32 {
        let viewport = f32::from(self.list_state.viewport_bounds().size.height);
        let row = self.items.iter().enumerate().find_map(|(ix, it)| match it {
            ListItem::Row { .. } => self.list_state.bounds_for_item(ix).map(|b| f32::from(b.size.height)),
            _ => None,
        });
        match row {
            Some(row) if row > 0. && viewport > 0. => ((viewport / row).floor() as i32).max(1),
            _ => PAGE_ROWS,
        }
    }

    /// Shift with a move: the games from the selection's start to `to`
    /// replace those the last Shift move picked (games picked otherwise stay).
    fn extend_selection(&mut self, from: Option<SharedString>, to: SharedString) {
        let order = self.shown_keys();
        let anchor = self.sel.anchor.clone().filter(|a| order.contains(a)).or(from).unwrap_or_else(|| to.clone());
        let at = |k: &SharedString| order.iter().position(|o| o == k);
        let (Some(a), Some(b)) = (at(&anchor), at(&to)) else { return };
        let range: Vec<SharedString> = order[a.min(b)..=a.max(b)].to_vec();
        let previous = std::mem::take(&mut self.sel.shift_range);
        let mut keys: Vec<SharedString> = self.sel.keys.iter().filter(|k| !previous.contains(k)).cloned().collect();
        keys.extend(range.iter().cloned());
        self.set_selection(keys);
        self.sel.anchor = Some(anchor);
        self.sel.shift_range = range;
    }
}

#[cfg(test)]
mod tests {
    use super::{nav_of, search_char, Nav};
    use gpui_kit::{Keystroke, Modifiers};

    fn ks(key: &str, ch: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke { key: key.into(), key_char: ch.map(Into::into), modifiers }
    }

    #[test]
    fn letters_and_digits_start_a_search() {
        assert_eq!(search_char(&ks("a", Some("a"), Modifiers::default())), Some("a"));
        assert_eq!(search_char(&ks("a", Some("A"), Modifiers::shift())), Some("A"));
        assert_eq!(search_char(&ks("7", Some("7"), Modifiers::default())), Some("7"));
        assert_eq!(search_char(&ks("c", Some("ç"), Modifiers::default())), Some("ç"));
    }

    #[test]
    fn shortcuts_space_and_punctuation_do_not() {
        assert_eq!(search_char(&ks("a", Some("a"), Modifiers::control())), None);
        assert_eq!(search_char(&ks("space", Some(" "), Modifiers::default())), None);
        assert_eq!(search_char(&ks("/", Some("/"), Modifiers::default())), None);
        assert_eq!(search_char(&ks("f5", None, Modifiers::default())), None);
    }

    #[test]
    fn list_moves() {
        assert_eq!(nav_of("down"), Some(Nav::Step(0, 1)));
        assert_eq!(nav_of("home"), Some(Nav::First));
        assert_eq!(nav_of("pagedown"), Some(Nav::PageDown));
        assert_eq!(nav_of("enter"), None);
    }
}
