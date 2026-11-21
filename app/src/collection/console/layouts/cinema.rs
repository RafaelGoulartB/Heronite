//! Cinema: the focused game's key art full bleed, its logo (or a quiet name
//! line) with its facts and genres at the left over its own shade, its
//! achievements on the right, and one row of large covers along the bottom.
//! ← → move along the row (LT / RT a page): while the focus travels the
//! game's block steps aside so the art shows whole, and it comes back once
//! the focus rests (`motion::reveal`). Under the row, the theme's shelves
//! over the tab's games (Continue playing, Favourites, a shelf per status...,
//! less the one the row already is), smaller and quieter, the first one's
//! title peeking in: ↓ goes down to them (the view glides so the shelf sits
//! near the top, the block goes, the art steps back), ↑ ↓ between them, ← →
//! along one, a cut one ending with "View all"; ↑ from the first comes back
//! to the row and its block. A and X follow `nav.aButton`, Y the Filters
//! panel, as on every screen.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::TabSpec;
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::hero::{logo_element, played_time};
use super::super::sources::{by_last_played, by_title, keys, Shelf};
use crate::collection::data::played_ago;
use crate::console::kit::{self, u};
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::pointer::Clickable;
use crate::console::{hint, look, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

/// Space between covers, and room above them for the focused one's lift and ring.
const GAP: f32 = 18.;
const RING_ROOM: f32 = 22.;
/// Room under the row for the hint bar that floats over the bottom.
const HINTS_ROOM: f32 = 70.;
/// How much of the first shelf (its title) shows under the main row.
const PEEK: f32 = 46.;
/// A shelf's title line, the room around its covers for the ring, the space
/// between them, and where a focused shelf comes up to under the top bar.
const LABEL_H: f32 = 40.;
const SHELF_RING: f32 = 14.;
const SHELF_GAP: f32 = 14.;
const SHELF_TOP: f32 = 24.;
/// How far the shelves' shade takes to come in under the top bar, design px.
const DEEP_FADE: f32 = 120.;
/// Between the game's block and the row.
const BLOCK_GAP: f32 = 46.;
/// The logo's box.
pub(super) const LOGO: (f32, f32) = (720., 210.);
/// Between the logo and its facts.
const LOGO_GAP: f32 = 8.;
/// Heights of the facts and genres lines.
const FACTS_H: f32 = 44.;
const GENRES_H: f32 = 24.;
/// The achievements block's width.
const ACH_W: f32 = 250.;
/// How long the focus must rest before the game's block comes back.
const SETTLE: Duration = Duration::from_millis(400);
/// How far a returning logo slides in, design px.
const LOGO_SLIDE: f32 = -30.;

/// The focus of a Cinema tab.
pub(crate) struct Nav {
    index: usize,
    row: Glide,
    /// The focus moved; the game's block stays away until then.
    moving_until: Option<Instant>,
    /// Counts the times the block came back (a new value replays its entrance).
    returns: u64,
    hidden: bool,
    wake: Option<Task<()>>,
    /// 0: the main row; else the shelf `level - 1` under it.
    level: usize,
    /// The whole view, up and down; each shelf's row and focused column.
    view: Glide,
    glides: Vec<Glide>,
    cols: Vec<usize>,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            index: 0,
            row: Glide::new(Axis::X),
            moving_until: None,
            returns: 0,
            hidden: false,
            wake: None,
            level: 0,
            view: Glide::new(Axis::Y),
            glides: Vec::new(),
            cols: Vec::new(),
        }
    }
}

impl Nav {
    /// Room for `shelves`, the columns inside them ("View all" one past a cut one).
    fn fit(&mut self, shelves: &[Shelf]) {
        self.cols.resize(shelves.len(), 0);
        while self.glides.len() < shelves.len() {
            self.glides.push(Glide::new(Axis::X));
        }
        for (c, s) in self.cols.iter_mut().zip(shelves) {
            *c = (*c).min(s.keys.len() + usize::from(s.cut()) - 1);
        }
        self.level = self.level.min(shelves.len());
    }
}

