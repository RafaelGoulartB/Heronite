//! Everything a period shows, computed off the UI thread. Changing the
//! period, the tab or the album's game starts a [`Job`]; the page keeps
//! showing what it has until the [`Snapshot`] is back (a few milliseconds,
//! more for all time on a big library), so the UI never waits on numbers.
//!
//! The render code only reads what is here. Anything that grows with the
//! library (games, sessions, screenshots) is worked out once per period, not
//! once per frame: the page renders again on every scroll and mouse move.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Datelike, Duration, NaiveDate};
use heronite_data::stats::{self, year_before, History, Insight, Journeys, Milestone, Period, Span, Stats};
use heronite_data::Library;

use super::album::{album_view, AlbumView};
use super::facts::GameFacts;
use super::viewer::Photo;
use super::Shot;

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

/// What a month card shows besides the numbers of [`Stats`].
#[derive(Default)]
pub(super) struct MonthExtra {
    /// Games beaten in the month.
    pub beaten: Vec<usize>,
    /// Up to two screenshots of the month's top games (any game if none).
    pub shots: Vec<(usize, PathBuf)>,
}

/// A period at a glance: its time and its games, most played first.
pub(super) struct Glance {
    pub period: Period,
    pub secs: f64,
    /// The five most played.
    pub top: Vec<usize>,
    pub games: usize,
}

/// The Discoveries' look back: the same span a year before, this month a
/// year ago, the screenshots of that week and where the history began.
#[derive(Default)]
pub(super) struct Memories {
    pub year_ago: Option<Glance>,
    pub month_ago: Option<Glance>,
    pub week_shots: Vec<Photo>,
    /// First day of the history and the game played most that month.
    pub first: Option<(NaiveDate, Option<usize>)>,
    /// All-time time, sessions and games.
    pub all: (f64, usize, usize),
}

/// The inputs of a period, cheap to clone (shared data) and sent to a
/// background thread.
pub(super) struct Job {
    pub history: Arc<History>,
    pub lib: Option<Arc<Library>>,
    pub shots: Option<Arc<HashMap<usize, Vec<Shot>>>>,
    pub facts: Arc<GameFacts>,
    pub period: Period,
    pub album_game: Option<usize>,
    pub today: NaiveDate,
}

pub(super) struct Snapshot {
    pub stats: Arc<Stats>,
    pub insights: Arc<Vec<Insight>>,
    pub milestones: Arc<Vec<Milestone>>,
    pub journeys: Arc<Journeys>,
    /// Year of the review button, its total and its top three games.
    pub year_summary: Option<(i32, f64, Vec<usize>)>,
    pub derived: Arc<Derived>,
    pub album: Option<Arc<AlbumView>>,
    /// One per bucket of [`Stats::buckets`].
    pub months: Arc<Vec<MonthExtra>>,
    pub memories: Arc<Memories>,
}

impl Job {
    pub fn run(self) -> Snapshot {
        let started = Instant::now();
        let h = &*self.history;
        let st = stats::compute(h, self.period, self.today);
        let insights = stats::insights(h, &st);
        let milestones = stats::milestones(h, self.today);
        let journeys = stats::journeys(h, self.period, self.today);
        // The review button tells the period's year (all time: this year).
        let year = if self.period.span == Span::All { self.today.year() } else { self.period.start.year() };
        let year_summary = NaiveDate::from_ymd_opt(year, 1, 1).map(|jan1| {
            let own = st.period.span == Span::Year && st.period.start == jan1;
            let other = (!own).then(|| stats::compute(h, Period::of(Span::Year, jan1), self.today));
            let y = other.as_ref().unwrap_or(&st);
            (year, y.total_secs, y.games.iter().take(3).map(|g| g.game).collect())
        });
        let shots = self.shots.as_deref();
        let derived = self.lib.as_deref().map_or_else(Derived::default, |lib| derive(&st, lib, shots));
        let album = shots.map(|all| Arc::new(album_view(&st, h, all, self.album_game)));
        let months = month_extras(&st, &self.facts, shots);
        let memories = memories(h, &st, shots, self.today);
        log::debug!("statistics of {:?} computed in {:?}", self.period, started.elapsed());
        Snapshot {
            stats: Arc::new(st),
            insights: Arc::new(insights),
            milestones: Arc::new(milestones),
            journeys: Arc::new(journeys),
            year_summary,
            derived: Arc::new(derived),
            album,
            months: Arc::new(months),
            memories: Arc::new(memories),
        }
    }
}

