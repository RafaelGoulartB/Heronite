//! Spotlight: one game at a time over its art, like a console's home. A row
//! of icons along the top (the first shelf, then "All games"): the focused
//! one at the left edge, larger, with its name beside it ([`row`]). Below,
//! the game's logo (or name) with its action buttons and, under them, cards
//! with what matters (playtime, How Long To Beat, journal) that peek in at
//! the bottom; going down to them slides the whole screen up and dims the
//! art. ← → move along the row; A or ↓ go down to the buttons, ↓ again to
//! the cards; ↑ or B come back; X plays from anywhere.
//!
//! The row's position, the room for the name, the focus ring and the slide
//! to the cards each follow a spring stepped every frame ([`motion::Follow`]),
//! so a held direction glides instead of stepping.

mod parts;
mod row;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Layout, Size3, SpotActions, TabSpec};
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::hero::{played_line, HeroMotion, HeroSize};
use super::super::sources::{by_last_played, keys};
use crate::collection::focus_left::STAR;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Follow, FrameClock};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use parts::{button, card, BUTTON_H, CARD_H};

/// Games in the row of icons.
const ROW_GAMES: usize = 10;
/// The left edge of the row, the logo and the cards (in line with the tabs), design px.
const SIDE: f32 = 46.;
/// Room under the buttons: at least this, design px, and this part of the
/// room between the row and the cards, so the logo sits a little under the
/// middle rather than on the cards.
const MAIN_PAD: f32 = 34.;
const MAIN_LIFT: f32 = 0.26;
/// The least room for the logo and the buttons between the row and the
/// cards; a shorter window shows no cards, design px.
const MAIN_MIN: f32 = 250.;
/// How much of the cards shows under the buttons until the focus goes down to them, design px.
const PEEK: f32 = 44.;
/// Under the cards while they have the focus: clear of a floating hint bar, design px.
const CARDS_BOTTOM: f32 = 78.;
const CARDS_GAP: f32 = 18.;
/// The heights that have room for the overview, design px.
const OVERVIEW_FROM: f32 = 400.;
/// How the row glides: a touch under critical damping, ~280 ms.
const GLIDE: SpringConfig = SpringConfig::new(260., 30., 1.);

/// A button of the actions row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpotAct {
    Play,
    Status,
    Favourite,
    More,
}

/// The buttons the theme gives the actions row, in order.
fn actions() -> &'static [SpotAct] {
    match look::theme().layouts.spotlight_actions {
        SpotActions::Full => &[SpotAct::Play, SpotAct::Status, SpotAct::Favourite, SpotAct::More],
        SpotActions::Play => &[SpotAct::Play, SpotAct::More],
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Zone {
    #[default]
    Row,
    Actions,
    Cards,
}

pub(crate) struct Nav {
    zone: Zone,
    /// Icon of the row; past the games it is the "All games" tile.
    index: usize,
    action: usize,
    card: usize,
    /// The side the last move along the row came from (-1 left, 1 right):
    /// the logo slides in from it.
    from: f32,
    /// The row's position (the focused index, fractional while it glides).
    pos: Follow,
    /// Width of the focused game's name, design px.
    name_w: Follow,
    /// The focus ring on the row (1) or below it (0).
    ring: Follow,
    /// 1 while the cards have the focus (the screen slid up).
    depth: Follow,
    /// 1 while a floating hint bar covers the cards that peek in (they step back).
    veil: Follow,
    clock: FrameClock,
    /// Shown once: the springs start where they are heading.
    shown: bool,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            zone: Zone::Row,
            index: 0,
            action: 0,
            card: 0,
            from: 0.,
            pos: Follow::new(0.),
            name_w: Follow::new(0.),
            ring: Follow::new(1.),
            depth: Follow::new(0.),
            veil: Follow::new(0.),
            clock: FrameClock::default(),
            shown: false,
        }
    }
}

/// This frame's sizes and animated values.
pub(super) struct Look {
    /// Size of an icon and of the focused one, design px.
    off: (f32, f32),
    on: (f32, f32),
    pos: f32,
    name_w: f32,
    /// The width the name is heading for (it is laid out at it).
    name_target: f32,
    ring: f32,
    depth: f32,
    veil: f32,
}