/// The left edge of the row and of the game's block: a share of the width,
/// within reason.
pub(super) fn side(vw: f32) -> f32 {
    (vw * 0.07).clamp(46., 150.)
}

/// The covers' height in a body `body_h` tall: about a third of the screen.
fn cover_h(body_h: f32) -> f32 {
    (body_h * 0.36).clamp(170., 290.)
}

/// Where the row of covers `tw` wide (`GAP` apart, from `side` in a view
/// `view` wide, now at `current`) scrolls for cover `i` to show whole with
/// a little more than half a cover beside it: as little as it takes.
fn row_offset(i: usize, tw: f32, side: f32, view: f32, current: f32) -> f32 {
    row_offset_with(i, tw, GAP, side, view, current)
}

/// One fact: a small label over its value, centred.
fn fact(label: &str, value: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(u(3.))
        .child(label_caps(label))
        .child(value.h(u(22.)).flex().items_center().gap(u(7.)).text_size(u(16.)).font_weight(theme::W_BOLD).text_color(kit::strong()))
}

/// A small label in spaced capitals (the facts', the sections' headings).
pub(super) fn label_caps(label: &str) -> Div {
    div().text_size(u(9.5)).font_weight(theme::W_BOLD).text_color(kit::ink(0.72)).whitespace_nowrap().child(kit::spaced_caps(label))
}

/// A thin rule of champagne between two facts.
fn rule() -> Div {
    let gold: Hsla = theme::console_focus().into();
    div().flex_none().w(px(1.)).h(u(30.)).mx(u(20.)).bg(gold.opacity(0.35))
}

/// A soft shade with no edges behind the game's block, so it reads over
/// bright art without darkening the rest (a blurred shadow of the
/// background colour spreading from a line with no height).
pub(super) fn block_shade(w: f32, h: f32) -> Div {
    let mut c = theme::console_bg();
    c.a = if kit::light() { 0.6 } else { 0.62 };
    div().absolute().left(u(w * 0.15)).right(u(w * 0.15)).top(u(h / 2.)).h(px(0.)).shadow(vec![BoxShadow {
        color: c.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: u(90.),
        spread_radius: u(h * 0.38),
        inset: false,
    }])
}

/// The row's games: the most recently played first (never played ones by
/// title); "Not played yet" by title.
fn order(source: &str, games: &[std::sync::Arc<CollectionGame>]) -> Vec<SharedString> {
    let all: Vec<&CollectionGame> = games.iter().map(|g| g.as_ref()).collect();
    if source.starts_with("status:not-played") {
        keys(by_title(all))
    } else {
        keys(by_last_played(all))
    }
}

impl CollectionView {
    fn cinema_keys(&self, spec: &TabSpec) -> Vec<SharedString> {
        order(&spec.source, &self.console_games(&spec.source))
    }

    /// Playtime, last played, where it plays (the store's icon and name),
    /// when it came out and how long the main story takes, between thin rules.
    fn cinema_facts(&self, g: &CollectionGame) -> Div {
        let dash = || "\u{2014}".to_string();
        let played = played_time(g).unwrap_or_else(dash);
        let last = g.last_played.as_deref().and_then(played_ago).unwrap_or_else(|| tr!("Never").to_string());
        let icon = super::super::super::store_badges::game_icons(&self.with_copies(&g.key)).first().copied();
        let platform = div().children(icon.map(|i| crate::ui::icon(i, u(17.), theme::text_primary()))).child(g.source_label.to_string());
        let mut row = div()
            .flex()
            .items_center()
            .child(fact(tr!("Playtime"), div().child(played)))
            .child(rule())
            .child(fact(tr!("Last played"), div().child(last)))
            .child(rule())
            .child(fact(tr!("Platform"), platform));
        if let Some(year) = release_year(g) {
            row = row.child(rule()).child(fact(tr!("Released"), div().child(year.to_string())));
        }
        let beat = self.wiki.hltb(&g.title, &g.game.title).map(|h| h.main_story).filter(|h| *h > 0.);
        if let Some(hours) = beat {
            row = row.child(rule()).child(fact(tr!("Time to beat"), div().child(heronite_data::hltb::format_hours(hours))));
        }
        row
    }

