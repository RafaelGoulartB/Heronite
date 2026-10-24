//! Spotlight: one game at a time over its art. A row of small square icons
//! on top (the first shelf, then "All games"), the focused game below with
//! its name, facts, overview and action buttons, and cards with what matters
//! (playtime, How Long To Beat, journal). ← → move along the row; A or ↓ go
//! down to the actions, ↓ again to the cards; ↑ or B come back; X plays from
//! anywhere.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Layout, Size3, SpotActions, TabSpec, TileShape, TileSize};
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::backdrop::Scrim;
use super::super::hero::{clip, HeroSize};
use super::super::sources::{by_last_played, keys};
use super::super::tile::label_room;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

/// Games in the row of icons.
const ROW_GAMES: usize = 10;
const ICON: f32 = 84.;
const ICON_ON: f32 = 104.;
const GAP: f32 = 14.;
const SIDE: f32 = 40.;
/// Room around the row for the focus ring.
const PAD: f32 = 10.;
/// The window heights (design px) that have room for the cards and the overview.
const CARDS_FROM: f32 = 700.;
const OVERVIEW_FROM: f32 = 760.;
/// Width and height of the row's icons and of the focused one, by the
/// theme's cover size and shape (a portrait cover keeps the app's cover
/// proportions, so nothing of it is cut).
fn icon_sizes(cover_aspect: f32) -> ((f32, f32), (f32, f32)) {
    let t = look::theme();
    let k = match t.tiles.size {
        TileSize::Small => 0.85,
        TileSize::Medium => 1.,
        TileSize::Large => 1.2,
        TileSize::Huge => 1.4,
    };
    let dims = |side: f32| match t.tiles.shape {
        TileShape::Square => (side, side),
        TileShape::Portrait => (side * 0.8, side * 0.8 * cover_aspect),
        TileShape::Landscape => (side * 1.6, side * 1.6 * 9. / 16.),
    };
    (dims(ICON * k), dims(ICON_ON * k))
}

/// Buttons: Play, Status, Favourite, More (or Play and More).
fn actions() -> usize {
    match look::theme().layouts.spotlight_actions {
        SpotActions::Full => 4,
        SpotActions::Play => 2,
    }
}

pub(super) const SCRIM: Scrim = Scrim::Spot;

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
    row: Glide,
}

impl Default for Nav {
    fn default() -> Self {
        Self { zone: Zone::Row, index: 0, action: 0, card: 0, row: Glide::new(Axis::X) }
    }
}

struct Card {
    title: SharedString,
    value: String,
    note: String,
}

/// Whether the window has room for the cards (and the theme wants them).
fn cards_fit(window: &Window) -> bool {
    look::theme().layouts.spotlight_cards && f32::from(window.viewport_size().height) / kit::scale() >= CARDS_FROM
}

/// A round or pill button of the actions row.
fn button(focused: bool, ic: Icon, label: Option<SharedString>, tint: Option<Rgba>) -> Div {
    let colour: Hsla = if focused { theme::console_on_focus().into() } else { tint.unwrap_or(theme::text_primary()).into() };
    let d = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .h(u(50.))
        .rounded_full()
        .border_1()
        .text_size(u(17.))
        .font_weight(theme::W_BOLD)
        .text_color(colour);
    let d = if label.is_some() { d.px(u(22.)) } else { d.w(u(50.)) };
    let d = if focused {
        d.border_color(transparent_black()).child(kit::focus_pill(25.))
    } else {
        d.border_color(kit::ink(0.08)).bg(kit::ink(0.1))
    };
    d.child(crate::ui::icon(ic, u(22.), colour)).children(label)
}

fn card(focused: bool, c: &Card) -> Div {
    div()
        .relative()
        .flex_none()
        .w(u(250.))
        .px(u(18.))
        .py(u(14.))
        .rounded(u(16.))
        .bg(kit::shade(0.35))
        .border_2()
        .border_color(if focused { Hsla::from(theme::console_focus()) } else { kit::ink(0.08) })
        .when(focused, |d| d.top(-u(3.)).shadow(kit::shadow(0.5, 8., 22.)))
        .child(div().text_size(u(13.)).text_color(theme::text_dim()).child(c.title.clone()))
        .child(kit::title(c.value.clone(), 22.).truncate())
        .child(div().mt(u(2.)).text_size(u(13.5)).text_color(theme::text_muted()).truncate().child(c.note.clone()))
}

