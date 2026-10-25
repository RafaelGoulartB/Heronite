//! Dashboard: a mosaic of tiles filling the body. A big "Continue" tile
//! with the latest game, its most recent neighbours as small tiles, and the
//! widgets the theme switches on (this week's play time, the download in
//! progress, a pick from the backlog, the latest journal note, shortcuts to
//! the Library, Statistics, Emulators and Stores). A widget that is off, or
//! has nothing to show, leaves its cells to older games. The d-pad moves to
//! the nearest tile in that direction ([`spatial`]).

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{DashboardWidgets, Layout, TabSpec};
use heronite_data::journal::{self, GameJournal};
use heronite_data::models::LocalGameSession;
use heronite_data::{stats, CollectionGame, Library};
use heronite_i18n::{tr, trn};

use super::super::super::CollectionView;
use super::super::hero::{played_line, HeroSize};
use super::super::tile::focus_frame;
use super::top_recent;
use crate::collection::data::played_ago;
use crate::console::kit::{self, u};
use crate::console::spatial::{self, Dir, Rect};
use crate::console::{hint, look, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;

const COLS: f32 = 6.;
const ROWS: f32 = 3.;
const GAP: f32 = 14.;
const SIDE: f32 = 46.;
/// Space above and under the mosaic.
const TOP: f32 = 12.;
const BOTTOM: f32 = 22.;
/// Recent games the mosaic and the digest look at.
const RECENT: usize = 16;
/// Games whose journal is searched for the newest note.
const JOURNAL_GAMES: usize = 6;
const DARK: u32 = 0x0d1117;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shortcut {
    Library,
    Stats,
    Emulators,
    Stores,
}

/// A tile of the mosaic. `Game(n)` is the n-th small game tile.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Id {
    #[default]
    Continue,
    Game(u8),
    Week,
    Downloads,
    Random,
    Journal,
    Shortcut(Shortcut),
}

impl Id {
    /// Tiles that stand for a game (A and X act on it).
    fn is_game(self) -> bool {
        matches!(self, Id::Continue | Id::Game(_) | Id::Random | Id::Journal)
    }
}

struct Tile<K> {
    id: Id,
    /// In cells of the 6×3 mosaic.
    rect: Rect,
    /// The game it shows or acts on.
    key: Option<K>,
}

/// The tiles for `games` (most recent first; the first is "Continue") and
/// the widgets that are on. `random` and `journal` are the games those two
/// widgets show, `None` = nothing to show, so their cells go to games.
fn plan<K: Clone>(show: &DashboardWidgets, mut games: impl Iterator<Item = K>, random: Option<K>, journal: Option<K>) -> Vec<Tile<K>> {
    let Some(first) = games.next() else { return Vec::new() };
    let mut out = vec![Tile { id: Id::Continue, rect: Rect::new(0., 0., 2., 2.), key: Some(first) }];
    let mut n = 0u8;
    let mut game = |out: &mut Vec<Tile<K>>, x: f32, y: f32| {
        if let Some(key) = games.next() {
            out.push(Tile { id: Id::Game(n), rect: Rect::new(x, y, 1., 1.), key: Some(key) });
            n += 1;
        }
    };
    let widget = |out: &mut Vec<Tile<K>>, id: Id, rect: Rect, key: Option<K>| out.push(Tile { id, rect, key });
    for (x, y) in [(2., 0.), (3., 0.), (2., 1.), (3., 1.)] {
        game(&mut out, x, y);
    }
    if show.week {
        widget(&mut out, Id::Week, Rect::new(4., 0., 1., 1.), None);
    } else {
        game(&mut out, 4., 0.);
    }
    if show.downloads {
        widget(&mut out, Id::Downloads, Rect::new(5., 0., 1., 1.), None);
    } else {
        game(&mut out, 5., 0.);
    }
    match random.filter(|_| show.random) {
        Some(key) => widget(&mut out, Id::Random, Rect::new(4., 1., 2., 1.), Some(key)),
        None => {
            game(&mut out, 4., 1.);
            game(&mut out, 5., 1.);
        }
    }
    if show.shortcuts {
        widget(&mut out, Id::Shortcut(Shortcut::Library), Rect::new(0., 2., 1., 1.), None);
        widget(&mut out, Id::Shortcut(Shortcut::Stats), Rect::new(1., 2., 1., 1.), None);
    } else {
        game(&mut out, 0., 2.);
        game(&mut out, 1., 2.);
    }
    match journal.filter(|_| show.journal) {
        Some(key) => widget(&mut out, Id::Journal, Rect::new(2., 2., 2., 1.), Some(key)),
        None => {
            game(&mut out, 2., 2.);
            game(&mut out, 3., 2.);
        }
    }
    if show.shortcuts {
        widget(&mut out, Id::Shortcut(Shortcut::Emulators), Rect::new(4., 2., 1., 1.), None);
        widget(&mut out, Id::Shortcut(Shortcut::Stores), Rect::new(5., 2., 1., 1.), None);
    } else {
        game(&mut out, 4., 2.);
        game(&mut out, 5., 2.);
    }
    out
}

