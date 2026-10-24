//! Shelves: the focused game large over its art (name or logo, facts,
//! overview) with the theme's shelves of covers below. ← → move along a
//! shelf, ↑ ↓ between shelves (the focused one rises to the top).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{FactStyle, Size3, TabSpec};

use super::super::super::CollectionView;
use super::super::hero::{shelf_heading, HeroSize};
use super::super::tile::label_room;
use crate::console::kit::u;
use crate::console::look;
use crate::console::motion;
use crate::theme;

const GAP: f32 = 18.;
const SIDE: f32 = 46.;
/// Room around a row of covers for the lifted one and its ring.
const PAD: f32 = 10.;

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
        let shelves = self.console_shelves(&spec.source);
        if shelves.is_empty() {
            return super::no_games();
        }
        let theme = look::theme();
        let (tw, th) = self.console_tile_size();
        let nav = &mut self.tab_nav(tab).shelves;
        let focused = nav.sync(&shelves);
        let (si, rows, shelves_glide, cols) = (nav.shelf, nav.lines.glides.clone(), nav.shelves.clone(), nav.lines.cols.clone());
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
            let seen = super::in_sight(rows[i].handle().offset().x, tw + GAP, window.viewport_size().width);
            for (c, key) in shelf.keys.iter().enumerate() {
                let visible = near && super::loads(&seen, c, cols[i]);
                row = row.child(self.console_tile(key, tw, th, i == si && c == cols[i], visible, window));
            }
            row = row.child(rows[i].driver());
            let on = i == si;
            let dim = theme.dim_others;
            let (title, count) = (shelf.title.clone(), shelf.keys.len());
            // The focused shelf is bright; the others step back.
            list = list.child(motion::spring(("shelf-fx", i), div().flex().flex_col(), motion::SOFT, on, move |d, t| {
                let t = if dim { t.clamp(0., 1.) } else { 1. };
                d.opacity(0.5 + 0.5 * t)
                    .child(div().px(u(PAD)).text_color(motion::mix(theme::text_muted(), theme::text_primary(), t)).child(shelf_heading(
                        &title,
                        Some(count),
                        18.,
                    )))
                    .child(row)
            }));
        }
        list = list.child(shelves_glide.driver());
        // Chips and genre tags take another line.
        let chips = theme.hero.facts && theme.hero.fact_style == FactStyle::Chips;
        let hero_h = hero_height(theme.hero.size) + if chips { 40. } else { 0. };
        let size = HeroSize {
            title: if theme.hero.size == Size3::Small { 36. } else { 46. },
            logo: (440., if theme.hero.size == Size3::Large { 150. } else { 110. }),
            overview: if theme.hero.size == Size3::Small { 0 } else { 300 },
            ..HeroSize::default()
        };
        let hero = self.console_hero(Some(&focused), size, window);
        // The numbers panel sits at the right edge beside the hero.
        let stats = theme.hero.stats.then(|| {
            let g = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned()?;
            let lines = (((hero_h - 52.) / 30.).floor() as usize).clamp(2, 6);
            Some(div().absolute().top(u(14.)).right_0().child(self.console_stats(&g, lines)))
        });
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex_none().h(u(hero_h)).px(u(SIDE)).pt(u(20.)).overflow_hidden().child(hero))
            .child(list.flex_1().min_h_0())
            .children(stats.flatten())
            .into_any_element()
    }
}
