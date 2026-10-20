//! The album: screenshots taken in game as statistics. Moments (many
//! captures close together: something happened there), how many captures a
//! session leaves, and when you capture against when you play.
//!
//! Screenshots come in as `(game, taken at)` in local time, the game being
//! the index into the [`super::history`] input.

use chrono::{NaiveDateTime, Timelike};

use super::{History, Period};

/// Captures this close (seconds) belong to the same moment.
pub const MOMENT_GAP_SECS: i64 = 180;
/// A moment has at least this many captures.
pub const MOMENT_MIN: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    pub game: usize,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    /// Indexes into the screenshot list given to [`moments`], oldest first.
    pub shots: Vec<usize>,
}

impl Moment {
    pub fn minutes(&self) -> i64 {
        (self.end - self.start).num_minutes().max(1)
    }
}

/// Runs of at least [`MOMENT_MIN`] captures of one game with at most
/// [`MOMENT_GAP_SECS`] between two, most captures first (then newest).
pub fn moments(shots: &[(usize, NaiveDateTime)]) -> Vec<Moment> {
    let mut order: Vec<usize> = (0..shots.len()).collect();
    order.sort_by_key(|i| (shots[*i].0, shots[*i].1));
    let mut out = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    let close = |run: &mut Vec<usize>, out: &mut Vec<Moment>| {
        if run.len() >= MOMENT_MIN {
            out.push(Moment { game: shots[run[0]].0, start: shots[run[0]].1, end: shots[*run.last().expect("run")].1, shots: run.clone() });
        }
        run.clear();
    };
    for i in order {
        if let Some(&last) = run.last() {
            let (g, at) = shots[last];
            if g != shots[i].0 || (shots[i].1 - at).num_seconds() > MOMENT_GAP_SECS {
                close(&mut run, &mut out);
            }
        }
        run.push(i);
    }
    close(&mut run, &mut out);
    out.sort_by(|a, b| b.shots.len().cmp(&a.shots.len()).then(b.start.cmp(&a.start)));
    out
}

/// Sessions of `period` by how many captures they left: 0, 1, 2–4, 5–9, 10+.
pub fn captures_per_session(h: &History, shots: &[(usize, NaiveDateTime)], period: Period) -> [usize; 5] {
    let mut out = [0; 5];
    for s in h.sessions.iter().filter(|s| period.contains(s.start.date())) {
        let end = s.start + chrono::Duration::seconds(s.secs as i64);
        let n = shots.iter().filter(|(g, at)| *g == s.game && *at >= s.start && *at <= end).count();
        out[match n {
            0 => 0,
            1 => 1,
            2..=4 => 2,
            5..=9 => 3,
            _ => 4,
        }] += 1;
    }
    out
}

/// Share (0..=1) of the captures, and of the play time of `period`, in each
/// hour of the day.
pub fn capture_hours(h: &History, shots: &[(usize, NaiveDateTime)], period: Period) -> ([f64; 24], [f64; 24]) {
    let mut caps = [0f64; 24];
    for (_, at) in shots.iter().filter(|(_, at)| period.contains(at.date())) {
        caps[at.hour() as usize] += 1.;
    }
    let mut play = [0f64; 24];
    for x in h.pieces.iter().filter(|x| period.contains(x.at.date())) {
        play[x.at.hour() as usize] += x.secs as f64;
    }
    let norm = |v: &mut [f64; 24]| {
        let t: f64 = v.iter().sum();
        if t > 0. {
            v.iter_mut().for_each(|x| *x /= t);
        }
    };
    norm(&mut caps);
    norm(&mut play);
    (caps, play)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LocalGameSession;
    use crate::stats::{history_in, GameInput, Span};
    use crate::JsNum;
    use chrono::{FixedOffset, NaiveDate};

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn moments_sessions_and_hours() {
        let mut shots: Vec<(usize, NaiveDateTime)> =
            (0..6).map(|i| (0, at("2026-10-03 20:00:00") + chrono::Duration::seconds(i * 60))).collect();
        // A gap of 10 min breaks the run; the other game never makes one.
        shots.push((0, at("2026-10-03 20:20:00")));
        shots.extend((0..3).map(|i| (1, at("2026-10-04 13:00:00") + chrono::Duration::seconds(i * 30))));
        let m = moments(&shots);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].game, m[0].shots.len(), m[0].minutes()), (0, 6, 5));

        let s = |started: &str, secs: f64| LocalGameSession {
            started_at: started.into(),
            ended_at: started.into(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            game_action_name: None,
            extra: Default::default(),
        };
        let a = [s("2026-10-03T19:30:00Z", 3600.), s("2026-10-05T10:00:00Z", 600.)];
        let b = [s("2026-10-04T12:50:00Z", 1800.)];
        let h = history_in(
            [
                GameInput { sessions: &a, lifetime_minutes: 0, first_played: None },
                GameInput { sessions: &b, lifetime_minutes: 0, first_played: None },
            ],
            &FixedOffset::east_opt(0).unwrap(),
        );
        let p = Period::of(Span::Month, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        // 19:30–20:30 holds 7 captures, Oct 4 holds 3, Oct 5 none.
        assert_eq!(captures_per_session(&h, &shots, p), [1, 0, 1, 1, 0]);
        let (caps, play) = capture_hours(&h, &shots, p);
        assert!((caps[20] - 0.7).abs() < 1e-9 && (caps[13] - 0.3).abs() < 1e-9);
        assert!((play.iter().sum::<f64>() - 1.).abs() < 1e-9);
    }
}
