//! Play statistics (☰ Statistics page): what was played in a week, month,
//! year or overall, built from the session history
//! (`local_library/sessions.json`) and, for the all-time totals, the lifetime
//! `totalPlayed` of `store/timestamp.json` (it also counts time played before
//! sessions were recorded).
//!
//! Sessions shorter than [`MIN_SESSION_SECS`] are launches that crashed or
//! were closed at once; they are left out. A session counts on the hours it
//! covered (`startedAt` + `elapsedSeconds`), so one that runs past midnight
//! adds to both days.

mod game;
mod journeys;

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use heronite_i18n::tr;

use crate::models::LocalGameSession;

pub use game::{game_sheet, GameSheet};
pub use journeys::{journeys, median, Journeys, Lane, LANE_MIN_SECS, PARALLEL_MIN_SECS};

/// Shorter sessions are failed launches, not play.
pub const MIN_SESSION_SECS: f64 = 120.;
/// Games with their own colour in the charts; the rest are "Other".
pub const TOP_N: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Week,
    Month,
    Year,
    All,
}

/// Days `[start, end)`. Weeks start on Monday. `All` is open-ended until
/// [`compute`] narrows it to the recorded history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Period {
    pub span: Span,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

pub use heronite_i18n::{month_name, month_short};

/// The month name in the middle of a sentence (Portuguese writes it in
/// lower case).
pub fn month_in_sentence(month: u32) -> String {
    match heronite_i18n::current() {
        heronite_i18n::Lang::En => month_name(month).to_string(),
        heronite_i18n::Lang::PtBr => month_name(month).to_lowercase(),
    }
}

fn first_of_month(y: i32, m: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, 1).expect("valid month")
}

fn next_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 12 {
        first_of_month(d.year() + 1, 1)
    } else {
        first_of_month(d.year(), d.month() + 1)
    }
}

impl Period {
    /// The week, month or year holding `day` (`All` ignores it).
    pub fn of(span: Span, day: NaiveDate) -> Self {
        let (start, end) = match span {
            Span::Week => {
                let start = day - Duration::days(day.weekday().num_days_from_monday() as i64);
                (start, start + Duration::days(7))
            }
            Span::Month => {
                let start = first_of_month(day.year(), day.month());
                (start, next_month(start))
            }
            Span::Year => (first_of_month(day.year(), 1), first_of_month(day.year() + 1, 1)),
            Span::All => (NaiveDate::MIN, NaiveDate::MAX),
        };
        Self { span, start, end }
    }

    pub fn prev(&self) -> Option<Self> {
        (self.span != Span::All).then(|| Self::of(self.span, self.start - Duration::days(1)))
    }

    pub fn next(&self) -> Option<Self> {
        (self.span != Span::All).then(|| Self::of(self.span, self.end))
    }

    pub fn contains(&self, day: NaiveDate) -> bool {
        self.start <= day && day < self.end
    }

    /// "Oct 6 \u{2013} 12, 2026", "October 2026", "2026", "All time".
    pub fn label(&self) -> String {
        match self.span {
            Span::Week => {
                let last = self.end - Duration::days(1);
                let (a, b) = (self.start, last);
                if a.year() != b.year() {
                    tr!(
                        "{m1} {d1}, {y1} \u{2013} {m2} {d2}, {y2}",
                        m1 = month_short(a.month()),
                        d1 = a.day(),
                        y1 = a.year(),
                        m2 = month_short(b.month()),
                        d2 = b.day(),
                        y2 = b.year()
                    )
                } else if a.month() != b.month() {
                    tr!(
                        "{m1} {d1} \u{2013} {m2} {d2}, {y}",
                        m1 = month_short(a.month()),
                        d1 = a.day(),
                        m2 = month_short(b.month()),
                        d2 = b.day(),
                        y = b.year()
                    )
                } else {
                    tr!("{m} {d1} \u{2013} {d2}, {y}", m = month_short(a.month()), d1 = a.day(), d2 = b.day(), y = b.year())
                }
            }
            Span::Month => tr!("{month} {year}", month = month_name(self.start.month()), year = self.start.year()),
            Span::Year => self.start.year().to_string(),
            Span::All => tr!("All time").into(),
        }
    }