/// Genres, stores, beaten games and screenshots of the period.
pub(super) fn derive(st: &Stats, lib: &Library, shots: Option<&HashMap<usize, Vec<Shot>>>) -> Derived {
    let slug_of: HashMap<&str, &str> = lib.statuses.iter().map(|s| (s.id.as_str(), s.slug.as_str())).collect();
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
        let slug = slug_of.get(game.status_id.as_str()).copied().unwrap_or(game.status_id.as_str());
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
                .map(|(g, list)| (*g, in_days(list, st.period.start, st.period.end).to_vec()))
                .filter(|(_, l)| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    in_period.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let shots_total = in_period.iter().map(|(_, l)| l.len()).sum();
    Derived { genres, sources, beaten, shots: in_period, shots_total }
}

/// The captures of a game's list (oldest first) taken on the days `[start, end)`.
fn in_days(list: &[Shot], start: NaiveDate, end: NaiveDate) -> &[Shot] {
    let from = list.partition_point(|s| s.at.date() < start);
    let to = from + list[from..].partition_point(|s| s.at.date() < end);
    &list[from..to]
}

/// The beaten games and the screenshots of each bucket.
fn month_extras(st: &Stats, facts: &GameFacts, shots: Option<&HashMap<usize, Vec<Shot>>>) -> Vec<MonthExtra> {
    st.buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if b.future || b.secs <= 0. {
                return MonthExtra::default();
            }
            let range = Period { span: Span::Month, start: b.start, end: b.end };
            let beaten = facts.beaten_in(&range).into_iter().map(|(g, _)| g).collect();
            let games = st.bucket_games.get(i).map_or(&[][..], Vec::as_slice);
            let first = |g: usize| shots?.get(&g).and_then(|l| in_days(l, b.start, b.end).first()).map(|s| (g, s.path.clone()));
            let mut picked: Vec<(usize, PathBuf)> = games.iter().take(3).filter_map(|(g, _)| first(*g)).take(2).collect();
            if picked.is_empty() {
                picked.extend(games.iter().skip(3).find_map(|(g, _)| first(*g)));
            }
            MonthExtra { beaten, shots: picked }
        })
        .collect()
}

fn glance(st: &Stats) -> Option<Glance> {
    (st.total_secs > 0.).then(|| Glance {
        period: st.period,
        secs: st.total_secs,
        top: st.games.iter().take(5).map(|g| g.game).collect(),
        games: st.games.len(),
    })
}

fn memories(h: &History, st: &Stats, shots: Option<&HashMap<usize, Vec<Shot>>>, today: NaiveDate) -> Memories {
    let before = year_before(st.period);
    let year_ago =
        (before != st.period && h.since.is_some_and(|s| s < before.end)).then(|| glance(&stats::compute(h, before, today))).flatten();
    let month_ago = glance(&stats::compute(h, Period::of(Span::Month, today - Duration::days(365)), today));
    let week = Period::of(Span::Week, today - Duration::days(364));
    let mut week_shots: Vec<Photo> = shots
        .into_iter()
        .flatten()
        .flat_map(|(g, l)| in_days(l, week.start, week.end).iter().map(move |s| Photo { game: *g, at: s.at, path: s.path.clone() }))
        .collect();
    week_shots.sort_by_key(|p| p.at);
    week_shots.truncate(4);
    let first = h.since.map(|since| (since, stats::compute(h, Period::of(Span::Month, since), today).games.first().map(|g| g.game)));
    let totals: Vec<f64> = (0..h.games()).map(|g| h.total_secs(g)).filter(|s| *s > 0.).collect();
    Memories { year_ago, month_ago, week_shots, first, all: (totals.iter().sum(), h.session_count(), totals.len()) }
}
