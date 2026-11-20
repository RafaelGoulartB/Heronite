//! Spotlight: a console's home whose game page goes on below it. Along the
//! top, the tab's games as a row of square icons ([`row`]), the focused one
//! larger with its name under it; below, the game's logo, Play / Options / ♥,
//! its facts (playtime, last played, platform, release) and genres, and a
//! strip of its store screenshots (else the user's) peeking at the bottom.
//!
//! Going down from the row, the whole screen rises (the row and the tabs
//! fade, the art darkens) into the page ([`page`]): the screenshots, About
//! this game beside Achievements and Details (with link buttons), the cards
//! (playtime, How Long To Beat, journal), the user's screenshots and More
//! like this, each when the theme's `spotlightSections` keeps it and the game
//! has something for it. ↑ ↓ move between sections, ← → inside one; B goes
//! back up to the buttons, then to the row; X plays from anywhere.
//!
//! The row's position, its focus ring, the page's rise and the strips'
//! slides each follow a spring stepped every frame ([`motion::Follow`]), so
//! a held direction glides instead of stepping.
//!
//! `HERONITE_DEBUG_SPOTLIGHT=<zone>[:<n>]` opens the page on a section
//! (`actions`, `media`, `viewer`, `about`, `sheet`, `achievements`, `links`,
//! `cards`, `shots`, `related`), on the `n`th game of the row, for screenshots.

mod bar;
mod page;
mod parts;
mod row;
mod viewer;

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Layout, SpotActions, TabSpec};
use heronite_data::details_url::details_url;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

pub(in crate::collection::console) use bar::{spotlight_clock, spotlight_tabs};
use page::{Link, Mid, MidMove, Shape, Zone, SIDE};

use super::super::super::vm::{DetailField, DetailValue};
use super::super::super::CollectionView;
use super::super::hero::HeroSize;
use super::top_recent;
use crate::collection::focus_left::STAR;
use crate::console::kit::{self, u};
use crate::console::look;
use crate::console::motion::{self, Axis, Follow, FrameClock, Glide};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use parts::{button, card, ring, section_title, BUTTON_H};

/// Games in the row of icons (the most recently played first).
const ROW_GAMES: usize = 40;
/// Height of the shell's top bar, design px (the page rises into it).
const TOP_BAR: f32 = 76.;
/// How the row glides: a touch under critical damping, ~280 ms.
const GLIDE: SpringConfig = SpringConfig::new(260., 30., 1.);
/// How the page rises and falls: slow and soft, without overshoot.
const RISE: SpringConfig = SpringConfig::new(95., 19.5, 1.);
/// Most store screenshots in the strip.
const STORE_SHOTS: usize = 10;
/// Most covers in More like this.
const RELATED: usize = 9;

/// The debug section asked for, until it is shown once.
static DEBUG_DONE: AtomicBool = AtomicBool::new(false);

/// A button of the actions row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpotAct {
    Play,
    Status,
    Favourite,
    Options,
}

/// The buttons the theme gives the actions row, in order.
fn actions() -> &'static [SpotAct] {
    match look::theme().layouts.spotlight_actions {
        SpotActions::Full => &[SpotAct::Play, SpotAct::Options, SpotAct::Status, SpotAct::Favourite],
        SpotActions::Play => &[SpotAct::Play, SpotAct::Options, SpotAct::Favourite],
    }
}

/// Something of the page a click focuses (or uses, when it has the focus).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Action(usize),
    Media(usize),
    Mid(Mid, usize),
    Card(usize),
    Shot(usize),
    Related(usize),
}

pub(crate) struct Nav {
    zone: Zone,
    /// Icon of the row; past the games it is the "All games" tile.
    index: usize,
    action: usize,
    /// The focus along each section.
    media: usize,
    mid: Mid,
    link: usize,
    card: usize,
    shot: usize,
    related: usize,
    /// The side the last move along the row came from (-1 left, 1 right):
    /// the logo slides in from it.
    from: f32,
    /// The row's position (the focused index, fractional while it glides).
    pos: Follow,
    /// The focus ring on the row (1) or below it (0).
    ring: Follow,
    /// How far the screen has risen, design px.
    rise: Follow,
    /// How far the media strip, the user's screenshots and More like this have slid, design px.
    strips: [Follow; 3],
    clock: FrameClock,
    /// Shown once: the springs start where they are heading.
    shown: bool,
    /// The page as last drawn.
    shape: Shape,
    /// The store screenshot open full screen.
    viewer: Option<usize>,
    /// About this game's whole text is open (its scroll).
    sheet: Option<Glide>,
    /// How far the page has risen last frame (0 on the row, 1 at the buttons and below).
    lift: f32,
    /// About's text of a game, flattened once.
    about: Option<(SharedString, SharedString)>,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            zone: Zone::Row,
            index: 0,
            action: 0,
            media: 0,
            mid: Mid::default(),
            link: 0,
            card: 0,
            shot: 0,
            related: 0,
            from: 0.,
            pos: Follow::new(0.),
            ring: Follow::new(1.),
            rise: Follow::new(0.),
            strips: [Follow::new(0.), Follow::new(0.), Follow::new(0.)],
            clock: FrameClock::default(),
            shown: false,
            shape: Shape::default(),
            viewer: None,
            sheet: None,
            lift: 0.,
            about: None,
        }
    }
}

impl Nav {
    /// How far the page has risen (for the top bar).
    pub(super) fn lift(&self) -> f32 {
        self.lift
    }

    /// The row has the focus (the wheel moves along it).
    pub(super) fn on_row(&self) -> bool {
        self.zone == Zone::Row && self.viewer.is_none() && self.sheet.is_none()
    }

    /// Moves the focus into `zone`, coming from above (`down`) or below.
    fn enter(&mut self, zone: Zone, down: bool) {
        if zone == Zone::Mid {
            self.mid = self.shape.mid_entry(down);
            self.link = self.link.min(self.shape.links.len().saturating_sub(1));
        }
        self.zone = zone;
    }

    /// The focus along `zone` (← →).
    fn along(&mut self, zone: Zone) -> Option<&mut usize> {
        match zone {
            Zone::Media => Some(&mut self.media),
            Zone::Cards => Some(&mut self.card),
            Zone::Shots => Some(&mut self.shot),
            Zone::Related => Some(&mut self.related),
            _ => None,
        }
    }