/// What the background job reads off the library once: the last seven
/// days of play and the newest journal note.
struct Digest {
    /// The library it was made from (its address), to tell a reload.
    lib: usize,
    day: NaiveDate,
    /// Seconds played, oldest day first; the last is today.
    week: [f64; 7],
    note: Option<Note>,
}

#[derive(Clone)]
struct Note {
    key: SharedString,
    text: String,
    /// RFC 3339, when it was written.
    at: String,
}

#[derive(Default)]
pub(crate) struct Nav {
    focus: Id,
    digest: Option<Digest>,
    loading: bool,
    /// Redraws the tab while a download moves.
    downloads: Option<Subscription>,
}

/// Seconds played on each of the seven days ending `today`, oldest first
/// (a session counts on the day it started; failed launches do not count).
fn week_secs<'a, Tz: TimeZone>(sessions: impl Iterator<Item = &'a LocalGameSession>, today: NaiveDate, tz: &Tz) -> [f64; 7] {
    let oldest = today - Duration::days(6);
    // The date in the text is at most a day off the local one: a cheap
    // test before parsing a long history.
    let earliest = (oldest - Duration::days(1)).format("%Y-%m-%d").to_string();
    let mut days = [0.; 7];
    for s in sessions {
        let secs = s.elapsed_seconds.get();
        if !secs.is_finite() || secs < stats::MIN_SESSION_SECS || s.started_at.get(..10).is_some_and(|d| d < earliest.as_str()) {
            continue;
        }
        let Ok(at) = DateTime::parse_from_rfc3339(&s.started_at) else { continue };
        let day = at.with_timezone(tz).date_naive();
        if day >= oldest && day <= today {
            days[(day - oldest).num_days() as usize] += secs;
        }
    }
    days
}

/// The newest note of a game's journal (an entry with words, or "where I
/// left off") as (text, written at, unix ms).
fn newest_note(j: &GameJournal) -> Option<(String, String, i64)> {
    let entry = j
        .entries
        .iter()
        .filter(|e| !e.text.trim().is_empty())
        .map(|e| (e.text.trim().to_string(), e.at.clone(), e.at_ms().unwrap_or(0)))
        .max_by_key(|n| n.2);
    let left_off = j.left_off.as_ref().map(|l| {
        let ms = DateTime::parse_from_rfc3339(&l.at).map_or(0, |d| d.timestamp_millis());
        (l.text.trim().to_string(), l.at.clone(), ms)
    });
    [entry, left_off].into_iter().flatten().max_by_key(|n| n.2)
}

/// Runs off the UI thread: reads the sessions and a few journals.
fn build_digest(lib: &Library, id: usize, today: NaiveDate) -> Digest {
    let week = week_secs(lib.games.iter().flat_map(|g| g.sessions.iter().flat_map(|s| s.iter())), today, &Local);
    let played: Vec<Arc<CollectionGame>> = lib.games.iter().filter(|g| g.last_played.is_some()).cloned().collect();
    let note = top_recent(&played, JOURNAL_GAMES)
        .into_iter()
        .filter_map(|g| Some((g.key.clone(), newest_note(&journal::load(&lib.paths, &g.key))?)))
        .max_by_key(|(_, n)| n.2)
        .map(|(key, (text, at, _))| Note { key: key.into(), text, at });
    Digest { lib: id, day: today, week, note }
}

