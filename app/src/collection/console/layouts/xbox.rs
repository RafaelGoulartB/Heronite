//! Xbox: the focused game's art fills the screen and a single row of tiles
//! sits in the lower middle: the latest game wide, recent games square, then
//! "My games". Above the row, the game's name and a line about it. ← → glide
//! the row so the focus stays near the left. With the theme's groups on, ↓
//! goes into the shelves under the row and the whole screen glides up so the
//! focused shelf sits where the row was (the art stays behind); ↑ returns.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{TabSpec, TileShape};
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use super::super::hero::HeroSize;
use super::super::tile::label_room;
use super::dashboard::{body_height, focus_frame, played_line, top_recent};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

pub(super) const SCRIM: Scrim = Scrim::Spot;

const GAP: f32 = 18.;
const SIDE: f32 = 56.;
/// Room around a row of tiles for the lifted one and its ring.
const PAD: f32 = 14.;
/// Height of a shelf's name.
const CAPTION: f32 = 32.;
/// Games in the row of tiles (before "My games").
const RECENT: usize = 12;
/// A shelf's tiles against the row's.
const SMALL: f32 = 0.8;
/// Empty space under the row of tiles when the screen is at rest (the next
/// shelf's name and the top of its tiles peek into it).
const FLOOR: f32 = 140.;

pub(crate) struct Nav {
    /// `None`: the row of tiles; `Some(g)`: shelf `g` under it.
    group: Option<usize>,
    /// Focused tile of the row (the last is "My games").
    col: usize,
    /// Focused tile of each shelf.
    cols: Vec<usize>,
    /// The focus is on "My games" (for the hints).
    mine: bool,
    row: Glide,
    rows: Vec<Glide>,
    /// The whole screen, up and down.
    screen: Glide,
}

impl Default for Nav {
    fn default() -> Self {
        Self { group: None, col: 0, cols: Vec::new(), mine: false, row: Glide::new(Axis::X), rows: Vec::new(), screen: Glide::new(Axis::Y) }
    }
}

impl Nav {
    fn fit(&mut self, shelves: usize) {
        self.cols.resize(shelves, 0);
        while self.rows.len() < shelves {
            self.rows.push(Glide::new(Axis::X));
        }
    }
}

/// Sizes of the screen, design px.
struct Metrics {
    /// Square tile.
    sq: f32,
    wide: f32,
    /// Shelf tile.
    small: (f32, f32),
    body: f32,
    /// Top of the row's tiles when the screen is at rest.
    row_top: f32,
    /// Height of the row's section.
    home: f32,
    /// Height of a shelf's section.
    shelf: f32,
}

impl Metrics {
    /// How far the screen is scrolled when shelf `g` is focused.
    fn offset(&self, g: usize) -> f32 {
        self.home + g as f32 * self.shelf + CAPTION + PAD - self.row_top
    }

    /// Left edge and width of tile `i` of the row.
    fn tile_span(&self, i: usize) -> (f32, f32) {
        if i == 0 {
            (SIDE, self.wide)
        } else {
            (SIDE + self.wide + GAP + (i - 1) as f32 * (self.sq + GAP), self.sq)
        }
    }
}

impl CollectionView {
    fn xbox_metrics(&self, window: &Window) -> Metrics {
        let theme = look::theme();
        let sq = look::tile_size(TileShape::Square, theme.tiles.size, self.cover_aspect).0;
        let body = body_height(window, self.hints_visible());
        let room = label_room();
        let row_top = (body - sq - room - FLOOR).max(190.);
        let sh = (sq * SMALL).round();
        Metrics {
            sq,
            wide: (sq * 16. / 9.).round(),
            small: ((sh / self.cover_aspect).round(), sh),
            body,
            row_top,
            home: row_top - PAD + sq + 2. * PAD + room + 18.,
            shelf: CAPTION + sh + 2. * PAD + room + 12.,
        }
    }