impl Nav {
    /// Moves the springs a frame towards the focus.
    fn animate(&mut self, name_w: f32, off: (f32, f32), on: (f32, f32), hints: bool, window: &mut Window) -> Look {
        let targets = [
            (&mut self.pos, self.index as f32),
            (&mut self.name_w, name_w),
            (&mut self.ring, if self.zone == Zone::Row { 1. } else { 0. }),
            (&mut self.depth, if self.zone == Zone::Cards { 1. } else { 0. }),
            (&mut self.veil, if hints { 1. } else { 0. }),
        ];
        for (f, t) in targets {
            if self.shown {
                f.go(t);
            } else {
                f.snap(t);
            }
        }
        self.shown = true;
        let dt = self.clock.dt();
        let moving = self.pos.step(GLIDE, dt)
            | self.name_w.step(GLIDE, dt)
            | self.ring.step(motion::SOFT, dt)
            | self.depth.step(motion::GENTLE, dt)
            | self.veil.step(motion::SOFT, dt);
        self.clock.done(moving, window);
        Look {
            off,
            on,
            pos: self.pos.value,
            name_w: self.name_w.value.max(0.),
            name_target: name_w,
            ring: self.ring.value.clamp(0., 1.),
            depth: self.depth.value.clamp(0., 1.),
            veil: self.veil.value.clamp(0., 1.),
        }
    }
}

/// The room of the screen, design px.
struct Room {
    /// Under the top bar (and the hint bar when it does not float).
    body: f32,
    row: f32,
    /// The window has room for the cards (and the theme wants them).
    cards: bool,
    /// Under the cards while they have the focus.
    cards_bottom: f32,
}

impl CollectionView {
    /// The games of the row: the first enabled shelf, else the latest played.
    fn spotlight_keys(&self, spec: &TabSpec) -> Vec<SharedString> {
        let mut found = match self.console_shelves(&spec.source).into_iter().next() {
            Some(shelf) => shelf.keys,
            None => {
                let games = self.console_games(&spec.source);
                keys(by_last_played(games.iter().map(|g| g.as_ref()).collect()))
            }
        };
        found.truncate(ROW_GAMES);
        found
    }

    fn spotlight_room(&self, window: &Window) -> Room {
        let float = super::hints_float(Layout::Spotlight);
        let body = super::body_height(window, self.hints_visible() && !float);
        let (_, on) = row::icon_sizes(self.cover_aspect);
        let row = row::row_height(on);
        let cards = look::theme().layouts.spotlight_cards && body - row - PEEK >= MAIN_MIN;
        Room { body, row, cards, cards_bottom: if float { CARDS_BOTTOM } else { 24. } }
    }

    /// The buttons of the actions row for `g`, then when it was last played.
    fn spotlight_actions(&self, g: &CollectionGame, zone: Zone, at: usize) -> Div {
        let wide = look::theme().layouts.spotlight_actions == SpotActions::Play;
        let (label, play_icon) = self.primary_state(g);
        let buttons = actions().iter().enumerate().map(|(i, act)| {
            let on = zone == Zone::Actions && at == i;
            let id = ("spot-act", i);
            match act {
                SpotAct::Play => button(id, on, play_icon, Some(label.into()), None, wide),
                SpotAct::Status => {
                    let name = self.console_status(g).map_or_else(|| tr!("Status").to_string(), |(name, _)| name);
                    button(id, on, Icon::Flag, Some(name.into()), None, false)
                }
                SpotAct::Favourite => button(id, on, Icon::Favorite, None, g.favourite.then(|| rgb(STAR)), false),
                SpotAct::More => button(id, on, Icon::MoreHoriz, None, None, false),
            }
        });
        let when = div().ml(u(10.)).text_size(u(15.)).text_color(theme::text_muted()).whitespace_nowrap().child(played_line(g, true));
        div().flex().items_center().gap(u(14.)).mt(u(18.)).h(u(BUTTON_H)).children(buttons).child(when)
    }

