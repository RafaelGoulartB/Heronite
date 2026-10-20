//! Statistics page (☰ → Statistics): play time of a week, month, year or
//! all time. The top of the page is the period's number one game as a hero
//! with the total; below it come the month-by-month strip (the year as a
//! "river" and one card per month), the top games, the habits, the records,
//! a calendar of every day and the genres. A Year in review ("Wrapped",
//! [`review`]) tells the same year as full-window stories.
//!
//! The numbers come from `heronite_data::stats` over the Collection's loaded
//! library (handed in with [`StatsPage::set_library`]); the only disk reads
//! are the screenshot listings, done once per library in the background.

mod album;
mod album_panels;
mod backlog;
mod console;
mod desktop;
mod discoveries;
mod journeys;
mod months;
mod palette;
mod review;
mod river;
mod sections;
mod sheet;
mod tabs;
mod tip;
mod viewer;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::stats::{
    self, fmt_hours_short, fmt_secs, month_short, GameInput, GameSheet, History, Journeys, Period, Span, Stats, StatsPrefs, TOP_N,
};
use heronite_data::Library;
use heronite_i18n::{n_, tr};
use heronite_images::ImageRef;
use heronite_media::{list_screenshots, GameQuery, ThumbnailCache};

use super::{PageCtx, PageEvent};
use crate::console::motion::{Axis, Glide};
use crate::services::widgets::Covers;
use crate::theme;
use tabs::Tab;

const WEEKDAYS_PLURAL: [&str; 7] =
    [n_!("Mondays"), n_!("Tuesdays"), n_!("Wednesdays"), n_!("Thursdays"), n_!("Fridays"), n_!("Saturdays"), n_!("Sundays")];
const WEEKDAYS_ON: [&str; 7] = [
    n_!("on Mondays"),
    n_!("on Tuesdays"),
    n_!("on Wednesdays"),
    n_!("on Thursdays"),
    n_!("on Fridays"),
    n_!("on Saturdays"),
    n_!("on Sundays"),
];
const DAY_PARTS: [(&str, &str); 4] =
    [(n_!("Night"), "0–6h"), (n_!("Morning"), "6–12h"), (n_!("Afternoon"), "12–18h"), (n_!("Evening"), "18–24h")];

/// "Mon" (Monday = 0).
fn weekday(d: usize) -> &'static str {
    heronite_i18n::weekday_short(d as u32)
}

/// "Monday" (Monday = 0).
fn weekday_long(d: usize) -> &'static str {
    heronite_i18n::weekday_name(d as u32)
}

/// "Mondays" (Monday = 0).
fn weekday_plural(d: usize) -> &'static str {
    heronite_i18n::tr(WEEKDAYS_PLURAL[d % 7])
}

/// "on Mondays" (Monday = 0).
fn weekday_on(d: usize) -> &'static str {
    heronite_i18n::tr(WEEKDAYS_ON[d % 7])
}

/// Name and hours of part of the day (0 = night).
fn day_part(i: usize) -> (&'static str, &'static str) {
    let (name, range) = DAY_PARTS[i];
    (heronite_i18n::tr(name), range)
}

/// A name in the middle of a sentence (Portuguese writes weekdays and months
/// in lower case).
fn in_text(name: &str) -> String {
    match heronite_i18n::current() {
        heronite_i18n::Lang::En => name.to_string(),
        heronite_i18n::Lang::PtBr => name.to_lowercase(),
    }
}

/// "October 2026".
fn month_year(d: NaiveDate) -> String {
    tr!("{month} {year}", month = stats::month_name(d.month()), year = d.year())
}

/// An in-game screenshot of a game (its capture folder matched by title).
#[derive(Clone, Debug)]
pub(super) struct Shot {
    pub path: PathBuf,
    pub at: NaiveDateTime,
}