    pub(super) fn render_xbox(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        let games = self.console_games(&spec.source);
        if games.is_empty() {
            return kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element();
        }
        let theme = look::theme();
        let recent: Vec<SharedString> = top_recent(&games, RECENT).into_iter().map(|g| SharedString::from(g.key.clone())).collect();
        let shelves = if theme.layouts.xbox_groups { self.console_shelves(&spec.source) } else { Vec::new() };
        let m = self.xbox_metrics(window);

        let nav = &mut self.tab_nav(tab).xbox;
        nav.fit(shelves.len());
        nav.col = nav.col.min(recent.len());
        for (c, s) in nav.cols.iter_mut().zip(&shelves) {
            *c = (*c).min(s.keys.len() - 1);
        }
        nav.group = nav.group.filter(|g| *g < shelves.len());
        nav.mine = nav.group.is_none() && nav.col == recent.len();
        let (group, col, mine) = (nav.group, nav.col, nav.mine);
        let cols = nav.cols.clone();
        let (row, rows, screen) = (nav.row.clone(), nav.rows.clone(), nav.screen.clone());

        let focused = match group {
            Some(g) => Some(shelves[g].keys[cols[g]].clone()),
            None => recent.get(col).cloned(),
        };
        // "My games" keeps the latest game's art behind it.
        self.console_focus_game(focused.clone().or_else(|| recent.first().cloned()), cx);

        let scale = kit::scale();
        let view_w = f32::from(window.viewport_size().width) / scale;
        let off_x = -f32::from(row.handle().offset().x) / scale;
        let off_y = -f32::from(screen.handle().offset().y) / scale;
        let at = group.map_or(0, |g| g + 1);

        // The row of tiles.
        let home_in_sight = off_y < m.home;
        let mut tiles = div()
            .id(("xbox-row", tab))
            .absolute()
            .left_0()
            .right_0()
            .top(u(m.row_top - PAD))
            .flex()
            .gap(u(GAP))
            .p(u(PAD))
            .pl(u(SIDE))
            .pb(u(PAD + label_room()))
            .overflow_x_scroll()
            .track_scroll(row.handle());
        for i in 0..=recent.len() {
            let (left, width) = m.tile_span(i);
            let on_screen = home_in_sight && left + width > off_x && left < off_x + view_w;
            let load = on_screen || (group.is_none() && i.abs_diff(col) <= 2);
            let on = group.is_none() && i == col;
            tiles = tiles.child(match recent.get(i) {
                Some(key) if i == 0 => self.console_tile_shaped(key, TileShape::Landscape, m.wide, m.sq, on, load, window),
                Some(key) => self.console_tile_shaped(key, TileShape::Square, m.sq, m.sq, on, load, window),
                None => self.xbox_mine_tile(on, m.sq, games.len()),
            });
        }
        let tiles = tiles.child(row.driver());
        let home = div().flex_none().relative().h(u(m.home)).child(tiles);

        // The shelves.
        let mut screen_col = div()
            .id(("xbox-screen", tab))
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(screen.handle())
            .child(fade(0, at, home.into_any_element()));
        let (sw, sh) = m.small;
        let step = sw + GAP * 0.7;
        for (g, shelf) in shelves.iter().enumerate() {
            let top = m.home + g as f32 * m.shelf;
            let in_sight = top + m.shelf > off_y && top < off_y + m.body;
            let first = (-f32::from(rows[g].handle().offset().x) / scale / step).floor().max(0.) as usize;
            let per_row = (view_w / step).ceil() as usize + 1;
            let mut line = div()
                .id(("xbox-shelf", g))
                .flex()
                .gap(u(GAP * 0.7))
                .p(u(PAD))
                .pl(u(SIDE))
                .pb(u(PAD + label_room()))
                .overflow_x_scroll()
                .track_scroll(rows[g].handle());
            for (c, key) in shelf.keys.iter().enumerate() {
                let load = in_sight && ((first..=first + per_row).contains(&c) || c.abs_diff(cols[g]) <= 2);
                // Portrait covers, so a game in the row above and in a shelf never share a tile's animation.
                line =
                    line.child(self.console_tile_shaped(key, TileShape::Portrait, sw, sh, group == Some(g) && c == cols[g], load, window));
            }
            let caption = div()
                .flex_none()
                .h(u(CAPTION))
                .px(u(SIDE))
                .flex()
                .items_end()
                .text_size(u(17.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(format!("{}  \u{b7}  {}", shelf.title, shelf.keys.len()));
            let section = div().flex_none().h(u(m.shelf)).flex().flex_col().child(caption).child(line.child(rows[g].driver()));
            screen_col = screen_col.child(fade(g + 1, at, section.into_any_element()));
        }
        // Room for the last shelf to rise to the row's place.
        let screen_col = screen_col.child(div().flex_none().h(u(m.body))).child(screen.driver());

        // Name and facts above the row; they stay while the screen glides.
        let info = self.xbox_info(focused.as_ref(), mine, games.len(), window);
        let info = div()
            .absolute()
            .left(u(SIDE))
            .right(u(SIDE))
            .top_0()
            .h(u(m.row_top - CAPTION - PAD - 4.))
            .flex()
            .flex_col()
            .justify_end()
            .child(info);
        div().relative().size_full().child(screen_col).child(info).into_any_element()
    }

    /// The last tile of the row: the way into the whole library.
    fn xbox_mine_tile(&self, on: bool, sq: f32, total: usize) -> Div {
        let content = div()
            .relative()
            .size_full()
            .rounded(u(look::tile_radius()))
            .bg(theme::white_a(0.12))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(u(8.))
            .child(crate::ui::icon(Icon::GridView, u(sq * 0.26), theme::text_primary()))
            .child(div().text_size(u(17.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("My games")))
            .child(div().text_size(u(12.)).text_color(theme::text_muted()).child(trn!(total, "{n} game", "{n} games")));
        div().flex_none().relative().w(u(sq)).h(u(sq)).child(focus_frame("xbox-mine", on, sq, sq, content))
    }

    /// The focused game's name (or logo) and a small line about it.
    fn xbox_info(&mut self, key: Option<&SharedString>, mine: bool, total: usize, window: &Window) -> AnyElement {
        let size = HeroSize { title: 44., logo: (440., 110.), ..HeroSize::default() };
        let (id, name, line) = if mine {
            (
                "xbox-info-mine".to_string(),
                kit::title(tr!("My games"), size.title).into_any_element(),
                Some(trn!(total, "{n} game", "{n} games")),
            )
        } else {
            let Some(g) = key.and_then(|k| self.lib.as_ref()?.get(k)).cloned() else { return div().into_any_element() };
            let line = look::theme().hero.facts.then(|| played_line(&g, true));
            (format!("xbox-info-{}", g.key), self.console_name(&g, size, window), line)
        };
        let info = div()
            .relative()
            .flex()
            .flex_col()
            .gap(u(10.))
            .child(name)
            .children(line.map(|l| div().text_size(u(17.)).text_color(theme::text_muted()).child(l)));
        // A new game slides in a little, never from blank.
        motion::enter_from(SharedString::from(id), info, 300, 14., 0., 0.25)
    }

    pub(super) fn xbox_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let games = self.console_games(&spec.source);
        if games.is_empty() {
            return false;
        }
        let recent: Vec<SharedString> = top_recent(&games, RECENT).into_iter().map(|g| SharedString::from(g.key.clone())).collect();
        let shelves = if look::theme().layouts.xbox_groups { self.console_shelves(&spec.source) } else { Vec::new() };
        let m = self.xbox_metrics(window);
        let wrap = look::theme().nav.wrap;
        let nav = &mut self.tab_nav(tab).xbox;
        nav.fit(shelves.len());
        nav.group = nav.group.filter(|g| *g < shelves.len());
        let slots = recent.len() + 1;
        match (pad, nav.group) {
            (Pad::Left | Pad::Right, None) => {
                let d = if pad == Pad::Left { -1 } else { 1 };
                nav.col = super::step(nav.col.min(slots - 1), slots, d, wrap);
                nav.row.go(Goal::Start(nav.col.saturating_sub(1)));
            }
            (Pad::Left | Pad::Right, Some(g)) => {
                let d = if pad == Pad::Left { -1 } else { 1 };
                let n = shelves[g].keys.len();
                nav.cols[g] = super::step(nav.cols[g].min(n - 1), n, d, wrap);
                nav.rows[g].go(Goal::Start(nav.cols[g].saturating_sub(1)));
            }
            (Pad::Down, group) if group.map_or(0, |g| g + 1) < shelves.len() => {
                let g = group.map_or(0, |g| g + 1);
                nav.group = Some(g);
                nav.screen.go(Goal::Offset(f32::from(u(m.offset(g)))));
            }
            (Pad::Up, Some(g)) => {
                nav.group = g.checked_sub(1);
                nav.screen.go(Goal::Offset(f32::from(u(nav.group.map_or(0., |g| m.offset(g))))));
            }
            (Pad::A | Pad::X, None) if nav.col >= recent.len() => {
                if pad == Pad::A {
                    self.console_open_library(window, cx);
                }
            }
            (Pad::A | Pad::X, group) => {
                let key = match group {
                    Some(g) => shelves[g].keys.get(nav.cols[g]),
                    None => recent.get(nav.col),
                };
                if let Some(key) = key.cloned() {
                    self.console_activate(key, pad, window, cx);
                }
            }
            (Pad::Y, _) => self.console_search(window, cx),
            // Up from the row, Down from the last shelf: nowhere to go.
            (Pad::Up | Pad::Down, _) => {}
            _ => return false,
        }
        true
    }

    pub(super) fn xbox_hints(&self, tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        if self.console.tab_navs.get(tab).is_some_and(|n| n.xbox.mine) {
            let mut h = vec![hint(Btn::A, tr!("Open")), hint(Btn::Y, tr!("Search"))];
            h.extend(self.console_tab_hints());
            return h;
        }
        self.console_cover_hints()
    }
}

/// Section `index` of the screen: bright at the focused one, dimmed below
/// it, gone above it (so the name over the row never meets a shelf).
fn fade(index: usize, focused: usize, content: AnyElement) -> AnyElement {
    motion::spring(("xbox-fade", index), div().flex_none(), motion::SOFT, index >= focused, move |d, visible| {
        d.opacity(visible.clamp(0., 1.)).child(motion::spring(
            ("xbox-dim", index),
            div(),
            motion::SOFT,
            index == focused,
            move |d, bright| d.opacity(0.55 + 0.45 * bright.clamp(0., 1.)).child(content),
        ))
    })
}