    /// The focused game's logo (or name) and buttons, or what "All games" opens.
    fn spotlight_main(&mut self, game: Option<&CollectionGame>, nav: (Zone, usize, f32), room: f32, window: &Window) -> AnyElement {
        let Some(g) = game else {
            let total = self.lib.as_ref().map_or(0, |l| l.games.len());
            let main = div()
                .flex()
                .flex_col()
                .gap(u(8.))
                .child(kit::title(tr!("All games"), 54.))
                .child(div().text_size(u(17.)).text_color(theme::text_muted()).child(trn!(total, "{n} game", "{n} games")))
                .child(kit::body_text(tr!("Press A to see them all.")));
            return motion::enter_from("spot-main-all", main, 300, 16. * nav.2, 0., 0.25);
        };
        let theme = look::theme();
        let (logo_w, logo_h) = match theme.hero.size {
            Size3::Small => (420., 84.),
            Size3::Medium => (440., 112.),
            Size3::Large => (540., 160.),
        };
        let size = HeroSize {
            title: match theme.hero.size {
                Size3::Small => 42.,
                Size3::Medium => 52.,
                Size3::Large => 60.,
            },
            logo: (logo_w, logo_h),
            max_w: 680.,
            overview: if room >= OVERVIEW_FROM { 200 } else { 0 },
        };
        let key = SharedString::from(g.key.clone());
        let how = HeroMotion { slide: (16. * nav.2, 0.), fit_title: true, ..HeroMotion::default() };
        let hero = self.console_hero_with(Some(&key), size, how, window);
        div().flex().flex_col().child(hero).child(self.spotlight_actions(g, nav.0, nav.1)).into_any_element()
    }

    pub(super) fn render_spotlight(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let found = self.spotlight_keys(spec);
        if found.is_empty() {
            return super::no_games();
        }
        let n = found.len();
        let room = self.spotlight_room(window);
        let nav = &mut self.tab_nav(tab).spotlight;
        nav.index = nav.index.min(n);
        if nav.index == n {
            nav.zone = Zone::Row;
        }
        if nav.zone == Zone::Cards && !room.cards {
            nav.zone = Zone::Actions;
        }
        let (zone, index, from) = (nav.zone, nav.index, nav.from);
        let action = nav.action.min(actions().len() - 1);
        let game_key = found.get(index).cloned();
        self.console_focus_game(Some(game_key.clone().unwrap_or_else(|| found[0].clone())), cx);
        let game = game_key.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).cloned();

        let name_w = row::name_width(&self.spotlight_name(&found, index), window);
        let (off, on) = row::icon_sizes(self.cover_aspect);
        let hints = super::hints_float(Layout::Spotlight) && self.hints_visible();
        let look = self.tab_nav(tab).spotlight.animate(name_w, off, on, hints, window);
        let cards = game.as_ref().filter(|_| room.cards).map(|g| self.spotlight_cards(g)).unwrap_or_default();
        let card_at = self.tab_nav(tab).spotlight.card.min(cards.len().saturating_sub(1));

