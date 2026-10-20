//! Numbers of the Backlog tab: the yearly goal and its pace, the hours a week
//! you usually play (to tell when a queued game would be beaten), and running
//! counts over time (games added, started and beaten).

use chrono::{Datelike, Duration, NaiveDate};

use super::History;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goal {
    pub target: u32,
    pub done: u32,
    /// Games the goal asks for by today (fractional).
    pub expected: f64,
    /// Games by Dec 31 at this year's pace so far.
    pub projected: f64,
    /// Days per game left to make it (`None` once made or once the year is over).
    pub every_days: Option<f64>,
}

impl Goal {
    pub fn left(&self) -> u32 {
        self.target.saturating_sub(self.done)
    }
}

/// The goal of `target` games in `year` with `done` beaten, as of `today`.
pub fn goal(target: u32, done: u32, year: i32, today: NaiveDate) -> Goal {
    let jan1 = NaiveDate::from_ymd_opt(year, 1, 1).expect("year");
    let next = NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("year");
    let len = (next - jan1).num_days() as f64;
    let elapsed = ((today.min(next - Duration::days(1)) - jan1).num_days() + 1).clamp(0, len as i64) as f64;
    let expected = target as f64 * elapsed / len;
    let projected = if elapsed > 0. { done as f64 / elapsed * len } else { 0. };
    let days_left = (len - elapsed).max(0.);
    let left = target.saturating_sub(done);
    let every_days = (left > 0 && days_left > 0.).then(|| days_left / left as f64);
    Goal { target, done, expected, projected, every_days }
}

/// Hours a week played over the `weeks` weeks before `today`, leaving out the
/// games `skip` says (endless games never end, so they do not count towards
/// how fast a queue moves).
pub fn weekly_hours(h: &History, today: NaiveDate, weeks: i64, skip: impl Fn(usize) -> bool) -> f64 {
    let from = today - Duration::weeks(weeks);
    let secs: f64 = h
        .pieces_between(from + Duration::days(1), today + Duration::days(1))
        .iter()
        .filter(|x| !skip(x.game as usize))
        .map(|x| x.secs as f64)
        .sum();
    secs / 3600. / weeks.max(1) as f64
}

/// How many of `dates` fall on or before each of `points`.
pub fn running_count(dates: &[NaiveDate], points: &[NaiveDate]) -> Vec<usize> {
    let mut sorted = dates.to_vec();
    sorted.sort_unstable();
    points.iter().map(|p| sorted.partition_point(|d| d <= p)).collect()
}

/// The last day of each month from `from`'s month to `to`'s (the points of
/// [`running_count`] for the pile chart).
pub fn month_ends(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut d = NaiveDate::from_ymd_opt(from.year(), from.month(), 1).expect("month");
    while d <= to {
        let next =
            if d.month() == 12 { NaiveDate::from_ymd_opt(d.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1) }
                .expect("month");
        out.push((next - Duration::days(1)).min(to));
        d = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LocalGameSession;
    use crate::stats::{history_in, GameInput};
    use crate::JsNum;
    use chrono::FixedOffset;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_goal_and_its_pace() {
        // Day 182 of 365.
        let g = goal(12, 5, 2026, day(2026, 7, 1));
        assert!((g.expected - 12. * 182. / 365.).abs() < 1e-9);
        assert!((g.projected - 5. / 182. * 365.).abs() < 1e-9);
        assert_eq!(g.left(), 7);
        assert!((g.every_days.unwrap() - 183. / 7.).abs() < 1e-9);
        assert_eq!(goal(3, 4, 2026, day(2026, 7, 1)).every_days, None);
        // A past year is over.
        let past = goal(10, 8, 2025, day(2026, 7, 1));
        assert_eq!((past.expected, past.every_days), (10., None));
    }

    #[test]
    fn weekly_hours_and_running_counts() {
        let s = |at: &str, secs: f64| LocalGameSession {
            started_at: at.into(),
            ended_at: at.into(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            game_action_name: None,
            extra: Default::default(),
        };
        let a = [s("2026-10-01T20:00:00Z", 7200.), s("2026-01-01T20:00:00Z", 7200.)];
        let b = [s("2026-10-02T20:00:00Z", 3600.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: None },
                GameInput { sessions: &b, lifetime_minutes: 0, first_played: None },
            ],
            &FixedOffset::east_opt(0).unwrap(),
        );
        assert!((weekly_hours(&h, day(2026, 10, 9), 4, |_| false) - 3. / 4.).abs() < 1e-9);
        assert!((weekly_hours(&h, day(2026, 10, 9), 4, |g| g == 1) - 2. / 4.).abs() < 1e-9);
        let ends = month_ends(day(2026, 1, 15), day(2026, 3, 10));
        assert_eq!(ends, [day(2026, 1, 31), day(2026, 2, 28), day(2026, 3, 10)]);
        assert_eq!(running_count(&[day(2026, 2, 1), day(2026, 1, 3), day(2026, 3, 9)], &ends), [1, 2, 3]);
    }
}