/// A game to pick from the backlog: "plan to play", or never played.
fn in_backlog(lib: &Library, g: &CollectionGame) -> bool {
    let plan_to_play = g.status_id == "plan-to-play" || lib.statuses.iter().any(|s| s.id == g.status_id && s.slug == "plan-to-play");
    plan_to_play || (g.playtime_minutes == 0 && g.last_played.is_none())
}

/// A number per game that changes every day, for the daily pick.
fn daily_rank(key: &str, day: u64) -> u64 {
    key.bytes()
        .fold(0xcbf2_9ce4_8422_2325 ^ day.wrapping_mul(0x9e37_79b9_7f4a_7c15), |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The index of the day's pick among `keys`: the same all day, another one tomorrow.
fn daily_pick<'a>(keys: impl Iterator<Item = &'a str>, day: u64) -> Option<usize> {
    keys.enumerate().min_by_key(|(_, k)| daily_rank(k, day)).map(|(i, _)| i)
}

/// Where the cells of the mosaic are, design px.
struct Cells {
    w: f32,
    h: f32,
}

impl Cells {
    fn new(window: &Window, hints: bool) -> Self {
        let body = super::body_height(window, hints);
        let width = f32::from(window.viewport_size().width) / kit::scale() - 2. * SIDE;
        Self { w: ((width - GAP * (COLS - 1.)) / COLS).max(60.), h: ((body - TOP - BOTTOM - GAP * (ROWS - 1.)) / ROWS).max(60.) }
    }

    /// (left, top, width, height) of a tile.
    fn place(&self, r: Rect) -> (f32, f32, f32, f32) {
        (SIDE + r.x * (self.w + GAP), TOP + r.y * (self.h + GAP), r.w * self.w + (r.w - 1.) * GAP, r.h * self.h + (r.h - 1.) * GAP)
    }
}

/// The download in progress.
struct Download {
    title: String,
    /// 0..=100.
    percent: Option<f64>,
    /// "1 active · 42%", "Paused · 42%", "3 queued".
    meta: Option<String>,
}

fn kicker_with(icon: Option<Icon>, text: &str) -> Div {
    div().flex().items_center().gap(u(6.)).children(icon.map(|i| crate::ui::icon(i, u(14.), theme::text_muted()))).child(kit::kicker(text))
}

/// A widget's base: a soft panel with padding.
fn panel() -> Div {
    div().relative().size_full().p(u(16.)).rounded(u(look::tile_radius())).overflow_hidden().bg(kit::ink(0.08)).flex().flex_col().gap(u(6.))
}

impl CollectionView {
    /// Reads the digest in the background the first time the tab shows and
    /// again when the library reloads or the day turns; keeps the tab
    /// redrawing while a download moves.
    fn dashboard_sync(&mut self, tab: usize, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let widgets = look::theme().layouts.dashboard.clone();
        let id = Arc::as_ptr(&lib) as usize;
        let today = Local::now().date_naive();
        let downloads = widgets.downloads.then(|| crate::services::try_downloads(cx)).flatten();
        let nav = &mut self.tab_nav(tab).dashboard;
        if nav.downloads.is_none() {
            nav.downloads = downloads.map(|d| {
                cx.observe(&d, |this, _, cx| {
                    let (_, spec) = this.console_tab_spec();
                    if crate::console::active() && spec.layout == Layout::Dashboard {
                        cx.notify();
                    }
                })
            });
        }
        let nav = &mut self.tab_nav(tab).dashboard;
        let fresh = nav.digest.as_ref().is_some_and(|d| d.lib == id && d.day == today);
        if nav.loading || fresh || !(widgets.week || widgets.journal) {
            return;
        }
        nav.loading = true;
        cx.spawn(async move |this, cx| {
            let digest = cx.background_spawn(async move { build_digest(&lib, id, today) }).await;
            this.update(cx, |this, cx| {
                if !crate::console::active() {
                    return;
                }
                let nav = &mut this.tab_nav(tab).dashboard;
                nav.loading = false;
                nav.digest = Some(digest);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The mosaic for the tab's games and the theme's widgets.
    fn dashboard_tiles(&mut self, tab: usize, spec: &TabSpec) -> Vec<Tile<SharedString>> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let widgets = look::theme().layouts.dashboard.clone();
        let games = self.console_games(&spec.source);
        let recent = top_recent(&games, RECENT);
        let note = self.tab_nav(tab).dashboard.digest.as_ref().and_then(|d| d.note.as_ref()).map(|n| n.key.clone());
        let note = note.filter(|k| lib.get(k).is_some());
        let day = Local::now().date_naive().num_days_from_ce() as u64;
        let random = widgets.random.then(|| {
            let pool: Vec<&Arc<CollectionGame>> =
                games.iter().filter(|g| in_backlog(&lib, g) && !recent.iter().any(|r| r.key == g.key)).collect();
            let i = daily_pick(pool.iter().map(|g| g.key.as_str()), day)?;
            Some(SharedString::from(pool[i].key.clone()))
        });
        plan(&widgets, recent.iter().map(|g| SharedString::from(g.key.clone())), random.flatten(), note)
    }

    fn dashboard_download(cx: &App) -> Option<Download> {
        let svc = crate::services::try_downloads(cx)?;
        let svc = svc.read(cx);
        let first = svc.snapshot().queue.first()?;
        let percent = svc.snapshot().current.as_ref().and_then(|(_, p)| p.as_ref()).map(|p| p.percent);
        Some(Download { title: first.params.title(), percent, meta: super::super::super::nav::downloads_meta(Some(svc)) })
    }

    pub(super) fn render_dashboard(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.dashboard_sync(tab, cx);
        let tiles = self.dashboard_tiles(tab, spec);
        if tiles.is_empty() {
            return super::no_games();
        }
        let nav = &mut self.tab_nav(tab).dashboard;
        if !tiles.iter().any(|t| t.id == nav.focus) {
            nav.focus = Id::Continue;
        }
        let focus = nav.focus;
        let week = nav.digest.as_ref().map(|d| d.week);
        let note = nav.digest.as_ref().and_then(|d| d.note.clone());
        let at = tiles.iter().position(|t| t.id == focus).unwrap_or(0);
        // Widgets without a game keep the background of the Continue game.
        self.console_focus_game(tiles[at].key.clone().or_else(|| tiles[0].key.clone()), cx);

        let download = Self::dashboard_download(cx);
        let cells = Cells::new(window, self.hints_visible());
        // The focused tile is drawn last, over its neighbours.
        let order = (0..tiles.len()).filter(|i| *i != at).chain([at]);
        let mut root = div().relative().size_full();
        for i in order {
            let t = &tiles[i];
            let (l, top, w, h) = cells.place(t.rect);
            let on = i == at;
            let body = match (t.id, t.key.as_ref()) {
                (Id::Continue, Some(k)) => self.dash_continue(k, w, h, on, window),
                (Id::Game(_), Some(k)) => self.dash_game(k, w, h, on, window),
                (Id::Random, Some(k)) => self.dash_random(k, w, h, on, window),
                (Id::Journal, Some(k)) => self.dash_journal(note.as_ref().filter(|n| &n.key == k), h),
                (Id::Week, _) => dash_week(week, h),
                (Id::Downloads, _) => dash_downloads(download.as_ref(), w),
                (Id::Shortcut(s), _) => self.dash_shortcut(s),
                _ => div(),
            };
            let id = SharedString::from(format!("dash-{:?}", t.id));
            root = root.child(div().absolute().left(u(l)).top(u(top)).w(u(w)).h(u(h)).child(focus_frame(
                id,
                on,
                w,
                h,
                body.relative().size_full(),
            )));
        }
        root.into_any_element()
    }

    /// A tile's picture (wide art, else the cover) with its rounded clip.
    fn dash_art(&mut self, key: &SharedString, w: f32, h: f32, on: bool, window: &Window) -> Div {
        let src = self.console_wide_art(key, w, h, if on { 200 } else { 50 }, window);
        div()
            .absolute()
            .inset_0()
            .rounded(u(look::tile_radius()))
            .overflow_hidden()
            .bg(kit::ink(0.07))
            .children(src.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
    }

    fn dash_game(&mut self, key: &SharedString, w: f32, h: f32, on: bool, window: &Window) -> Div {
        let game = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        let art = self.dash_art(key, w, h, on, window);
        let r = u(look::tile_radius());
        div()
            .child(art)
            .child(div().absolute().inset_0().rounded(r).bg(linear_gradient(
                0.,
                linear_color_stop(kit::veil(0.8), 0.),
                linear_color_stop(kit::veil(0.), 0.62),
            )))
            .children(game.map(|g| {
                div()
                    .absolute()
                    .left(u(12.))
                    .right(u(12.))
                    .bottom(u(10.))
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_size(u(15.)).font_weight(theme::W_BOLD).text_color(kit::strong()).child(g.title.clone()))
                    .children(
                        g.last_played
                            .as_deref()
                            .and_then(played_ago)
                            .map(|ago| div().truncate().text_size(u(12.)).text_color(theme::text_muted()).child(ago)),
                    )
            }))
    }

    fn dash_continue(&mut self, key: &SharedString, w: f32, h: f32, on: bool, window: &Window) -> Div {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return div() };
        let art = self.dash_art(key, w, h, on, window);
        let name = self.console_name(&game, HeroSize { title: 30., logo: (w * 0.7, 66.), ..HeroSize::default() }, window);
        let running = self.playing.contains_key(&game.app_name) || self.preparing.contains_key(&game.app_name);
        let label = if game.installed && !running { tr!("Continue") } else { self.primary_label(&game) };
        let line = played_line(&game, false);
        let r = u(look::tile_radius());
        let pill_ink: Rgba = if kit::light() { rgb(0xffffff) } else { rgb(DARK) };
        div()
            .child(art)
            .child(div().absolute().inset_0().rounded(r).bg(linear_gradient(
                0.,
                linear_color_stop(kit::veil(0.88), 0.),
                linear_color_stop(kit::veil(0.05), 0.75),
            )))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .p(u(22.))
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap(u(10.))
                    .child(kit::kicker(tr!("Continue")))
                    .child(name)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(u(12.))
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(u(6.))
                                    .h(u(38.))
                                    .px(u(16.))
                                    .rounded_full()
                                    .bg(if kit::light() { kit::ink(0.92) } else { white().opacity(0.92) })
                                    .text_size(u(15.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(pill_ink)
                                    .child(crate::ui::icon(Icon::PlayArrow, u(18.), pill_ink))
                                    .child(label),
                            )
                            .child(div().truncate().text_size(u(14.)).text_color(theme::text_muted()).child(line)),
                    ),
            )
    }

    fn dash_random(&mut self, key: &SharedString, w: f32, h: f32, on: bool, window: &Window) -> Div {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return div() };
        let ph = h - 24.;
        let pw = (ph / self.cover_aspect).min(w * 0.4);
        let cover = self.cover(key, pw * kit::scale(), ph * kit::scale(), window.scale_factor(), if on { 200 } else { 50 });
        let beat = self.wiki.hltb(&game.title, &game.game.title).map(|e| e.main_story).filter(|h| *h > 0.);
        let line = match beat {
            Some(hours) => tr!("About {hours} h to beat", hours = hours.round() as u32),
            None => tr!("Not played yet").to_string(),
        };
        panel()
            .flex_row()
            .items_center()
            .gap(u(16.))
            .py(u(12.))
            .px(u(12.))
            .child(
                div()
                    .flex_none()
                    .w(u(pw))
                    .h(u(ph))
                    .rounded(u(8.))
                    .overflow_hidden()
                    .bg(kit::ink(0.07))
                    .children(cover.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(u(5.))
                    .child(kicker_with(Some(Icon::AutoAwesome), tr!("Backlog pick")))
                    .child(
                        div()
                            .text_size(u(19.))
                            .line_height(u(23.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .line_clamp(2)
                            .child(game.title.clone()),
                    )
                    .child(div().truncate().text_size(u(13.)).text_color(theme::text_muted()).child(line)),
            )
    }

    fn dash_journal(&self, note: Option<&Note>, h: f32) -> Div {
        let Some(note) = note else { return panel() };
        let title = self.lib.as_ref().and_then(|l| l.get(&note.key)).map(|g| g.title.clone()).unwrap_or_default();
        let ago = played_ago(&note.at).map(|a| format!(" \u{b7} {a}")).unwrap_or_default();
        panel()
            .child(kicker_with(Some(Icon::Notes), tr!("Journal")))
            .child(
                div()
                    .text_size(u(17.))
                    .line_height(u(23.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .line_clamp(if h > 150. { 3 } else { 2 })
                    .child(format!("\u{201c}{}\u{201d}", super::super::hero::clip(&note.text, 140))),
            )
            .child(div().flex_1())
            .child(div().truncate().text_size(u(12.)).text_color(theme::text_muted()).child(format!("{title}{ago}")))
    }

    fn dash_shortcut(&self, s: Shortcut) -> Div {
        let (icon, label, sub) = match s {
            Shortcut::Library => {
                let n = self.lib.as_ref().map_or(0, |l| l.games.len());
                (Icon::GridView, tr!("Library"), Some(trn!(n, "{n} game", "{n} games")))
            }
            Shortcut::Stats => (Icon::BarChart, tr!("Statistics"), None),
            Shortcut::Emulators => (Icon::SportsEsports, tr!("Emulators"), None),
            Shortcut::Stores => (Icon::Storefront, tr!("Stores"), None),
        };
        panel()
            .items_center()
            .justify_center()
            .gap(u(8.))
            .child(crate::ui::icon(icon, u(34.), theme::text_primary()))
            .child(div().text_size(u(18.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(label))
            .children(sub.map(|s| div().text_size(u(12.)).text_color(theme::text_muted()).child(s)))
    }

    /// A / X on the tile `id`.
    fn dashboard_open(&mut self, id: Id, key: Option<SharedString>, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        if let (true, Some(key)) = (id.is_game(), key) {
            return self.console_activate(key, pad, window, cx);
        }
        if pad != Pad::A {
            return;
        }
        match id {
            Id::Week | Id::Shortcut(Shortcut::Stats) => self.navigate(Page::Statistics, window, cx),
            Id::Downloads => self.navigate(Page::Downloads, window, cx),
            Id::Shortcut(Shortcut::Emulators) => self.navigate(Page::Emulators, window, cx),
            Id::Shortcut(Shortcut::Stores) => {
                let page = self.pages.stores_page();
                self.navigate(page, window, cx);
            }
            Id::Shortcut(Shortcut::Library) => self.console_open_library(window, cx),
            _ => {}
        }
    }

    pub(super) fn dashboard_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let tiles = self.dashboard_tiles(tab, spec);
        if tiles.is_empty() {
            return false;
        }
        let nav = &mut self.tab_nav(tab).dashboard;
        let at = tiles.iter().position(|t| t.id == nav.focus).unwrap_or(0);
        if let Some(dir) = Dir::from_pad(pad) {
            let rects: Vec<Rect> = tiles.iter().map(|t| t.rect).collect();
            if let Some(next) = spatial::nearest(&rects, at, dir) {
                nav.focus = tiles[next].id;
            }
            return true;
        }
        match pad {
            Pad::A | Pad::X => {
                self.dashboard_open(tiles[at].id, tiles[at].key.clone(), pad, window, cx);
                true
            }
            Pad::Y => {
                self.open_console_filters(window, cx);
                true
            }
            _ => false,
        }
    }

    pub(super) fn dashboard_hints(&self, tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        let focus = self.console.tab_navs.get(tab).map(|n| n.dashboard.focus).unwrap_or_default();
        if focus.is_game() {
            return self.console_cover_hints();
        }
        let mut h = vec![hint(Btn::A, tr!("Open")), hint(Btn::Y, tr!("Search"))];
        h.extend(self.console_tab_hints());
        h
    }
}

/// The week's play time and a bar per day (today in the focus colour).
fn dash_week(week: Option<[f64; 7]>, h: f32) -> Div {
    let days = week.unwrap_or([0.; 7]);
    let total: f64 = days.iter().sum();
    let most = days.iter().copied().fold(1., f64::max);
    let bars_h = (h - 104.).max(24.);
    let bars = div().flex().items_end().gap(u(5.)).h(u(bars_h)).children(days.iter().enumerate().map(|(i, secs)| {
        let today = i == days.len() - 1;
        div().flex_1().h(u((bars_h * (*secs / most) as f32).max(5.))).rounded(u(3.)).bg(if today {
            Hsla::from(theme::console_focus())
        } else {
            kit::ink(0.28)
        })
    }));
    panel()
        .child(kit::kicker(tr!("This week")))
        .child(if total > 0. {
            kit::title(stats::fmt_secs(total), 26.)
        } else {
            div().text_size(u(14.)).text_color(theme::text_muted()).child(tr!("Nothing played this week"))
        })
        .child(div().flex_1())
        .child(bars)
}

/// The download in progress, its bar and the queue.
fn dash_downloads(download: Option<&Download>, w: f32) -> Div {
    let tile = panel().child(kicker_with(Some(Icon::Download), tr!("Downloads")));
    let Some(d) = download else {
        return tile.child(div().text_size(u(14.)).text_color(theme::text_muted()).child(tr!("Nothing downloading")));
    };
    let filled = (w - 32.) * (d.percent.unwrap_or(0.) / 100.).clamp(0., 1.) as f32;
    tile.child(div().truncate().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(d.title.clone()))
        .child(
            div()
                .mt(u(4.))
                .h(u(6.))
                .rounded_full()
                .bg(kit::ink(0.14))
                .child(div().h_full().w(u(filled)).rounded_full().bg(theme::console_focus())),
        )
        .children(d.meta.clone().map(|m| div().text_size(u(12.)).text_color(theme::text_muted()).child(m)))
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone, Utc};
    use heronite_data::journal::{Entry, LeftOff};
    use heronite_data::sessions::session_from;

    use super::{
        daily_pick, newest_note, plan, week_secs, DashboardWidgets, Dir, GameJournal, Id, LocalGameSession, NaiveDate, Rect, Shortcut,
        COLS, ROWS,
    };
    use crate::console::spatial;

    const ALL: DashboardWidgets = DashboardWidgets { week: true, downloads: true, random: true, journal: true, shortcuts: true };
    const NONE: DashboardWidgets = DashboardWidgets { week: false, downloads: false, random: false, journal: false, shortcuts: false };

    fn games(n: usize) -> impl Iterator<Item = usize> {
        0..n
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn the_mosaic_fills_six_by_three_without_overlaps() {
        let t = plan(&ALL, games(30), Some(100), Some(101));
        assert_eq!(t.len(), 13);
        assert_eq!(t[0].id, Id::Continue);
        assert_eq!(t[0].key, Some(0));
        for (i, a) in t.iter().enumerate() {
            assert!(a.rect.x >= 0. && a.rect.x + a.rect.w <= COLS && a.rect.y >= 0. && a.rect.y + a.rect.h <= ROWS);
            for b in &t[i + 1..] {
                assert!(!overlap(a.rect, b.rect), "{:?} over {:?}", a.id, b.id);
            }
        }
        assert!(t.iter().any(|t| t.id == Id::Random && t.key == Some(100)));
        assert!(t.iter().any(|t| t.id == Id::Journal && t.key == Some(101)));
    }

    #[test]
    fn widgets_that_are_off_leave_their_cells_to_games() {
        // 18 cells, 4 for Continue: Continue + 14 games.
        let t = plan(&NONE, games(30), Some(100), Some(101));
        assert_eq!(t.len(), 15);
        assert!(t.iter().all(|t| matches!(t.id, Id::Continue | Id::Game(_))));
        let keys: Vec<_> = t.iter().filter_map(|t| t.key).collect();
        assert_eq!(keys, (0..15).collect::<Vec<_>>(), "games in order, none twice");
    }

    #[test]
    fn a_widget_with_nothing_to_show_gives_up_its_cells() {
        let t = plan(&ALL, games(30), None, None);
        assert!(!t.iter().any(|t| matches!(t.id, Id::Random | Id::Journal)));
        // The two wide cells became four game cells.
        assert_eq!(t.len(), 13 - 2 + 4);
    }

    #[test]
    fn few_games_leave_cells_empty_but_keep_the_widgets() {
        let t = plan(&ALL, games(3), None, None);
        let with_game = t.iter().filter(|t| t.key.is_some()).count();
        assert_eq!(with_game, 3);
        assert!(t.iter().any(|t| t.id == Id::Week) && t.iter().any(|t| t.id == Id::Shortcut(Shortcut::Stores)));
        assert!(plan(&ALL, games(0), None, None).is_empty());
    }

    #[test]
    fn every_tile_can_be_reached_with_the_d_pad() {
        let t = plan(&ALL, games(30), Some(100), Some(101));
        let rects: Vec<Rect> = t.iter().map(|t| t.rect).collect();
        let mut seen = vec![false; t.len()];
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend([Dir::Left, Dir::Right, Dir::Up, Dir::Down].iter().filter_map(|d| spatial::nearest(&rects, i, *d)));
        }
        assert!(seen.iter().all(|s| *s));
    }

    fn at(day: u32, hour: u32, secs: u64) -> LocalGameSession {
        session_from(Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap(), secs, "heroic")
    }

    #[test]
    fn the_week_sums_play_by_day_and_skips_failed_launches() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        let s = [
            at(24, 12, 3600),
            at(24, 20, 1800),
            at(23, 12, 600),
            // The oldest day of the window, and one day before it.
            at(18, 12, 900),
            at(17, 12, 7200),
            // Too short to be play.
            at(22, 12, 30),
            // Tomorrow is not this week yet.
            at(25, 12, 3600),
        ];
        let week = week_secs(s.iter(), today, &FixedOffset::east_opt(0).unwrap());
        assert_eq!(week, [900., 0., 0., 0., 0., 600., 5400.]);
    }

    #[test]
    fn the_week_follows_the_local_day() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        // 01:00 UTC on the 24th is still the 23rd three hours west.
        let s = [at(24, 1, 3600)];
        let week = week_secs(s.iter(), today, &FixedOffset::west_opt(3 * 3600).unwrap());
        assert_eq!(week[5], 3600.);
        assert_eq!(week[6], 0.);
    }

    #[test]
    fn the_newest_note_is_an_entry_with_words_or_where_it_was_left_off() {
        let when = |d| Utc.with_ymd_and_hms(2026, 10, d, 12, 0, 0).unwrap();
        let mut j = GameJournal::default();
        assert_eq!(newest_note(&j), None);
        j.entries = vec![Entry::new(Some(when(20)), "old words", None, "app"), Entry::new(Some(when(22)), "  ", None, "overlay")];
        assert_eq!(newest_note(&j).map(|n| n.0), Some("old words".into()));
        j.left_off = Some(LeftOff { text: "the cave".into(), at: when(23).to_rfc3339(), ..Default::default() });
        assert_eq!(newest_note(&j).map(|n| n.0), Some("the cave".into()));
        j.entries.push(Entry::new(Some(when(24)), "newer", None, "app"));
        assert_eq!(newest_note(&j).map(|n| n.0), Some("newer".into()));
    }

    #[test]
    fn the_daily_pick_is_stable_and_changes_with_the_day() {
        let keys: Vec<String> = (0..40).map(|i| format!("epic_{i}")).collect();
        let pick = |day| daily_pick(keys.iter().map(String::as_str), day).unwrap();
        assert_eq!(pick(7), pick(7));
        let picks: std::collections::HashSet<usize> = (0..20).map(pick).collect();
        assert!(picks.len() > 3, "{picks:?}");
        assert_eq!(daily_pick(std::iter::empty(), 1), None);
    }
}