    /// What the previous period is called in a comparison ("previous week",
    /// "September", "2025").
    pub fn short_name(&self) -> String {
        match self.span {
            Span::Week => tr!("previous week").into(),
            Span::Month => month_in_sentence(self.start.month()),
            Span::Year => self.start.year().to_string(),
            Span::All => String::new(),
        }
    }
}

/// One game's sessions, as the library holds them.
pub struct GameInput<'a> {
    pub sessions: &'a [LocalGameSession],
    pub lifetime_minutes: u64,
    /// `firstPlayed` of `timestamp.json` (may predate the sessions).
    pub first_played: Option<&'a str>,
}

/// Part of a session inside one clock hour.
#[derive(Clone, Copy, Debug)]
struct Piece {
    game: u32,
    at: NaiveDateTime,
    secs: f32,
}

/// A whole session (local start time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    pub game: usize,
    pub start: NaiveDateTime,
    pub secs: f64,
}

/// Sessions of every game, cut into hours. Build once per library load
/// ([`history`]), then [`compute`] any period from it.
#[derive(Default)]
pub struct History {
    pieces: Vec<Piece>,
    sessions: Vec<Played>,
    lifetime_secs: Vec<f64>,
    first_day: Vec<Option<NaiveDate>>,
    /// Day of the oldest recorded session.
    pub since: Option<NaiveDate>,
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.lifetime_secs.iter().all(|s| *s <= 0.)
    }
}

fn parse_local<Tz: TimeZone>(text: &str, tz: &Tz) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(tz).naive_local())
}

/// [`History`] in the local time zone.
pub fn history<'a>(games: impl IntoIterator<Item = GameInput<'a>>) -> History {
    history_in(games, &Local)
}

pub fn history_in<'a, Tz: TimeZone>(games: impl IntoIterator<Item = GameInput<'a>>, tz: &Tz) -> History {
    let mut h = History::default();
    for (game, input) in games.into_iter().enumerate() {
        let mut first = input.first_played.and_then(|t| parse_local(t, tz)).map(|d| d.date());
        for s in input.sessions {
            let secs = s.elapsed_seconds.get();
            if !secs.is_finite() || secs < MIN_SESSION_SECS {
                continue;
            }
            let Some(start) = parse_local(&s.started_at, tz) else { continue };
            first = Some(first.map_or(start.date(), |f| f.min(start.date())));
            h.sessions.push(Played { game, start, secs });
            // Cut at each hour boundary.
            let (mut at, mut left) = (start, secs);
            while left > 0. {
                let into_hour = (at.minute() * 60 + at.second()) as f64 + at.nanosecond() as f64 / 1e9;
                let take = left.min(3600. - into_hour);
                h.pieces.push(Piece { game: game as u32, at, secs: take as f32 });
                left -= take;
                at = at.date().and_hms_opt(at.hour(), 0, 0).expect("hour") + Duration::hours(1);
            }
        }
        h.lifetime_secs.push(input.lifetime_minutes as f64 * 60.);
        h.first_day.push(first);
    }
    h.since = h.sessions.iter().map(|s| s.start.date()).min();
    h
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameStat {
    /// Index into the [`history`] input.
    pub game: usize,
    pub secs: f64,
    pub sessions: usize,
    pub last: Option<NaiveDateTime>,
    /// First session inside the period.
    pub first: Option<NaiveDateTime>,
    /// First day the game was ever played (may predate the period).
    pub first_played: Option<NaiveDate>,
    /// First played inside the period.
    pub new: bool,
}

/// The longest stretch without play between two days with play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Break {
    /// Last day played before the break.
    pub last: NaiveDate,
    /// First day played after it.
    pub back: NaiveDate,
    /// Days without play in between.
    pub days: usize,
}