    /// The game's achievements, when it has a list: a trophy, how many of
    /// how many, the share on the right and a thin bar of it.
    fn cinema_achievements(&self, g: &CollectionGame, cx: &mut Context<Self>) -> Option<Div> {
        if !crate::services::achievements::enabled() {
            return None;
        }
        let svc = crate::services::achievements::get(cx);
        let svc = svc.read(cx);
        let set = svc.entry(&crate::services::achievements::key_of(&g.runner, &g.app_name))?.primary()?;
        let (got, all) = (set.unlocked(), set.total());
        let share = got as f32 / all.max(1) as f32;
        let gold: Hsla = theme::console_focus().into();
        Some(
            div()
                .w(u(ACH_W))
                .flex()
                .items_center()
                .gap(u(14.))
                .child(
                    div()
                        .flex_none()
                        .size(u(40.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg({
                            let mut c = theme::console_bg();
                            c.a = 0.5;
                            c
                        })
                        .border_1()
                        .border_color(gold.opacity(0.75))
                        .child(crate::ui::icon(Icon::Trophy, u(20.), gold)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(u(4.))
                        .child(
                            div()
                                .text_size(u(10.))
                                .font_weight(theme::W_BOLD)
                                .text_color(kit::ink(0.72))
                                .child(kit::spaced_caps(tr!("Achievements"))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(u(4.))
                                        .child(
                                            div()
                                                .text_size(u(17.))
                                                .font_weight(theme::W_BOLD)
                                                .text_color(kit::strong())
                                                .child(got.to_string()),
                                        )
                                        .child(div().text_size(u(13.)).text_color(kit::ink(0.72)).child(format!("/ {all}"))),
                                )
                                .child(div().text_size(u(13.)).text_color(kit::ink(0.72)).child(format!("{:.0}%", share * 100.))),
                        )
                        .child(
                            div()
                                .h(u(3.))
                                .w_full()
                                .rounded_full()
                                .bg(kit::ink(0.16))
                                .child(div().h_full().rounded_full().bg(gold).w(relative(share.max(0.01)))),
                        ),
                ),
        )
    }

    /// The game's block: the logo (or the name), the facts and the genres,
    /// each from the block's left edge; it comes in as `id` says.
    pub(super) fn cinema_block(&mut self, g: &CollectionGame, id: &str, slide: f32, width: f32, logo_h: f32, window: &Window) -> Div {
        let key = SharedString::from(g.key.clone());
        let logo_w = LOGO.0.min(width);
        // No logo: no title in its place, only the name in a quiet line over
        // the facts (the block is not left without it).
        let name = match self.console_logo(&key, logo_w, logo_h, true, window) {
            Some(src) => Some(logo_element(src, logo_w, logo_h, window)),
            None => Some(
                div()
                    .text_size(u(12.5))
                    .font_weight(theme::W_BOLD)
                    .text_color(kit::ink(0.86))
                    .whitespace_nowrap()
                    .max_w(u(width))
                    .truncate()
                    .child(g.title.clone())
                    .into_any_element(),
            ),
        };
        let genres = div()
            .h(u(GENRES_H))
            .flex()
            .items_center()
            .gap(u(18.))
            .text_size(u(14.))
            .text_color(kit::ink(0.72))
            .children(g.details.genres.iter().take(5).map(|genre| div().whitespace_nowrap().child(genre.clone())));
        let rest = div()
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .gap(u(14.))
            .child(div().h(u(FACTS_H)).flex().items_center().child(self.cinema_facts(g)))
            .child(genres);
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(u(LOGO_GAP))
            .children(name.map(|name| {
                motion::reveal(
                    SharedString::from(format!("{id}-logo")),
                    div().relative().flex().child(name),
                    motion::NAME_HOLD,
                    motion::NAME_FADE,
                    slide,
                    0.,
                )
            }))
            .child(motion::reveal(SharedString::from(format!("{id}-rest")), rest, motion::REST_HOLD, motion::REST_FADE, 0., 0.))
    }

    /// The tab's shelves under the main row (the theme's, over the tab's
    /// games), less the one that is the main row itself.
    fn cinema_shelves(&self, spec: &TabSpec) -> Vec<Shelf> {
        let main = spec.source.as_str();
        self.console_shelves(main).into_iter().filter(|s| s.kind.as_ref() != main).collect()
    }

    pub(super) fn render_cinema(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let all = self.cinema_keys(spec);
        if all.is_empty() {
            return super::no_games();
        }
        let shelves = self.cinema_shelves(spec);
        let n = all.len();
        let nav = &mut self.tab_nav(tab).cinema;
        nav.index = nav.index.min(n - 1);
        nav.fit(&shelves);
        let moving = nav.moving_until.is_some_and(|t| Instant::now() < t);
        // The focus came to rest: the block comes back, its entrance anew.
        if nav.hidden && !moving {
            nav.hidden = false;
            nav.returns += 1;
        }
        let (index, glide, returns, level) = (nav.index, nav.row.clone(), nav.returns, nav.level);
        let (view_glide, shelf_glides) = (nav.view.clone(), nav.glides.clone());
        let mut cols = vec![index];
        cols.extend(nav.cols.iter().copied());
        // The focused game: the main row's, else the focused shelf's (its last
        // one while "View all" has the focus).
        let (focused, _on_more) = match level.checked_sub(1).and_then(|l| shelves.get(l)) {
            Some(s) => {
                let c = cols[level];
                (s.keys[c.min(s.keys.len() - 1)].clone(), c >= s.keys.len())
            }
            None => (all[index].clone(), false),
        };
        self.console_focus_game(Some(focused.clone()), cx);
        let game = self.lib.as_ref().and_then(|l| l.get(&all[index])).cloned();
        let deep = level > 0;

        let vw = super::body_width(window);
        let body_h = super::body_height(window, false);
        let side = side(vw);
        let th = cover_h(body_h);
        let tw = th / self.cover_aspect.max(0.5);
        let geo = Geo::new(body_h, th, self.cover_aspect, shelves.len());
        let width = crate::ui::content_size(window).width;

        // The main row: one scrolling line of covers, the ones out of sight empty boxes.
        let seen = super::in_sight(glide.handle().offset().x, tw + GAP, width);
        let mut row = div()
            .id(("console-cinema-row", tab))
            .absolute()
            .left_0()
            .right_0()
            .top(u(geo.row_top))
            .h(u(th + 2. * RING_ROOM))
            .flex()
            .items_center()
            .gap(u(GAP))
            .px(u(side))
            .overflow_x_scroll()
            .track_scroll(glide.handle());
        for (i, key) in all.iter().enumerate() {
            let visible = super::loads(&seen, i, index);
            row = row.child(
                self.console_tile(key, tw, th, !deep && i == index, visible, window).on_pads(super::shelf_click(level, &cols, 0, i)),
            );
        }
        row = row.child(div().flex_none().w(u(side))).child(glide.driver());
        let row = motion::spring(("cinema-main-row", tab), row, motion::SOFT, deep, |d, k| d.opacity(1. - k.clamp(0., 1.)));

        // The game's block, and its achievements on the right, while the focus rests on the main row.
        let block_bottom = body_h - geo.row_top + BLOCK_GAP - RING_ROOM;
        let mut over = div().absolute().left_0().right_0().top_0().h(u(body_h));
        if let (Some(g), false, false) = (game.as_ref(), moving, deep) {
            let id = format!("cinema-{tab}-{}-{returns}", all[index]);
            let block_w = (vw * 0.5).clamp(520., 900.);
            let block = self.cinema_block(g, &id, LOGO_SLIDE, block_w, LOGO.1, window);
            // Behind the facts, the genres and the foot of the logo: a tall
            // logo's top stays clear, so the shade fades out well under the
            // top bar (it is drawn over the bar's art, after it).
            let shade_h = FACTS_H + GENRES_H + 130.;
            over = over
                .child(
                    div().absolute().left(u(side)).w(u(block_w)).bottom(u(block_bottom)).h(u(shade_h)).child(block_shade(block_w, shade_h)),
                )
                // At the left edge of the row, as wide as its widest line.
                .child(div().absolute().left(u(side)).max_w(u(block_w)).bottom(u(block_bottom)).child(block));
            if let Some(ach) = self.cinema_achievements(g, cx).filter(|_| vw >= block_w + side * 2. + ACH_W + 40.) {
                let ach =
                    motion::reveal(SharedString::from(format!("{id}-ach")), ach.relative(), motion::REST_HOLD, motion::REST_FADE, 0., 0.);
                // A smaller shade of its own, so it reads over bright art too.
                let (sw, sh) = (ACH_W + 80., FACTS_H + 40.);
                let at = block_bottom + GENRES_H + 14.;
                over = over
                    .child(div().absolute().right(u(side - 40.)).bottom(u(at - 20.)).w(u(sw)).h(u(sh)).child(block_shade(sw, sh)))
                    .child(div().absolute().right(u(side)).bottom(u(at)).h(u(FACTS_H)).flex().items_center().child(ach));
            }
        }

        // The shelves under it, quieter: smaller covers, the focused one bright.
        let (sw, sh) = (geo.shelf_w, geo.shelf_h);
        let mut content = div().relative().w_full().h(u(geo.total)).child(row);
        for (si, shelf) in shelves.iter().enumerate() {
            let lv = si + 1;
            let top = geo.shelf_top(si);
            // On the main row only the first shelf's title peeks in: its covers
            // come with the focus.
            let near = level > 0 && lv + 1 >= level && lv <= level + 3;
            let mut line = div()
                .id(("console-cinema-shelf", si))
                .absolute()
                .left_0()
                .right_0()
                .top(u(LABEL_H))
                .h(u(sh + 2. * SHELF_RING))
                .flex()
                .items_center()
                .gap(u(SHELF_GAP))
                .px(u(side))
                .overflow_x_scroll()
                .track_scroll(shelf_glides[si].handle());
            if near {
                let seen = super::in_sight(shelf_glides[si].handle().offset().x, sw + SHELF_GAP, width);
                for (c, key) in shelf.keys.iter().enumerate() {
                    let visible = super::loads(&seen, c, cols[lv]);
                    let tile = self.console_tile(key, sw, sh, level == lv && cols[lv] == c, visible, window);
                    line = line.child(tile.on_pads(super::shelf_click(level, &cols, lv, c)));
                }
                if shelf.cut() {
                    let c = shelf.keys.len();
                    let tile = super::shelves::more_tile(("cinema-more", si), level == lv && cols[lv] == c, shelf.total, sw, sh);
                    line = line.child(tile.on_pads(super::shelf_click(level, &cols, lv, c)));
                }
            }
            line = line.child(div().flex_none().w(u(side))).child(shelf_glides[si].driver());
            let block = div()
                .absolute()
                .left_0()
                .right_0()
                .top(u(top))
                .h(u(geo.shelf_block))
                .child(shelf_title(&shelf.title, shelf.total, side))
                .child(line);
            let dim = look::theme().dim_others;
            content = content.child(motion::spring(("cinema-shelf-fx", si), block, motion::SOFT, level == lv, move |d, k| {
                let low = if dim { 0.55 } else { 0.85 };
                d.opacity(low + (1. - low) * k.clamp(0., 1.))
            }));
        }
        let view = div()
            .id(("console-cinema-view", tab))
            .absolute()
            .inset_0()
            .overflow_y_scroll()
            .track_scroll(view_glide.handle())
            .child(content)
            .child(view_glide.driver());
        // In the shelves the art steps back behind them. The body starts under
        // the top bar: the shade comes in over its first px instead of
        // starting on a line under the bar's own lighter art.
        let veil = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .child(div().flex_none().h(u(DEEP_FADE)).bg(linear_gradient(
                180.,
                linear_color_stop(kit::veil(0.), 0.),
                linear_color_stop(kit::veil(0.55), 1.),
            )))
            .child(div().flex_1().bg(kit::veil(0.55)));
        let shade = motion::spring(("cinema-deep", tab), veil, motion::SOFT, deep, |d, k| d.opacity(k.clamp(0., 1.)));
        // Deep in them, the shelves passing under the hints fade into the dark.
        let foot = motion::spring(
            ("cinema-deep-foot", tab),
            div().absolute().left_0().right_0().bottom_0().h(u(130.)).bg(linear_gradient(
                0.,
                linear_color_stop(kit::dim(0.85), 0.),
                linear_color_stop(kit::dim(0.), 1.),
            )),
            motion::SOFT,
            deep,
            |d, k| d.opacity(k.clamp(0., 1.)),
        );
        // The game's block lies over the view, not in it: the scroll area clips
        // what it holds, and the block's soft shade would end on a line at
        // its top edge (it only shows on the main row, the view at its top).
        div().relative().size_full().child(shade).child(over).child(view).child(foot).into_any_element()
    }

    pub(super) fn cinema_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let all = self.cinema_keys(spec);
        let n = all.len();
        if n == 0 {
            return false;
        }
        let shelves = self.cinema_shelves(spec);
        let body_h = super::body_height(window, false);
        let th = cover_h(body_h);
        let geo = Geo::new(body_h, th, self.cover_aspect, shelves.len());
        let view = super::body_width(window);
        let k = kit::scale();
        let nav = &mut self.tab_nav(tab).cinema;
        nav.fit(&shelves);
        let level = nav.level.min(shelves.len());
        nav.level = level;
        let i = nav.index.min(n - 1);
        // In a shelf: its covers, and "View all" one past them when it is cut.
        if level > 0 {
            let si = level - 1;
            let shelf = &shelves[si];
            let count = shelf.keys.len() + usize::from(shelf.cut());
            let c = nav.cols[si].min(count - 1);
            let to = match pad {
                Pad::Left | Pad::Right => super::step(c, count, pad.dir(), false),
                Pad::LT | Pad::RT => {
                    let page = ((view / (geo.shelf_w + SHELF_GAP)).floor() as isize - 1).max(1);
                    super::step(c, count, if pad == Pad::LT { -page } else { page }, false)
                }
                Pad::Up | Pad::Down => {
                    let to = if pad == Pad::Up { level - 1 } else { (level + 1).min(shelves.len()) };
                    if to != level {
                        nav.level = to;
                        nav.view.go(Goal::Offset(geo.view_offset(to) * k));
                        if to == 0 {
                            // Back on the main row: the game's block comes in again.
                            nav.returns += 1;
                        }
                    }
                    return true;
                }
                Pad::A | Pad::X => {
                    match shelf.keys.get(c).cloned() {
                        Some(key) => self.console_activate(key, pad, window, cx),
                        None if pad == Pad::A => self.console_open_shelf(shelf, window, cx),
                        None => {}
                    }
                    return true;
                }
                Pad::Y => {
                    self.open_console_filters(window, cx);
                    return true;
                }
                _ => return false,
            };
            if to != c {
                nav.cols[si] = to;
                let current = -f32::from(nav.glides[si].handle().offset().x) / k;
                let goal = Goal::Offset(row_offset_with(to, geo.shelf_w, SHELF_GAP, side(view), view, current) * k);
                nav.glides[si].go(goal);
            }
            return true;
        }
        let key = all[i].clone();
        let wrap = look::theme().nav.wrap;
        let to = match pad {
            Pad::Left | Pad::Right => super::step(i, n, pad.dir(), wrap),
            Pad::LT | Pad::RT => {
                let page = ((view / (th / self.cover_aspect + GAP)).floor() as isize - 1).max(1);
                super::step(i, n, if pad == Pad::LT { -page } else { page }, false)
            }
            Pad::Down if !shelves.is_empty() => {
                nav.level = 1;
                nav.view.go(Goal::Offset(geo.view_offset(1) * k));
                return true;
            }
            Pad::A | Pad::X => {
                self.console_activate(key, pad, window, cx);
                return true;
            }
            Pad::Y => {
                self.open_console_filters(window, cx);
                return true;
            }
            _ => return false,
        };
        if to != i {
            let tw = th / self.cover_aspect.max(0.5);
            let nav = &mut self.tab_nav(tab).cinema;
            nav.index = to;
            nav.hidden = true;
            nav.moving_until = Some(Instant::now() + SETTLE);
            // Worked out from the row's fixed geometry (not the last layout's
            // bounds), so the focused cover is always whole in view. Round the
            // end of the row: there at once, not a glide past every cover.
            let current = -f32::from(nav.row.handle().offset().x) / k;
            let goal = Goal::Offset(row_offset(to, tw, side(view), view, current) * k);
            if to.abs_diff(i) > 1 {
                nav.row.jump(goal);
            } else {
                nav.row.go(goal);
            }
            // Drawn again once the focus rests, for the block to come back.
            nav.wake = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SETTLE + Duration::from_millis(20)).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }));
        }
        true
    }

    /// The covers' hints every layout shows (A and X as `nav.aButton` says,
    /// Y the search and filters, the tabs, the menu), the menu and the tabs
    /// with a small second line: what the menu leads to, the next tab; on a
    /// shelf's "View all", A opens it.
    pub(super) fn cinema_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        let theme = look::theme();
        let next = theme.tabs.get((self.console.tab + 1) % theme.tabs.len().max(1)).map(|t| heronite_i18n::tr(&t.name).to_string());
        let on_more = self.console.tab_navs.get(tab).is_some_and(|n| {
            let nav = &n.cinema;
            nav.level > 0
                && self
                    .cinema_shelves(spec)
                    .get(nav.level - 1)
                    .is_some_and(|s| nav.cols.get(nav.level - 1).is_some_and(|&c| c >= s.keys.len()))
        });
        let hints = if on_more {
            let mut h = vec![hint(Btn::A, tr!("View all")), hint(Btn::Y, tr!("Search"))];
            h.extend(self.console_tab_hints());
            h
        } else {
            self.console_cover_hints()
        };
        hints
            .into_iter()
            .map(|h| match h.btn {
                Btn::Start => h.sub(tr!("Settings, downloads, power")),
                Btn::Bumpers => match &next {
                    Some(next) => h.sub(tr!("Next: {tab}", tab = next)),
                    None => h,
                },
                _ => h,
            })
            .collect()
    }
}

