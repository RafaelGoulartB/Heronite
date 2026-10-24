//! Shelves: the focused game large over its art (name or logo, facts,
//! overview) with the theme's shelves of covers below. ← → move along a
//! shelf, LT / RT a page along it, ↑ ↓ between shelves (the focused one
//! rises to the top). A shelf the theme's limit cut ends with a "View all"
//! tile that opens the Library on the same games.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{FactStyle, Size3, TabSpec};
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::hero::{shelf_heading, HeroSize};
use super::super::sources::Shelf;
use super::super::tile::{focus_frame, label_room};
use super::columns;
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::console::{hint, look, Btn, Hint};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

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

/// The tile at the end of a cut shelf: "View all" and how many games it has.
fn more_tile(id: impl Into<ElementId>, focused: bool, total: usize, w: f32, h: f32) -> Div {
    let radius = u(look::tile_radius());
    let (disc, mark) = if focused {
        (Hsla::from(theme::console_focus()), Hsla::from(theme::console_on_focus()))
    } else {
        (kit::ink(0.12), theme::text_primary().into())
    };
    let face = div()
        .size_full()
        .rounded(radius)
        .bg(linear_gradient(180., linear_color_stop(kit::ink(0.05), 0.), linear_color_stop(kit::ink(0.11), 1.)))
        .border_1()
        .border_color(kit::ink(0.1))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .px(u(10.))
        .child(div().size(u(58.)).rounded_full().bg(disc).flex().items_center().justify_center().child(icon(Icon::GridView, u(26.), mark)))
        .child(div().pt(u(4.)).text_size(u(16.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("View all")))
        .child(div().text_size(u(13.)).text_color(theme::text_muted()).child(trn!(total, "{n} game", "{n} games")));
    div().flex_none().relative().w(u(w)).h(u(h)).child(focus_frame(id, focused, w, h, face))
}

/// What the top shows while a "View all" tile has the focus: the shelf's
/// name and size, and what A does.
fn shelf_summary(shelf: &Shelf, si: usize) -> AnyElement {
    let block = div()
        .relative()
        .flex()
        .flex_col()
        .gap(u(10.))
        .max_w(u(760.))
        .child(kit::title(kit::heading_text(&shelf.title), 46.))
        .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(trn!(shelf.total, "{n} game", "{n} games")))
        .child(kit::body_text(tr!("Press A to see them all.")));
    motion::enter_from(SharedString::from(format!("hero-more-{si}")), block, 300, 14., 0., 0.25)
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
        let (focused, on_more) = nav.sync_more(&shelves);
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
            if shelf.cut() {
                let c = shelf.keys.len();
                row = row.child(more_tile(("shelf-more", i), i == si && cols[i] == c, shelf.total, tw, th));
            }
            row = row.child(rows[i].driver());
            let on = i == si;
            let dim = theme.dim_others;
            let (title, count) = (shelf.title.clone(), shelf.total);
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
        let hero =
            if on_more { shelf_summary(&shelves[si], si) } else { self.console_hero(Some(&focused), size, window).into_any_element() };
        // The numbers panel sits at the right edge beside the hero.
        let stats = (theme.hero.stats && !on_more).then(|| {
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

    /// A / X on a cover, or "View all" on a shelf's last tile; Y, the tabs.
    pub(super) fn shelves_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        let shelves = self.console_shelves(&spec.source);
        let on_more = self.console.tab_navs.get(tab).is_some_and(|n| {
            let nav = &n.shelves;
            let si = nav.shelf.min(shelves.len().saturating_sub(1));
            shelves.get(si).is_some_and(|s| nav.lines.cols.get(si).is_some_and(|&c| c.min(columns(s, true) - 1) >= s.keys.len()))
        });
        if !on_more {
            return self.console_cover_hints();
        }
        let mut h = vec![hint(Btn::A, tr!("View all")), hint(Btn::Y, tr!("Search"))];
        h.extend(self.console_tab_hints());
        h
    }
}
