//! Timeline panel data: play sessions and screenshots grouped by local day.
//!
//! A session belongs to the day it started. A screenshot taken during a
//! session (or shortly after it) goes to that session's day, so a game played
//! past midnight keeps its screenshots together; other screenshots go to the
//! day they were taken.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone};

use crate::models::LocalGameSession;
use crate::stats::MIN_SESSION_SECS;

/// A screenshot up to this long after a session ended still belongs to it.
pub const SHOT_GRACE_SECS: i64 = 600;

/// One day of the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineDay {
    /// Local day.
    pub date: NaiveDate,
    /// Sum of the counted sessions that started this day.
    pub played_secs: f64,
    /// Counted sessions that started this day.
    pub sessions: usize,
    /// Earliest session start this day, local time.
    pub first_start: Option<NaiveTime>,
    /// Indices into the `shots` input, oldest first.
    pub shots: Vec<usize>,
}

/// A counted session as a window of unix milliseconds.
struct Window {
    start_ms: i64,
    end_ms: i64,
    date: NaiveDate,
}

fn local_date<Tz: TimeZone>(ms: i64, tz: &Tz) -> Option<NaiveDate> {
    tz.timestamp_millis_opt(ms).single().map(|d| d.date_naive())
}

fn parse_ms(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.timestamp_millis())
}

fn day_entry(days: &mut BTreeMap<NaiveDate, TimelineDay>, date: NaiveDate) -> &mut TimelineDay {
    days.entry(date).or_insert_with(|| TimelineDay {
        date,
        played_secs: 0.,
        sessions: 0,
        first_start: None,
        shots: Vec::new(),
    })
}