/// A shelf's title in Cinema's style: its name and how many games it has,
/// a thin champagne rule under them.
fn shelf_title(title: &str, total: usize, side: f32) -> Div {
    let gold: Hsla = theme::console_focus().into();
    div()
        .absolute()
        .top_0()
        .left(u(side))
        .h(u(LABEL_H))
        .flex()
        .flex_col()
        .justify_center()
        .gap(u(5.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(u(10.))
                .child(
                    div()
                        .text_size(u(16.))
                        .font_weight(theme::W_BOLD)
                        .text_color(kit::strong())
                        .whitespace_nowrap()
                        .child(title.to_string()),
                )
                .child(div().text_size(u(13.)).text_color(kit::ink(0.6)).child(total.to_string())),
        )
        .child(div().flex_none().w(u(40.)).h(u(2.)).rounded_full().bg(gold.opacity(0.9)))
}

/// Where the home's parts sit, design px from the top of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    body_h: f32,
    row_top: f32,
    /// Where the shelves start: right under the main row, the first one's
    /// title peeking in above the hints.
    shelves: f32,
    shelf_w: f32,
    shelf_h: f32,
    /// A shelf: its title, its row and the space under it.
    shelf_block: f32,
    total: f32,
}

impl Geo {
    fn new(body_h: f32, th: f32, aspect: f32, shelves: usize) -> Self {
        let row_top = body_h - HINTS_ROOM - PEEK - th - RING_ROOM;
        let shelf_h = (th * 0.62).max(110.);
        let shelf_w = shelf_h / aspect.max(0.5);
        let shelf_block = LABEL_H + shelf_h + 2. * SHELF_RING + 8.;
        let start = row_top + th + 2. * RING_ROOM - 6.;
        // Room after the last shelf so it can rise to the top too.
        let total = if shelves == 0 { body_h } else { start + shelves as f32 * shelf_block + (body_h - shelf_block - SHELF_TOP).max(0.) };
        Self { body_h, row_top, shelves: start, shelf_w, shelf_h, shelf_block, total }
    }

