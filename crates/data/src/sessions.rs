//! Play session history of a game (`SessionHistory.tsx`): the list stored in
//! `local_library/sessions.json`, newest first, as the Collection shows it.

use chrono::{DateTime, Datelike, Local, Timelike};

use crate::models::LocalGameSession;
use crate::text::DateLocale;

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

/// Subtitle source: Playnite imports say "Playnite", everything else "Heroic".
pub fn session_source_label(source: &str) -> &'static str {
    if source == "playnite" {
        "Playnite"
    } else {
        "Heroic"
    }
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
    let (hour, minute) = (dt.hour(), dt.minute());
    match locale {
        DateLocale::En => {
            let h12 = match hour % 12 {
                0 => 12,
                h => h,
            };
            let suffix = if hour < 12 { "AM" } else { "PM" };
            format!("{date}, {h12}:{minute:02} {suffix}")
        }
        _ => format!("{date} {hour:02}:{minute:02}"),
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
        assert_eq!(session_source_label("heroic"), "Heroic");
        assert_eq!(session_source_label("other"), "Heroic");
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