/// One bar of the activity chart: a day (week, month) or a month (year, all
/// time).
#[derive(Clone, Debug, PartialEq)]
pub struct Bucket {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub secs: f64,
    /// Seconds of each game of [`Stats::series`] (same order); the remainder
    /// is `secs - sum(top)`.
    pub top: [f64; TOP_N],
    /// After today: drawn as an empty slot.
    pub future: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    /// For `All`, narrowed to the recorded history.
    pub period: Period,
    pub total_secs: f64,
    /// Total of the previous period (`None` for `All`).
    pub prev_secs: Option<f64>,
    pub sessions: usize,
    pub days_played: usize,
    /// Days of the period up to today.
    pub days_elapsed: usize,
    /// Longest run of consecutive days with play.
    pub streak: usize,
    /// Most played first.
    pub games: Vec<GameStat>,
    /// Games with their own colour in the chart: the [`TOP_N`] with the most
    /// session time in the period (for `All` this can differ from the head
    /// of `games`, which also counts time from before the sessions).
    pub series: Vec<usize>,
    pub buckets: Vec<Bucket>,
    /// Seconds by weekday (Monday = 0) and hour.
    pub heat: [[f64; 24]; 7],
    /// Night (0–6), morning (6–12), afternoon (12–18), evening (18–24).
    pub day_parts: [f64; 4],
    pub weekend_secs: f64,
    pub longest: Option<Played>,
    pub busiest: Option<(NaiveDate, f64)>,
    /// Seconds of each day with play, oldest first.
    pub days: Vec<(NaiveDate, f64)>,
    /// First and last day of [`Stats::streak`].
    pub streak_range: Option<(NaiveDate, NaiveDate)>,
    /// Most played game during the streak.
    pub streak_game: Option<usize>,
    pub longest_break: Option<Break>,
    /// For each bucket, every game played in it, most played first.
    pub bucket_games: Vec<Vec<(usize, f64)>>,
}

impl Stats {
    /// Chart colour slot of `game`, if it has one.
    pub fn series_of(&self, game: usize) -> Option<usize> {
        self.series.iter().position(|&g| g == game)
    }

    pub fn new_games(&self) -> usize {
        self.games.iter().filter(|g| g.new).count()
    }

    /// Seconds played on `day`.
    pub fn day_secs(&self, day: NaiveDate) -> f64 {
        self.days.binary_search_by(|(d, _)| d.cmp(&day)).map_or(0., |i| self.days[i].1)
    }

    /// Days with play in `[start, end)` and the busiest of them.
    pub fn days_between(&self, start: NaiveDate, end: NaiveDate) -> (usize, Option<(NaiveDate, f64)>) {
        let days = self.days.iter().filter(|(d, _)| start <= *d && *d < end);
        let (n, best) = days.fold((0, None::<(NaiveDate, f64)>), |(n, best), &(d, s)| {
            (n + 1, if best.is_none_or(|b| s > b.1) { Some((d, s)) } else { best })
        });
        (n, best)
    }

    /// Statistics of the game at `game` (index into the history input).
    pub fn game(&self, game: usize) -> Option<&GameStat> {
        self.games.iter().find(|g| g.game == game)
    }

    pub fn avg_session_secs(&self) -> f64 {
        let in_sessions: f64 = self.heat.iter().flatten().sum();
        if self.sessions == 0 {
            0.
        } else {
            in_sessions / self.sessions as f64
        }
    }
}

fn period_secs(h: &History, p: Period) -> f64 {
    h.pieces.iter().filter(|x| p.contains(x.at.date())).map(|x| x.secs as f64).sum()
}