    /// Keeps the focus on what the page has now.
    fn settle(&mut self) {
        let zones = self.shape.zones();
        if !zones.contains(&self.zone) {
            self.zone = Zone::Actions;
        }
        for z in [Zone::Media, Zone::Cards, Zone::Shots, Zone::Related] {
            let n = self.shape.count(z);
            if let Some(at) = self.along(z) {
                *at = (*at).min(n.saturating_sub(1));
            }
        }
        if self.zone == Zone::Mid && !self.shape.mid_has(self.mid) {
            self.mid = self.shape.mid_entry(true);
        }
        self.link = self.link.min(self.shape.links.len().saturating_sub(1));
        if self.viewer.is_some_and(|i| i >= self.shape.store.len()) {
            self.viewer = None;
        }
    }
}

/// This frame's sizes and animated values.
pub(super) struct Look {
    /// Size of an icon and of the focused one, design px.
    off: (f32, f32),
    on: (f32, f32),
    pos: f32,
    ring: f32,
}

/// The text of About this game: the store's description as paragraphs,
/// else the overview.
fn about_text(f: &crate::collection::vm::FocusVm) -> Option<SharedString> {
    if let Some(blocks) = &f.overview_html {
        let paras: Vec<String> = blocks
            .iter()
            .filter_map(|b| match b {
                crate::collection::html::Block::Heading(s) | crate::collection::html::Block::Para(s) => {
                    Some(s.iter().map(|x| x.text.as_str()).collect::<String>())
                }
                crate::collection::html::Block::ListItem(s) => {
                    Some(format!("\u{2022} {}", s.iter().map(|x| x.text.as_str()).collect::<String>()))
                }
                crate::collection::html::Block::Image { .. } => None,
            })
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty())
            .collect();
        if !paras.is_empty() {
            return Some(paras.join("\n").into());
        }
    }
    f.overview.clone().filter(|o| !o.trim().is_empty())
}

/// A detail's value as one line.
fn detail_text(v: &DetailValue) -> String {
    match v {
        DetailValue::Text(t) => t.to_string(),
        DetailValue::Chips { values, .. } => values.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", "),
        DetailValue::Folder { path, .. } => path.to_string(),
    }
}

/// The section the debug hook asks for (`HERONITE_DEBUG_SPOTLIGHT`), and the game of the row.
fn debug_target() -> Option<(String, Option<usize>)> {
    if DEBUG_DONE.load(Ordering::Relaxed) {
        return None;
    }
    let v = crate::env::value("HERONITE_DEBUG_SPOTLIGHT")?;
    let (zone, n) = v.split_once(':').map_or((v.as_str(), None), |(z, n)| (z, n.parse().ok()));
    Some((zone.to_string(), n))
}

impl CollectionView {
    /// The games of the row: the tab's, the most recently played first.
    fn spotlight_keys(&self, spec: &TabSpec) -> Vec<SharedString> {
        let games = self.console_games(&spec.source);
        top_recent(&games, ROW_GAMES).into_iter().map(|g| SharedString::from(g.key.clone())).collect()
    }

    /// Whether the focused game's store media are wanted: a Spotlight tab is
    /// on screen and the theme shows its media.
    pub(in crate::collection) fn console_wants_store_media(&self) -> bool {
        self.console_shown_tab().1.layout == Layout::Spotlight && look::theme().layouts.spotlight_sections.media
    }

    /// What the page of `g` has, laid out for a view `view` design px wide.
    fn spotlight_shape(&mut self, g: &CollectionGame, view: f32, tab: usize, cx: &mut Context<Self>) -> Shape {
        let theme = look::theme();
        let sections = theme.layouts.spotlight_sections.clone();
        let key = SharedString::from(g.key.clone());
        let mut s = Shape { key: key.clone(), ..Shape::default() };
        let own_shots = self.game_shot_count_of(&key);
        if sections.media {
            if let Some(m) = self.store_media_of(&key) {
                s.store = m.screenshots.iter().take(STORE_SHOTS).map(|x| x.full.clone()).collect();
            }
            s.media = s.store.len();
            if s.media == 0 && own_shots > 0 {
                s.media = own_shots;
                s.media_shots = true;
            }
        }
        if sections.screenshots && !s.media_shots {
            s.shots = own_shots;
        }
        let focus = self.focus.clone().filter(|f| f.key == key);
        let text = if sections.about {
            let nav = &mut self.tab_nav(tab).spotlight;
            match (&nav.about, &focus) {
                (Some((k, t)), _) if *k == key => Some(t.clone()),
                (_, Some(f)) => {
                    let t = about_text(f);
                    if let Some(t) = &t {
                        nav.about = Some((key.clone(), t.clone()));
                    }
                    t
                }
                _ => None,
            }
        } else {
            None
        };
        s.ach = sections.achievements && self.spotlight_ach_counts(g, cx).is_some();
        if sections.details {
            s.details = self.spotlight_details(g).len();
            s.links = self.spotlight_links(g);
        }
        if theme.layouts.spotlight_cards {
            s.cards = self.spotlight_cards(g).len();
        }
        if sections.related {
            if let Some(f) = &focus {
                let mut seen = std::collections::HashSet::new();
                s.related = f
                    .related
                    .iter()
                    .flat_map(|r| r.games.iter())
                    .filter(|r| r.key != key && seen.insert(r.key.clone()))
                    .map(|r| r.key.clone())
                    .take(RELATED)
                    .collect();
            }
        }
        let width = page::page_width(view);
        if let Some(t) = &text {
            s.about = true;
            s.fit_about(t.chars().count(), width);
        }
        let (_, on) = row::icon_sizes(self.cover_aspect);
        s.layout(row::row_height(on), page::logo_h(theme.hero.size), width, page::COVER_W * self.cover_aspect);
        s
    }

    /// How many of the user's screenshots `key` has (they are listed for the focused game).
    fn game_shot_count_of(&self, key: &SharedString) -> usize {
        if self.shots.game.as_ref() == Some(key) {
            self.shots.items.len()
        } else {
            0
        }
    }

