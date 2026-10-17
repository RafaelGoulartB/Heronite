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
//!    deduplicated by `startedAt:elapsedSeconds`, newest
//!    [`MAX_SESSIONS_PER_GAME`] kept);
//! 4. the game moves to the front of `games.recent` (no length cap on write).
//!
//! Unlike Heroic, a running session is also *checkpointed* (see
//! [`Checkpointer`]) so a crash, a kill or a power loss costs at most one
//! checkpoint interval of playtime. Checkpoints are idempotent: every write
//! derives from the totals before the session began and the elapsed time so
//! far, so the final write on exit replaces them instead of adding to them.

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use heronite_data::json_store;
use heronite_data::models::{LocalGameSession, SessionsFile, MAX_SESSIONS_PER_GAME};
use heronite_data::{DataPaths, JsNum};
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
    let entry = map.entry(app_name.to_string()).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry.as_object_mut().unwrap()
}

/// Launch-time bookkeeping: `firstPlayed` if the game has no entry yet.
pub fn mark_started(paths: &DataPaths, app_name: &str, started_at: DateTime<Utc>) -> Result<()> {
    json_store::update_value(&paths.timestamps(), |root| {
        let exists = root.as_object().map(|m| m.contains_key(app_name)).unwrap_or(false);
        if !exists {
            entry_mut(root, app_name).insert("firstPlayed".into(), Value::String(iso(started_at)));
        }
        Ok(())
    })
}

/// Updates `lastPlayed` / `totalPlayed` in `store/timestamp.json`.
pub fn update_timestamps(paths: &DataPaths, rec: &PlayRecord) -> Result<i64> {
    update_timestamps_with_base(paths, rec, &mut None)
}

/// Same, but `totalPlayed` is `floor(base + session minutes)` where `base` is
/// the stored total before the session. `base` is read from the file the
/// first time (`None`) and then kept by the caller, so repeating the call with
/// a longer session overwrites the previous result instead of adding to it.
fn update_timestamps_with_base(paths: &DataPaths, rec: &PlayRecord, base: &mut Option<f64>) -> Result<i64> {
    let known = *base;
    let (total, used) = json_store::update_value(&paths.timestamps(), |root| {
        let entry = entry_mut(root, &rec.app_name);
        if !entry.contains_key("firstPlayed") && entry.is_empty() {
            entry.insert("firstPlayed".into(), Value::String(iso(rec.started_at)));
        }
        entry.insert("lastPlayed".into(), Value::String(iso(rec.ended_at)));
        let previous = known.unwrap_or_else(|| entry.get("totalPlayed").and_then(Value::as_f64).unwrap_or(0.0));
        let session_minutes = (rec.ended_at - rec.started_at).num_milliseconds() as f64 / 1000.0 / 60.0;
        let total = (session_minutes + previous).floor() as i64;
        entry.insert("totalPlayed".into(), json!(total));
        Ok((total, previous))
    })?;
    *base = Some(used);
    Ok(total)
}

