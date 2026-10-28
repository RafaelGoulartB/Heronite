//! Play session history of a game (`SessionHistory.tsx`): the list stored in
//! `local_library/sessions.json`, newest first, as the Collection shows it,
//! and the sessions the user adds, changes or deletes by hand.

use anyhow::{bail, Result};
use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Timelike, Utc,
};
use heronite_i18n::tr;

use crate::json_store;
use crate::models::{LocalGameSession, SessionsFile, MAX_SESSIONS_PER_GAME};
use crate::paths::DataPaths;
use crate::text::DateLocale;
use crate::JsNum;

/// `source` of a session added by hand.
pub const MANUAL_SOURCE: &str = "manual";

/// How many rows the history shows (`sessions.slice(0, 12)`).
pub const SHOWN_SESSIONS: usize = 12;

/// `getLocalSessions`: newest `startedAt` first (string compare, like the TS).
pub fn sorted_newest_first(sessions: &[LocalGameSession]) -> Vec<&LocalGameSession> {
    let mut list: Vec<&LocalGameSession> = sessions.iter().collect();
    list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    list
}

/// `formatDuration`: `HH:MM` (zero padded; hours are not wrapped).
pub fn format_session_duration(seconds: f64) -> String {
    let seconds = if seconds.is_finite() { seconds.max(0.0) as u64 } else { 0 };
    format!("{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60)
}

/// Subtitle source: Playnite imports say "Playnite", sessions added by hand
/// "Added by hand", everything else (stored as `heroic`, the launcher's own
/// sessions) "Heronite".
pub fn session_source_label(source: &str) -> &'static str {
    match source {
        "playnite" => "Playnite",
        MANUAL_SOURCE => tr!("Added by hand"),
        _ => "Heronite",
    }
}

/// The time played the sessions add up to, in whole minutes (like
/// `totalPlayed`, the seconds left over are dropped).
pub fn sessions_total_minutes(sessions: &[LocalGameSession]) -> u64 {
    let seconds: f64 = sessions.iter().map(|s| s.elapsed_seconds.get()).filter(|s| s.is_finite() && *s > 0.0).sum();
    (seconds / 60.0).floor() as u64
}

/// Start of a session in local time (`None` when `startedAt` is not a date).
pub fn session_start_local(session: &LocalGameSession) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(&session.started_at).ok().map(|d| d.with_timezone(&Local))
}

/// The date as the session editor shows it: the locale's numeric date.
pub fn date_text(dt: &DateTime<Local>, locale: DateLocale) -> String {
    locale.format(dt.year(), dt.month(), dt.day())
}

/// The time as the session editor and the history show it: `9:30 PM` in
/// English, `21:30` otherwise.
pub fn time_text(hour: u32, minute: u32, locale: DateLocale) -> String {
    match locale {
        DateLocale::En => {
            let h12 = match hour % 12 {
                0 => 12,
                h => h,
            };
            let suffix = if hour < 12 { "AM" } else { "PM" };
            format!("{h12}:{minute:02} {suffix}")
        }
        _ => format!("{hour:02}:{minute:02}"),
    }
}

/// A date typed in the session editor: the locale's order (`10/17/2026` in
/// English, `17/10/2026` in Portuguese; `/`, `.` or `-` between the parts)
/// or ISO `2026-10-17`. The year has four digits.
pub fn parse_date(text: &str, locale: DateLocale) -> Option<NaiveDate> {
    let parts: Vec<&str> = text.trim().split(['/', '.', '-']).map(str::trim).collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let num = |i: usize| parts[i].parse::<u32>().ok();
    let (y, m, d) = if parts[0].len() == 4 {
        (parts[0], num(1)?, num(2)?)
    } else {
        match locale {
            DateLocale::En => (parts[2], num(0)?, num(1)?),
            _ => (parts[2], num(1)?, num(0)?),
        }
    };
    if y.len() != 4 {
        return None;
    }
    NaiveDate::from_ymd_opt(y.parse().ok()?, m, d)
}

