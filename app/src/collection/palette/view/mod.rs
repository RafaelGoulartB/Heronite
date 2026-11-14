//! The palette overlay: input, grouped results, footer.
//!
//! Two levels: the main list, and the actions of one game (→ or Ctrl Enter
//! on a game, its chevron, or "All actions"). Esc, or Backspace in the empty
//! box, goes back to the main list with its text as it was.

mod preview;
mod render;

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::rank::{actions_list, search, Filter, ItemRef, Results};
use super::sources::{Act, Entry};

/// Where the palette is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    /// Over the main window (Ctrl K).
    InApp,
    /// The quick launcher's own window (`crate::quick_launch`): it fills the
    /// window, Enter plays a game, and what opens Heronite is marked ↗.
    Launcher,
}

/// What a palette opens with.
pub(crate) struct Setup {
    pub entries: Rc<Vec<Entry>>,
    /// Ids run lately, most recent first.
    pub recent: Rc<Vec<String>>,
    /// Each game's cover chain, by key.
    pub covers: Rc<HashMap<SharedString, Vec<ImageRef>>>,
    /// Where the image cache is.
    pub data_paths: DataPaths,
    pub mode: Mode,
}

#[derive(Clone, Debug)]
pub(crate) enum PaletteEvent {
    /// Run `act`; `id` goes to the history.
    Run {
        act: Act,
        id: SharedString,
    },
    /// List every action of this game (the host answers with
    /// [`PaletteView::show_game`]).
    OpenGame(String),
    Close,
}

const ROW_H: f32 = 38.;
const CAPTION_H: f32 = 30.;
const LIST_PAD: f32 = 6.;
/// Room for about nine rows (and a caption); the rest scrolls.
const LIST_MAX_H: f32 = 9. * ROW_H + CAPTION_H;
const PANEL_W: f32 = 620.;
const INPUT_H: f32 = 54.;
const FOOTER_H: f32 = 42.;
/// Room around the quick launcher's panel for its shadow.
const LAUNCHER_MARGIN: f32 = 24.;
const TOP: f32 = 90.;
fn root_placeholder() -> &'static str {
    tr!("Type a command, page or game\u{2026}")
}

/// The second level: one game's actions.
struct GameLevel {
    title: SharedString,
    entries: Rc<Vec<Entry>>,
    /// The main list as it was, restored by [`PaletteView::back`].
    root_query: String,
    root_selected: usize,
}

pub(crate) struct PaletteView {
    mode: Mode,
    entries: Rc<Vec<Entry>>,
    /// Ids run lately, most recent first.
    recent: Rc<Vec<String>>,
    game: Option<GameLevel>,
    input: Entity<InputState>,
    query: String,
    filter: Filter,
    results: Results,
    selected: usize,
    scroll: ScrollHandle,
    covers: preview::Covers,
    _sub: Subscription,
}

impl EventEmitter<PaletteEvent> for PaletteView {}

impl PaletteView {
    pub fn new(setup: Setup, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Setup { entries, recent, covers, data_paths, mode } = setup;
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(root_placeholder(), window, cx);
            s.set_value(query.to_string(), window, cx);
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                if value != this.query {
                    this.query = value;
                    this.recompute(cx);
                }
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let mut this = Self {
            mode,
            entries,
            recent,
            game: None,
            input,
            query: query.to_string(),
            filter: Filter::All,
            results: Results::default(),
            selected: 0,
            scroll: ScrollHandle::new(),
            covers: preview::Covers::new(covers, data_paths),
            _sub: sub,
        };
        this.results = this.rank();
        this
    }

    fn rank(&self) -> Results {
        match &self.game {
            Some(g) => actions_list(&g.entries, &self.query, &self.recent, &g.title),
            None => search(&self.entries, &self.query, self.filter, &self.recent),
        }
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.results = self.rank();
        self.selected = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn entry(&self, r: ItemRef) -> &Entry {
        match (r, &self.game) {
            (ItemRef::Entry(i), Some(g)) => &g.entries[i],
            (ItemRef::Entry(i), None) => &self.entries[i],
            (ItemRef::Verb(i), _) => &self.results.verbs[i],
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.results.items().nth(self.selected).map(|i| self.entry(i))
    }

    /// Top and bottom of the selected row inside the list content (a group's
    /// first row includes its caption).
    fn selected_span(&self) -> Option<(f32, f32)> {
        let mut y = LIST_PAD;
        let mut n = 0;
        for g in &self.results.groups {
            let cap_top = y;
            y += CAPTION_H;
            for k in 0..g.items.len() {
                if n == self.selected {
                    return Some((if k == 0 { cap_top } else { y }, y + ROW_H));
                }
                y += ROW_H;
                n += 1;
            }
        }
        None
    }

    fn content_h(&self) -> f32 {
        let mut h = 2. * LIST_PAD;
        for g in &self.results.groups {
            h += CAPTION_H + ROW_H * g.items.len() as f32;
        }
        h
    }

    fn reveal_selected(&mut self) {
        let Some((top, bottom)) = self.selected_span() else { return };
        let view_h = self.content_h().min(LIST_MAX_H);
        let shown = f32::from(-self.scroll.offset().y);
        let new = if top < shown {
            top
        } else if bottom > shown + view_h {
            bottom - view_h
        } else {
            return;
        };
        self.scroll.set_offset(point(px(0.), px(-new)));
    }

    /// Moves the selection by `delta` rows, wrapping around.
    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.results.count() as isize;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        self.reveal_selected();
        cx.notify();
    }

    /// Next / previous filter (the main list only).
    pub(super) fn cycle_filter(&mut self, back: bool, cx: &mut Context<Self>) {
        if self.game.is_some() {
            return;
        }
        self.filter = if back { self.filter.prev() } else { self.filter.next() };
        self.recompute(cx);
    }

    fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        if self.filter != f {
            self.filter = f;
            self.recompute(cx);
        }
    }

