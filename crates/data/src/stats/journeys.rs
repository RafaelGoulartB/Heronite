//! Journeys: each game of a period as a lane in time (the days it was played
//! between its first and last day), and how many games were going at once.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate};

use super::{History, Period, Span};

/// Games with less than this in the period are "only tried".
pub const LANE_MIN_SECS: f64 = 3600.;
/// A game counts as going in a week (or day) from this much play in it.
pub const PARALLEL_MIN_SECS: f64 = 15. * 60.;

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub game: usize,
    pub secs: f64,
    /// Sessions that started inside the period.
    pub sessions: usize,
    /// Days with play inside the period, oldest first.
    pub days: Vec<(NaiveDate, f64)>,
    /// First day the game was ever played (may predate the period).
    pub first_ever: Option<NaiveDate>,
    /// Weeks (Monday to Sunday) with play inside the period.
    pub weeks: usize,
}

impl Lane {
    pub fn first(&self) -> Option<NaiveDate> {
        self.days.first().map(|d| d.0)
    }

    pub fn last(&self) -> Option<NaiveDate> {
        self.days.last().map(|d| d.0)
    }

    /// Played before the period started.
    pub fn from_before(&self, period: &Period) -> bool {
        self.first_ever.is_some_and(|f| f < period.start)
    }

    /// Hours per week with play.
    pub fn hours_per_week(&self) -> f64 {
        self.secs / 3600. / self.weeks.max(1) as f64
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Journeys {
    /// Period as computed (all time narrowed to the recorded history).
    pub period: Option<Period>,
    /// Games with at least [`LANE_MIN_SECS`], by first day in the period.
    pub lanes: Vec<Lane>,
    /// The rest, most played first.
    pub tried: Vec<Lane>,
    /// Games going in each week (each day for a week or a month): `(start, n)`.
    pub parallel: Vec<(NaiveDate, usize)>,
}

impl Journeys {
    /// The step of [`Journeys::parallel`] in days.
    pub fn step(span: Span) -> i64 {
        if matches!(span, Span::Week | Span::Month) {
            1
        } else {
            7
        }
    }

    /// The busiest week (or day) and its count.
    pub fn peak(&self) -> Option<(NaiveDate, usize)> {
        self.parallel.iter().copied().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).filter(|p| p.1 > 0)
    }
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// The journeys of `period`, as of `today`.
pub fn journeys(h: &History, period: Period, today: NaiveDate) -> Journeys {
    let period = if period.span == Span::All {
        Period { span: Span::All, start: h.since.unwrap_or(today).min(today), end: today + Duration::days(1) }
    } else {
        period
    };
    let mut per_game: BTreeMap<usize, BTreeMap<NaiveDate, f64>> = BTreeMap::new();
    for x in h.pieces_in(period) {
        *per_game.entry(x.game as usize).or_default().entry(x.at.date()).or_default() += x.secs as f64;
    }
    let mut sessions: BTreeMap<usize, usize> = BTreeMap::new();
    for s in h.sessions_in(period) {
        *sessions.entry(s.game).or_default() += 1;
    }
    let step = Journeys::step(period.span);
    let start = if step == 7 { monday(period.start) } else { period.start };
    let end = period.end.min(today + Duration::days(1));
    let slots = ((end - start).num_days().max(0) + step - 1) / step;
    let mut parallel: Vec<(NaiveDate, usize)> = (0..slots).map(|i| (start + Duration::days(i * step), 0)).collect();
    let (mut lanes, mut tried) = (Vec::new(), Vec::new());
    for (game, days) in per_game {
        let mut per_slot: BTreeMap<i64, f64> = BTreeMap::new();
        for (d, s) in &days {
            *per_slot.entry((*d - start).num_days() / step).or_default() += s;
        }
        for (slot, s) in &per_slot {
            if *s >= PARALLEL_MIN_SECS {
                if let Some(p) = parallel.get_mut(*slot as usize) {
                    p.1 += 1;
                }
            }
        }
        let weeks = days.keys().map(|d| monday(*d)).collect::<std::collections::BTreeSet<_>>().len();
        let lane = Lane {
            game,
            secs: days.values().sum(),
            sessions: sessions.get(&game).copied().unwrap_or(0),
            days: days.into_iter().collect(),
            first_ever: h.first_day.get(game).copied().flatten(),
            weeks,
        };
        if lane.secs >= LANE_MIN_SECS {
            lanes.push(lane);
        } else {
            tried.push(lane);
        }
    }
    lanes.sort_by(|a, b| a.first().cmp(&b.first()).then(b.secs.total_cmp(&a.secs)));
    tried.sort_by(|a, b| b.secs.total_cmp(&a.secs).then(a.game.cmp(&b.game)));
    Journeys { period: Some(period), lanes, tried, parallel }
}

/// The middle value of `days` (`None` when empty).
pub fn median(mut days: Vec<i64>) -> Option<i64> {
    days.sort_unstable();
    days.get(days.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LocalGameSession;
    use crate::stats::{history_in, GameInput};
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

    #[test]
    fn lanes_tried_games_and_games_at_once() {
        let a = [s("2026-03-02T20:00:00Z", 7200.), s("2026-03-10T20:00:00Z", 3600.), s("2026-05-01T20:00:00Z", 1800.)];
        let b = [s("2026-03-03T20:00:00Z", 3600.)];
        let c = [s("2026-03-04T20:00:00Z", 600.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: Some("2025-06-01T00:00:00Z") },
                GameInput { sessions: &b, lifetime_minutes: 0, first_played: None },
                GameInput { sessions: &c, lifetime_minutes: 0, first_played: None },
            ],
            &FixedOffset::east_opt(0).unwrap(),
        );
        let today = day(2026, 10, 9);
        let j = journeys(&h, Period::of(Span::Year, today), today);
        assert_eq!(j.lanes.iter().map(|l| l.game).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(j.tried.iter().map(|l| l.game).collect::<Vec<_>>(), [2]);
        let a = &j.lanes[0];
        assert_eq!((a.first(), a.last(), a.sessions, a.weeks), (Some(day(2026, 3, 2)), Some(day(2026, 5, 1)), 3, 3));
        assert!(a.from_before(&j.period.unwrap()));
        assert!((a.hours_per_week() - 3.5 / 3.).abs() < 1e-9);
        // Week of Mon Mar 2: games 0 and 1 (game 2 has only 10 min).
        assert_eq!(j.peak(), Some((day(2026, 3, 2), 2)));
        assert_eq!(j.parallel.first().map(|p| p.0), Some(day(2025, 12, 29)));

        let m = journeys(&h, Period::of(Span::Month, day(2026, 3, 1)), today);
        assert_eq!(m.parallel.len(), 31);
        assert_eq!(m.parallel[1], (day(2026, 3, 2), 1));
        assert_eq!(median(vec![30, 10, 20]), Some(20));
        assert_eq!(median(vec![]), None);
    }
}