/// A time typed in the session editor: `21:30`, `9:30 pm`, `9 PM`, `21h30`.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let lower = text.trim().to_ascii_lowercase();
    let (clock, pm) = match lower.strip_suffix("pm").or_else(|| lower.strip_suffix("p.m.")) {
        Some(rest) => (rest.trim(), Some(true)),
        None => match lower.strip_suffix("am").or_else(|| lower.strip_suffix("a.m.")) {
            Some(rest) => (rest.trim(), Some(false)),
            None => (lower.as_str(), None),
        },
    };
    let (h, m) = match clock.split_once([':', 'h']) {
        Some((h, m)) => (h.trim(), if m.trim().is_empty() { "0" } else { m.trim() }),
        None => (clock, "0"),
    };
    if h.is_empty() || !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) || m.len() > 2 {
        return None;
    }
    let (mut hour, minute) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    if let Some(pm) = pm {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = match (hour, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, true) => h + 12,
            (h, false) => h,
        };
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// `start` (local time) as an instant. `None` when that time does not exist
/// (skipped by a DST change).
pub fn local_instant(start: NaiveDateTime) -> Option<DateTime<Utc>> {
    match Local.from_local_datetime(&start) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

/// The longest session the history takes (what Statistics counts), in
/// minutes.
pub const MAX_SESSION_MINUTES: u64 = (crate::stats::MAX_SESSION_SECS / 60.) as u64;

/// A session that starts at `start` and lasts `seconds`. A length that
/// reaches past the last date chrono has ends the session at its start
/// instead of panicking.
pub fn session_from(start: DateTime<Utc>, seconds: u64, source: &str) -> LocalGameSession {
    let iso = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = i64::try_from(seconds).ok().and_then(Duration::try_seconds).and_then(|d| start.checked_add_signed(d)).unwrap_or(start);
    LocalGameSession {
        started_at: iso(start),
        ended_at: iso(end),
        elapsed_seconds: JsNum(seconds as f64),
        source: source.to_string(),
        game_action_name: None,
        extra: Default::default(),
    }
}

/// The same stored session: same start, source and length (a running game
/// keeps rewriting the length of its own session).
fn same_session(a: &LocalGameSession, b: &LocalGameSession) -> bool {
    a.started_at == b.started_at && a.source == b.source && a.elapsed_seconds.get() == b.elapsed_seconds.get()
}

/// Changes one session of `app_name` in `local_library/sessions.json`:
/// `old` = `None` adds `new`, `new` = `None` deletes `old`, both replace it.
/// Only that entry changes, so sessions written meanwhile (a game running
/// now) stay. Fails when `old` is no longer in the file. Returns the game's
/// sessions after the change.
pub fn edit_session(
    paths: &DataPaths,
    app_name: &str,
    old: Option<&LocalGameSession>,
    new: Option<LocalGameSession>,
) -> Result<Vec<LocalGameSession>> {
    json_store::update::<SessionsFile, _>(&paths.sessions(), |doc| {
        let list = doc.sessions.entry(app_name.to_string()).or_default();
        let at = match old {
            Some(old) => match list.iter().position(|s| same_session(s, old)) {
                Some(i) => Some(i),
                None => bail!(tr!("This session changed in the meantime. Close the settings and open them again.")),
            },
            None => None,
        };
        match (at, new) {
            (Some(i), Some(new)) => list[i] = new,
            (Some(i), None) => {
                list.remove(i);
            }
            (None, Some(new)) => {
                list.push(new);
                if list.len() > MAX_SESSIONS_PER_GAME {
                    list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                    list.truncate(MAX_SESSIONS_PER_GAME);
                }
            }
            (None, None) => {}
        }
        let result = list.clone();
        if result.is_empty() {
            doc.sessions.shift_remove(app_name);
        }
        Ok(result)
    })
}

/// `Intl.DateTimeFormat(undefined, { year, month, day, hour, minute: 'numeric' })`
/// in local time (12-hour clock with AM/PM for English, 24-hour otherwise).
/// Unparseable text is returned as is.
pub fn format_session_date(value: &str, locale: DateLocale) -> String {
    match DateTime::parse_from_rfc3339(value) {
        Ok(instant) => format_local(instant.with_timezone(&Local), locale),
        Err(_) => value.to_string(),
    }
}

fn format_local<Tz: chrono::TimeZone>(dt: DateTime<Tz>, locale: DateLocale) -> String {
    let date = locale.format(dt.year(), dt.month(), dt.day());
    let time = time_text(dt.hour(), dt.minute(), locale);
    match locale {
        DateLocale::En => format!("{date}, {time}"),
        _ => format!("{date} {time}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JsNum;
    use chrono::{FixedOffset, TimeZone};

    fn session(started: &str, secs: f64) -> LocalGameSession {
        LocalGameSession {
            started_at: started.into(),
            ended_at: started.into(),
            elapsed_seconds: JsNum(secs),
            source: "heroic".into(),
            game_action_name: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn newest_first() {
        let list = vec![
            session("2025-05-26T04:11:44.558Z", 60.),
            session("2026-01-01T00:00:00.000Z", 5.),
            session("2025-05-30T03:21:39.257Z", 399.),
        ];
        let sorted = sorted_newest_first(&list);
        assert_eq!(sorted[0].started_at, "2026-01-01T00:00:00.000Z");
        assert_eq!(sorted[2].started_at, "2025-05-26T04:11:44.558Z");
    }

    #[test]
    fn duration_is_hh_mm() {
        assert_eq!(format_session_duration(60.), "00:01");
        assert_eq!(format_session_duration(399.), "00:06");
        assert_eq!(format_session_duration(3600. * 12. + 59. * 60. + 59.), "12:59");
        assert_eq!(format_session_duration(100. * 3600.), "100:00");
        assert_eq!(format_session_duration(-4.), "00:00");
    }

    #[test]
    fn source_labels() {
        assert_eq!(session_source_label("playnite"), "Playnite");
        assert_eq!(session_source_label("heroic"), "Heronite");
        assert_eq!(session_source_label("other"), "Heronite");
    }

    #[test]
    fn total_drops_the_leftover_seconds() {
        let list = vec![session("a", 90.), session("b", 100.), session("c", -5.), session("d", f64::NAN)];
        assert_eq!(sessions_total_minutes(&list), 3);
        assert_eq!(sessions_total_minutes(&[]), 0);
    }

    #[test]
    fn dates_follow_the_locale_or_iso() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd);
        assert_eq!(parse_date("10/17/2026", DateLocale::En), d(2026, 10, 17));
        assert_eq!(parse_date("17/10/2026", DateLocale::Pt), d(2026, 10, 17));
        assert_eq!(parse_date("17.10.2026", DateLocale::De), d(2026, 10, 17));
        assert_eq!(parse_date(" 2026-10-17 ", DateLocale::Pt), d(2026, 10, 17));
        assert_eq!(parse_date("17/10/2026", DateLocale::En), None);
        assert_eq!(parse_date("17/10/26", DateLocale::Pt), None);
        assert_eq!(parse_date("31/02/2026", DateLocale::Pt), None);
        assert_eq!(parse_date("today", DateLocale::Pt), None);
    }

    #[test]
    fn times_take_24_hours_and_am_pm() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_time("21:30"), t(21, 30));
        assert_eq!(parse_time("9:05 PM"), t(21, 5));
        assert_eq!(parse_time("12:10 am"), t(0, 10));
        assert_eq!(parse_time("12 pm"), t(12, 0));
        assert_eq!(parse_time("21h30"), t(21, 30));
        assert_eq!(parse_time("7"), t(7, 0));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("13 pm"), None);
        assert_eq!(parse_time("9:5:3"), None);
        assert_eq!(parse_time(""), None);
        assert_eq!(time_text(21, 5, DateLocale::En), "9:05 PM");
        assert_eq!(time_text(0, 10, DateLocale::En), "12:10 AM");
        assert_eq!(time_text(21, 5, DateLocale::Pt), "21:05");
    }

    #[test]
    fn a_session_at_a_local_time_ends_after_its_length() {
        let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap().and_hms_opt(20, 0, 0).unwrap();
        let s = session_from(local_instant(start).unwrap(), 5400, MANUAL_SOURCE);
        let (a, b) = (DateTime::parse_from_rfc3339(&s.started_at).unwrap(), DateTime::parse_from_rfc3339(&s.ended_at).unwrap());
        assert_eq!((b - a).num_seconds(), 5400);
        assert_eq!(a.with_timezone(&Local).naive_local(), start);
        assert!(s.started_at.ends_with('Z'));
        assert_eq!(s.source, "manual");
    }

    #[test]
    fn a_session_too_long_for_the_calendar_does_not_panic() {
        let start = Utc.with_ymd_and_hms(2026, 3, 1, 20, 0, 0).unwrap();
        for seconds in [u64::MAX, i64::MAX as u64, 10_000_000_000_000, 36_000_000_000_000] {
            let s = session_from(start, seconds, MANUAL_SOURCE);
            assert_eq!(s.elapsed_seconds.get(), seconds as f64);
            assert!(DateTime::parse_from_rfc3339(&s.ended_at).is_ok());
        }
        let near_end = session_from(DateTime::<Utc>::MAX_UTC, 3600, MANUAL_SOURCE);
        assert_eq!(near_end.started_at, near_end.ended_at);
        assert_eq!(MAX_SESSION_MINUTES, 28 * 24 * 60);
    }

    #[test]
    fn edits_touch_only_their_session() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let a = session("2026-01-01T00:00:00.000Z", 60.);
        let b = session("2026-01-02T00:00:00.000Z", 120.);
        edit_session(&paths, "g", None, Some(a.clone())).unwrap();
        edit_session(&paths, "g", None, Some(b.clone())).unwrap();
        edit_session(&paths, "other", None, Some(a.clone())).unwrap();

        let longer = session("2026-01-01T00:00:00.000Z", 600.);
        let list = edit_session(&paths, "g", Some(&a), Some(longer.clone())).unwrap();
        assert_eq!(list.iter().map(|s| s.elapsed_seconds.get()).collect::<Vec<_>>(), vec![600., 120.]);

        // `a` is gone: changing it again fails and writes nothing.
        assert!(edit_session(&paths, "g", Some(&a), None).is_err());
        let list = edit_session(&paths, "g", Some(&b), None).unwrap();
        assert_eq!(list.len(), 1);
        let list = edit_session(&paths, "g", Some(&longer), None).unwrap();
        assert!(list.is_empty());

        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert!(!doc.sessions.contains_key("g"));
        assert_eq!(doc.sessions["other"].len(), 1);
    }

    #[test]
    fn date_layouts() {
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let dt = tz.with_ymd_and_hms(2026, 10, 8, 0, 5, 0).unwrap();
        assert_eq!(format_local(dt, DateLocale::En), "10/8/2026, 12:05 AM");
        let dt = tz.with_ymd_and_hms(2026, 10, 8, 15, 30, 0).unwrap();
        assert_eq!(format_local(dt, DateLocale::En), "10/8/2026, 3:30 PM");
        assert_eq!(format_local(dt, DateLocale::Pt), "08/10/2026 15:30");
        assert_eq!(format_session_date("garbage", DateLocale::En), "garbage");
    }
}
