//! Recording what was played, exactly like Heronite does when a game exits:
//! `store/timestamp.json`, `local_library/sessions.json` and
//! `store/config.json` -> `games.recent`.
//!
//! Heroic's own sequence (`launchEventCallback` + `recordLocalSession` +
//! `addRecentGame`):
//! 1. at launch, `firstPlayed` is set only if the game has no entry yet;
//! 2. on exit, `lastPlayed` = end time, `totalPlayed` (minutes) =
//!    `floor(sessionMinutes + previousTotal)`;
//! 3. a session is stored unless it lasted under 5 s (source `heroic`,
//!    deduplicated by `startedAt:elapsedSeconds`, newest 200 kept);
//! 4. the game moves to the front of `games.recent` (no length cap on write).

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use heronite_data::json_store;
use heronite_data::models::{LocalGameSession, SessionsFile, MAX_SESSIONS_PER_GAME};
use heronite_data::{HeroicPaths, JsNum};
use serde_json::{json, Map, Value};

/// Sessions shorter than this are not stored (playtime is still counted).
pub const MIN_SESSION_SECONDS: i64 = 5;

/// JavaScript's `Date.prototype.toISOString()`: `2026-10-07T12:00:00.123Z`.
pub fn iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Debug, Clone)]
pub struct PlayRecord {
    pub app_name: String,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordOutcome {
    pub elapsed_seconds: i64,
    /// Whether a row was added to `sessions.json` (false if < 5 s or duplicate).
    pub session_recorded: bool,
    /// New `totalPlayed`, minutes.
    pub total_played_minutes: i64,
}

fn entry_mut<'a>(root: &'a mut Value, app_name: &str) -> &'a mut Map<String, Value> {
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let map = root.as_object_mut().unwrap();
    let entry = map
        .entry(app_name.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry.as_object_mut().unwrap()
}

/// Launch-time bookkeeping: `firstPlayed` if the game has no entry yet.
pub fn mark_started(paths: &HeroicPaths, app_name: &str, started_at: DateTime<Utc>) -> Result<()> {
    json_store::update_value(&paths.timestamps(), |root| {
        let exists = root
            .as_object()
            .map(|m| m.contains_key(app_name))
            .unwrap_or(false);
        if !exists {
            entry_mut(root, app_name).insert("firstPlayed".into(), Value::String(iso(started_at)));
        }
        Ok(())
    })
}

/// Updates `lastPlayed` / `totalPlayed` in `store/timestamp.json`.
pub fn update_timestamps(paths: &HeroicPaths, rec: &PlayRecord) -> Result<i64> {
    json_store::update_value(&paths.timestamps(), |root| {
        let entry = entry_mut(root, &rec.app_name);
        if !entry.contains_key("firstPlayed") && entry.is_empty() {
            entry.insert("firstPlayed".into(), Value::String(iso(rec.started_at)));
        }
        entry.insert("lastPlayed".into(), Value::String(iso(rec.ended_at)));
        let previous = entry
            .get("totalPlayed")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let session_minutes =
            (rec.ended_at - rec.started_at).num_milliseconds() as f64 / 1000.0 / 60.0;
        let total = (session_minutes + previous).floor() as i64;
        entry.insert("totalPlayed".into(), json!(total));
        Ok(total)
    })
}

/// `mergeLocalSessions`: append, drop duplicates, keep the newest 200.
pub fn add_session(paths: &HeroicPaths, app_name: &str, session: LocalGameSession) -> Result<bool> {
    json_store::update::<SessionsFile, _>(&paths.sessions(), |doc| {
        let list = doc.sessions.entry(app_name.to_string()).or_default();
        let key = |s: &LocalGameSession| format!("{}:{}", s.started_at, s.elapsed_seconds.get());
        let new_key = key(&session);
        if list.iter().any(|s| key(s) == new_key) {
            return Ok(false);
        }
        list.push(session);
        if list.len() > MAX_SESSIONS_PER_GAME {
            list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            list.truncate(MAX_SESSIONS_PER_GAME);
        }
        Ok(true)
    })
}