/// The wide Play button of `SpotActions::Play` (a console's home screen).
fn wide_button(focused: bool, ic: Icon, label: SharedString) -> Div {
    let colour: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_primary().into() };
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .min_w(u(240.))
        .h(u(56.))
        .px(u(28.))
        .rounded(u(14.))
        .text_size(u(19.))
        .font_weight(theme::W_BOLD)
        .text_color(colour)
        .when(focused, |d| d.bg(theme::console_focus()).shadow(kit::shadow(0.5, 6., 20.)))
        .when(!focused, |d| d.bg(kit::shade(0.45)).border_1().border_color(kit::ink(0.12)))
        .child(crate::ui::icon(ic, u(24.), colour))
        .child(label)
}

/// The "All games" icon.
fn all_tile(w: f32, h: f32, ring: bool) -> Div {
    div()
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(u(w))
        .h(u(h))
        .rounded(u(look::tile_radius()))
        .bg(kit::ink(0.1))
        .when(ring, |d| d.border_3().border_color(theme::console_focus()).shadow(kit::shadow(0.6, 8., 24.)))
        .child(crate::ui::icon(Icon::GridView, u(w.min(h) * 0.4), theme::text_primary()))
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

    /// Playtime, How Long To Beat and journal; the last one only once the
    /// journal of the focused game is read.
    fn spotlight_cards(&self, g: &CollectionGame) -> Vec<Card> {
        let last = match g.last_played.as_deref().and_then(super::super::super::data::days_since) {
            Some(days) => tr!("Last played: {when}", when = super::super::super::data::days_ago(days)),
            None => tr!("Never played").to_string(),
        };
        let played = if g.playtime_minutes > 0 {
            heronite_data::collection::format_playtime_minutes(g.playtime_minutes)
        } else {
            "\u{2014}".to_string()
        };
        let mut cards = vec![Card { title: tr!("Playtime").into(), value: played, note: last }];
        let main = self.wiki.hltb(&g.title, &g.game.title).map(|h| h.main_story).filter(|h| *h > 0.);
        cards.push(match main {
            Some(hours) => {
                let note = if g.playtime_minutes > 0 {
                    let percent = ((g.playtime_minutes as f64 / 60. / hours) * 100.).round().min(99.) as u32;
                    tr!("Main story \u{b7} you are at {percent}%", percent = percent)
                } else {
                    tr!("Main story").to_string()
                };
                Card { title: tr!("How Long To Beat").into(), value: heronite_data::hltb::format_hours(hours), note }
            }
            None => Card { title: tr!("How Long To Beat").into(), value: "\u{2014}".into(), note: tr!("No data").into() },
        });
        let key = SharedString::from(g.key.clone());
        if let Some(j) = self.journal.data.as_ref().filter(|_| self.journal.is_for(&key)) {
            let n = j.entries.len();
            let note = j.entries.last().map(|e| clip(&e.text, 60)).filter(|t| !t.is_empty());
            cards.push(Card {
                title: tr!("Journal").into(),
                value: if n == 0 { tr!("No notes yet").to_string() } else { trn!(n, "{n} note", "{n} notes") },
                note: note.map_or(String::new(), |t| format!("\u{201c}{t}\u{201d}")),
            });
        }
        cards
    }

    /// Buttons of the actions row for `g`.
    fn spotlight_actions(&self, g: &CollectionGame, zone: Zone, at: usize) -> Div {
        let on = |i: usize| zone == Zone::Actions && at == i;
        let icon = if self.preparing.contains_key(&g.app_name) {
            Icon::Close
        } else if self.playing.contains_key(&g.app_name) {
            Icon::Stop
        } else if g.installed {
            Icon::PlayArrow
        } else {
            Icon::Download
        };
        let status = self
            .lib
            .as_ref()
            .and_then(|l| l.statuses.iter().find(|s| s.id == g.status_id))
            .map(|s| heronite_data::status::status_label(&s.name).to_string())
            .unwrap_or_else(|| tr!("Status").to_string());
        if look::theme().layouts.spotlight_actions == SpotActions::Play {
            let more = button(on(1), Icon::MoreHoriz, None, None).h(u(56.)).w(u(56.));
            return div()
                .flex()
                .items_center()
                .gap(u(14.))
                .mt(u(10.))
                .child(wide_button(on(0), icon, self.primary_label(g).into()))
                .child(more);
        }
        let star = g.favourite.then(|| rgb(0xf4d35e));
        div()
            .flex()
            .gap(u(12.))
            .mt(u(6.))
            .child(button(on(0), icon, Some(self.primary_label(g).into()), None))
            .child(button(on(1), Icon::Flag, Some(status.into()), None))
            .child(button(on(2), Icon::Favorite, None, star))
            .child(button(on(3), Icon::Menu, None, None))
    }

    pub(super) fn render_spotlight(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.is_none() {
            return kit::empty(Icon::GridView, tr!("Loading your library\u{2026}"), "").into_any_element();
        }
        let found = self.spotlight_keys(spec);
        if found.is_empty() {
            return kit::empty(Icon::GridView, tr!("No games yet"), tr!("Add a game from the menu (\u{2630}).")).into_any_element();
        }
        let n = found.len();
        let theme = look::theme();
        let nav = &mut self.tab_nav(tab).spotlight;
        nav.index = nav.index.min(n);
        if nav.index == n {
            nav.zone = Zone::Row;
        }
        let (zone, index, action, card_at) = (nav.zone, nav.index, nav.action.min(actions() - 1), nav.card);
        let glide = nav.row.clone();
        let game_key = found.get(index).cloned();
        self.console_focus_game(Some(game_key.clone().unwrap_or_else(|| found[0].clone())), cx);

        let (icon, icon_on) = icon_sizes(self.cover_aspect);
        let shape = look::theme().tiles.shape;
        let step = f32::from(u(icon.0 + GAP));
        let per_row = (f32::from(window.viewport_size().width) / step).ceil() as usize + 1;
        let first = (-f32::from(glide.handle().offset().x) / step).floor().max(0.) as usize;
        let ring = zone == Zone::Row;
        let radius = look::tile_radius();
        let mut row = div()
            .id(("spot-row", tab))
            .flex()
            .items_center()
            .gap(u(GAP))
            .p(u(PAD))
            .pb(u(PAD + label_room()))
            .overflow_x_scroll()
            .track_scroll(glide.handle());
        for j in 0..=n {
            let on = j == index;
            let (w, h) = if on { icon_on } else { icon };
            let visible = (first..=first + per_row).contains(&j) || j.abs_diff(index) <= 2;
            let tile = match found.get(j) {
                Some(key) => self.console_tile_shaped(key, shape, w, h, on && ring, visible, window),
                None => all_tile(w, h, on && ring),
            };
            // The focused icon keeps a thin ring while the focus is below.
            let tile =
                tile.when(on && !ring, |d| d.child(div().absolute().inset_0().rounded(u(radius)).border_2().border_color(kit::ink(0.5))));
            let name = on.then(|| {
                let text = match found.get(j).and_then(|k| self.lib.as_ref()?.get(k)) {
                    Some(g) => g.title.clone(),
                    None => tr!("All games").to_string(),
                };
                let id = SharedString::from(format!("spot-name-{}", found.get(j).map_or("all", |k| k.as_ref())));
                motion::enter_from(
                    id,
                    div().relative().flex_none().max_w(u(320.)).truncate().text_size(u(19.)).font_weight(theme::W_BOLD).child(text),
                    320,
                    8.,
                    0.,
                    0.,
                )
            });
            // The icon grows and its neighbours make room, smoothly.
            let grow = motion::spring(
                ("spot-icon", j),
                div().flex().items_center().justify_center().child(tile),
                motion::GENTLE,
                on,
                move |d, t| {
                    let t = t.clamp(0., 1.);
                    d.w(u(icon.0 + (icon_on.0 - icon.0) * t)).h(u(icon.1 + (icon_on.1 - icon.1) * t))
                },
            );
            row = row.child(div().flex_none().flex().items_center().gap(u(10.)).child(grow).children(name));
        }
        row = row.child(glide.driver());

        let tall = f32::from(window.viewport_size().height) / kit::scale();
        let main = match game_key.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).cloned() {
            Some(g) => {
                let (logo_w, logo_h) = match theme.hero.size {
                    Size3::Small => (420., 84.),
                    Size3::Medium => (420., 104.),
                    Size3::Large => (560., 170.),
                };
                let size = HeroSize {
                    title: match theme.hero.size {
                        Size3::Small => 42.,
                        Size3::Medium => 54.,
                        Size3::Large => 64.,
                    },
                    logo: (logo_w, logo_h),
                    max_w: 640.,
                    overview: if tall >= OVERVIEW_FROM { 200 } else { 0 },
                };
                let hero = self.console_hero(game_key.as_ref(), size, window);
                div().flex().flex_col().gap(u(14.)).child(hero).child(self.spotlight_actions(&g, zone, action))
            }
            None => div()
                .flex()
                .flex_col()
                .gap(u(8.))
                .child(kit::title(tr!("All games"), 54.))
                .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(trn!(
                    self.lib.as_ref().map_or(0, |l| l.games.len()),
                    "{n} game",
                    "{n} games"
                )))
                .child(kit::body_text(tr!("Press A to see them all."))),
        };
        let cards = game_key
            .as_ref()
            .filter(|_| cards_fit(window))
            .and_then(|k| self.lib.as_ref()?.get(k))
            .map(|g| self.spotlight_cards(g))
            .unwrap_or_default();
        let cards_row = (!cards.is_empty()).then(|| {
            let at = card_at.min(cards.len() - 1);
            div()
                .flex_none()
                .flex()
                .gap(u(14.))
                .pt(u(10.))
                .pb(u(20.))
                .children(cards.iter().enumerate().map(|(i, c)| card(zone == Zone::Cards && at == i, c)))
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .px(u(SIDE - PAD))
            .child(row.flex_none())
            .child(div().flex_1().min_h_0().px(u(PAD)).pt(u(18.)).child(main))
            .children(cards_row.map(|c| c.px(u(PAD))))
            .into_any_element()
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
        let cards = match game.as_ref().filter(|_| cards_fit(window)).and_then(|k| self.lib.as_ref()?.get(k)) {
            Some(g) => self.spotlight_cards(g).len(),
            None => 0,
        };
        if pad == Pad::Y {
            self.console_search(window, cx);
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
                nav.index = super::step(index, n + 1, if pad == Pad::Left { -1 } else { 1 }, wrap);
                nav.row.go(Goal::Start(nav.index.saturating_sub(3)));
            }
            (Zone::Row, Pad::Down | Pad::A) if game.is_some() => {
                nav.zone = Zone::Actions;
                nav.action = 0;
            }
            (Zone::Row, Pad::A) => {
                // "All games": the first Grid tab, else the search.
                let grid = look::theme().tabs.iter().position(|t| t.layout == Layout::Grid);
                match grid {
                    Some(i) => self.console.tab = i,
                    None => self.console_search(window, cx),
                }
            }
            (Zone::Actions, Pad::Left) => nav.action = nav.action.saturating_sub(1),
            (Zone::Actions, Pad::Right) => nav.action = (nav.action + 1).min(actions() - 1),
            (Zone::Actions, Pad::Up) => nav.zone = Zone::Row,
            (Zone::Actions, Pad::Down) if cards > 0 => {
                nav.zone = Zone::Cards;
                nav.card = 0;
            }
            (Zone::Actions, Pad::A) => {
                let (Some(key), at) = (game, nav.action) else { return true };
                self.spotlight_act(&key, at, window, cx);
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
    fn spotlight_act(&mut self, key: &SharedString, action: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Play and More only: the second one is More.
        let action = if actions() == 2 && action == 1 { 3 } else { action };
        match action {
            0 => self.install_or_play(key, window, cx),
            1 => {
                let Some(lib) = self.lib.clone() else { return };
                let Some(g) = lib.get(key) else { return };
                if lib.statuses.is_empty() {
                    return;
                }
                let at = lib.statuses.iter().position(|s| s.id == g.status_id).unwrap_or(0);
                let next = lib.statuses[(at + 1) % lib.statuses.len()].id.clone();
                self.set_status(key, next, cx);
            }
            2 => {
                if self.selected.as_ref() != Some(key) {
                    self.select(key.clone(), false);
                }
                self.toggle_favourite(cx);
            }
            _ => self.open_game_page(key.clone()),
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
