//! One game inside a period: what the Statistics page's game sheet shows
//! (its time, a bar per day or week, its best session and its habits).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use super::{History, Period, Played, Span};

#[derive(Clone, Debug, PartialEq)]
pub struct GameSheet {
    pub game: usize,
    /// Seconds played inside the period.
    pub secs: f64,
    /// Sessions that started inside the period.
    pub sessions: usize,
    /// Days of the period with play.
    pub days_played: usize,
    /// Recorded session time of every period, or the lifetime total when it
    /// is bigger (it also counts time from before sessions were recorded).
    pub life_secs: f64,
    /// First day the game was ever played.
    pub first: Option<NaiveDate>,
    /// Start of the last session ever.
    pub last: Option<NaiveDateTime>,
    /// One bar per day (week, month) or per week starting on Monday (year,
    /// all time) across the period: `(start, seconds)`.
    pub bars: Vec<(NaiveDate, f64)>,
    /// Longest session inside the period.
    pub best: Option<Played>,
    /// Usual start hour of its sessions in the period (median, with the day
    /// turning at 6h so a night of play stays in one evening).
    pub usual_hour: Option<u32>,
    /// Share of its time in the period played on Saturdays and Sundays (0..=1).
    pub weekend: f64,
}

/// Where the bars of `period` start and how long each is (days).
fn bar_steps(period: Period, today: NaiveDate) -> (NaiveDate, NaiveDate, i64) {
    let end = period.end.min(today + Duration::days(1));
    match period.span {
        Span::Week | Span::Month => (period.start, period.end, 1),
        Span::Year | Span::All => {
            let start = period.start - Duration::days(period.start.weekday().num_days_from_monday() as i64);
            (start, end, 7)
        }
    }
}

/// The game sheet of `game` for `period` (narrowed like [`super::compute`]
/// for all time).
pub fn game_sheet(h: &History, game: usize, period: Period, today: NaiveDate) -> GameSheet {
    let period = if period.span == Span::All {
        Period { span: Span::All, start: h.since.unwrap_or(today).min(today), end: today + Duration::days(1) }
    } else {
        period
    };
    let (start, end, step) = bar_steps(period, today);
    let n = ((end - start).num_days().max(0) + step - 1) / step;
    let mut bars: Vec<(NaiveDate, f64)> = (0..n).map(|i| (start + Duration::days(i * step), 0.)).collect();
    let (mut secs, mut weekend) = (0., 0.);
    let mut days = std::collections::BTreeSet::new();
    for x in h.pieces.iter().filter(|x| x.game as usize == game && period.contains(x.at.date())) {
        let s = x.secs as f64;
        secs += s;
        days.insert(x.at.date());
        if x.at.weekday().num_days_from_monday() >= 5 {
            weekend += s;
        }
        let i = ((x.at.date() - start).num_days() / step) as usize;
        if let Some(b) = bars.get_mut(i) {
            b.1 += s;
        }
    }
    let mine: Vec<&Played> = h.sessions.iter().filter(|s| s.game == game).collect();
    let inside: Vec<&&Played> = mine.iter().filter(|s| period.contains(s.start.date())).collect();
    let best = inside.iter().max_by(|a, b| a.secs.total_cmp(&b.secs)).map(|s| ***s);
    let mut hours: Vec<u32> = inside.iter().map(|s| (s.start.hour() + 18) % 24).collect();
    hours.sort_unstable();
    let usual_hour = hours.get(hours.len() / 2).map(|h| (h + 6) % 24);
    let recorded: f64 = mine.iter().map(|s| s.secs).sum();
    GameSheet {
        game,
        secs,
        sessions: inside.len(),
        days_played: days.len(),
        life_secs: recorded.max(h.lifetime_secs.get(game).copied().unwrap_or(0.)),
        first: h.first_day.get(game).copied().flatten(),
        last: mine.iter().map(|s| s.start).max(),
        bars,
        best,
        usual_hour,
        weekend: if secs > 0. { weekend / secs } else { 0. },
    }
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
    fn a_game_in_a_month_and_in_a_year() {
        let a = [
            s("2026-10-03T20:00:00Z", 7200.), // Saturday
            s("2026-10-05T23:30:00Z", 3600.), // Monday, past midnight
            s("2026-10-07T01:00:00Z", 1800.),
            s("2026-09-01T20:00:00Z", 600.),
        ];
        let b = [s("2026-10-04T12:00:00Z", 5400.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: Some("2025-01-02T00:00:00Z") },
                GameInput { sessions: &b, lifetime_minutes: 500, first_played: None },
            ],
            &FixedOffset::east_opt(0).unwrap(),
        );
        let today = day(2026, 10, 9);
        let m = game_sheet(&h, 0, Period::of(Span::Month, today), today);
        assert_eq!((m.secs, m.sessions, m.days_played), (12600., 3, 4));
        assert_eq!(m.bars.len(), 31);
        assert_eq!(m.bars[2], (day(2026, 10, 3), 7200.));
        assert_eq!(m.bars[5].1, 1800.); // Oct 6, after midnight
        assert_eq!(m.best.unwrap().secs, 7200.);
        assert_eq!(m.usual_hour, Some(23));
        assert!((m.weekend - 7200. / 12600.).abs() < 1e-9);
        assert_eq!(m.first, Some(day(2025, 1, 2)));
        assert_eq!(m.life_secs, 13200.);
        assert_eq!(m.last.map(|l| l.date()), Some(day(2026, 10, 7)));

        let y = game_sheet(&h, 0, Period::of(Span::Year, today), today);
        // Weeks from Monday Dec 29, 2025 to today.
        assert_eq!(y.bars.first().map(|b| b.0), Some(day(2025, 12, 29)));
        assert_eq!(y.bars.len(), 41);
        assert_eq!(y.secs, 13200.);
        // Lifetime minutes beat the recorded sessions.
        assert_eq!(game_sheet(&h, 1, Period::of(Span::Year, today), today).life_secs, 30000.);
    }
}