        let row = self.spotlight_row(&found, index, &look, window);
        let main_h = room.body - room.row - if cards.is_empty() { 0. } else { PEEK };
        let pad = MAIN_PAD.max(main_h * MAIN_LIFT);
        let main = self.spotlight_main(game.as_deref(), (zone, action, from), main_h - pad, window);
        // The cards peek in under the buttons; with the focus on them the
        // whole screen slides up until they sit clear of the bottom.
        let (cards_h, shift) = if cards.is_empty() { (0., 0.) } else { (CARD_H + room.cards_bottom, CARD_H + room.cards_bottom - PEEK) };
        let depth = look.depth;
        let cards_row = (!cards.is_empty()).then(|| {
            div()
                .flex_none()
                .flex()
                .gap(u(CARDS_GAP))
                .h(u(cards_h))
                .px(u(SIDE))
                .opacity((0.55 + 0.45 * depth) * (1. - 0.8 * look.veil * (1. - depth)))
                .children(cards.iter().enumerate().map(|(i, c)| card(i, zone == Zone::Cards && i == card_at, c)))
        });
        // The art darkens from the bottom as the cards come up.
        let dim = (depth > 0.005).then(|| {
            div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(kit::dim(0.62 * depth), 0.),
                linear_color_stop(kit::dim(0.), 0.9),
            ))
        });
        let column = div()
            .absolute()
            .left_0()
            .right_0()
            .top(u(-shift * depth))
            .h(u(room.row + main_h + cards_h))
            .flex()
            .flex_col()
            .child(row.opacity(1. - depth))
            .child(div().flex_none().h(u(main_h)).px(u(SIDE)).pb(u(pad)).flex().flex_col().justify_end().overflow_hidden().child(main))
            .children(cards_row);
        div().relative().size_full().overflow_hidden().children(dim).child(column).into_any_element()
    }

    pub(super) fn spotlight_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let found = self.spotlight_keys(spec);
        if found.is_empty() {
            return false;
        }
        let n = found.len();
        let wrap = look::theme().nav.wrap;
        let nav = &self.tab_nav(tab).spotlight;
        let index = nav.index.min(n);
        let game = found.get(index).cloned();
        let zone = if game.is_some() { nav.zone } else { Zone::Row };
        let cards = match game.as_ref().filter(|_| self.spotlight_room(window).cards).and_then(|k| self.lib.as_ref()?.get(k)) {
            Some(g) => self.spotlight_cards(g).len(),
            None => 0,
        };
        if pad == Pad::Y {
            self.open_console_filters(window, cx);
            return true;
        }
        if pad == Pad::X {
            if let Some(k) = &game {
                self.primary_action(k, cx);
                return true;
            }
        }
        let nav = &mut self.tab_nav(tab).spotlight;
        nav.index = index;
        nav.zone = zone;
        match (zone, pad) {
            (Zone::Row, Pad::Left | Pad::Right) => {
                let to = super::step(index, n + 1, pad.dir(), wrap);
                if to != index {
                    nav.from = if to > index { 1. } else { -1. };
                }
                nav.index = to;
            }
            (Zone::Row, Pad::Down | Pad::A) if game.is_some() => {
                nav.zone = Zone::Actions;
                nav.action = 0;
            }
            // "All games".
            (Zone::Row, Pad::A) => self.console_open_library(window, cx),
            (Zone::Actions, Pad::Left) => nav.action = nav.action.saturating_sub(1),
            (Zone::Actions, Pad::Right) => nav.action = (nav.action + 1).min(actions().len() - 1),
            (Zone::Actions, Pad::Up) => nav.zone = Zone::Row,
            (Zone::Actions, Pad::Down) if cards > 0 => {
                nav.zone = Zone::Cards;
                nav.card = nav.card.min(cards - 1);
            }
            (Zone::Actions, Pad::A) => {
                let (Some(key), at) = (game, nav.action) else { return true };
                self.spotlight_act(&key, actions()[at.min(actions().len() - 1)], window, cx);
            }
            (Zone::Cards, Pad::Left) => nav.card = nav.card.saturating_sub(1),
            (Zone::Cards, Pad::Right) => nav.card = (nav.card + 1).min(cards.saturating_sub(1)),
            (Zone::Cards, Pad::Up) => nav.zone = Zone::Actions,
            (Zone::Cards, Pad::A) => {
                if let Some(k) = game {
                    self.open_game_page(k);
                }
            }
            (Zone::Actions | Zone::Cards, Pad::B) => nav.zone = Zone::Row,
            _ => return false,
        }
        true
    }

    /// A on a button of the actions row.
    fn spotlight_act(&mut self, key: &SharedString, action: SpotAct, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            SpotAct::Play => self.install_or_play(key, window, cx),
            SpotAct::Status => {
                let Some(lib) = self.lib.clone() else { return };
                let Some(g) = lib.get(key) else { return };
                if lib.statuses.is_empty() {
                    return;
                }
                let at = lib.statuses.iter().position(|s| s.id == g.status_id).unwrap_or(0);
                let next = lib.statuses[(at + 1) % lib.statuses.len()].id.clone();
                self.set_status(key, next, cx);
            }
            SpotAct::Favourite => {
                if self.selected.as_ref() != Some(key) {
                    self.select(key.clone(), false);
                }
                self.toggle_favourite(cx);
            }
            SpotAct::More => self.open_game_page(key.clone()),
        }
    }

    pub(super) fn spotlight_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        let nav = self.console.tab_navs.get(tab).map(|n| &n.spotlight);
        let found = self.spotlight_keys(spec);
        let game = found.get(nav.map_or(0, |n| n.index)).and_then(|k| self.lib.as_ref()?.get(k));
        let primary = game.map_or(tr!("Play"), |g| self.primary_label(g));
        let mut h = match nav.map(|n| n.zone).filter(|_| game.is_some()) {
            Some(Zone::Actions | Zone::Cards) => {
                vec![hint(Btn::A, tr!("Select")), hint(Btn::X, primary), hint(Btn::B, tr!("Back"))]
            }
            _ if game.is_some() => vec![hint(Btn::A, tr!("Actions")), hint(Btn::X, primary), hint(Btn::Y, tr!("Search"))],
            _ => vec![hint(Btn::A, tr!("Open")), hint(Btn::Y, tr!("Search"))],
        };
        h.extend(self.console_tab_hints());
        h
    }
}