/// Groups sessions and screenshots into days, newest day first.
///
/// Sessions shorter than [`MIN_SESSION_SECS`] and sessions with an unparseable
/// start are ignored. `shots_taken_at_ms` holds the unix time of each
/// screenshot in milliseconds; `None` entries are skipped.
pub fn build_timeline<Tz: TimeZone>(
    sessions: &[LocalGameSession],
    shots_taken_at_ms: &[Option<f64>],
    tz: &Tz,
) -> Vec<TimelineDay> {
    let mut days: BTreeMap<NaiveDate, TimelineDay> = BTreeMap::new();
    let mut windows = Vec::new();
    for session in sessions {
        let elapsed = session.elapsed_seconds.get();
        if elapsed.is_nan() || elapsed < MIN_SESSION_SECS {
            continue;
        }
        let Some(start_ms) = parse_ms(&session.started_at) else { continue };
        let Some(start) = tz.timestamp_millis_opt(start_ms).single() else { continue };
        let end_ms = parse_ms(&session.ended_at)
            .unwrap_or(start_ms + (elapsed * 1000.) as i64);
        let date = start.date_naive();
        let day = day_entry(&mut days, date);
        day.played_secs += elapsed;
        day.sessions += 1;
        let time = start.time();
        day.first_start = Some(day.first_start.map_or(time, |t| t.min(time)));
        windows.push(Window { start_ms, end_ms, date });
    }

    let grace_ms = SHOT_GRACE_SECS * 1000;
    let mut placed: Vec<(i64, usize, NaiveDate)> = Vec::new();
    for (index, taken) in shots_taken_at_ms.iter().enumerate() {
        let Some(ms) = taken.filter(|t| t.is_finite()).map(|t| t as i64) else { continue };
        // The latest-starting matching session wins when windows overlap.
        let session = windows
            .iter()
            .filter(|w| w.start_ms <= ms && ms <= w.end_ms + grace_ms)
            .max_by_key(|w| w.start_ms);
        let Some(date) = session.map(|w| w.date).or_else(|| local_date(ms, tz)) else { continue };
        placed.push((ms, index, date));
    }
    placed.sort_unstable();
    for (_, index, date) in placed {
        day_entry(&mut days, date).shots.push(index);
    }

    days.into_values().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::JsNum;
    use chrono::{Duration, FixedOffset, Utc};

    fn tz() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    fn session(start: &str, secs: f64) -> LocalGameSession {
        let started = DateTime::parse_from_rfc3339(start).unwrap();
        let ended = started + Duration::milliseconds((secs * 1000.) as i64);
        LocalGameSession {
            started_at: start.to_owned(),
            ended_at: ended.with_timezone(&Utc).to_rfc3339(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            ..Default::default()
        }
    }

    fn ms(rfc3339: &str) -> Option<f64> {
        Some(DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp_millis() as f64)
    }

    #[test]
    fn groups_sessions_by_local_day_newest_first() {
        let sessions = [
            // 2026-10-07 22:00 local (-03:00), and 2026-10-08 00:30 local.
            session("2026-10-08T01:00:00Z", 3600.),
            session("2026-10-08T03:30:00Z", 1800.),
            session("2026-10-08T18:00:00Z", 600.),
        ];
        let days = build_timeline(&sessions, &[], &tz());
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert_eq!((days[0].sessions, days[0].played_secs), (2, 2400.));
        assert_eq!(days[0].first_start, NaiveTime::from_hms_opt(0, 30, 0));
        assert_eq!(days[1].date, NaiveDate::from_ymd_opt(2026, 10, 7).unwrap());
        assert_eq!((days[1].sessions, days[1].played_secs), (1, 3600.));
        assert_eq!(days[1].first_start, NaiveTime::from_hms_opt(22, 0, 0));
    }

    #[test]
    fn short_and_unparseable_sessions_are_ignored() {
        let mut broken = session("2026-10-08T12:00:00Z", 3600.);
        broken.started_at = "yesterday".into();
        let sessions = [session("2026-10-08T12:00:00Z", 119.), broken];
        assert!(build_timeline(&sessions, &[], &tz()).is_empty());
        let days = build_timeline(&[session("2026-10-08T12:00:00Z", 120.)], &[], &tz());
        assert_eq!(days.len(), 1);
    }

    #[test]
    fn session_crossing_midnight_keeps_its_shots_on_the_start_day() {
        // 23:30 local on the 7th for two hours, ends 01:30 on the 8th.
        let sessions = [session("2026-10-08T02:30:00Z", 7200.)];
        let shots = [
            ms("2026-10-08T03:30:00Z"), // 00:30 local on the 8th, during the session
            ms("2026-10-08T04:35:00Z"), // 01:35 local, inside the grace period
            ms("2026-10-08T04:41:00Z"), // 01:41 local, past the grace period
        ];
        let days = build_timeline(&sessions, &shots, &tz());
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert_eq!((days[0].sessions, days[0].shots.clone()), (0, vec![2]));
        assert_eq!(days[1].date, NaiveDate::from_ymd_opt(2026, 10, 7).unwrap());
        assert_eq!(days[1].shots, vec![0, 1]);
    }

    #[test]
    fn screenshot_only_days_and_ordering() {
        let shots = [
            ms("2026-10-05T15:00:00Z"),
            None,
            ms("2026-10-05T12:00:00Z"),
            ms("2026-10-06T01:00:00Z"), // 22:00 local on the 5th
        ];
        let days = build_timeline(&[], &shots, &tz());
        assert_eq!(days.len(), 1);
        let day = &days[0];
        assert_eq!(day.date, NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
        assert_eq!((day.sessions, day.played_secs, day.first_start), (0, 0., None));
        assert_eq!(day.shots, vec![2, 0, 3]);
    }

    #[test]
    fn unparseable_end_falls_back_to_elapsed() {
        // Starts 23:55 local on the 7th and lasts ten minutes.
        let mut s = session("2026-10-08T02:55:00Z", 600.);
        s.ended_at = String::new();
        let shots = [ms("2026-10-08T03:10:00Z"), ms("2026-10-08T03:20:00Z")];
        let days = build_timeline(&[s], &shots, &tz());
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].shots, vec![1]);
        assert_eq!(days[1].shots, vec![0]);
    }
}