fn buckets_for(p: Period, today: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    match p.span {
        Span::Week | Span::Month => {
            let mut d = p.start;
            while d < p.end {
                out.push((d, d + Duration::days(1)));
                d += Duration::days(1);
            }
        }
        Span::Year => {
            let mut d = p.start;
            while d < p.end {
                out.push((d, next_month(d)));
                d = next_month(d);
            }
        }
        Span::All => {
            let months = (today.year() - p.start.year()) * 12 + today.month() as i32 - p.start.month() as i32 + 1;
            let mut d = first_of_month(p.start.year(), p.start.month());
            if months > 48 {
                // Long histories: one bar per year.
                d = first_of_month(d.year(), 1);
                while d <= today {
                    let n = first_of_month(d.year() + 1, 1);
                    out.push((d, n));
                    d = n;
                }
            } else {
                while d <= today {
                    out.push((d, next_month(d)));
                    d = next_month(d);
                }
            }
        }
    }
    out
}

/// Everything the page shows for `period`, as of `today`.
pub fn compute(h: &History, period: Period, today: NaiveDate) -> Stats {
    let period = if period.span == Span::All {
        let start = h.since.unwrap_or(today).min(today);
        Period { span: Span::All, start, end: today + Duration::days(1) }
    } else {
        period
    };
    let mut per_game: HashMap<usize, GameStat> = HashMap::new();
    let mut per_day: HashMap<NaiveDate, f64> = HashMap::new();
    let mut heat = [[0f64; 24]; 7];
    let mut day_parts = [0f64; 4];
    let mut weekend_secs = 0.;
    for x in h.pieces.iter().filter(|x| period.contains(x.at.date())) {
        let secs = x.secs as f64;
        let g = per_game.entry(x.game as usize).or_insert_with(|| GameStat::empty(x.game as usize));
        g.secs += secs;
        *per_day.entry(x.at.date()).or_default() += secs;
        let wd = x.at.weekday().num_days_from_monday() as usize;
        heat[wd][x.at.hour() as usize] += secs;
        day_parts[x.at.hour() as usize / 6] += secs;
        if wd >= 5 {
            weekend_secs += secs;
        }
    }
    let mut sessions = 0;
    let mut longest: Option<Played> = None;
    for s in h.sessions.iter().filter(|s| period.contains(s.start.date())) {
        sessions += 1;
        if let Some(g) = per_game.get_mut(&s.game) {
            g.sessions += 1;
            g.last = g.last.max(Some(s.start));
            g.first = Some(g.first.map_or(s.start, |f| f.min(s.start)));
        }
        if longest.is_none_or(|l| s.secs > l.secs) {
            longest = Some(*s);
        }
    }
    if period.span == Span::All {
        // Lifetime totals also hold the time before sessions were recorded.
        for (game, &life) in h.lifetime_secs.iter().enumerate() {
            if life > 0. {
                let g = per_game.entry(game).or_insert_with(|| GameStat::empty(game));
                g.secs = g.secs.max(life);
            }
        }
    }
    for g in per_game.values_mut() {
        g.first_played = h.first_day.get(g.game).copied().flatten();
        g.new = period.span != Span::All && g.first_played.is_some_and(|d| period.contains(d));
    }
    let mut games: Vec<GameStat> = per_game.into_values().filter(|g| g.secs > 0.).collect();
    games.sort_by(|a, b| b.secs.total_cmp(&a.secs).then(a.game.cmp(&b.game)));
    let top: Vec<usize> = {
        let mut by_sessions: HashMap<usize, f64> = HashMap::new();
        for x in h.pieces.iter().filter(|x| period.contains(x.at.date())) {
            *by_sessions.entry(x.game as usize).or_default() += x.secs as f64;
        }
        let mut v: Vec<(usize, f64)> = by_sessions.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().take(TOP_N).map(|(g, _)| g).collect()
    };

    let mut buckets: Vec<Bucket> = buckets_for(period, today)
        .into_iter()
        .map(|(start, end)| Bucket { start, end, secs: 0., top: [0.; TOP_N], future: start > today })
        .collect();
    let mut by_bucket: Vec<HashMap<usize, f64>> = vec![HashMap::new(); buckets.len()];
    for x in h.pieces.iter().filter(|x| period.contains(x.at.date())) {
        let day = x.at.date();
        let i = buckets.partition_point(|b| b.end <= day);
        if let Some(b) = buckets.get_mut(i) {
            b.secs += x.secs as f64;
            if let Some(t) = top.iter().position(|&g| g == x.game as usize) {
                b.top[t] += x.secs as f64;
            }
            *by_bucket[i].entry(x.game as usize).or_default() += x.secs as f64;
        }
    }
    let bucket_games = by_bucket.into_iter().map(ranked).collect();

    let mut days: Vec<NaiveDate> = per_day.keys().copied().collect();
    days.sort();
    let (mut streak, mut run) = (0, 0);
    let mut streak_range = None;
    let mut longest_break: Option<Break> = None;
    for (i, d) in days.iter().enumerate() {
        run = if i > 0 && days[i - 1] + Duration::days(1) == *d { run + 1 } else { 1 };
        if run > streak {
            streak = run;
            streak_range = Some((days[i + 1 - run], *d));
        }
        if i > 0 {
            let gap = (*d - days[i - 1]).num_days() as usize - 1;
            if gap > 0 && longest_break.is_none_or(|b| gap > b.days) {
                longest_break = Some(Break { last: days[i - 1], back: *d, days: gap });
            }
        }
    }
    let streak_game = streak_range.and_then(|(a, b)| {
        let mut per: HashMap<usize, f64> = HashMap::new();
        for x in h.pieces.iter().filter(|x| (a..=b).contains(&x.at.date())) {
            *per.entry(x.game as usize).or_default() += x.secs as f64;
        }
        ranked(per).first().map(|(g, _)| *g)
    });
    let busiest = per_day.iter().max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0))).map(|(d, s)| (*d, *s));
    let elapsed_end = period.end.min(today + Duration::days(1));
    let days_elapsed = (elapsed_end - period.start).num_days().max(0) as usize;
    let total_secs = games.iter().map(|g| g.secs).sum();
    let mut days: Vec<(NaiveDate, f64)> = per_day.into_iter().collect();
    days.sort_by_key(|(d, _)| *d);
    Stats {
        period,
        total_secs,
        prev_secs: period.prev().map(|p| period_secs(h, p)),
        sessions,
        days_played: days.len(),
        days_elapsed,
        streak,
        games,
        series: top,
        buckets,
        heat,
        day_parts,
        weekend_secs,
        longest,
        busiest,
        days,
        streak_range,
        streak_game,
        longest_break,
        bucket_games,
    }
}