    /// Developer, publisher, size, store, score and time to beat, as far as known.
    fn spotlight_details(&self, g: &CollectionGame) -> Vec<(Icon, SharedString, String)> {
        let Some(f) = self.focus.as_ref().filter(|f| f.key.as_ref() == g.key) else { return Vec::new() };
        let field =
            |field: DetailField| f.details.iter().find(|d| d.field == field).map(|d| detail_text(&d.value)).filter(|t| !t.is_empty());
        let mut rows: Vec<(Icon, SharedString, String)> = Vec::new();
        if let Some(t) = field(DetailField::Developer) {
            rows.push((Icon::Build, tr!("Developer").into(), t));
        }
        if let Some(t) = field(DetailField::Publisher) {
            rows.push((Icon::Storefront, tr!("Publisher").into(), t));
        }
        if let Some(size) = f.install_size.as_ref().filter(|s| f.installed && !s.is_empty() && s.as_ref() != "0") {
            rows.push((Icon::FaHardDrive, tr!("Size on disk").into(), size.to_string()));
        }
        let place = if f.installed { tr!("Installed") } else { tr!("Not installed") };
        rows.push((Icon::Folder, tr!("Store").into(), format!("{} \u{b7} {place}", f.library)));
        if let Some(score) = f.score.filter(|s| *s > 0.) {
            let from = f.score_source.as_ref().map_or(String::new(), |s| format!(" \u{b7} {s}"));
            rows.push((Icon::Star, tr!("Critic score").into(), format!("{score:.0}/100{from}")));
        }
        if let Some(main) = f.hltb.as_ref().and_then(|h| h.main).filter(|h| *h > 0.) {
            rows.push((
                Icon::Flag,
                tr!("Time to beat").into(),
                tr!("Main story {hours}", hours = heronite_data::hltb::format_hours(main as f64)),
            ));
        }
        rows
    }

    /// The link buttons under Details: the install folder, the store page
    /// and How Long To Beat, as far as the game has them.
    fn spotlight_links(&self, g: &CollectionGame) -> Vec<Link> {
        let mut links = Vec::new();
        if g.installed {
            links.push(Link::Folder);
        }
        if details_url(g).is_some() {
            links.push(Link::Store);
        }
        let hltb = self.focus.as_ref().filter(|f| f.key.as_ref() == g.key).and_then(|f| f.hltb.as_ref()?.link.clone());
        if let Some(url) = hltb {
            links.push(Link::Url { label: "HowLongToBeat".into(), url: url.to_string() });
        }
        links
    }

    /// The game's achievements: unlocked, total, and whether all are.
    fn spotlight_ach_counts(&self, g: &CollectionGame, cx: &mut Context<Self>) -> Option<(usize, usize, bool)> {
        if !crate::services::achievements::enabled() {
            return None;
        }
        let service = crate::services::achievements::get(cx);
        let service = service.read(cx);
        let set = service.entry(&crate::services::achievements::key_of(&g.runner, &g.app_name))?.primary()?;
        (set.total() > 0).then(|| (set.unlocked(), set.total(), set.is_complete()))
    }

    /// The icons of the latest unlocks (at most three), `size` design px.
    fn spotlight_ach_icons(&mut self, g: &CollectionGame, size: f32, cx: &mut Context<Self>) -> Vec<Option<ImageSource>> {
        let urls: Vec<String> = {
            let service = crate::services::achievements::get(cx);
            let service = service.read(cx);
            match service.entry(&crate::services::achievements::key_of(&g.runner, &g.app_name)).and_then(|e| e.primary()) {
                Some(set) => set.recent().into_iter().take(3).map(|a| a.icon_now().to_string()).collect(),
                None => Vec::new(),
            }
        };
        let scale = kit::scale() * 1.5;
        urls.iter().map(|u| self.achievement_icon(u, size, scale)).collect()
    }

    /// The buttons of the actions row for `g`.
    fn spotlight_actions(&self, g: &CollectionGame, focused: Option<usize>) -> Div {
        let (label, play_icon) = self.primary_state(g);
        let buttons = actions().iter().enumerate().map(|(i, act)| {
            let on = focused == Some(i);
            let id = ("spot-act", i);
            let el = match act {
                SpotAct::Play => button(id, on, play_icon, Some(label.into()), None, true),
                SpotAct::Options => button(id, on, Icon::MoreHoriz, Some(tr!("Options").into()), None, false),
                SpotAct::Status => {
                    let name = self.console_status(g).map_or_else(|| tr!("Status").to_string(), |(name, _)| name);
                    button(id, on, Icon::Flag, Some(name.into()), None, false)
                }
                SpotAct::Favourite if g.favourite => button(id, on, Icon::Favorite, None, Some(rgb(STAR)), false),
                SpotAct::Favourite => button(id, on, Icon::FavoriteBorder, None, None, false),
            };
            div()
                .flex_none()
                .on_mouse_down(MouseButton::Left, self.cover_click(move |this| this.spotlight_focus(Spot::Action(i))))
                .child(el)
        });
        div().flex().items_center().gap(u(12.)).h(u(BUTTON_H)).children(buttons)
    }

    /// Moves the focus to `spot` (a click); `false` when it was there already.
    fn spotlight_focus(&mut self, spot: Spot) -> bool {
        let (tab, _) = self.console_shown_tab();
        let nav = &mut self.tab_nav(tab).spotlight;
        let before = (nav.zone, nav.action, nav.media, nav.mid, nav.link, nav.card, nav.shot, nav.related);
        match spot {
            Spot::Action(i) => (nav.zone, nav.action) = (Zone::Actions, i),
            Spot::Media(i) => (nav.zone, nav.media) = (Zone::Media, i),
            Spot::Mid(m, i) => (nav.zone, nav.mid, nav.link) = (Zone::Mid, m, if m == Mid::Link { i } else { nav.link }),
            Spot::Card(i) => (nav.zone, nav.card) = (Zone::Cards, i),
            Spot::Shot(i) => (nav.zone, nav.shot) = (Zone::Shots, i),
            Spot::Related(i) => (nav.zone, nav.related) = (Zone::Related, i),
        }
        before != (nav.zone, nav.action, nav.media, nav.mid, nav.link, nav.card, nav.shot, nav.related)
    }