    fn shelf_top(&self, i: usize) -> f32 {
        self.shelves + i as f32 * self.shelf_block
    }

    /// Where the view scrolls for `level` (0 the main row, else shelf
    /// `level - 1`): at the top for the main row; a shelf comes up near the
    /// top of the body.
    fn view_offset(&self, level: usize) -> f32 {
        if level == 0 {
            0.
        } else {
            (self.shelf_top(level - 1) - SHELF_TOP).max(0.)
        }
    }
}

/// [`row_offset`] for a row of items `w` wide, `gap` apart.
fn row_offset_with(i: usize, w: f32, gap: f32, side: f32, view: f32, current: f32) -> f32 {
    let x = side + i as f32 * (w + gap);
    let margin = (w * 0.6).min((view - w) / 2.).max(0.);
    if x - margin < current {
        (x - side).max(0.)
    } else if x + w + margin > current + view {
        x + w + margin - view
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn the_shelves_peek_under_the_row_and_each_comes_up_near_the_top() {
        let g = Geo::new(724., 260., 1.5, 4);
        // The first shelf's title shows above the hints' room.
        assert!(g.shelves + LABEL_H <= 724. - 60. + 0.5, "{g:?}");
        assert!(g.shelves > g.row_top + 260.);
        assert_eq!(g.view_offset(0), 0.);
        assert_eq!(g.view_offset(2), g.shelf_top(1) - SHELF_TOP);
        // The last shelf can rise as high as the others.
        assert!(g.total - g.view_offset(4) >= 724. - 0.5);
        assert!(g.shelf_h < 260.);
        // No shelves: the home is one screen.
        assert_eq!(Geo::new(724., 260., 1.5, 0).total, 724.);
    }

    #[test]
    fn the_row_scrolls_just_enough_to_show_the_focused_cover_whole() {
        // In view: stays.
        assert_eq!(row_offset(1, 200., 100., 2000., 0.), 0.);
        // Past the right edge: it comes in whole with room beside it.
        let o = row_offset(9, 200., 100., 2000., 0.);
        let x = 100. + 9. * (200. + GAP);
        assert!(x >= o && x + 200. <= o + 2000. && x + 200. + 120. - 0.01 <= o + 2000.);
        // Back to the start: the row's own start.
        assert_eq!(row_offset(0, 200., 100., 2000., 900.), 0.);
    }

    #[test]
    fn the_row_takes_about_a_third_and_the_side_follows_the_width() {
        assert_eq!(cover_h(724.), 724. * 0.36);
        assert_eq!(cover_h(300.), 170.);
        assert_eq!(side(1911.), 1911. * 0.07);
        assert_eq!(side(400.), 46.);
        assert_eq!(side(4000.), 150.);
    }
}