/// `mergeLocalSessions`: append, drop duplicates, keep the newest
/// [`MAX_SESSIONS_PER_GAME`].
pub fn add_session(paths: &DataPaths, app_name: &str, session: LocalGameSession) -> Result<bool> {
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

/// Stores a running (or just ended) session: the entry with the same
/// `startedAt` from this app is updated in place, otherwise it is added.
/// Returns whether the file changed.
pub fn upsert_session(paths: &DataPaths, app_name: &str, session: LocalGameSession) -> Result<bool> {
    json_store::update::<SessionsFile, _>(&paths.sessions(), |doc| {
        let list = doc.sessions.entry(app_name.to_string()).or_default();
        if let Some(existing) = list.iter_mut().find(|s| s.source == session.source && s.started_at == session.started_at) {
            if existing.ended_at == session.ended_at && existing.elapsed_seconds.get() == session.elapsed_seconds.get() {
                return Ok(false);
            }
            existing.ended_at = session.ended_at;
            existing.elapsed_seconds = session.elapsed_seconds;
            return Ok(true);
        }
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
pub fn add_recent_game(paths: &DataPaths, app_name: &str, title: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let map = root.as_object_mut().unwrap();
        let games = map.entry("games").or_insert_with(|| Value::Object(Map::new()));
        if !games.is_object() {
            *games = Value::Object(Map::new());
        }
        let games = games.as_object_mut().unwrap();
        let recent = games.entry("recent").or_insert_with(|| Value::Array(Vec::new()));
        if !recent.is_array() {
            *recent = Value::Array(Vec::new());
        }
        let list = recent.as_array_mut().unwrap();
        // Entries without an appName are dropped, like Heroic's filter does.
        list.retain(|g| g.get("appName").and_then(Value::as_str).map(|a| !a.is_empty() && a != app_name).unwrap_or(false));
        list.insert(0, json!({ "appName": app_name, "title": title }));
        Ok(())
    })
}

/// Everything Heronite does after a game exits.
pub fn record_play(paths: &DataPaths, rec: &PlayRecord, min_session_seconds: i64) -> Result<RecordOutcome> {
    Checkpointer::default().write(paths, rec, min_session_seconds)
}

/// Writes the progress of one session, any number of times, and finally its
/// end. Keep one per session; nothing is double counted however often
/// [`Checkpointer::write`] runs, because each call recomputes the totals from
/// the state before the session (`totalPlayed`) and updates the session row in
/// place (same `startedAt`).
#[derive(Debug, Default)]
pub struct Checkpointer {
    /// `totalPlayed` before this session, read at the first write.
    base_total: Option<f64>,
    /// `games.recent` is touched once per session.
    recent_done: bool,
}

impl Checkpointer {
    /// Picks a session up where another run of the app left it (see
    /// [`Self::base_total`] and [`Self::has_written`]).
    pub fn resume(base_total: Option<f64>, written: bool) -> Self {
        Self { base_total, recent_done: written }
    }

    /// Reads `totalPlayed` now, before the session writes anything, so the
    /// base is known (and can be written down) from the start.
    pub fn begin(&mut self, paths: &DataPaths, app_name: &str) -> Result<()> {
        if self.base_total.is_none() {
            let root = json_store::read_value(&paths.timestamps())?;
            self.base_total = Some(root.get(app_name).and_then(|e| e.get("totalPlayed")).and_then(Value::as_f64).unwrap_or(0.0));
        }
        Ok(())
    }

    /// `totalPlayed` before this session, once known.
    pub fn base_total(&self) -> Option<f64> {
        self.base_total
    }

    /// A write (a checkpoint) already stored part of this session.
    pub fn has_written(&self) -> bool {
        self.recent_done
    }

    /// `rec.ended_at` is "now" for a checkpoint and the real end for the last
    /// write. Sessions under `min_session_seconds` only touch the timestamps.
    pub fn write(&mut self, paths: &DataPaths, rec: &PlayRecord, min_session_seconds: i64) -> Result<RecordOutcome> {
        let total = update_timestamps_with_base(paths, rec, &mut self.base_total)?;
        let elapsed = (rec.ended_at - rec.started_at).num_milliseconds().div_euclid(1000);
        let mut session_recorded = false;
        if elapsed >= min_session_seconds {
            session_recorded = upsert_session(
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
        if !self.recent_done {
            add_recent_game(paths, &rec.app_name, &rec.title)?;
            self.recent_done = true;
        }
        Ok(RecordOutcome { elapsed_seconds: elapsed, session_recorded, total_played_minutes: total })
    }
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
        let paths = DataPaths::at(dir.path());
        mark_started(&paths, "g1", t(0)).unwrap();
        assert_eq!(read(&paths.timestamps()), "{\n\t\"g1\": {\n\t\t\"firstPlayed\": \"2026-10-07T12:00:00.000Z\"\n\t}\n}");
        // Second launch does not move firstPlayed.
        mark_started(&paths, "g1", t(99_000)).unwrap();

        let rec = PlayRecord { app_name: "g1".into(), title: "Game One".into(), started_at: t(0), ended_at: t(90_500) };
        let out = record_play(&paths, &rec, MIN_SESSION_SECONDS).unwrap();
        assert_eq!(out, RecordOutcome { elapsed_seconds: 90, session_recorded: true, total_played_minutes: 1 });

        assert_eq!(
            read(&paths.timestamps()),
            "{\n\t\"g1\": {\n\t\t\"firstPlayed\": \"2026-10-07T12:00:00.000Z\",\n\t\t\"lastPlayed\": \"2026-10-07T12:01:30.500Z\",\n\t\t\"totalPlayed\": 1\n\t}\n}"
        );
        let sessions: Value = serde_json::from_str(&read(&paths.sessions())).unwrap();
        assert_eq!(
            sessions["sessions"]["g1"][0],
            json!({"startedAt":"2026-10-07T12:00:00.000Z","endedAt":"2026-10-07T12:01:30.500Z","elapsedSeconds":90,"source":"heroic"})
        );
        let cfg: Value = serde_json::from_str(&read(&paths.store_config())).unwrap();
        assert_eq!(cfg["games"]["recent"], json!([{"appName":"g1","title":"Game One"}]));
    }

    #[test]
    fn total_played_accumulates_fractional_minutes_then_floors() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.timestamps().parent().unwrap()).unwrap();
        std::fs::write(
            paths.timestamps(),
            r#"{"g1":{"firstPlayed":"2025-01-01T00:00:00.000Z","lastPlayed":"2025-01-01T01:00:00.000Z","totalPlayed":200,"other":1},"g2":{"totalPlayed":5}}"#,
        )
        .unwrap();
        let rec = PlayRecord { app_name: "g1".into(), title: "x".into(), started_at: t(0), ended_at: t(150_000) }; // 2.5 min
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
        let paths = DataPaths::at(dir.path());
        let rec = PlayRecord { app_name: "g1".into(), title: "x".into(), started_at: t(0), ended_at: t(4_999) };
        let out = record_play(&paths, &rec, MIN_SESSION_SECONDS).unwrap();
        assert_eq!(out.elapsed_seconds, 4);
        assert!(!out.session_recorded);
        assert!(!paths.sessions().exists());
        assert!(paths.timestamps().exists() && paths.store_config().exists());
    }

    #[test]
    fn a_resumed_session_is_not_counted_twice() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let rec = |end: i64| PlayRecord { app_name: "g".into(), title: "G".into(), started_at: t(0), ended_at: t(end * 60_000) };
        record_play(&paths, &rec(100), 5).unwrap();
        // A new session: base read at its start, a checkpoint at 30 min.
        let mut cp = Checkpointer::default();
        cp.begin(&paths, "g").unwrap();
        assert_eq!(cp.base_total(), Some(100.0));
        let next = |end: i64| PlayRecord { started_at: t(200 * 60_000), ended_at: t((200 + end) * 60_000), ..rec(0) };
        cp.write(&paths, &next(30), 5).unwrap();
        // The app restarts; the session goes on to 45 min.
        let mut again = Checkpointer::resume(cp.base_total(), cp.has_written());
        assert_eq!(again.write(&paths, &next(45), 5).unwrap().total_played_minutes, 145);
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert_eq!(doc.sessions["g"].len(), 2, "the row of the session is updated, not added again");
    }

    #[test]
    fn sessions_dedupe_and_cap_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let mk =
            |i: i64| PlayRecord { app_name: "g1".into(), title: "x".into(), started_at: t(i * 60_000), ended_at: t(i * 60_000 + 30_000) };
        // A full history, then three more sessions.
        let max = MAX_SESSIONS_PER_GAME as i64;
        let full: Vec<LocalGameSession> = (0..max)
            .map(|i| LocalGameSession {
                started_at: iso(t(i * 60_000)),
                ended_at: iso(t(i * 60_000 + 30_000)),
                elapsed_seconds: 30.0.into(),
                source: "heroic".into(),
                ..Default::default()
            })
            .collect();
        let mut doc = SessionsFile::default();
        doc.sessions.insert("g1".into(), full);
        json_store::write(&paths.sessions(), &doc).unwrap();
        for i in max..max + 3 {
            record_play(&paths, &mk(i), 5).unwrap();
        }
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        let list = &doc.sessions["g1"];
        assert_eq!(list.len(), MAX_SESSIONS_PER_GAME);
        // Trimming sorts newest first and drops the oldest.
        assert_eq!(list[0].started_at, iso(t((max + 2) * 60_000)));
        assert_eq!(list[MAX_SESSIONS_PER_GAME - 1].started_at, iso(t(3 * 60_000)));
        // The same session twice is stored once.
        let before = list.len();
        assert!(!add_session(&paths, "g1", list[0].clone()).unwrap());
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert_eq!(doc.sessions["g1"].len(), before);
    }

    #[test]
    fn recent_moves_to_front_without_duplicates_and_keeps_other_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.store_config().parent().unwrap()).unwrap();
        std::fs::write(
            paths.store_config(),
            r#"{"language":"en","games":{"recent":[{"appName":"a","title":"A"},{"title":"orphan"},{"appName":"b","title":"B"}],"hidden":[{"appName":"h","title":"H"}]}}"#,
        )
        .unwrap();
        add_recent_game(&paths, "b", "B2").unwrap();
        let v: Value = serde_json::from_str(&read(&paths.store_config())).unwrap();
        assert_eq!(v["games"]["recent"], json!([{"appName":"b","title":"B2"},{"appName":"a","title":"A"}]));
        assert_eq!(v["games"]["hidden"][0]["appName"], "h");
        assert_eq!(v["language"], "en");
    }

    #[test]
    fn checkpoints_never_double_count() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let original = r#"{"g1":{"firstPlayed":"2025-01-01T00:00:00.000Z","totalPlayed":100}}"#;
        std::fs::create_dir_all(paths.timestamps().parent().unwrap()).unwrap();
        std::fs::write(paths.timestamps(), original).unwrap();
        let at = |end_ms: i64| PlayRecord { app_name: "g1".into(), title: "G".into(), started_at: t(0), ended_at: t(end_ms) };
        let mut cp = Checkpointer::default();
        // Checkpoints every 2 minutes, one repeated, then the end.
        for min in [2, 4, 4, 6] {
            let out = cp.write(&paths, &at(min * 60_000), 5).unwrap();
            assert_eq!(out.total_played_minutes, 100 + min);
        }
        let out = cp.write(&paths, &at(6 * 60_000 + 40_000), 5).unwrap();
        assert_eq!(out.total_played_minutes, 106);
        let v: Value = serde_json::from_str(&read(&paths.timestamps())).unwrap();
        assert_eq!(v["g1"]["totalPlayed"], 106);
        assert_eq!(v["g1"]["lastPlayed"], "2026-10-07T12:06:40.000Z");
        assert_eq!(v["g1"]["firstPlayed"], "2025-01-01T00:00:00.000Z");
        // One session row, updated in place.
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert_eq!(doc.sessions["g1"].len(), 1);
        assert_eq!(doc.sessions["g1"][0].elapsed_seconds.get(), 400.0);
        assert_eq!(doc.sessions["g1"][0].ended_at, iso(t(400_000)));
        // The recent list got the game once.
        let cfg: Value = serde_json::from_str(&read(&paths.store_config())).unwrap();
        assert_eq!(cfg["games"]["recent"], json!([{"appName":"g1","title":"G"}]));

        // The same totals as a single write at the end of the session.
        let dir2 = tempfile::tempdir().unwrap();
        let p2 = DataPaths::at(dir2.path());
        std::fs::create_dir_all(p2.timestamps().parent().unwrap()).unwrap();
        std::fs::write(p2.timestamps(), original).unwrap();
        assert_eq!(record_play(&p2, &at(400_000), 5).unwrap().total_played_minutes, 106);
    }

    #[test]
    fn a_crash_after_a_checkpoint_keeps_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let rec = PlayRecord { app_name: "g1".into(), title: "G".into(), started_at: t(0), ended_at: t(130_000) };
        // First run: one checkpoint, then the process dies (no final write).
        Checkpointer::default().write(&paths, &rec, 5).unwrap();
        // Next day: a new session starts from the checkpointed total.
        let next = PlayRecord { app_name: "g1".into(), title: "G".into(), started_at: t(86_400_000), ended_at: t(86_400_000 + 60_000) };
        let out = Checkpointer::default().write(&paths, &next, 5).unwrap();
        assert_eq!(out.total_played_minutes, 2 + 1);
        let doc: SessionsFile = json_store::read(&paths.sessions()).unwrap();
        assert_eq!(doc.sessions["g1"].len(), 2);
    }

    #[test]
    fn sessions_under_the_minimum_are_not_stored_by_checkpoints_either() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let rec = PlayRecord { app_name: "g1".into(), title: "G".into(), started_at: t(0), ended_at: t(3_000) };
        let out = Checkpointer::default().write(&paths, &rec, 5).unwrap();
        assert!(!out.session_recorded);
        assert!(!paths.sessions().exists());
    }
}
