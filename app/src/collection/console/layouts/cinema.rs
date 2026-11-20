//! Cinema: the focused game's art full screen with its logo (or name) large
//! and its facts, the first shelf of covers under it and the next one's
//! title peeking in. ← → move along a shelf (LT / RT a page), ↓ takes the
//! whole view down through the shelves: the game's large block scrolls away,
//! its logo stays small at the top, the art darkens, the shelf with the
//! focus sits right under it and the next ones follow, dimmer; ↑ back to the
//! first shelf brings the large block back. A shelf the theme's limit cut
//! ends with a "View all" tile that opens the Library on the same games.
//! A new game's logo holds, then fades in sliding from the side; its facts
//! follow (`motion::reveal`); the same game never replays them.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{FactStyle, HeroAlign, TabSpec};
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::hero::{played_time, shelf_heading, HeroMotion, HeroSize};
use super::super::tile::label_room;
use super::shelves::{more_tile, shelf_summary};
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Goal};
use crate::console::pointer::Clickable;
use crate::theme;

const GAP: f32 = 18.;
const SIDE: f32 = 46.;
/// Room around a row of covers for the lifted one and its ring.
const PAD: f32 = 10.;
/// Height of a shelf's title line.
const LABEL: f32 = 34.;
/// How much of the next shelf shows under the first one.
const PEEK: f32 = 46.;
/// Height of the line of genres above the covers (Chips).
const GENRES: f32 = 36.;
/// The game's small block on top while the view is down in the shelves:
/// its height, and the box of its logo.
const COMPACT: f32 = 132.;
const COMPACT_LOGO: (f32, f32) = (300., 70.);
/// How far a new game's logo slides in (design px).
const LOGO_SLIDE: f32 = 30.;

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

/// Where the view sits with shelf `si` focused, design px: at the top on the
/// first shelf (the game's large block over it); further down, the shelf
/// right under the game's small block.
fn view_offset(si: usize, hero_h: f32, block_h: f32) -> f32 {
    if si == 0 {
        0.
    } else {
        hero_h + si as f32 * block_h - COMPACT
    }
}

/// The shade over the art while the view is down in the shelves: they fill
/// the screen, so the art steps back behind them.
fn deep_shade() -> Div {
    // Clear at the very top, where it meets the top bar over the bare art.
    div().absolute().inset_0().bg(linear_gradient(180., linear_color_stop(kit::veil(0.), 0.), linear_color_stop(kit::veil(0.74), 0.3)))
}

impl CollectionView {
    /// The logo (or name) large and the facts under it: one line, or chips.
    /// `free` is the height the block may take, design px; `dx` where the
    /// logo slides from.
    fn cinema_info(&mut self, tab: usize, g: &CollectionGame, free: f32, dx: f32, window: &Window) -> AnyElement {
        let theme = look::theme();
        let centre = theme.hero.align == HeroAlign::Center;
        // A big logo (about a third of the width) as far as the room allows.
        let size = HeroSize { title: 56., logo: (560., (free - 70.).clamp(100., 200.)), max_w: 760., overview: 0 };
        let name = self.console_name(g, size, window);
        let meta = theme.hero.facts.then(|| match theme.hero.fact_style {
            FactStyle::Line => self.console_fact_line(g, meta_line(g), 16.),
            FactStyle::Chips => self.console_chips(g),
        });
        let id = |part: &str| SharedString::from(format!("cinema-{part}-{tab}-{}", g.key));
        let name = motion::reveal(id("logo"), div().relative().child(name), motion::NAME_HOLD, motion::NAME_FADE, dx, 0.);
        let meta = meta.map(|m| {
            let m = if centre { m.justify_center() } else { m };
            motion::reveal(id("facts"), div().relative().child(m), motion::REST_HOLD, motion::REST_FADE, 0., 0.)
        });
        let info = div().relative().flex().flex_col().gap(u(14.)).max_w(u(size.max_w)).child(name).children(meta);
        if centre {
            info.items_center().text_center().into_any_element()
        } else {
            info.into_any_element()
        }
    }

    /// The game's small block on top while the view is down: the logo (or
    /// name) small and one line of facts.
    fn cinema_compact(&mut self, tab: usize, g: &CollectionGame, window: &Window) -> AnyElement {
        let size = HeroSize { title: 30., logo: COMPACT_LOGO, max_w: 900., overview: 0 };
        let name = self.console_name(g, size, window);
        let id = |part: &str| SharedString::from(format!("cinema-small-{part}-{tab}-{}", g.key));
        div()
            .flex()
            .flex_col()
            .gap(u(8.))
            .child(motion::reveal(id("logo"), div().relative().child(name), motion::NAME_HOLD, motion::NAME_FADE, 0., 0.))
            .child(motion::reveal(
                id("facts"),
                div().relative().text_size(u(15.)).text_color(theme::text_muted()).child(meta_line(g)),
                motion::REST_HOLD,
                motion::REST_FADE,
                0.,
                0.,
            ))
            .into_any_element()
    }

    pub(super) fn render_cinema(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return super::no_games();
        }
        let theme = look::theme();
        let (tw, th) = self.console_tile_size();
        let room = label_room();
        let block_h = LABEL + th + 2. * PAD + room;
        // The hint bar floats over the bottom: the view has the whole body.
        let body_h = super::body_height(window, false);
        let hero_h = (body_h - block_h - PEEK).max(180.);