/// `addRecentGame`: the game goes to the front of `games.recent`.
pub fn add_recent_game(paths: &HeroicPaths, app_name: &str, title: &str) -> Result<()> {
    json_store::update_value(&paths.config(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let map = root.as_object_mut().unwrap();
        let games = map
            .entry("games")
            .or_insert_with(|| Value::Object(Map::new()));
        if !games.is_object() {
            *games = Value::Object(Map::new());
        }
        let games = games.as_object_mut().unwrap();
        let recent = games
            .entry("recent")
            .or_insert_with(|| Value::Array(Vec::new()));
        if !recent.is_array() {
            *recent = Value::Array(Vec::new());
        }
        let list = recent.as_array_mut().unwrap();
        // Entries without an appName are dropped, like Heroic's filter does.
        list.retain(|g| {
            g.get("appName")
                .and_then(Value::as_str)
                .map(|a| !a.is_empty() && a != app_name)
                .unwrap_or(false)
        });
        list.insert(0, json!({ "appName": app_name, "title": title }));
        Ok(())
    })
}

/// Everything Heronite does after a game exits.
pub fn record_play(
    paths: &HeroicPaths,
    rec: &PlayRecord,
    min_session_seconds: i64,
) -> Result<RecordOutcome> {
    let total = update_timestamps(paths, rec)?;
    let elapsed = (rec.ended_at - rec.started_at)
        .num_milliseconds()
        .div_euclid(1000);
    let mut session_recorded = false;
    if elapsed >= min_session_seconds {
        session_recorded = add_session(
            paths,
            &rec.app_name,
            LocalGameSession {
                started_at: iso(rec.started_at),
                ended_at: iso(rec.ended_at),
                elapsed_seconds: JsNum(elapsed as f64),
                source: "heroic".into(),
                game_action_name: None,
                extra: Default::default(),
            },
        )?;
    }
    add_recent_game(paths, &rec.app_name, &rec.title)?;
    Ok(RecordOutcome {
        elapsed_seconds: elapsed,
        session_recorded,
        total_played_minutes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(ms: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap() + Duration::milliseconds(ms)
    }

    fn read(p: &std::path::Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn iso_matches_javascript() {
        assert_eq!(iso(t(123)), "2026-10-07T12:00:00.123Z");
        assert_eq!(iso(t(0)), "2026-10-07T12:00:00.000Z");
    }

    #[test]
    fn first_play_creates_files_in_electron_store_format() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        mark_started(&paths, "g1", t(0)).unwrap();
        assert_eq!(
            read(&paths.timestamps()),
            "{\n\t\"g1\": {\n\t\t\"firstPlayed\": \"2026-10-07T12:00:00.000Z\"\n\t}\n}"
        );
        // Second launch does not move firstPlayed.
        mark_started(&paths, "g1", t(99_000)).unwrap();

        let rec = PlayRecord {
            app_name: "g1".into(),
            title: "Game One".into(),
            started_at: t(0),
            ended_at: t(90_500),
        };
        let out = record_play(&paths, &rec, MIN_SESSION_SECONDS).unwrap();
        assert_eq!(
            out,
            RecordOutcome {
                elapsed_seconds: 90,
                session_recorded: true,
                total_played_minutes: 1
            }
        );

        assert_eq!(
            read(&paths.timestamps()),
            "{\n\t\"g1\": {\n\t\t\"firstPlayed\": \"2026-10-07T12:00:00.000Z\",\n\t\t\"lastPlayed\": \"2026-10-07T12:01:30.500Z\",\n\t\t\"totalPlayed\": 1\n\t}\n}"
        );
        let sessions: Value = serde_json::from_str(&read(&paths.sessions())).unwrap();
        assert_eq!(
            sessions["sessions"]["g1"][0],
            json!({"startedAt":"2026-10-07T12:00:00.000Z","endedAt":"2026-10-07T12:01:30.500Z","elapsedSeconds":90,"source":"heroic"})
        );
        let cfg: Value = serde_json::from_str(&read(&paths.config())).unwrap();
        assert_eq!(
            cfg["games"]["recent"],
            json!([{"appName":"g1","title":"Game One"}])
        );
    }

    #[test]
    fn total_played_accumulates_fractional_minutes_then_floors() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        std::fs::create_dir_all(paths.timestamps().parent().unwrap()).unwrap();
        std::fs::write(
            paths.timestamps(),
            r#"{"g1":{"firstPlayed":"2025-01-01T00:00:00.000Z","lastPlayed":"2025-01-01T01:00:00.000Z","totalPlayed":200,"other":1},"g2":{"totalPlayed":5}}"#,
        )
        .unwrap();
        let rec = PlayRecord {
            app_name: "g1".into(),
            title: "x".into(),
            started_at: t(0),
            ended_at: t(150_000),
        }; // 2.5 min
        assert_eq!(update_timestamps(&paths, &rec).unwrap(), 202);
        let v: Value = serde_json::from_str(&read(&paths.timestamps())).unwrap();
        assert_eq!(v["g1"]["totalPlayed"], 202);
        assert_eq!(v["g1"]["firstPlayed"], "2025-01-01T00:00:00.000Z");
        assert_eq!(v["g1"]["other"], 1, "unknown fields survive");
        assert_eq!(v["g2"]["totalPlayed"], 5, "other games untouched");
    }

    #[test]
    fn short_sessions_only_touch_timestamps_and_recent() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        let rec = PlayRecord {
            app_name: "g1".into(),
            title: "x".into(),
            started_at: t(0),
            ended_at: t(4_999),
        };
        let out = record_play(&paths, &rec, MIN_SESSION_SECONDS).unwrap();
        assert_eq!(out.elapsed_seconds, 4);
        assert!(!out.session_recorded);
        assert!(!paths.sessions().exists());
        assert!(paths.timestamps().exists() && paths.config().exists());
    }

    #[test]
    fn sessions_dedupe_and_cap_at_200_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        let mk = |i: i64| PlayRecord {
            app_name: "g1".into(),
            title: "x".into(),
            started_at: t(i * 60_000),
            ended_at: t(i * 60_000 + 30_000),
        };
        for i in 0..203 {
            record_play(&paths, &mk(i), 5).unwrap();
        }
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        let list = &doc.sessions["g1"];
        assert_eq!(list.len(), 200);
        // Trimming sorts newest first and drops the oldest.
        assert_eq!(list[0].started_at, iso(t(202 * 60_000)));
        assert_eq!(list[199].started_at, iso(t(3 * 60_000)));
        // The same session twice is stored once.
        let before = list.len();
        assert!(!add_session(&paths, "g1", list[0].clone()).unwrap());
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert_eq!(doc.sessions["g1"].len(), before);
    }

    #[test]
    fn recent_moves_to_front_without_duplicates_and_keeps_other_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        std::fs::create_dir_all(paths.config().parent().unwrap()).unwrap();
        std::fs::write(
            paths.config(),
            r#"{"language":"en","games":{"recent":[{"appName":"a","title":"A"},{"title":"orphan"},{"appName":"b","title":"B"}],"hidden":[{"appName":"h","title":"H"}]}}"#,
        )
        .unwrap();
        add_recent_game(&paths, "b", "B2").unwrap();
        let v: Value = serde_json::from_str(&read(&paths.config())).unwrap();
        assert_eq!(
            v["games"]["recent"],
            json!([{"appName":"b","title":"B2"},{"appName":"a","title":"A"}])
        );
        assert_eq!(v["games"]["hidden"][0]["appName"], "h");
        assert_eq!(v["language"], "en");
    }
}
