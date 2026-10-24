//! Cinema: the focused game's art full screen with almost no text (its logo
//! or name and one line of facts at the bottom left) and ONE row of covers
//! of the theme's shape under it. ← → move along the row, ↑ ↓ change the
//! shelf (a vertical glide); the next shelf's title peeks under the row.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{FactStyle, HeroAlign, TabSpec};
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::hero::{played_time, shelf_heading, HeroSize};
use super::super::tile::label_room;
use crate::console::kit::u;
use crate::console::look;
use crate::console::motion;
use crate::theme;

const GAP: f32 = 18.;
const SIDE: f32 = 46.;
/// Room around a row of covers for the lifted one and its ring.
const PAD: f32 = 10.;
/// Height of a shelf's title line.
const LABEL: f32 = 34.;
/// How much of the next shelf shows under the focused one.
const PEEK: f32 = 40.;
/// Height of the line of genres above the covers (Chips).
const GENRES: f32 = 36.;

/// "2019 · Action · 3h 54m played" (the store when none of it is known).
fn meta_line(g: &CollectionGame) -> String {
    let year = release_year(g).map(|y| y.to_string());
    let genre = g.details.genres.first().cloned();
    let played = played_time(g).map(|time| tr!("{time} played", time = time));
    let parts: Vec<String> = year.into_iter().chain(genre).chain(played).collect();
    if parts.is_empty() {
        g.source_label.to_string()
    } else {
        parts.join(" \u{b7} ")
    }
}

impl CollectionView {
    /// The logo (or name) and the facts under it: one line, or chips.
    /// `free` is the height the block may take, design px.
    fn cinema_info(&mut self, g: &CollectionGame, free: f32, window: &Window) -> AnyElement {
        let theme = look::theme();
        let centre = theme.hero.align == HeroAlign::Center;
        // A big logo (about a third of the width) as far as the room allows.
        let size = HeroSize { title: 56., logo: (560., (free - 62.).clamp(100., 200.)), max_w: 760., overview: 0 };
        let name = self.console_name(g, size, window);
        let meta = theme.hero.facts.then(|| match theme.hero.fact_style {
            FactStyle::Line => self.console_fact_line(g, meta_line(g), 16.),
            FactStyle::Chips => self.console_chips(g),
        });
        let mut info = div().relative().flex().flex_col().gap(u(12.)).max_w(u(size.max_w)).child(name);
        if centre {
            info = info.items_center().text_center();
            // The facts centre under the name.
            info = info.children(meta.map(|m| m.justify_center()));
        } else {
            info = info.children(meta);
        }
        motion::enter_from(SharedString::from(format!("cinema-{}", g.key)), info, 300, 14., 0., 0.25)
    }

    pub(super) fn render_cinema(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return super::no_games();
        }
        let theme = look::theme();
        let (tw, th) = self.console_tile_size();
        let nav = &mut self.tab_nav(tab).cinema;
        let focused = nav.sync(&shelves);
        let (si, rows, shelves_glide, cols) = (nav.shelf, nav.lines.glides.clone(), nav.shelves.clone(), nav.lines.cols.clone());
        self.console_focus_game(Some(focused.clone()), cx);

        let room = label_room();
        let block_h = LABEL + th + 2. * PAD + room;
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
                let seen = super::in_sight(rows[i].handle().offset().x, tw + GAP, window.viewport_size().width);
                for (c, key) in shelf.keys.iter().enumerate() {
                    let visible = super::loads(&seen, c, cols[i]);
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
                                .text_color(motion::mix(theme::text_muted(), theme::text_primary(), t))
                                .child(shelf_heading(&title, None, 18.)),
                        )
                        .child(row)
                },
            ));
        }
        // Room after the last shelf so it can rise to the top of the area.
        stack = stack.child(div().flex_none().h(u(PEEK))).child(shelves_glide.driver());

        // What the game takes above the genres and the covers, and the
        // genres of the Chips style, right above the covers.
        let chips = theme.hero.facts && theme.hero.fact_style == FactStyle::Chips;
        let genres_h = if chips { GENRES } else { 0. };
        let free = (super::body_height(window, true) - block_h - PEEK - genres_h - 10.).max(120.);
        let game = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned();
        let info = game.as_ref().map(|g| self.cinema_info(g, free, window));
        let genres = game.as_ref().filter(|_| chips).and_then(|g| Self::console_genre_line(g)).map(|line| {
            let line = div().relative().child(line);
            motion::enter_from(SharedString::from(format!("cinema-genres-{focused}")), line, 300, 0., 8., 0.25)
        });
        let centre = theme.hero.align == HeroAlign::Center;
        let mut top = div().relative().flex().flex_col().flex_1().min_h_0().px(u(SIDE)).pb(u(10.));
        top = if centre { top.pt(u(6.)).items_center() } else { top.justify_end() };
        top = top.children(info);
        if theme.hero.stats {
            // At the right edge, centred in the room above the covers.
            let lines = (((free - 30.) / 30.).floor() as usize).clamp(2, 6);
            let panel = game.as_ref().map(|g| self.console_stats(g, lines));
            top = top.child(div().absolute().top_0().bottom_0().right_0().flex().items_center().children(panel));
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(top)
            .children(genres.map(|g| div().flex_none().h(u(GENRES)).px(u(SIDE)).flex().items_center().justify_end().child(g)))
            .child(stack.flex_none().h(u(block_h + PEEK)))
            .into_any_element()
    }
}