        let nav = &mut self.tab_nav(tab).cinema;
        let (focused, on_more) = nav.sync_more(&shelves);
        let si = nav.shelf;
        // The view goes where the focused shelf wants it (once per change).
        let goal = view_offset(si, hero_h, block_h);
        if nav.goal != Some(goal) {
            nav.shelves.go(Goal::Offset(goal * kit::scale()));
            nav.goal = Some(goal);
        }
        let (rows, glide, cols) = (nav.lines.glides.clone(), nav.shelves.clone(), nav.lines.cols.clone());
        let how = HeroMotion { slide: (14. * nav.from.0, 10. * nav.from.1), replay: nav.moves, ..HeroMotion::default() };
        let dx = if nav.from.0 != 0. { LOGO_SLIDE * nav.from.0 } else { -LOGO_SLIDE };
        let deep = si > 0;
        self.console_focus_game(Some(focused.clone()), cx);
        let game = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned();

        // The game's large block, at the bottom of the room over the first shelf.
        let chips = theme.hero.facts && theme.hero.fact_style == FactStyle::Chips;
        let genres_h = if chips { GENRES } else { 0. };
        let free = hero_h - genres_h - 20.;
        let info = if on_more && !deep {
            Some(shelf_summary(&shelves[si], si, how))
        } else {
            game.as_ref().map(|g| self.cinema_info(tab, g, free, dx, window))
        };
        let centre = theme.hero.align == HeroAlign::Center;
        let mut hero = div().relative().flex().flex_col().flex_none().h(u(hero_h)).px(u(SIDE)).pb(u(14.));
        hero = if centre { hero.pt(u(10.)).items_center() } else { hero.justify_end() };
        hero = hero.children(info);
        let genres = game.as_ref().filter(|_| chips).and_then(|g| Self::console_genre_line(g)).map(|line| {
            let id = SharedString::from(format!("cinema-genres-{tab}-{focused}"));
            motion::reveal(id, div().relative().child(line), motion::REST_HOLD, motion::REST_FADE, 0., 0.)
        });
        if let Some(g) = genres {
            hero = hero.child(div().absolute().right(u(SIDE)).bottom(u(12.)).h(u(GENRES)).flex().items_center().child(g));
        }
        if theme.hero.stats {
            // At the right edge, centred in the room above the covers.
            let lines = (((free - 30.) / 30.).floor() as usize).clamp(2, 6);
            let panel = game.as_ref().map(|g| self.console_stats(g, lines));
            hero = hero.child(div().absolute().top_0().bottom(u(genres_h)).right(u(SIDE)).flex().items_center().children(panel));
        }

        // The shelves under it, in the same view.
        let mut view =
            div().id(("console-cinema", tab)).flex().flex_col().size_full().overflow_y_scroll().track_scroll(glide.handle()).child(hero);
        let width = crate::ui::content_size(window).width;
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
            // Covers only on the shelves in sight (and the one just passed,
            // while the view glides); the others are empty.
            if i + 1 >= si && i <= si + 3 {
                let seen = super::in_sight(rows[i].handle().offset().x, tw + GAP, width);
                for (c, key) in shelf.keys.iter().enumerate() {
                    let visible = super::loads(&seen, c, cols[i]);
                    let tile = self.console_tile(key, tw, th, i == si && c == cols[i], visible, window);
                    row = row.child(tile.on_pads(super::shelf_click(si, &cols, i, c)));
                }
                if shelf.cut() {
                    let c = shelf.keys.len();
                    let tile = more_tile(("cinema-more", i), i == si && cols[i] == c, shelf.total, tw, th);
                    row = row.child(tile.on_pads(super::shelf_click(si, &cols, i, c)));
                }
            }
            row = row.child(rows[i].driver());
            let dim = theme.dim_others;
            let above = i < si;
            let (title, count) = (shelf.title.clone(), shelf.total);
            // The focused shelf is bright; the next ones step back, those
            // passed fade out under the game's small block.
            view = view.child(motion::spring(
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
                                .child(shelf_heading(&title, Some(count), 18.)),
                        )
                        .child(row)
                },
            ));
        }
        // Room after the last shelf so it can rise under the small block.
        let tail = (body_h - COMPACT - block_h).max(PEEK);
        view = view.child(div().flex_none().h(u(tail))).child(glide.driver());

        // Down in the shelves: the art darkens and the game's small block shows on top.
        let compact = if on_more {
            let s = &shelves[si];
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(6.))
                    .child(kit::title(kit::heading_text(&s.title), 30.))
                    .child(div().text_size(u(15.)).text_color(theme::text_muted()).child(trn!(s.total, "{n} game", "{n} games")))
                    .into_any_element(),
            )
        } else {
            game.as_ref().map(|g| self.cinema_compact(tab, g, window))
        };
        let shade = motion::spring(("cinema-deep-shade", tab), deep_shade(), motion::SOFT, deep, |d, k| d.opacity(k.clamp(0., 1.)));
        let top = div().absolute().top_0().left_0().right_0().h(u(COMPACT)).px(u(SIDE)).flex().items_center().children(compact);
        let top = motion::spring(("cinema-deep-top", tab), top, motion::SOFT, deep, |d, k| d.opacity(k.clamp(0., 1.)));
        div().relative().size_full().child(shade).child(view).child(top).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn the_view_starts_at_the_top_and_puts_a_deeper_shelf_under_the_small_block() {
        assert_eq!(view_offset(0, 400., 200.), 0.);
        // Shelf 1 starts at 400 + 200 in the view: it rises to just under the block.
        assert_eq!(view_offset(1, 400., 200.), 600. - COMPACT);
        assert_eq!(view_offset(3, 400., 200.) - view_offset(2, 400., 200.), 200.);
    }
}
