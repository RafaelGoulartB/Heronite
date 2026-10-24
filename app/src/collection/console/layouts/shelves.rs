//! Shelves: the focused game large over its art (name or logo, facts,
//! overview) with the theme's shelves of covers below. ← → move along a
//! shelf, ↑ ↓ between shelves (the focused one rises to the top).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Size3, TabSpec};
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use super::super::hero::HeroSize;
use super::super::tile::label_room;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{Hint, Pad};
use crate::icons::Icon;
use crate::theme;

const GAP: f32 = 18.;
const SIDE: f32 = 46.;
/// Room around a row of covers for the lifted one and its ring.
const PAD: f32 = 10.;

pub(super) const SCRIM: Scrim = Scrim::Classic;

pub(crate) struct Nav {
    pub shelf: usize,
    /// Focused column of each shelf (kept when moving between shelves).
    cols: Vec<usize>,
    shelves: Glide,
    rows: Vec<Glide>,
}

impl Default for Nav {
    fn default() -> Self {
        Self { shelf: 0, cols: Vec::new(), shelves: Glide::new(Axis::Y), rows: Vec::new() }
    }
}

impl Nav {
    fn fit(&mut self, n: usize) {
        self.cols.resize(n, 0);
        while self.rows.len() < n {
            self.rows.push(Glide::new(Axis::X));
        }
    }
}

/// Height of the game shown on top, design px.
fn hero_height(size: Size3) -> f32 {
    match size {
        Size3::Small => 170.,
        Size3::Medium => 250.,
        Size3::Large => 320.,
    }
}

impl CollectionView {
    pub(super) fn render_shelves(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element();
        }
        let theme = look::theme();
        let (tw, th) = self.console_tile_size();
        let nav = &mut self.tab_nav(tab).shelves;
        nav.shelf = nav.shelf.min(shelves.len() - 1);
        nav.fit(shelves.len());
        for (i, s) in shelves.iter().enumerate() {
            nav.cols[i] = nav.cols[i].min(s.keys.len() - 1);
        }
        let (si, ci) = (nav.shelf, nav.cols[nav.shelf]);
        let focused = shelves[si].keys[ci].clone();
        let rows = nav.rows.clone();
        let shelves_glide = nav.shelves.clone();
        let cols = nav.cols.clone();
        self.console_focus_game(Some(focused.clone()), cx);

        let mut list = div()
            .id(("console-shelves", tab))
            .flex()
            .flex_col()
            .gap(u(10.))
            .overflow_y_scroll()
            .track_scroll(shelves_glide.handle())
            .px(u(SIDE - PAD))
            .pb(u(20.));
        let step = f32::from(u(tw + GAP));
        let per_row = (f32::from(window.viewport_size().width) / step).ceil() as usize + 1;
        let room = label_room();
        for (i, shelf) in shelves.iter().enumerate() {
            let mut row = div()
                .id(("console-shelf", i))
                .flex()
                .gap(u(GAP))
                .p(u(PAD))
                .pb(u(PAD + room))
                .overflow_x_scroll()
                .track_scroll(rows[i].handle());
            // On screen: the shelves around the focused one, the columns
            // each is scrolled to (and the focus, while it glides there).
            let near = i + 1 >= si && i <= si + 2;
            let first = (-f32::from(rows[i].handle().offset().x) / step).floor().max(0.) as usize;
            for (c, key) in shelf.keys.iter().enumerate() {
                let visible = near && ((first..=first + per_row).contains(&c) || c.abs_diff(cols[i]) <= 2);
                row = row.child(self.console_tile(key, tw, th, i == si && c == cols[i], visible, window));
            }
            row = row.child(rows[i].driver());
            let on = i == si;
            let dim = theme.dim_others;
            let label = format!("{}  \u{b7}  {}", shelf.title, shelf.keys.len());
            // The focused shelf is bright; the others step back.
            list = list.child(motion::spring(("shelf-fx", i), div().flex().flex_col(), motion::SOFT, on, move |d, t| {
                let t = if dim { t.clamp(0., 1.) } else { 1. };
                d.opacity(0.5 + 0.5 * t)
                    .child(
                        div()
                            .px(u(PAD))
                            .text_size(u(18.))
                            .font_weight(theme::W_BOLD)
                            .text_color(motion::mix(theme::text_muted(), theme::text_primary(), t))
                            .child(label),
                    )
                    .child(row)
            }));
        }
        list = list.child(shelves_glide.driver());
        let hero_h = hero_height(theme.hero.size);
        let size = HeroSize {
            title: if theme.hero.size == Size3::Small { 36. } else { 46. },
            logo: (440., if theme.hero.size == Size3::Large { 150. } else { 110. }),
            overview: if theme.hero.size == Size3::Small { 0 } else { 300 },
            ..HeroSize::default()
        };
        let hero = self.console_hero(Some(&focused), size, window);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex_none().h(u(hero_h)).px(u(SIDE)).pt(u(20.)).overflow_hidden().child(hero))
            .child(list.flex_1().min_h_0())
            .into_any_element()
    }

    pub(super) fn shelves_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return false;
        }
        let (tw, _) = self.console_tile_size();
        let wrap = look::theme().nav.wrap;
        let nav = &mut self.tab_nav(tab).shelves;
        nav.fit(shelves.len());
        let si = nav.shelf.min(shelves.len() - 1);
        let n = shelves[si].keys.len();
        let ci = nav.cols[si].min(n - 1);
        let key = shelves[si].keys[ci].clone();
        match pad {
            Pad::Left => nav.cols[si] = super::step(ci, n, -1, wrap),
            Pad::Right => nav.cols[si] = super::step(ci, n, 1, wrap),
            Pad::Up => nav.shelf = si.saturating_sub(1),
            Pad::Down => nav.shelf = (si + 1).min(shelves.len() - 1),
            Pad::A | Pad::X => {
                self.console_activate(key, pad, window, cx);
                return true;
            }
            Pad::Y => {
                self.console_search(window, cx);
                return true;
            }
            _ => return false,
        }
        // The focused shelf to the top, its cover in sight with a peek of
        // the next ones.
        let si = nav.shelf;
        nav.shelves.go(Goal::Start(si));
        let ci = nav.cols[si].min(shelves[si].keys.len() - 1);
        nav.rows[si].go(Goal::Reveal { ix: ci, margin: tw * 0.6 });
        true
    }

    pub(super) fn shelves_hints(&self, _tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        self.console_cover_hints()
    }
}