    /// The click of something of the page.
    fn spot_click(&self, spot: Spot) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        self.cover_click(move |this| this.spotlight_focus(spot))
    }

    /// Applies `HERONITE_DEBUG_SPOTLIGHT` once the page has the section.
    fn spotlight_debug(&mut self, tab: usize, found: usize, cx: &mut Context<Self>) {
        let Some((zone, n)) = debug_target() else { return };
        let nav = &mut self.tab_nav(tab).spotlight;
        if let Some(n) = n.filter(|n| *n < found && nav.index != *n && !nav.shown) {
            nav.index = n;
            nav.pos.snap(n as f32);
            return;
        }
        let Some((z, mid)) = Zone::parse(&zone) else {
            DEBUG_DONE.store(true, Ordering::Relaxed);
            return;
        };
        if !nav.shape.zones().contains(&z) {
            return;
        }
        if let Some(m) = mid {
            let has = match m {
                Mid::About => nav.shape.about_more,
                Mid::Ach => nav.shape.ach,
                Mid::Link => !nav.shape.links.is_empty(),
            };
            if !has {
                return;
            }
            nav.mid = m;
        }
        nav.zone = z;
        match zone.as_str() {
            "viewer" if !nav.shape.media_shots => nav.viewer = Some(0),
            "sheet" => nav.sheet = Some(Glide::new(Axis::Y)),
            _ => {}
        }
        DEBUG_DONE.store(true, Ordering::Relaxed);
        cx.notify();
    }

    pub(super) fn render_spotlight(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let found = self.spotlight_keys(spec);
        if found.is_empty() {
            return super::no_games();
        }
        let n = found.len();
        let k = kit::scale();
        let size = crate::ui::content_size(window);
        let (view_w, view_h) = (f32::from(size.width) / k, f32::from(size.height) / k);
        self.spotlight_debug(tab, n, cx);
        let nav = &mut self.tab_nav(tab).spotlight;
        nav.index = nav.index.min(n);
        let index = nav.index;
        let game_key = found.get(index).cloned();
        self.console_focus_game(Some(game_key.clone().unwrap_or_else(|| found[0].clone())), cx);
        let game = game_key.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).cloned();
        let shape = match &game {
            Some(g) => self.spotlight_shape(g, view_w, tab, cx),
            None => Shape::default(),
        };
        let (off, on) = row::icon_sizes(self.cover_aspect);
        let nav = &mut self.tab_nav(tab).spotlight;
        nav.shape = shape.clone();
        if game.is_none() {
            nav.zone = Zone::Row;
        }
        nav.settle();
        let zone = nav.zone;

        // The springs, a frame towards the focus.
        let first = shape.rise(Zone::Actions, view_h, TOP_BAR).max(1.);
        let rise_to = if game.is_some() { shape.rise(zone, view_h, TOP_BAR) } else { 0. };
        let width = shape.geo.width;
        let strip_to = [
            page::strip_offset(nav.media, shape.media, page::TILE_W, page::TILE_W + page::STRIP_GAP, width),
            page::strip_offset(nav.shot, shape.shots, page::SHOT_W, page::SHOT_W + page::STRIP_GAP, width),
            page::strip_offset(nav.related, shape.related.len(), page::COVER_W, page::COVER_W + page::STRIP_GAP, width),
        ];
        let targets = [(nav.index as f32, 0usize), (if zone == Zone::Row { 1. } else { 0. }, 1), (rise_to, 2)];
        for (t, i) in targets {
            let f = match i {
                0 => &mut nav.pos,
                1 => &mut nav.ring,
                _ => &mut nav.rise,
            };
            if nav.shown {
                f.go(t);
            } else {
                f.snap(t);
            }
        }
        for (f, t) in nav.strips.iter_mut().zip(strip_to) {
            if nav.shown {
                f.go(t);
            } else {
                f.snap(t);
            }
        }
        nav.shown = true;
        let dt = nav.clock.dt();
        let mut moving = nav.pos.step(GLIDE, dt) | nav.ring.step(motion::SOFT, dt) | nav.rise.step(RISE, dt);
        for f in nav.strips.iter_mut() {
            moving |= f.step(motion::GENTLE, dt);
        }
        nav.clock.done(moving, window);
        let rise = nav.rise.value.max(0.);
        let lift = (rise / first).clamp(0., 1.);
        nav.lift = lift;
        let deep = ((rise - first) / 160.).clamp(0., 1.);
        let strips = [nav.strips[0].value, nav.strips[1].value, nav.strips[2].value];
        let look = Look { off, on, pos: nav.pos.value, ring: nav.ring.value.clamp(0., 1.) };
        let from = nav.from;
        let focus = Focus {
            zone,
            action: nav.action.min(actions().len() - 1),
            media: nav.media,
            mid: nav.mid,
            link: nav.link,
            card: nav.card,
            shot: nav.shot,
            related: nav.related,
        };
        let viewer = nav.viewer;
        let sheet = nav.sheet.clone();

        // The presses that bring the focus back up to the row (for its clicks).
        let to_row = match zone {
            Zone::Row => Vec::new(),
            Zone::Actions => vec![Pad::B],
            _ => vec![Pad::B, Pad::B],
        };
        let mut page = div().absolute().left_0().right_0().top(u(-rise)).h(u(shape.geo.end.max(view_h)));
        if lift < 0.99 {
            let row = self.spotlight_row(&found, index, &look, &to_row, window);
            page = page.child(div().absolute().top_0().left_0().right_0().opacity(1. - lift).child(row));
        }
        match &game {
            Some(g) => {
                page = page.children(self.spotlight_head(g, &shape, &focus, from, lift, strips[0], window));
                if lift > 0.01 {
                    page = page.children(self.spotlight_below(g, &shape, &focus, strips, window, cx));
                }
            }
            None => page = page.child(self.spotlight_all_games(from)),
        }
        // The art darkens as the page rises, more deeper in it.
        let dim = (lift > 0.005).then(|| {
            div().absolute().top(u(-TOP_BAR)).left_0().right_0().bottom_0().bg(linear_gradient(
                0.,
                linear_color_stop(kit::dim(0.35 * lift + 0.4 * deep), 0.),
                linear_color_stop(kit::dim(0.18 * lift + 0.3 * deep), 1.),
            ))
        });
        // Deep in the page, what rises past the top fades into the dark.
        let top_fade = (deep > 0.005).then(|| {
            div().absolute().top(u(-TOP_BAR)).left_0().right_0().h(u(110.)).bg(linear_gradient(
                180.,
                linear_color_stop(kit::dim(0.75 * deep), 0.),
                linear_color_stop(kit::dim(0.), 1.),
            ))
        });
        let mut root = div().relative().size_full().children(dim).child(page).children(top_fade);
        if let (Some(i), false) = (viewer, shape.media_shots) {
            root = root.child(self.spotlight_viewer(&shape.store, i, window));
        } else if let Some(scroll) = &sheet {
            let title = game.as_ref().map_or(String::new(), |g| g.title.clone());
            let text = self.tab_nav(tab).spotlight.about.as_ref().filter(|(k, _)| *k == shape.key).map(|(_, t)| t.clone());
            if let Some(text) = text {
                root = root.child(self.spotlight_about_sheet(&title, &text, scroll, window));
            }
        } else if let Some(v) = self.shot_viewer() {
            let hints = self.shot_viewer_hints().unwrap_or_default();
            let layer = div().relative().size_full().child(v).child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(kit::hint_bar(&hints).bg(transparent_black()).border_color(transparent_black())),
            );
            root = root.child(kit::window_layer(window, layer));
        }
        root.into_any_element()
    }

    /// What the "All games" icon opens, in place of a game's head.
    fn spotlight_all_games(&self, from: f32) -> AnyElement {
        let total = self.lib.as_ref().map_or(0, |l| l.games.len());
        let (_, on) = row::icon_sizes(self.cover_aspect);
        let main = div()
            .absolute()
            .top(u(row::row_height(on) + 40.))
            .left(u(SIDE))
            .flex()
            .flex_col()
            .gap(u(8.))
            .child(kit::title(tr!("All games"), 54.))
            .child(div().text_size(u(17.)).text_color(theme::text_muted()).child(trn!(total, "{n} game", "{n} games")))
            .child(kit::body_text(tr!("Press A to see them all.")));
        motion::enter_from("spot-main-all", main, 300, 16. * from, 0., 0.25)
    }

    /// The logo (or name), the buttons, the facts and genres, and the media strip.
    #[allow(clippy::too_many_arguments)]
    fn spotlight_head(
        &mut self,
        g: &CollectionGame,
        shape: &Shape,
        f: &Focus,
        from: f32,
        lift: f32,
        strip_x: f32,
        window: &Window,
    ) -> Vec<AnyElement> {
        let geo = shape.geo;
        let size = HeroSize { title: 54., logo: (600., geo.logo_h), max_w: 760., overview: 0 };
        let name = self.console_name(g, size, window);
        let key = &g.key;
        let logo = div().absolute().top(u(geo.logo)).left(u(SIDE)).w(u(760.)).h(u(geo.logo_h)).flex().items_end().child(
            motion::enter_from(SharedString::from(format!("spot-logo-{key}")), div().relative().child(name), 360, 22. * from, 0., 0.2),
        );
        let actions = (f.zone == Zone::Actions).then_some(f.action);
        let rest = div()
            .absolute()
            .top(u(geo.buttons))
            .left(u(SIDE))
            .flex()
            .flex_col()
            .child(self.spotlight_actions(g, actions))
            .child(div().mt(u(18.)).child(self.spotlight_facts(g)))
            .child(div().mt(u(10.)).child(parts::genre_chips(g)));
        let rest = motion::enter_from(SharedString::from(format!("spot-rest-{key}")), rest, 420, 0., 8., 0.);
        let mut out = vec![logo.into_any_element(), rest];
        if shape.media > 0 {
            out.push(self.spotlight_media(shape, f, lift, strip_x, window));
        }
        out
    }

    /// The media strip: the store's screenshots, else the user's.
    fn spotlight_media(&mut self, shape: &Shape, f: &Focus, lift: f32, strip_x: f32, window: &Window) -> AnyElement {
        let geo = shape.geo;
        let dpr = window.scale_factor();
        let k = kit::scale();
        let (w, h) = (page::TILE_W, page::TILE_H);
        let step = w + page::STRIP_GAP;
        let first = (strip_x / step).floor().max(0.) as usize;
        let last = ((strip_x + geo.width) / step).ceil() as usize + 2;
        let mut tiles = Vec::new();
        for i in first..shape.media.min(last) {
            let on = f.zone == Zone::Media && f.media == i;
            let face = if shape.media_shots {
                self.shot_thumb(i, w, h, false).into_any_element()
            } else {
                let thumb = self.store_media_of(&shape.key).and_then(|m| m.screenshots.get(i).map(|s| s.thumb.clone()));
                let src = thumb.and_then(|t| viewer::store_image(self, &t, (w * k * dpr) as u32, (h * k * dpr) as u32, 120 - i as i32));
                div()
                    .size_full()
                    .rounded(u(12.))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
                    .into_any_element()
            };
            tiles.push(
                page_tile(("spot-media", i), i as f32 * step, w, h, 12., on, face)
                    .on_mouse_down(MouseButton::Left, self.spot_click(Spot::Media(i))),
            );
        }
        // On the row the strip dissolves downwards: there is more below.
        let fade = (1. - lift).clamp(0., 1.);
        let veil = (fade > 0.01).then(|| {
            div().absolute().left_0().right_0().top(u(STRIP_ROOM + h * 0.2)).h(u(h * 0.8)).bg(linear_gradient(
                180.,
                linear_color_stop(kit::dim(0.), 0.),
                linear_color_stop(kit::dim(0.85 * fade), 1.),
            ))
        });
        // It runs to the window's right edge.
        let clip = f32::from(crate::ui::content_size(window).width) / k - SIDE;
        strip(geo.media, clip, h, strip_x, tiles).children(veil).into_any_element()
    }

    /// The sections under the head, while the page has risen.
    fn spotlight_below(
        &mut self,
        g: &CollectionGame,
        shape: &Shape,
        f: &Focus,
        strips: [f32; 3],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let geo = shape.geo;
        let mut out = Vec::new();
        if geo.mid_h > 0. {
            out.push(self.spotlight_mid(g, shape, f, cx));
        }
        if shape.cards > 0 {
            let cards = self.spotlight_cards(g);
            let row =
                div().absolute().top(u(geo.cards)).left(u(SIDE)).flex().gap(u(18.)).children(cards.iter().enumerate().map(|(i, c)| {
                    div().flex_none().on_mouse_down(MouseButton::Left, self.spot_click(Spot::Card(i))).child(card(
                        i,
                        f.zone == Zone::Cards && f.card == i,
                        c,
                    ))
                }));
            out.push(row.into_any_element());
        }
        if shape.shots > 0 {
            let (w, h) = (page::SHOT_W, page::SHOT_H);
            let step = w + page::STRIP_GAP;
            let tiles: Vec<Div> = (0..shape.shots)
                .map(|i| {
                    let face = self.shot_thumb(i, w, h, false).into_any_element();
                    page_tile(("spot-shot", i), i as f32 * step, w, h, 12., f.zone == Zone::Shots && f.shot == i, face)
                        .on_mouse_down(MouseButton::Left, self.spot_click(Spot::Shot(i)))
                })
                .collect();
            out.push(titled(geo.shots, geo.width, tr!("Your screenshots"), strip(0., geo.width, h, strips[1], tiles)));
        }
        if !shape.related.is_empty() {
            let w = page::COVER_W;
            let h = w * self.cover_aspect;
            let step = w + page::STRIP_GAP;
            let dpr = window.scale_factor();
            let k = kit::scale();
            let tiles: Vec<Div> = shape
                .related
                .iter()
                .enumerate()
                .map(|(i, key)| {
                    let src = self.cover(key, w * k, h * k, dpr, 60 - i as i32);
                    let title = self.lib.as_ref().and_then(|l| l.get(key)).map_or(String::new(), |g| g.title.clone());
                    let face = div()
                        .size_full()
                        .rounded(u(10.))
                        .overflow_hidden()
                        .bg(kit::ink(0.08))
                        .child(match src {
                            Some(s) => img(s).size_full().object_fit(ObjectFit::Cover).into_any_element(),
                            None => div()
                                .size_full()
                                .p(u(8.))
                                .flex()
                                .items_end()
                                .text_size(u(12.))
                                .text_color(theme::text_primary())
                                .child(title)
                                .into_any_element(),
                        })
                        .into_any_element();
                    page_tile(("spot-related", i), i as f32 * step, w, h, 10., f.zone == Zone::Related && f.related == i, face)
                        .on_mouse_down(MouseButton::Left, self.spot_click(Spot::Related(i)))
                })
                .collect();
            out.push(titled(geo.related, geo.width, tr!("More like this"), strip(0., geo.width, h, strips[2], tiles)));
        }
        out
    }

    /// About this game on the left; Achievements and Details on the right.
    fn spotlight_mid(&mut self, g: &CollectionGame, shape: &Shape, f: &Focus, cx: &mut Context<Self>) -> AnyElement {
        let geo = shape.geo;
        let about_w = page::about_width(geo.width);
        let on = |m: Mid| f.zone == Zone::Mid && f.mid == m;
        let mut left = div().w(u(about_w)).flex().flex_col();
        if shape.about {
            let text = self
                .console
                .tab_navs
                .iter()
                .find_map(|n| n.spotlight.about.as_ref().filter(|(k, _)| *k == shape.key).map(|(_, t)| t.clone()))
                .unwrap_or_default();
            left = left.child(section_title(tr!("About this game"))).child(
                div()
                    .text_size(u(page::ABOUT_SIZE))
                    .line_height(u(page::LINE_H))
                    .text_color(theme::text_primary())
                    .line_clamp(shape.about_lines)
                    .child(text),
            );
            if shape.about_more {
                let focused = on(Mid::About);
                let more = motion::spring("spot-more", div().relative().flex_none().h(u(page::MORE_H)), motion::SOFT, focused, |d, k| {
                    let t = k.clamp(0., 1.);
                    let fill: Hsla = motion::mix(parts::glass(0.4), theme::console_focus(), t).into();
                    d.child(div().absolute().inset_0().rounded_full().bg(fill).border_1().border_color(kit::ink(0.14 * (1. - t)))).child(
                        div()
                            .relative()
                            .h_full()
                            .flex()
                            .items_center()
                            .px(u(16.))
                            .text_size(u(13.))
                            .font_weight(theme::W_BOLD)
                            .text_color(motion::mix(theme::text_primary(), theme::console_on_focus(), t))
                            .child(tr!("Read more")),
                    )
                });
                left = left.child(
                    div()
                        .mt(u(12.))
                        .flex()
                        .child(div().on_mouse_down(MouseButton::Left, self.spot_click(Spot::Mid(Mid::About, 0))).child(more)),
                );
            }
        }
        let mut column = div().w(u(page::COLUMN_W)).flex().flex_col().gap(u(26.));
        if shape.ach {
            column = column.child(div().flex().flex_col().child(section_title(tr!("Achievements"))).child(self.spotlight_ach_card(
                g,
                on(Mid::Ach),
                cx,
            )));
        }
        let details = self.spotlight_details(g);
        if !details.is_empty() || !shape.links.is_empty() {
            let rows = details.into_iter().enumerate().map(|(i, (ic, label, value))| {
                div()
                    .flex()
                    .items_center()
                    .h(u(page::DETAIL_H))
                    .when(i > 0, |d| d.border_t_1().border_color(kit::ink(0.1)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(u(9.))
                            .w(u(150.))
                            .flex_none()
                            .text_size(u(12.5))
                            .text_color(theme::text_muted())
                            .child(crate::ui::icon(ic, u(13.), theme::text_muted()))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(u(12.5))
                            .font_weight(theme::W_BOLD)
                            .text_color(kit::strong())
                            .child(value),
                    )
            });
            let links = div().flex().gap(u(8.)).mt(u(14.)).children(shape.links.iter().enumerate().map(|(i, l)| {
                let label: SharedString = match l {
                    Link::Folder => tr!("Open folder").into(),
                    Link::Store => tr!("Store page").into(),
                    Link::Url { label, .. } => label.clone(),
                };
                let focused = f.zone == Zone::Mid && f.mid == Mid::Link && f.link == i;
                let base = div().relative().flex_none().h(u(page::LINK_H)).min_w(u(120.));
                let el = motion::spring(("spot-link", i), base, motion::SOFT, focused, move |d, k| {
                    let t = k.clamp(0., 1.);
                    let fill: Hsla = motion::mix(parts::glass(0.4), theme::console_focus(), t).into();
                    d.child(div().absolute().inset_0().rounded(u(9.)).bg(fill).border_1().border_color(kit::ink(0.14 * (1. - t)))).child(
                        div()
                            .relative()
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px(u(14.))
                            .text_size(u(12.5))
                            .font_weight(theme::W_BOLD)
                            .whitespace_nowrap()
                            .text_color(motion::mix(theme::text_primary(), theme::console_on_focus(), t))
                            .child(label),
                    )
                });
                div().on_mouse_down(MouseButton::Left, self.spot_click(Spot::Mid(Mid::Link, i))).child(el)
            }));
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .child(section_title(tr!("Details")))
                    .children(rows)
                    .when(!shape.links.is_empty(), |d| d.child(links)),
            );
        }
        div()
            .absolute()
            .top(u(geo.mid))
            .left(u(SIDE))
            .w(u(geo.width))
            .flex()
            .gap(u(page::COLUMN_GAP))
            .child(left)
            .child(column)
            .into_any_element()
    }

    /// The Achievements card: a trophy, unlocked of total, a bar, the latest unlocks.
    fn spotlight_ach_card(&mut self, g: &CollectionGame, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let (unlocked, total, complete) = self.spotlight_ach_counts(g, cx).unwrap_or((0, 0, false));
        let icons = self.spotlight_ach_icons(g, 40., cx);
        let gold: Rgba = theme::achievement_gold();
        let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
        let note = if complete { tr!("All achievements").to_string() } else { tr!("{percent}% complete", percent = pct) };
        let trophy = div()
            .flex_none()
            .size(u(46.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(if complete { Hsla { a: 0.18, ..gold.into() } } else { kit::ink(0.1) })
            .child(crate::ui::icon(Icon::Trophy, u(24.), if complete { gold } else { theme::text_primary() }));
        let recent = div().flex().gap(u(6.)).children(icons.into_iter().map(|src| {
            div()
                .size(u(40.))
                .rounded(u(8.))
                .overflow_hidden()
                .bg(kit::ink(0.1))
                .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover)))
        }));
        let content = move |t: f32| {
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .gap(u(14.))
                .px(u(16.))
                .child(trophy)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(u(3.))
                        .child(
                            div()
                                .text_size(u(17.))
                                .font_weight(theme::W_BOLD)
                                .text_color(kit::strong())
                                .child(format!("{unlocked}/{total}")),
                        )
                        .child(div().text_size(u(12.)).text_color(theme::text_muted()).child(if t > 0.5 {
                            tr!("View all").to_string()
                        } else {
                            note.clone()
                        }))
                        .when(!complete, |d| {
                            d.child(div().mt(u(3.)).h(u(4.)).w(u(140.)).rounded_full().bg(kit::ink(0.15)).child(
                                div().h_full().w(relative((pct as f32 / 100.).clamp(0.02, 1.))).rounded_full().bg(theme::console_focus()),
                            ))
                        }),
                )
                .child(recent)
        };
        let base = div().relative().w_full().h(u(page::ACH_H));
        let el = motion::spring("spot-ach", base, motion::SOFT, focused, move |d, k| {
            let t = k.clamp(0., 1.);
            d.child(
                div().absolute().inset_0().rounded(u(14.)).bg(kit::ink(0.06 + 0.06 * t)).border_1().border_color(kit::ink(0.16 + 0.1 * t)),
            )
            .child(content(t))
            .when(t > 0.01, |d| d.child(ring(14., t)))
        });
        div().on_mouse_down(MouseButton::Left, self.spot_click(Spot::Mid(Mid::Ach, 0))).child(el).into_any_element()
    }

    pub(super) fn spotlight_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let found = self.spotlight_keys(spec);
        if found.is_empty() {
            return false;
        }
        let n = found.len();
        let wrap = look::theme().nav.wrap;
        // What is open over the page takes every press.
        let nav = &mut self.tab_nav(tab).spotlight;
        if let Some(i) = nav.viewer {
            let count = nav.shape.store.len().max(1);
            match pad {
                Pad::Left | Pad::Right => {
                    let next = super::step(i, count, pad.dir(), true);
                    nav.viewer = Some(next);
                    nav.media = next;
                }
                Pad::A | Pad::B => nav.viewer = None,
                _ => {}
            }
            return true;
        }
        if let Some(scroll) = nav.sheet.clone() {
            if !Self::spotlight_sheet_pad(&scroll, pad) {
                nav.sheet = None;
            }
            return true;
        }
        if self.shot_viewer_pad(pad) {
            return true;
        }
        let nav = &self.tab_nav(tab).spotlight;
        let index = nav.index.min(n);
        let game = found.get(index).cloned();
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
        // Presses faster than frames: the page of a game not drawn yet has
        // only its buttons.
        let drawn = game.as_ref() == Some(&nav.shape.key);
        if game.is_none() {
            nav.zone = Zone::Row;
        } else if !drawn && !matches!(nav.zone, Zone::Row | Zone::Actions) {
            nav.zone = Zone::Actions;
        }
        let zones = if drawn { nav.shape.zones() } else { vec![Zone::Row, Zone::Actions] };
        let at = zones.iter().position(|z| *z == nav.zone).unwrap_or(0);
        let next = zones.get(at + 1).copied();
        let prev = at.checked_sub(1).map(|i| zones[i]);
        match (nav.zone, pad) {
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
            (Zone::Row, _) => return false,
            (Zone::Actions, Pad::Left) => nav.action = nav.action.saturating_sub(1),
            (Zone::Actions, Pad::Right) => nav.action = (nav.action + 1).min(actions().len() - 1),
            (Zone::Actions, Pad::Up | Pad::B) => nav.zone = Zone::Row,
            (Zone::Actions, Pad::A) => {
                let (Some(key), at) = (game, nav.action) else { return true };
                self.spotlight_act(&key, actions()[at.min(actions().len() - 1)], window, cx);
            }
            (_, Pad::B) => nav.zone = Zone::Actions,
            (Zone::Mid, Pad::Up | Pad::Down | Pad::Left | Pad::Right) => match nav.shape.mid_move(nav.mid, nav.link, pad) {
                MidMove::To(m, link) => (nav.mid, nav.link) = (m, link),
                MidMove::Out(d) if d < 0 => nav.enter(prev.unwrap_or(Zone::Actions), false),
                MidMove::Out(_) => {
                    if let Some(z) = next {
                        nav.enter(z, true);
                    }
                }
                MidMove::Stay => {}
            },
            (z, Pad::Down) => {
                if let Some(next) = next.filter(|_| z != Zone::Row) {
                    nav.enter(next, true);
                }
            }
            (_, Pad::Up) => nav.enter(prev.unwrap_or(Zone::Actions), false),
            (z, Pad::Left | Pad::Right) => {
                let count = nav.shape.count(z);
                if let Some(at) = nav.along(z) {
                    *at = super::step(*at, count, pad.dir(), false);
                }
            }
            (z, Pad::A) => {
                let Some(key) = game else { return true };
                self.spotlight_open(tab, &found, &key, z, window, cx);
            }
            _ => return false,
        }
        true
    }

    /// A on something of the page (`zone` has the focus).
    fn spotlight_open(
        &mut self,
        tab: usize,
        found: &[SharedString],
        key: &SharedString,
        zone: Zone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nav = &mut self.tab_nav(tab).spotlight;
        match zone {
            Zone::Media if nav.shape.media_shots => {
                let i = nav.media;
                self.shot_open(i, window, cx);
            }
            Zone::Media => nav.viewer = Some(nav.media),
            Zone::Shots => {
                let i = nav.shot;
                self.shot_open(i, window, cx);
            }
            Zone::Mid => match nav.mid {
                Mid::About => nav.sheet = Some(Glide::new(Axis::Y)),
                Mid::Ach => self.open_console_achievements(key.clone(), cx),
                Mid::Link => {
                    let Some(link) = nav.shape.links.get(nav.link).cloned() else { return };
                    match link {
                        Link::Folder => self.open_install_folder(key, cx),
                        Link::Store => self.open_details(key, cx),
                        Link::Url { url, .. } => self.open_url(url, cx),
                    }
                }
            },
            Zone::Cards => self.open_game_page(key.clone()),
            Zone::Related => {
                let Some(other) = nav.shape.related.get(nav.related).cloned() else { return };
                // In the row: the row goes to it; else its game page.
                match found.iter().position(|k| *k == other) {
                    Some(i) => {
                        nav.from = if i > nav.index { 1. } else { -1. };
                        nav.index = i;
                        nav.zone = Zone::Row;
                    }
                    None => self.open_game_page(other),
                }
            }
            Zone::Row | Zone::Actions => {}
        }
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
            SpotAct::Options => self.open_game_page(key.clone()),
        }
    }

    pub(super) fn spotlight_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        let Some(nav) = self.console.tab_navs.get(tab).map(|n| &n.spotlight) else { return Vec::new() };
        if nav.viewer.is_some() {
            return vec![hint(Btn::LeftRight, tr!("Browse")), hint(Btn::B, tr!("Close"))];
        }
        if nav.sheet.is_some() {
            return vec![hint(Btn::B, tr!("Close"))];
        }
        if let Some(h) = self.shot_viewer_hints() {
            return h;
        }
        let found = self.spotlight_keys(spec);
        let game = found.get(nav.index).and_then(|k| self.lib.as_ref()?.get(k));
        let primary = game.map_or(tr!("Play"), |g| self.primary_label(g));
        let a = match nav.zone {
            Zone::Row if game.is_some() => tr!("Actions"),
            Zone::Row => tr!("Open"),
            Zone::Actions => tr!("Select"),
            Zone::Media | Zone::Shots => tr!("View"),
            Zone::Mid if nav.mid == Mid::About => tr!("Read more"),
            Zone::Mid | Zone::Cards | Zone::Related => tr!("Open"),
        };
        let mut h = vec![hint(Btn::A, a)];
        if game.is_some() {
            h.push(hint(Btn::X, primary));
        }
        if nav.zone == Zone::Row {
            h.push(hint(Btn::Y, tr!("Search")));
            h.extend(self.console_tab_hints());
        } else {
            h.push(hint(Btn::B, tr!("Back")));
        }
        h
    }
}