    /// Runs the selected item. A disabled one only tells why (it is shown).
    /// In the quick launcher a game of the list is played (or stopped).
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.selected_entry() else { return };
        if e.disabled.is_some() {
            return;
        }
        let act = match self.mode {
            Mode::InApp => e.act.clone(),
            Mode::Launcher => super::launcher::enter_act(e),
        };
        let ev = match act {
            Act::GameActions(key) => PaletteEvent::OpenGame(key),
            act => PaletteEvent::Run { act, id: e.id.clone() },
        };
        cx.emit(ev);
    }

    /// Shift Enter in the quick launcher: the selected game (or the game the
    /// selected action is about) shown in Heronite.
    fn show_in_app(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_entry().and_then(|e| e.game_key()).map(str::to_string) else { return };
        cx.emit(PaletteEvent::Run { act: Act::SelectGame(key.clone()), id: format!("game:{key}").into() });
    }

    /// The main list's entries changed (the quick launcher's games came in);
    /// the text typed and the filter stay.
    pub fn set_entries(&mut self, entries: Rc<Vec<Entry>>, covers: Rc<HashMap<SharedString, Vec<ImageRef>>>, cx: &mut Context<Self>) {
        self.entries = entries;
        self.covers.set_chains(covers);
        if self.game.is_none() {
            self.recompute(cx);
        }
    }

    /// What the main list knows of the game under `key`.
    pub fn game_bits_of(&self, key: &str) -> Option<super::sources::GameBits> {
        self.entries.iter().find(|e| matches!(&e.act, Act::SelectGame(k) if k == key)).and_then(|e| e.game.clone())
    }

    /// The window the quick launcher needs for the palette (logical px).
    pub fn launcher_size() -> Size<Pixels> {
        let m = 2. * LAUNCHER_MARGIN;
        size(px(PANEL_W + preview::PANE_W + 2. + m), px(INPUT_H + LIST_MAX_H + FOOTER_H + 4. + m))
    }

    /// Asks for the actions of the selected game (or of the game the
    /// selected verb is about). Returns whether there was one.
    pub fn open_selected_game(&mut self, cx: &mut Context<Self>) -> bool {
        if self.game.is_some() {
            return false;
        }
        let Some(key) = self.selected_entry().and_then(|e| e.game_key()).map(str::to_string) else { return false };
        cx.emit(PaletteEvent::OpenGame(key));
        true
    }

    /// The second level, for the game `title` (entries from `action_entries`).
    /// Shown again over a game's list (another random pick), Esc still goes
    /// back to the main list as it was.
    pub fn show_game(&mut self, title: SharedString, entries: Vec<Entry>, window: &mut Window, cx: &mut Context<Self>) {
        let (root_query, root_selected) = match self.game.take() {
            Some(g) => (g.root_query, g.root_selected),
            None => (std::mem::take(&mut self.query), self.selected),
        };
        self.query.clear();
        let placeholder = tr!("Search the actions of {title}\u{2026}", title);
        self.game = Some(GameLevel { title, entries: Rc::new(entries), root_query, root_selected });
        self.input.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.set_placeholder(placeholder, window, cx);
        });
        self.recompute(cx);
    }

    /// Back to the main list from a game's actions; `false` when on it.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.game.take() else { return false };
        self.query = g.root_query.clone();
        self.input.update(cx, |s, cx| {
            s.set_value(g.root_query, window, cx);
            s.set_placeholder(root_placeholder(), window, cx);
        });
        self.results = self.rank();
        self.selected = g.root_selected.min(self.results.count().saturating_sub(1));
        self.reveal_selected();
        cx.notify();
        true
    }

    /// The palette's keys, fed by its host while it is open (before the
    /// input sees them). Returns whether the key was taken. Esc on the main
    /// list asks the host to close it ([`PaletteEvent::Close`]).
    pub fn key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        if m.control && !m.alt && !m.platform && !m.shift && key == "enter" {
            self.open_selected_game(cx);
            return true;
        }
        if m.control || m.alt || m.platform {
            return false;
        }
        match key {
            "escape" => {
                if !self.back(window, cx) {
                    cx.emit(PaletteEvent::Close);
                }
            }
            // In the empty box of a game's actions: back to the main list.
            "backspace" if self.in_game() && self.query_is_empty() => {
                self.back(window, cx);
            }
            // On a game: its actions (else the input moves its caret).
            "right" => return self.open_selected_game(cx),
            "up" => self.move_selection(-1, cx),
            "down" => self.move_selection(1, cx),
            "pageup" => self.move_selection(-8, cx),
            "pagedown" => self.move_selection(8, cx),
            "tab" => self.cycle_filter(m.shift, cx),
            "enter" if m.shift && self.mode == Mode::Launcher => self.show_in_app(cx),
            "enter" => self.activate(cx),
            _ => return false,
        }
        true
    }

    pub fn in_game(&self) -> bool {
        self.game.is_some()
    }

    pub fn query_is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// Currently listed titles (tests and debugging).
    #[allow(dead_code)]
    pub fn listed(&self) -> Vec<String> {
        self.results.items().map(|i| self.entry(i).title.to_string()).collect()
    }
}
