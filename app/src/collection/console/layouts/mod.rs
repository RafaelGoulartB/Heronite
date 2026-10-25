//! The layouts a console tab can use (`heronite_data::console_theme::Layout`).
//! Each lives in its own file with the same three entry points
//! (`render_<layout>`, `<layout>_pad`, `<layout>_hints`) and its own focus
//! state (`Nav`; Shelves and Cinema share [`ShelfNav`] and [`shelf_pad`]); a
//! tab keeps the focus of every layout, so switching a tab's layout and back
//! finds its place. The shell (`super`) draws the top bar, the hint bar and
//! the background around them.

mod carousel;
mod cinema;
mod dashboard;
mod grid;
mod shelves;
mod spotlight;
mod wheel;
mod xbox;

use std::ops::RangeInclusive;
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::console_theme::{GridPanel, Hints, Layout, TabSpec};
use heronite_data::CollectionGame;
use heronite_i18n::tr;

pub(super) use grid::{category_options, status_options, Filters, SHOW};

use super::super::CollectionView;
use super::backdrop::Scrim;
use super::sources::Shelf;
use crate::console::kit::{self, u};
use crate::console::motion::{Axis, Glide, Goal};
use crate::console::{look, Hint, Pad};
use crate::icons::Icon;

/// Focus of one tab in every layout.
#[derive(Default)]
pub(crate) struct TabNav {
    pub shelves: ShelfNav,
    pub spotlight: spotlight::Nav,
    pub cinema: ShelfNav,
    pub dashboard: dashboard::Nav,
    pub grid: grid::Nav,
    pub wheel: wheel::Nav,
    pub carousel: carousel::Nav,
    pub xbox: xbox::Nav,
}

/// The background shading a layout wants under it.
pub(super) fn scrim(layout: Layout) -> Scrim {
    match layout {
        Layout::Shelves => Scrim::Classic,
        Layout::Spotlight => Scrim::Home,
        Layout::Xbox => Scrim::Spot,
        Layout::Cinema => Scrim::Cinema,
        Layout::Carousel => Scrim::Carousel,
        Layout::Dashboard | Layout::Wheel => Scrim::Full,
        // The game panel sits over the art: evenly dark, like the lists.
        Layout::Grid if look::theme().layouts.grid_panel == GridPanel::Right => Scrim::Full,
        Layout::Grid => Scrim::Library,
    }
}

/// Whether the hint bar floats over the layout and fades in and out
/// (`Hints::Auto`) instead of taking its room: the screen never moves when it
/// comes and goes. Spotlight keeps that room clear itself.
pub(super) fn hints_float(layout: Layout) -> bool {
    layout == Layout::Spotlight && look::theme().controls.hints == Hints::Auto
}

/// `i` moved by `delta` in `0..n`, going round when `wrap`.
pub(crate) fn step(i: usize, n: usize, delta: isize, wrap: bool) -> usize {
    if n == 0 {
        return 0;
    }
    let j = i as isize + delta;
    if wrap {
        j.rem_euclid(n as isize) as usize
    } else {
        j.clamp(0, n as isize - 1) as usize
    }
}

impl CollectionView {
    /// The focus state of tab `tab` (created on first use).
    pub(super) fn tab_nav(&mut self, tab: usize) -> &mut TabNav {
        let navs = &mut self.console.tab_navs;
        if navs.len() <= tab {
            navs.resize_with(tab + 1, TabNav::default);
        }
        &mut navs[tab]
    }

    pub(super) fn render_layout(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        match spec.layout {
            Layout::Shelves => self.render_shelves(tab, spec, window, cx),
            Layout::Spotlight => self.render_spotlight(tab, spec, window, cx),
            Layout::Cinema => self.render_cinema(tab, spec, window, cx),
            Layout::Dashboard => self.render_dashboard(tab, spec, window, cx),
            Layout::Grid => self.render_grid(tab, spec, window, cx),
            Layout::Wheel => self.render_wheel(tab, spec, window, cx),
            Layout::Carousel => self.render_carousel(tab, spec, window, cx),
            Layout::Xbox => self.render_xbox(tab, spec, window, cx),
        }
    }

    /// `false` lets the shell use the press (LB / RB switch tabs, B opens the menu).
    pub(super) fn layout_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match spec.layout {
            Layout::Shelves => self.shelf_pad(tab, spec, pad, window, cx, |n| &mut n.shelves),
            Layout::Spotlight => self.spotlight_pad(tab, spec, pad, window, cx),
            Layout::Cinema => self.shelf_pad(tab, spec, pad, window, cx, |n| &mut n.cinema),
            Layout::Dashboard => self.dashboard_pad(tab, spec, pad, window, cx),
            Layout::Grid => self.grid_pad(tab, spec, pad, window, cx),
            Layout::Wheel => self.wheel_pad(tab, spec, pad, window, cx),
            Layout::Carousel => self.carousel_pad(tab, spec, pad, window, cx),
            Layout::Xbox => self.xbox_pad(tab, spec, pad, window, cx),
        }
    }

    pub(super) fn layout_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        match spec.layout {
            Layout::Shelves => self.shelves_hints(tab, spec),
            Layout::Cinema => self.console_cover_hints(),
            Layout::Spotlight => self.spotlight_hints(tab, spec),
            Layout::Dashboard => self.dashboard_hints(tab, spec),
            Layout::Grid => self.grid_hints(tab, spec),
            Layout::Wheel => self.wheel_hints(tab, spec),
            Layout::Carousel => self.carousel_hints(tab, spec),
            Layout::Xbox => self.xbox_hints(tab, spec),
        }
    }
}