/// Games by seconds, most first (ties by index).
fn ranked(per: HashMap<usize, f64>) -> Vec<(usize, f64)> {
    let mut v: Vec<(usize, f64)> = per.into_iter().filter(|(_, s)| *s > 0.).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

impl GameStat {
    fn empty(game: usize) -> Self {
        GameStat { game, secs: 0., sessions: 0, last: None, first: None, first_played: None, new: false }
    }
}

/// "42h 10m", "35m", "0m" (rounded to the minute).
pub fn fmt_secs(secs: f64) -> String {
    crate::collection::format_playtime_minutes((secs / 60.).round().max(0.) as u64)
}

/// "1,409" (or "1.409").
pub fn group_thousands(n: u64) -> String {
    heronite_i18n::grouped(n as i64)
}

/// Rounded hours for big totals: "1,402h" from 100 h up.
pub fn fmt_hours_short(secs: f64) -> String {
    let h = secs / 3600.;
    if h >= 100. {
        format!("{}h", group_thousands(h.round() as u64))
    } else {
        fmt_secs(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JsNum;
    use chrono::FixedOffset;

    fn s(started: &str, secs: f64) -> LocalGameSession {
        LocalGameSession {
            started_at: started.into(),
            ended_at: started.into(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            game_action_name: None,
            extra: Default::default(),
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn periods_and_labels() {
        let wed = day(2026, 10, 7);
        let w = Period::of(Span::Week, wed);
        assert_eq!((w.start, w.end), (day(2026, 10, 5), day(2026, 10, 12)));
        assert_eq!(w.label(), "Oct 5 \u{2013} 11, 2026");
        assert_eq!(Period::of(Span::Week, day(2026, 10, 1)).label(), "Sep 28 \u{2013} Oct 4, 2026");
        assert_eq!(Period::of(Span::Week, day(2026, 1, 1)).label(), "Dec 29, 2025 \u{2013} Jan 4, 2026");
        let m = Period::of(Span::Month, wed);
        assert_eq!((m.start, m.end), (day(2026, 10, 1), day(2026, 11, 1)));
        assert_eq!(m.label(), "October 2026");
        assert_eq!(m.prev().unwrap().label(), "September 2026");
        assert_eq!(Period::of(Span::Month, day(2026, 12, 3)).next().unwrap().label(), "January 2027");
        assert_eq!(Period::of(Span::Year, wed).label(), "2026");
        assert_eq!(Period::of(Span::All, wed).prev(), None);
    }

    #[test]
    fn sessions_split_at_midnight_and_short_ones_are_dropped() {
        let sessions = [s("2026-10-03T23:30:00Z", 3600.), s("2026-10-05T10:00:00Z", 60.)];
        let h = history_in([GameInput { sessions: &sessions, lifetime_minutes: 0, first_played: None }], &utc());
        let st = compute(&h, Period::of(Span::Month, day(2026, 10, 9)), day(2026, 10, 9));
        assert_eq!(st.sessions, 1);
        assert_eq!(st.total_secs, 3600.);
        assert_eq!(st.buckets[2].secs, 1800.); // Oct 3
        assert_eq!(st.buckets[3].secs, 1800.); // Oct 4
        assert_eq!(st.days_played, 2);
        assert_eq!(st.streak, 2);
        assert_eq!(st.days_elapsed, 9);
        assert!(st.buckets[9].future);
        // Saturday 23h and Sunday 0h.
        assert_eq!(st.heat[5][23], 1800.);
        assert_eq!(st.heat[6][0], 1800.);
        assert_eq!(st.weekend_secs, 3600.);
        assert_eq!(st.day_parts, [1800., 0., 0., 1800.]);
    }

    #[test]
    fn ranking_previous_period_new_games_and_lifetime() {
        let a = [s("2026-10-01T20:00:00Z", 7200.), s("2026-10-02T20:00:00Z", 3600.), s("2026-09-10T20:00:00Z", 1800.)];
        let b = [s("2026-10-04T12:00:00Z", 5400.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 600, first_played: Some("2025-01-01T00:00:00Z") },
                GameInput { sessions: &b, lifetime_minutes: 1000, first_played: None },
            ],
            &utc(),
        );
        assert_eq!(h.since, Some(day(2026, 9, 10)));
        let st = compute(&h, Period::of(Span::Month, day(2026, 10, 9)), day(2026, 10, 9));
        assert_eq!(
            st.games.iter().map(|g| (g.game, g.secs, g.sessions, g.new)).collect::<Vec<_>>(),
            [(0, 10800., 2, false), (1, 5400., 1, true)]
        );
        assert_eq!(st.prev_secs, Some(1800.));
        assert_eq!(st.longest.unwrap().secs, 7200.);
        assert_eq!(st.busiest, Some((day(2026, 10, 1), 7200.)));
        assert_eq!(st.buckets[0].top[0], 7200.);
        assert_eq!(st.buckets[3].top[1], 5400.);
        assert_eq!(st.avg_session_secs(), 5400.);

        let all = compute(&h, Period::of(Span::All, day(2026, 10, 9)), day(2026, 10, 9));
        assert_eq!(all.period.start, day(2026, 9, 10));
        assert_eq!(all.buckets.len(), 2); // Sep, Oct
                                          // Lifetime totals beat the sessions: game 1 (1000 min) leads the list...
        assert_eq!(all.games.iter().map(|g| (g.game, g.secs)).collect::<Vec<_>>(), [(1, 60000.), (0, 36000.)]);
        // ...but the chart colours go by session time, where game 0 leads.
        assert_eq!(all.series, [0, 1]);
        assert_eq!(all.buckets[1].top, [10800., 5400., 0., 0., 0., 0.]);
        assert!(all.games.iter().all(|g| !g.new));
        assert_eq!(all.prev_secs, None);
    }

    #[test]
    fn streak_break_days_and_games_per_bucket() {
        let a = [
            s("2026-01-01T10:00:00Z", 3600.),
            s("2026-01-02T10:00:00Z", 3600.),
            s("2026-01-03T10:00:00Z", 600.),
            s("2026-01-20T10:00:00Z", 1200.),
        ];
        let b = [s("2026-01-03T12:00:00Z", 7200.), s("2026-02-05T12:00:00Z", 1800.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: None },
                GameInput { sessions: &b, lifetime_minutes: 0, first_played: Some("2025-06-01T00:00:00Z") },
            ],
            &utc(),
        );
        let st = compute(&h, Period::of(Span::Year, day(2026, 3, 1)), day(2026, 3, 1));
        assert_eq!(st.streak, 3);
        assert_eq!(st.streak_range, Some((day(2026, 1, 1), day(2026, 1, 3))));
        // Jan 1–3: game 0 has 2h10m, game 1 has 2h.
        assert_eq!(st.streak_game, Some(0));
        // Jan 3 -> Jan 20 is 16 days off; Jan 20 -> Feb 5 is 15.
        assert_eq!(st.longest_break, Some(Break { last: day(2026, 1, 3), back: day(2026, 1, 20), days: 16 }));
        assert_eq!(st.days.len(), 5);
        assert_eq!(st.day_secs(day(2026, 1, 3)), 7800.);
        assert_eq!(st.day_secs(day(2026, 1, 4)), 0.);
        assert_eq!(st.days_between(day(2026, 1, 1), day(2026, 2, 1)), (4, Some((day(2026, 1, 3), 7800.))));
        assert_eq!(st.bucket_games[0], [(0, 9000.), (1, 7200.)]);
        assert_eq!(st.bucket_games[1], [(1, 1800.)]);
        assert!(st.bucket_games[2].is_empty());
        let g1 = st.game(1).unwrap();
        assert_eq!(g1.first.map(|f| f.date()), Some(day(2026, 1, 3)));
        assert_eq!(g1.first_played, Some(day(2025, 6, 1)));
        assert!(!g1.new && st.game(0).unwrap().new);
    }

    #[test]
    fn year_buckets_are_months() {
        let a = [s("2026-03-15T10:00:00Z", 600.)];
        let h = history_in([GameInput { sessions: &a, lifetime_minutes: 0, first_played: None }], &utc());
        let st = compute(&h, Period::of(Span::Year, day(2026, 10, 9)), day(2026, 10, 9));
        assert_eq!(st.buckets.len(), 12);
        assert_eq!(st.buckets[2].secs, 600.);
        assert!(st.buckets[10].future && !st.buckets[9].future);
    }

    #[test]
    fn number_formats() {
        assert_eq!(fmt_secs(3600. * 42. + 600.), "42h 10m");
        assert_eq!(fmt_secs(29.), "0m");
        assert_eq!(fmt_hours_short(1401.6 * 3600.), "1,402h");
        assert_eq!(fmt_hours_short(5400.), "1h 30m");
        assert_eq!(group_thousands(1409), "1,409");
        assert_eq!(group_thousands(12), "12");
    }
}