/// What the cursor is over; the tooltip ([`tip`]) and the highlights read it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Hover {
    /// Bar of the day-by-day chart (week and month).
    Bar(usize),
    /// Weekday × hour cell.
    Heat(usize, usize),
    /// A day (calendar cell, bar of a month card).
    Day(NaiveDate),
    /// Bucket column of the river.
    River(usize),
    /// Band of the river / legend row (chart slot, `TOP_N` = Other).
    Slot(usize),
    /// Cover of the shelf (game index).
    Game(usize),
    Genre(usize),
    Source(usize),
    Part(usize),
}

/// Numbers of the period that need the library (not only the sessions).
#[derive(Default)]
pub(super) struct Derived {
    /// Time per genre (a game's time split over its first three genres).
    pub genres: Vec<(String, f64)>,
    /// Time per store ("Steam", "Epic", "Emulator", "Local"...).
    pub sources: Vec<(&'static str, f64)>,
    /// Games played in the period that are Beaten or Completed, most played first.
    pub beaten: Vec<usize>,
    /// Screenshots taken in the period, per game (most first).
    pub shots: Vec<(usize, Vec<Shot>)>,
    pub shots_total: usize,
}

pub struct StatsPage {
    app: heronite_data::AppPaths,
    /// The page's choices kept in `ui-state.json` ([`StatsPrefs`]).
    prefs: StatsPrefs,
    covers: Covers,
    lib: Option<Arc<Library>>,
    chains: HashMap<SharedString, Vec<ImageRef>>,
    /// Wide art (stage chains) for the hero, the month cards and the review.
    stages: HashMap<SharedString, Vec<ImageRef>>,
    history: Option<Arc<History>>,
    /// Screenshots of every played game (index into the library), oldest first.
    shots: Option<Arc<HashMap<usize, Vec<Shot>>>>,
    /// The Collection's 320×180 screenshot previews.
    thumb_cache: Option<ThumbnailCache>,
    /// Preview of each screenshot shown so far (`None` while it is made).
    thumbs: HashMap<PathBuf, Option<PathBuf>>,
    /// Previews still to make, one at a time (the cache blocks while it works).
    thumb_queue: std::collections::VecDeque<PathBuf>,
    thumb_loading: bool,
    tab: Tab,
    period: Period,
    stats: Option<Arc<Stats>>,
    /// The lanes of the Journeys tab (computed while it is shown).
    journeys: Option<Arc<Journeys>>,
    jv: journeys::JourneyView,
    /// Status changes, How Long To Beat times and added dates (Backlog tab).
    backlog: Arc<backlog::BacklogData>,
    /// Discoveries of the period and milestones (computed while the tab is shown).
    insights: Option<Arc<Vec<stats::Insight>>>,
    milestones: Option<Arc<Vec<stats::Milestone>>>,
    /// Hidden kinds of discoveries are listed (dimmed).
    show_hidden: bool,
    queue_sort: backlog::QueueSort,
    /// Year of the review button, its total and its top three games.
    year_summary: Option<(i32, f64, Vec<usize>)>,
    derived: Arc<Derived>,
    hover: Option<Hover>,
    show_all: bool,
    scroll: ScrollHandle,
    /// The month-by-month strip.
    strip: Glide,
    review: Option<review::Review>,
    /// Open the review once the history is read.
    review_wanted: Option<i32>,
    /// The Album tab ([`album`]): its numbers, the game picked in its chips
    /// and the moment open.
    album: Option<Arc<album::AlbumView>>,
    album_game: Option<usize>,
    moment_open: Option<usize>,
    /// The screenshot viewer over the page.
    lightbox: Option<viewer::Lightbox>,
    /// The game sheet open over the page ([`sheet`]).
    sheet: Option<Arc<GameSheet>>,
    sheet_scroll: ScrollHandle,
    focus: FocusHandle,
    /// Scroll of the console layout.
    console_scroll: ScrollHandle,
    debug: Debug,
}

/// `HERONITE_DEBUG_PAGE=stats[:week|month|year|all][:review[:N]]`, plus
/// `HERONITE_DEBUG_STATS_SCROLL=px` (page), `HERONITE_DEBUG_STATS_CARD=N`
/// (month strip) and `HERONITE_DEBUG_STATS_SHEET=N` (the sheet of the
/// period's game N, 0 = the first), `HERONITE_DEBUG_STATS_TAB=overview|journeys`
/// `HERONITE_DEBUG_STATS_OPEN=N` (open lane N of the Journeys tab, or moment
/// N of the Album) and `HERONITE_DEBUG_STATS_PHOTO=N` (capture N of the
/// Album in the viewer), for
/// screenshots without clicks.
#[derive(Default)]
struct Debug {
    scroll: Option<f32>,
    card: Option<usize>,
    sheet: Option<usize>,
    lane: Option<usize>,
    photo: Option<usize>,
    applied: bool,
}

impl EventEmitter<PageEvent> for StatsPage {}

impl Focusable for StatsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn build_history(lib: &Library) -> History {
    stats::history(lib.games.iter().map(|g| GameInput {
        sessions: g.sessions.as_deref().map_or(&[], Vec::as_slice),
        lifetime_minutes: g.playtime_minutes,
        first_played: g.first_played.as_deref(),
    }))
}

/// Screenshots of each game that has sessions. `games` comes most played
/// first: when two games match the same capture folder (a demo and the full
/// game), the folder goes to the first.
fn list_shots(folder: &Path, games: Vec<(usize, GameQuery)>) -> HashMap<usize, Vec<Shot>> {
    let mut out = HashMap::new();
    let mut taken = std::collections::HashSet::new();
    for (game, query) in games {
        let found = list_screenshots(folder, &query);
        if found.matched_folder.as_ref().is_some_and(|f| !taken.insert(f.clone())) {
            continue;
        }
        let items: Vec<Shot> = found
            .items
            .into_iter()
            .filter_map(|s| {
                let at = DateTime::from_timestamp_millis(s.taken_at? as i64)?.with_timezone(&Local).naive_local();
                Some(Shot { path: s.path, at })
            })
            .collect();
        if !items.is_empty() {
            out.insert(game, items);
        }
    }
    out
}

impl StatsPage {
    pub fn new(_ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ctx = _ctx;
        let covers = Covers::new(&ctx.data_paths, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        let page = crate::env::value("HERONITE_DEBUG_PAGE").unwrap_or_default();
        let mut parts = page.split(':').skip(1);
        let span = match parts.next() {
            Some("week") => Span::Week,
            Some("year") => Span::Year,
            Some("all") => Span::All,
            _ => Span::Month,
        };
        let review_wanted = (parts.next() == Some("review")).then(|| {
            let slide = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
            review::set_debug_slide(slide);
            today().year()
        });
        let debug = Debug {
            scroll: crate::env::parse("HERONITE_DEBUG_STATS_SCROLL"),
            card: crate::env::parse("HERONITE_DEBUG_STATS_CARD"),
            sheet: crate::env::parse("HERONITE_DEBUG_STATS_SHEET"),
            lane: crate::env::parse("HERONITE_DEBUG_STATS_OPEN"),
            photo: crate::env::parse("HERONITE_DEBUG_STATS_PHOTO"),
            applied: false,
        };
        let prefs = StatsPrefs::load(&ctx.app);
        let tab = crate::env::value("HERONITE_DEBUG_STATS_TAB")
            .or_else(|| prefs.tab.clone())
            .and_then(|t| Tab::from_id(&t))
            .unwrap_or(Tab::Overview);
        let focus = cx.focus_handle();
        if review_wanted.is_some() {
            focus.focus(window, cx);
        }
        Self {
            app: ctx.app.clone(),
            prefs,
            covers,
            lib: None,
            chains: HashMap::new(),
            stages: HashMap::new(),
            history: None,
            shots: None,
            thumb_cache: None,
            thumbs: HashMap::new(),
            thumb_queue: Default::default(),
            thumb_loading: false,
            tab,
            period: tabs::period_for(tab, Period::of(span, today())),
            stats: None,
            journeys: None,
            jv: Default::default(),
            backlog: Arc::default(),
            insights: None,
            milestones: None,
            show_hidden: false,
            queue_sort: Default::default(),
            year_summary: None,
            derived: Arc::default(),
            hover: None,
            show_all: false,
            scroll: ScrollHandle::new(),
            strip: Glide::new(Axis::X),
            review: None,
            review_wanted,
            album: None,
            album_game: None,
            moment_open: None,
            lightbox: None,
            sheet: None,
            sheet_scroll: ScrollHandle::new(),
            focus,
            console_scroll: ScrollHandle::new(),
            debug,
        }
    }

    /// The Collection's library and its art chains (covers and wide art);
    /// the history and the screenshot listings are rebuilt off the UI thread
    /// when the library changed.
    pub fn set_library(
        &mut self,
        lib: Arc<Library>,
        chains: &HashMap<SharedString, Vec<ImageRef>>,
        stages: &HashMap<SharedString, Vec<ImageRef>>,
        cx: &mut Context<Self>,
    ) {
        if self.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
            // Same data, maybe a new day: keep "today" right.
            self.recompute();
            return;
        }
        self.lib = Some(lib.clone());
        self.chains = chains.clone();
        self.stages = stages.clone();
        self.thumb_cache = Some(ThumbnailCache::new(lib.paths.screenshot_thumbnails_dir(), lib.settings.screenshots.cache_limit_mb.get()));
        let folder = lib.settings.screenshots_folder().map(Path::to_path_buf);
        let mut played: Vec<(usize, u64)> = lib
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| g.sessions.as_ref().is_some_and(|s| !s.is_empty()))
            .map(|(i, g)| (i, g.playtime_minutes))
            .collect();
        played.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let queries: Vec<(usize, GameQuery)> = played.into_iter().map(|(i, _)| (i, GameQuery::from_game(&lib.games[i]))).collect();
        cx.spawn(async move |this, cx| {
            let built = lib.clone();
            let history = cx.background_spawn(async move { build_history(&built) }).await;
            let fresh = this
                .update(cx, |this, cx| {
                    let fresh = this.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib));
                    if fresh {
                        this.history = Some(Arc::new(history));
                        this.recompute();
                        if let Some(year) = this.review_wanted.take() {
                            this.open_review(year, cx);
                        }
                        cx.notify();
                    }
                    fresh
                })
                .unwrap_or(false);
            if fresh {
                let built = lib.clone();
                let data = cx.background_spawn(async move { backlog::BacklogData::read(&built) }).await;
                this.update(cx, |this, cx| {
                    if this.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
                        this.backlog = Arc::new(data);
                        cx.notify();
                    }
                })
                .ok();
            }
            let Some(folder) = folder.filter(|_| fresh) else { return };
            let shots = cx.background_spawn(async move { list_shots(&folder, queries) }).await;
            this.update(cx, |this, cx| {
                if this.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
                    this.shots = Some(Arc::new(shots));
                    this.recompute();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn recompute(&mut self) {
        self.stats = self.history.as_ref().map(|h| Arc::new(stats::compute(h, self.period, today())));
        (self.insights, self.milestones) = match (&self.history, &self.stats) {
            (Some(h), Some(st)) if matches!(self.tab, Tab::Discoveries | Tab::Overview) => {
                let (i, m) = discoveries::compute_for(h, st);
                (Some(i), Some(m))
            }
            _ => (None, None),
        };
        self.journeys = self
            .history
            .as_ref()
            .filter(|_| matches!(self.tab, Tab::Journeys | Tab::Overview))
            .map(|h| Arc::new(stats::journeys(h, self.period, today())));
        self.year_summary = self.history.as_ref().and_then(|h| {
            let year = self.review_year();
            let jan1 = NaiveDate::from_ymd_opt(year, 1, 1)?;
            let st = match self.stats.as_deref() {
                Some(st) if st.period.span == Span::Year && st.period.start == jan1 => None,
                _ => Some(stats::compute(h, Period::of(Span::Year, jan1), today())),
            };
            let st = st.as_ref().or(self.stats.as_deref())?;
            Some((year, st.total_secs, st.games.iter().take(3).map(|g| g.game).collect()))
        });
        self.derived = Arc::new(match (&self.stats, &self.lib) {
            (Some(st), Some(lib)) => derive(st, lib, self.shots.as_deref()),
            _ => Derived::default(),
        });
        self.recompute_album();
        // An open sheet follows the period.
        if let (Some(sh), Some(h)) = (&self.sheet, &self.history) {
            self.sheet = Some(Arc::new(stats::game_sheet(h, sh.game, self.period, today())));
        }
        if let Some(r) = self.review.as_mut() {
            if let Some(h) = &self.history {
                r.refresh(h, self.lib.as_deref(), self.shots.as_deref());
            }
        }
        self.hover = None;
    }

    fn go(&mut self, period: Period, cx: &mut Context<Self>) {
        self.period = period;
        self.show_all = false;
        self.moment_open = None;
        self.recompute();
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.strip.reset();
        cx.notify();
    }

    fn set_span(&mut self, span: Span, cx: &mut Context<Self>) {
        let today = today();
        let anchor = if self.period.span == Span::All || self.period.contains(today) { today } else { self.period.start };
        self.go(Period::of(span, anchor), cx);
    }

    fn can_prev(&self) -> bool {
        self.period.span != Span::All && self.history.as_ref().and_then(|h| h.since).is_some_and(|s| s < self.period.start)
    }

    fn can_next(&self) -> bool {
        self.period.span != Span::All && self.period.end <= today()
    }

    fn game_key(&self, game: usize) -> Option<SharedString> {
        self.lib.as_ref()?.games.get(game).map(|g| SharedString::from(g.key.clone()))
    }

    fn game_title(&self, game: usize) -> String {
        self.lib.as_ref().and_then(|l| l.games.get(game)).map_or_else(String::new, |g| g.title.clone())
    }

    /// Cover of `game` at `w`x`h` logical px.
    fn cover(&mut self, game: usize, w: f32, h: f32, window: &Window) -> Option<ImageSource> {
        let key = self.game_key(game)?;
        let chain = self.chains.get(&key)?;
        self.covers.get_chain(&key, chain, w, h, window.scale_factor())
    }

    /// Wide art of `game` cropped to `w`x`h`; the cover when the game has none.
    fn art(&mut self, game: usize, w: f32, h: f32, window: &Window) -> Option<ImageSource> {
        let key = self.game_key(game)?;
        let scale = window.scale_factor();
        match self.stages.get(&key).filter(|c| !c.is_empty()) {
            Some(chain) => self.covers.get_art(&key, chain, w, h, scale),
            None => {
                let chain = self.chains.get(&key)?;
                self.covers.get_art(&format!("{key}/cover"), chain, w, h, scale)
            }
        }
    }

    /// Blurred wide art of `game` for a background of `w`x`h` logical px.
    fn blurred(&mut self, game: usize, w: f32, h: f32) -> Option<ImageSource> {
        let key = self.game_key(game)?;
        let chain = self.stages.get(&key).filter(|c| !c.is_empty()).or_else(|| self.chains.get(&key))?;
        self.covers.get_blurred(&key, chain, w, h, 34., 1.6)
    }

    /// Preview texture of the screenshot at `path` (`w`x`h` logical px); the
    /// preview is made in the background the first time.
    fn shot(&mut self, path: &Path, w: f32, h: f32, window: &Window, cx: &mut Context<Self>) -> Option<ImageSource> {
        match self.thumbs.get(path) {
            Some(Some(thumb)) => {
                let thumb = thumb.clone();
                self.covers.get_file(&thumb, w, h, window.scale_factor())
            }
            Some(None) => None,
            None => {
                let cache = self.thumb_cache.clone()?;
                self.thumbs.insert(path.to_path_buf(), None);
                self.thumb_queue.push_back(path.to_path_buf());
                if !std::mem::replace(&mut self.thumb_loading, true) {
                    cx.spawn(async move |this, cx| {
                        while let Ok(Some(source)) = this.update(cx, |this, _| {
                            let next = this.thumb_queue.pop_front();
                            this.thumb_loading = next.is_some();
                            next
                        }) {
                            let (c, src) = (cache.clone(), source.clone());
                            let thumb = cx.background_spawn(async move { c.thumbnail_path(&src).ok() }).await;
                            this.update(cx, |this, cx| {
                                if let Some(t) = thumb {
                                    this.thumbs.insert(source, Some(t));
                                    cx.notify();
                                }
                            })
                            .ok();
                        }
                    })
                    .detach();
                }
                None
            }
        }
    }

    /// Shows the Year in review of `year` (once the history is read).
    pub fn open_review(&mut self, year: i32, cx: &mut Context<Self>) {
        let Some(h) = self.history.clone() else {
            self.review_wanted = Some(year);
            return;
        };
        self.review = Some(review::Review::new(year, &h, self.lib.as_deref(), self.shots.as_deref()));
        self.hover = None;
        cx.notify();
    }

    fn close_review(&mut self, cx: &mut Context<Self>) {
        self.review = None;
        cx.notify();
    }

    /// Saves [`StatsPrefs`] off the UI thread.
    fn save_prefs(&self, cx: &mut Context<Self>) {
        let (prefs, app) = (self.prefs.clone(), self.app.clone());
        cx.background_spawn(async move {
            if let Err(err) = prefs.save(&app) {
                log::warn!("saving the Statistics choices: {err:#}");
            }
        })
        .detach();
    }

    /// Debug hooks that need a laid-out page (scroll positions).
    fn apply_debug(&mut self, window: &mut Window) {
        if self.debug.applied || self.stats.is_none() {
            return;
        }
        // The Album needs the screenshot listing, read after the history.
        if self.tab == Tab::Album && self.album.is_none() && (self.debug.lane.is_some() || self.debug.photo.is_some()) {
            window.request_animation_frame();
            return;
        }
        if let (Some(n), Some(j)) = (self.debug.lane, &self.journeys) {
            self.jv.open = j.lanes.get(n).map(|l| l.game);
        }
        if let (Some(n), Some(a)) = (self.debug.lane, &self.album) {
            self.moment_open = (n < a.moments.len()).then_some(n);
        }
        if let Some(a) = self.album.clone().filter(|a| !a.photos.is_empty()) {
            if let Some(n) = self.debug.photo.take() {
                self.lightbox = Some(viewer::Lightbox { photos: a.photos.clone(), index: n.min(a.photos.len() - 1) });
            }
        }
        if let (Some(n), Some(st), Some(h)) = (self.debug.sheet.take(), &self.stats, &self.history) {
            if let Some(g) = st.games.get(n) {
                self.sheet = Some(Arc::new(stats::game_sheet(h, g.game, self.period, today())));
            }
        }
        if self.debug.scroll.is_none() && self.debug.card.is_none() {
            self.debug.applied = true;
            return;
        }
        // Wait for a frame with the page laid out.
        if self.scroll.max_offset().y <= px(0.) {
            window.request_animation_frame();
            return;
        }
        self.debug.applied = true;
        if let Some(y) = self.debug.scroll {
            self.scroll.set_offset(point(px(0.), -px(y)));
        }
        if let Some(card) = self.debug.card {
            self.strip.jump(crate::console::motion::Goal::Start(card));
        }
        window.request_animation_frame();
    }
}

/// Genres, stores, beaten games and screenshots of the period.
fn derive(st: &Stats, lib: &Library, shots: Option<&HashMap<usize, Vec<Shot>>>) -> Derived {
    let mut genres: HashMap<String, f64> = HashMap::new();
    let mut sources: HashMap<&'static str, f64> = HashMap::new();
    let mut beaten = Vec::new();
    for g in &st.games {
        let Some(game) = lib.games.get(g.game) else { continue };
        let names: Vec<&String> = game.details.genres.iter().take(3).collect();
        for name in &names {
            *genres.entry((*name).clone()).or_default() += g.secs / names.len() as f64;
        }
        *sources.entry(game.source_label).or_default() += g.secs;
        let slug = lib.statuses.iter().find(|s| s.id == game.status_id).map_or(game.status_id.as_str(), |s| s.slug.as_str());
        if matches!(slug, "beaten" | "completed") {
            beaten.push(g.game);
        }
    }
    let by_secs = |a: &f64, b: &f64| b.total_cmp(a);
    let mut genres: Vec<(String, f64)> = genres.into_iter().collect();
    genres.sort_by(|a, b| by_secs(&a.1, &b.1).then(a.0.cmp(&b.0)));
    genres.truncate(6);
    let mut sources: Vec<(&'static str, f64)> = sources.into_iter().filter(|s| s.1 > 0.).collect();
    sources.sort_by(|a, b| by_secs(&a.1, &b.1).then(a.0.cmp(b.0)));
    let mut in_period: Vec<(usize, Vec<Shot>)> = shots
        .map(|all| {
            all.iter()
                .map(|(g, list)| (*g, list.iter().filter(|s| st.period.contains(s.at.date())).cloned().collect::<Vec<_>>()))
                .filter(|(_, l)| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    in_period.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let shots_total = in_period.iter().map(|(_, l)| l.len()).sum();
    Derived { genres, sources, beaten, shots: in_period, shots_total }
}

// ---------------------------------------------------------------- texts

fn day_label(d: NaiveDate) -> String {
    let wd = weekday(d.weekday().num_days_from_monday() as usize);
    if d.year() == today().year() {
        tr!("{weekday}, {month} {day}", weekday = wd, month = month_short(d.month()), day = d.day())
    } else {
        tr!("{weekday}, {month} {day}, {year}", weekday = wd, month = month_short(d.month()), day = d.day(), year = d.year())
    }
}

/// "Mar 14".
fn short_date(d: NaiveDate) -> String {
    tr!("{month} {day}", month = month_short(d.month()), day = d.day())
}

/// "Mar 14, 2025".
fn full_date(d: NaiveDate) -> String {
    tr!("{month} {day}, {year}", month = month_short(d.month()), day = d.day(), year = d.year())
}

/// "today", "yesterday", "3 days ago", "Oct 2", "Oct 2, 2025".
fn when_label(at: NaiveDateTime) -> String {
    let (d, today) = (at.date(), today());
    match (today - d).num_days() {
        0 => tr!("today").into(),
        1 => tr!("yesterday").into(),
        n @ 2..=6 => tr!("{n} days ago", n),
        _ if d.year() == today.year() => short_date(d),
        _ => full_date(d),
    }
}

fn bucket_label(span: Span, start: NaiveDate, end: NaiveDate) -> String {
    match span {
        Span::Week | Span::Month => day_label(start),
        _ if (end - start).num_days() > 31 => start.year().to_string(),
        _ => month_year(start),
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The change against the previous period once this one is over, its total
/// while this one is still running.
fn compare_line(st: &Stats) -> (String, Option<Rgba>) {
    let Some(prev) = st.prev_secs else {
        let recorded: f64 = st.heat.iter().flatten().sum();
        return (tr!("{time} in recorded sessions", time = fmt_hours_short(recorded)), None);
    };
    let name = st.period.prev().map(|p| p.short_name()).unwrap_or_default();
    let over = st.period.end <= today();
    if prev <= 0. {
        (tr!("Nothing played in {name}", name), None)
    } else if over {
        let change = (st.total_secs / prev - 1.) * 100.;
        if change.abs() < 1. {
            (tr!("Same as {name}", name), None)
        } else if change > 0. {
            (tr!("\u{25b2} {pct}% vs {name}", pct = format!("{change:.0}"), name), Some(theme::success()))
        } else {
            (tr!("\u{25bc} {pct}% vs {name}", pct = format!("{:.0}", -change), name), Some(theme::danger_text()))
        }
    } else {
        (tr!("{name}: {time}", name = capitalised(&name), time = fmt_secs(prev)), None)
    }
}

/// Change against the previous period as a signed percentage (`None` for
/// all time or when nothing was played before).
fn change_pct(st: &Stats) -> Option<f64> {
    let prev = st.prev_secs.filter(|p| *p > 0.)?;
    Some((st.total_secs / prev - 1.) * 100.)
}

/// Axis step and top (hours) for a chart whose tallest bar is `max_h`.
fn nice_axis(max_h: f64) -> (f64, f64) {
    const STEPS: [f64; 13] = [0.25, 0.5, 1., 2., 3., 5., 10., 20., 25., 50., 100., 200., 500.];
    let step = STEPS.into_iter().find(|s| (max_h / s).ceil() <= 4.).unwrap_or(1000.);
    (step, ((max_h / step).ceil()).max(1.) * step)
}

fn axis_label(h: f64) -> String {
    if h == 0. {
        "0".into()
    } else if h < 1. {
        tr!("{m}m", m = (h * 60.).round())
    } else if h.fract() == 0. {
        tr!("{h}h", h)
    } else {
        tr!("{h}h", h = heronite_i18n::decimal(h, 1))
    }
}

/// Colour of chart slot `slot` (see `Stats::series`); `None` = "Other".
fn series_color(slot: Option<usize>) -> Hsla {
    match slot {
        Some(i) if i < TOP_N => theme::chart_series(i).into(),
        _ => theme::chart_other(),
    }
}

/// Hours with the minutes only below 100 h ("711h", "62h 11m").
fn hours_text(secs: f64) -> String {
    if secs >= 100. * 3600. {
        tr!("{h}h", h = (secs / 3600.).floor() as u64)
    } else {
        fmt_secs(secs)
    }
}

impl Render for StatsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.covers.drop_evicted(window);
        if self.review.is_some() {
            return self.render_review(window, cx);
        }
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.apply_debug(window);
        self.render_desktop(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{axis_label, capitalised, hours_text, nice_axis};

    #[test]
    fn axis_steps_are_round() {
        assert_eq!(nice_axis(0.), (0.25, 0.25));
        assert_eq!(nice_axis(0.9), (0.25, 1.));
        assert_eq!(nice_axis(3.2), (1., 4.));
        assert_eq!(nice_axis(7.5), (2., 8.));
        assert_eq!(nice_axis(130.), (50., 150.));
        assert_eq!(axis_label(0.5), "30m");
        assert_eq!(axis_label(4.), "4h");
        assert_eq!(axis_label(1.5), "1.5h");
    }

    #[test]
    fn capitalise_first_letter() {
        assert_eq!(capitalised("previous week"), "Previous week");
        assert_eq!(capitalised(""), "");
    }

    #[test]
    fn hours_drop_minutes_from_100h() {
        assert_eq!(hours_text(711.3 * 3600.), "711h");
        assert_eq!(hours_text(62. * 3600. + 660.), "62h 11m");
    }
}