/// What a layout shows when the tab has no games.
fn no_games() -> AnyElement {
    kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element()
}

/// The columns (of a row of covers) in sight in a row scrolled by `scroll`
/// px, items `step` design px apart, in a view `view` px wide: from the first
/// one showing to one more than fit.
fn in_sight(scroll: Pixels, step: f32, view: Pixels) -> RangeInclusive<usize> {
    let step = f32::from(u(step));
    let first = (-f32::from(scroll) / step).floor().max(0.) as usize;
    first..=first + (f32::from(view) / step).ceil() as usize + 1
}

/// Whether item `i` of a row loads its picture: in sight (`seen`) or near the focus.
fn loads(seen: &RangeInclusive<usize>, i: usize, focus: usize) -> bool {
    seen.contains(&i) || i.abs_diff(focus) <= 2
}

/// Height of the body under the shell's top bar (and above its hint bar), design px.
fn body_height(window: &Window, hints: bool) -> f32 {
    /// Height of the shell's top bar and hint bar around the body, design px.
    const TOP_BAR: f32 = 76.;
    const HINT_BAR: f32 = 60.;
    f32::from(window.viewport_size().height) / kit::scale() - TOP_BAR - if hints { HINT_BAR } else { 0. }
}

/// The `k` most recently played games, newest first (never-played ones
/// follow, by title).
fn top_recent(games: &[Arc<CollectionGame>], k: usize) -> Vec<Arc<CollectionGame>> {
    let order = |a: &&Arc<CollectionGame>, b: &&Arc<CollectionGame>| {
        b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower))
    };
    let mut v: Vec<&Arc<CollectionGame>> = games.iter().collect();
    if v.len() > k {
        v.select_nth_unstable_by(k, order);
        v.truncate(k);
    }
    v.sort_by(order);
    v.into_iter().cloned().collect()
}

/// A focused column in each of a layout's shelves (kept when moving
/// between shelves) and the glide of each shelf, left and right.
#[derive(Default)]
pub(crate) struct ShelfLines {
    cols: Vec<usize>,
    glides: Vec<Glide>,
}

impl ShelfLines {
    /// Room for `n` shelves.
    fn fit(&mut self, n: usize) {
        self.cols.resize(n, 0);
        while self.glides.len() < n {
            self.glides.push(Glide::new(Axis::X));
        }
    }

    /// Room for the (non-empty) `shelves`, the columns inside them.
    fn sync(&mut self, shelves: &[Shelf]) {
        self.sync_with(shelves, false);
    }

    /// [`Self::sync`]; `more`: a shelf the theme cut ends with a "View all"
    /// tile, one more column.
    fn sync_with(&mut self, shelves: &[Shelf], more: bool) {
        self.fit(shelves.len());
        for (col, s) in self.cols.iter_mut().zip(shelves) {
            *col = (*col).min(columns(s, more) - 1);
        }
    }
}

/// The columns of `shelf`: its covers, and its "View all" tile when `more`
/// and the theme's limit cut it.
fn columns(shelf: &Shelf, more: bool) -> usize {
    shelf.keys.len() + usize::from(more && shelf.cut())
}

/// The focus of a layout of shelves of covers (Shelves and Cinema).
pub(crate) struct ShelfNav {
    pub shelf: usize,
    lines: ShelfLines,
    /// The shelves, up and down.
    shelves: Glide,
    /// Where the last move came from (x: -1 left, 1 right; y: -1 up, 1
    /// down), so the focused game's text slides in from that side.
    from: (f32, f32),
    /// Counts the moves (part of the slide's id: every move replays it).
    moves: u64,
}

impl Default for ShelfNav {
    fn default() -> Self {
        Self { shelf: 0, lines: ShelfLines::default(), shelves: Glide::new(Axis::Y), from: (1., 0.), moves: 0 }
    }
}

impl ShelfNav {
    /// Keeps the focus on the (non-empty) `shelves`; the focused game's key.
    fn sync(&mut self, shelves: &[Shelf]) -> SharedString {
        self.shelf = self.shelf.min(shelves.len() - 1);
        self.lines.sync(shelves);
        shelves[self.shelf].keys[self.lines.cols[self.shelf]].clone()
    }

    /// [`Self::sync`] for shelves that end with a "View all" tile: the
    /// focused game (the last one of the shelf while that tile has the
    /// focus) and whether the tile has it.
    fn sync_more(&mut self, shelves: &[Shelf]) -> (SharedString, bool) {
        self.shelf = self.shelf.min(shelves.len() - 1);
        self.lines.sync_with(shelves, true);
        let s = &shelves[self.shelf];
        let col = self.lines.cols[self.shelf];
        (s.keys[col.min(s.keys.len() - 1)].clone(), col >= s.keys.len())
    }
}

