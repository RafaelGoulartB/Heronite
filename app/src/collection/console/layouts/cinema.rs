//! Cinema: the focused game's art full screen with almost no text (its logo
//! or name and one line of facts at the bottom left) and ONE row of covers
//! of the theme's shape under it. ← → move along the row, ↑ ↓ change the
//! shelf (a vertical glide); the next shelf's title peeks under the row.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::TabSpec;
use heronite_data::CollectionGame;
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
/// Height of a shelf's title line.
const LABEL: f32 = 34.;
/// How much of the next shelf shows under the focused one.
const PEEK: f32 = 40.;

pub(super) const SCRIM: Scrim = Scrim::Cinema;

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

/// "2019 · Action · 3h 54m played" (the store when none of it is known).
fn meta_line(g: &CollectionGame) -> String {
    let year = g.details.release_date.as_deref().and_then(heronite_data::organize::release_ymd).map(|(y, ..)| y.to_string());
    let genre = g.details.genres.first().cloned();
    let played = (g.playtime_minutes > 0)
        .then(|| tr!("{time} played", time = heronite_data::collection::format_playtime_minutes(g.playtime_minutes)));
    let parts: Vec<String> = year.into_iter().chain(genre).chain(played).collect();
    if parts.is_empty() {
        g.source_label.to_string()
    } else {
        parts.join(" \u{b7} ")
    }
}

impl CollectionView {
    /// The logo (or name) and the one line under it.
    fn cinema_info(&mut self, g: &CollectionGame, window: &Window) -> AnyElement {
        let theme = look::theme();
        let size = HeroSize { title: 56., logo: (460., 120.), max_w: 640., overview: 0 };
        let name = self.console_name(g, size, window);
        let meta = theme.hero.facts.then(|| {
            div()
                .flex()
                .items_center()
                .gap(u(12.))
                .text_size(u(16.))
                .text_color(theme::text_muted())
                .child(meta_line(g))
                .children(self.console_status_badge(g))
        });
        let info = div().relative().flex().flex_col().gap(u(12.)).max_w(u(640.)).child(name).children(meta);
        motion::enter_from(SharedString::from(format!("cinema-{}", g.key)), info, 300, 14., 0., 0.25)
    }

    pub(super) fn render_cinema(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element();
        }
        let theme = look::theme();
        let (tw, th) = self.console_tile_size();
        let nav = &mut self.tab_nav(tab).cinema;
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

        let room = label_room();
        let block_h = LABEL + th + 2. * PAD + room;
        let step = f32::from(u(tw + GAP));
        let per_row = (f32::from(window.viewport_size().width) / step).ceil() as usize + 1;
        let mut stack = div().id(("console-cinema", tab)).flex().flex_col().overflow_y_scroll().track_scroll(shelves_glide.handle());
        for (i, shelf) in shelves.iter().enumerate() {
            let mut row = div()
                .id(("console-cinema-row", i))
                .flex()
                .flex_none()
                .gap(u(GAP))
                .p(u(PAD))
                .pb(u(PAD + room))
                .px(u(SIDE))
                .overflow_x_scroll()
                .track_scroll(rows[i].handle());
            // Only the focused shelf and its neighbours (which show while
            // the shelves glide) have covers; off screen ones are empty.
            if i.abs_diff(si) <= 1 {
                let first = (-f32::from(rows[i].handle().offset().x) / step).floor().max(0.) as usize;
                for (c, key) in shelf.keys.iter().enumerate() {
                    let visible = (first..=first + per_row).contains(&c) || c.abs_diff(cols[i]) <= 2;
                    row = row.child(self.console_tile(key, tw, th, i == si && c == cols[i], visible, window));
                }
            }
            row = row.child(rows[i].driver());
            let dim = theme.dim_others;
            let above = i < si;
            let title = shelf.title.clone();
            // The focused shelf is bright; the next ones step back, those
            // already passed fade out.
            stack = stack.child(motion::spring(
                ("cinema-shelf", i),
                div().flex().flex_col().flex_none().h(u(block_h)),
                motion::SOFT,
                i == si,
                move |d, t| {
                    let t = t.clamp(0., 1.);
                    let low = if above {
                        0.
                    } else if dim {
                        0.5
                    } else {
                        0.85
                    };
                    d.opacity(low + (1. - low) * t)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .h(u(LABEL))
                                .px(u(SIDE))
                                .text_size(u(18.))
                                .font_weight(theme::W_BOLD)
                                .text_color(motion::mix(theme::text_muted(), theme::text_primary(), t))
                                .child(title),
                        )
                        .child(row)
                },
            ));
        }
        // Room after the last shelf so it can rise to the top of the area.
        stack = stack.child(div().flex_none().h(u(PEEK))).child(shelves_glide.driver());

        let info = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned().map(|g| self.cinema_info(&g, window));
        div()
            .flex()
            .flex_col()
            .justify_end()
            .size_full()
            .child(div().flex_none().px(u(SIDE)).pb(u(10.)).children(info))
            .child(stack.flex_none().h(u(block_h + PEEK)))
            .into_any_element()
    }

    pub(super) fn cinema_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return false;
        }
        let (tw, _) = self.console_tile_size();
        let wrap = look::theme().nav.wrap;
        let nav = &mut self.tab_nav(tab).cinema;
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
        let si = nav.shelf;
        nav.shelves.go(Goal::Start(si));
        let ci = nav.cols[si].min(shelves[si].keys.len() - 1);
        nav.rows[si].go(Goal::Reveal { ix: ci, margin: tw * 0.6 });
        true
    }

    pub(super) fn cinema_hints(&self, _tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        self.console_cover_hints()
    }
}