/// The focus, copied for a frame's drawing.
struct Focus {
    zone: Zone,
    action: usize,
    media: usize,
    mid: Mid,
    link: usize,
    card: usize,
    shot: usize,
    related: usize,
}

/// A tile of a strip at `x` (`w`×`h`, corners `radius`): focused, it grows a
/// touch and wears the ring.
fn page_tile(id: (&'static str, usize), x: f32, w: f32, h: f32, radius: f32, focused: bool, face: AnyElement) -> Div {
    let base = div().relative().size_full();
    let el = motion::spring(id, base, motion::SOFT, focused, move |d, k| {
        let t = k.clamp(0., 1.);
        d.child(div().size_full().rounded(u(radius)).shadow(kit::shadow(0.4 + 0.2 * t, 6. + 6. * t, 16. + 12. * t)).child(face))
            .when(t > 0.01, |d| d.child(ring(radius, t)))
    });
    div().absolute().left(u(x)).top_0().w(u(w)).h(u(h)).child(el)
}

/// Room around a strip's tiles for the focus ring, design px.
const STRIP_ROOM: f32 = 10.;

/// A strip at `top` (`width` wide, tiles `h` tall) slid by `x`; it keeps
/// room around the tiles for the focus ring.
fn strip(top: f32, width: f32, h: f32, x: f32, tiles: Vec<Div>) -> Div {
    let room = STRIP_ROOM;
    div()
        .absolute()
        .top(u(top - room))
        .left(u(SIDE - room))
        .w(u(width + 2. * room))
        .h(u(h + 2. * room))
        .overflow_hidden()
        .child(div().absolute().top(u(room)).left(u(room - x)).h(u(h)).w(u(width + x + 400.)).children(tiles))
}

/// A section at `top` with its title over `body` (placed inside at the title's foot).
fn titled(top: f32, width: f32, title: &str, body: Div) -> AnyElement {
    div()
        .absolute()
        .top(u(top))
        .left_0()
        .right_0()
        .h(u(parts::TITLE_H + 400.))
        .child(div().absolute().top_0().left(u(SIDE)).w(u(width)).child(section_title(title)))
        .child(div().absolute().top(u(parts::TITLE_H)).left_0().right_0().child(body))
        .into_any_element()
}