impl CollectionView {
    /// A press in a layout of shelves: ← → along a shelf, ↑ ↓ between shelves
    /// (the focused one rises to the top, its cover in sight with a peek of
    /// the next ones), A / X on the game, Y the search. `nav` picks the
    /// layout's focus.
    fn shelf_pad(
        &mut self,
        tab: usize,
        spec: &TabSpec,
        pad: Pad,
        window: &mut Window,
        cx: &mut Context<Self>,
        nav: fn(&mut TabNav) -> &mut ShelfNav,
    ) -> bool {
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return false;
        }
        let (tw, _) = self.console_tile_size();
        let wrap = look::theme().nav.wrap;
        // Shelves end a cut shelf with a "View all" tile.
        let more = spec.layout == Layout::Shelves;
        let nav = nav(self.tab_nav(tab));
        nav.lines.fit(shelves.len());
        let si = nav.shelf.min(shelves.len() - 1);
        let n = columns(&shelves[si], more);
        let ci = nav.lines.cols[si].min(n - 1);
        let key = shelves[si].keys.get(ci).cloned();
        let (before, before_shelf) = (ci, si);
        match pad {
            Pad::Left | Pad::Right => nav.lines.cols[si] = step(ci, n, pad.dir(), wrap),
            Pad::Up => nav.shelf = si.saturating_sub(1),
            Pad::Down => nav.shelf = (si + 1).min(shelves.len() - 1),
            // A page along the shelf (LT / RT), ends included.
            Pad::LT | Pad::RT if more => {
                let page = ((f32::from(window.viewport_size().width) / kit::scale() / (tw + 18.)).floor() as isize - 1).max(1);
                let back = pad == Pad::LT;
                nav.lines.cols[si] = step(ci, n, if back { -page } else { page }, false);
            }
            Pad::A | Pad::X => {
                match key {
                    Some(key) => self.console_activate(key, pad, window, cx),
                    None => self.console_open_shelf(&shelves[si], window, cx),
                }
                return true;
            }
            Pad::Y => {
                self.open_console_filters(window, cx);
                return true;
            }
            _ => return false,
        }
        let si = nav.shelf;
        let ci = nav.lines.cols[si].min(columns(&shelves[si], more) - 1);
        if (si, ci) != (before_shelf, before) {
            nav.moves += 1;
            nav.from = match pad {
                Pad::Up => (0., -1.),
                Pad::Down => (0., 1.),
                _ => ((ci as f32 - before as f32).signum(), 0.),
            };
        }
        nav.shelves.go(Goal::Start(si));
        nav.lines.glides[si].go(Goal::Reveal { ix: ci, margin: tw * 0.6 });
        true
    }

    /// A on a shelf's "View all" tile: the Library (or the results grid of
    /// a theme without one) with the filters that show the same games, the
    /// most recently played first.
    fn console_open_shelf(&mut self, shelf: &Shelf, _window: &mut Window, _cx: &mut Context<Self>) {
        let status = shelf.kind.strip_prefix("status:").and_then(|id| self.lib.as_ref()?.statuses.iter().position(|s| s.id == id));
        let kind = shelf.kind.to_string();
        let (i, _) = self.filters_target();
        self.console.search.clear();
        self.tab_nav(i).grid.show_like(&kind, status);
        self.console_show_filtered();
    }

    /// Opens the library: the first Grid tab, else the results grid.
    pub(super) fn console_open_library(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        let (i, _) = self.filters_target();
        if i < look::theme().tabs.len() {
            self.console.tab = i;
            self.console.tab_dir = 1.;
        } else {
            self.console.results = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{columns, step, Shelf, ShelfLines};

    fn shelf(shown: usize, total: usize) -> Shelf {
        Shelf { title: "s".into(), keys: (0..shown).map(|i| i.to_string().into()).collect(), total, kind: "installed".into() }
    }

    #[test]
    fn a_cut_shelf_ends_with_view_all() {
        assert_eq!(columns(&shelf(20, 28), true), 21);
        assert_eq!(columns(&shelf(20, 28), false), 20);
        assert_eq!(columns(&shelf(5, 5), true), 5);
        // The focus may rest on "View all", one past the covers, and comes
        // back to the last cover when the tile goes away.
        let mut lines = ShelfLines::default();
        lines.sync_with(&[shelf(20, 28)], true);
        lines.cols[0] = 20;
        lines.sync_with(&[shelf(20, 28)], true);
        assert_eq!(lines.cols[0], 20);
        lines.sync_with(&[shelf(20, 20)], true);
        assert_eq!(lines.cols[0], 19);
    }

    #[test]
    fn steps_clamp_or_wrap() {
        assert_eq!(step(0, 5, -1, false), 0);
        assert_eq!(step(0, 5, -1, true), 4);
        assert_eq!(step(4, 5, 1, true), 0);
        assert_eq!(step(2, 5, 7, false), 4);
        assert_eq!(step(0, 0, 1, true), 0);
    }
}
