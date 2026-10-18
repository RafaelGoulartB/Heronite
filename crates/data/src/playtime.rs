//! Manual playtime edit (`playtime.ts` of the Collection screen +
//! `CollectionPlaytimeDialog.tsx`): the total time played is stored in
//! `store/timestamp.json` as `totalPlayed` minutes, next to `firstPlayed` /
//! `lastPlayed`. Also the play time a store counted ([`import_store_playtime`]),
//! taken in only for games never played here.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::json_store;
use crate::library::Library;
use crate::models::SessionsFile;
use crate::paths::DataPaths;

/// `Math.floor(minutes / 60)`, `minutes % 60`: what the dialog's two fields
/// start with.
pub fn split_minutes(minutes: u64) -> (u64, u64) {
    (minutes / 60, minutes % 60)
}

/// `clampNumber`: `parseInt(value, 10)`, negatives and garbage become 0,
/// values above `max` become `max`.
pub fn clamp_number(value: &str, max: Option<u64>) -> u64 {
    let trimmed = value.trim_start();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let end = digits.bytes().take_while(u8::is_ascii_digit).count();
    let parsed = if end == 0 || negative { 0 } else { digits[..end].parse::<u64>().unwrap_or(u64::MAX) };
    match max {
        Some(max) if parsed > max => max,
        _ => parsed,
    }
}

/// `getPlaytimeMinutes`.
pub fn get_playtime_minutes(paths: &DataPaths, app_name: &str) -> u64 {
    json_store::read_value(&paths.timestamps())
        .ok()
        .and_then(|doc| doc.get(app_name)?.get("totalPlayed")?.as_f64())
        .map(|m| m.max(0.0) as u64)
        .unwrap_or(0)
}

/// `setPlaytimeMinutes`: replaces `totalPlayed` with `round(minutes)` (never
/// negative). `firstPlayed` / `lastPlayed` keep their value, or become `""`
/// when the game had no entry (the TS writes empty strings).
pub fn set_playtime_minutes(paths: &DataPaths, app_name: &str, minutes: f64) -> Result<()> {
    let total = if minutes.is_finite() { minutes.round().max(0.0) as i64 } else { 0 };
    json_store::update_value(&paths.timestamps(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let map = root.as_object_mut().expect("object");
        let current = map.get(app_name).and_then(Value::as_object);
        let text = |key: &str| current.and_then(|c| c.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
        let mut entry = Map::new();
        entry.insert("firstPlayed".into(), json!(text("firstPlayed")));
        entry.insert("lastPlayed".into(), json!(text("lastPlayed")));
        entry.insert("totalPlayed".into(), json!(total));
        // Fields this app does not know survive, after the three above.
        if let Some(current) = current {
            for (k, v) in current {
                if !entry.contains_key(k) {
                    entry.insert(k.clone(), v.clone());
                }
            }
        }
        map.insert(app_name.to_string(), Value::Object(entry));
        Ok(())
    })
}

/// The play time a store counted for one of its games.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePlaytime {
    pub app_name: String,
    pub minutes: u64,
}

/// Whether a `store/timestamp.json` entry holds no play: none at all, or no
/// minutes and no date. A game started here always has a date, since
/// `firstPlayed` is written as it starts (before its session counts).
fn unplayed(entry: Option<&Value>) -> bool {
    let Some(entry) = entry else { return true };
    let minutes = entry.get("totalPlayed").and_then(Value::as_f64).unwrap_or(0.0);
    let dated = |key: &str| entry.get(key).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    minutes <= 0.0 && !dated("firstPlayed") && !dated("lastPlayed")
}

/// The Collection's games from `runner` (DLC are not games) with no play
/// here yet: no minutes, no date, no session. What a store's play time may
/// fill in.
pub fn unplayed_games(paths: &DataPaths, runner: &str) -> Result<HashSet<String>> {
    let library = Library::load_with(paths, true)?;
    Ok(library
        .games
        .iter()
        .filter(|g| g.runner == runner && g.playtime_minutes == 0 && g.first_played.is_none() && g.last_played.is_none())
        .filter(|g| g.sessions.as_ref().is_none_or(|s| s.is_empty()))
        .map(|g| g.app_name.clone())
        .collect())
}

/// Takes in a store's play time for the games that have none here (see
/// [`unplayed_games`]): a game already played here keeps its own time, even
/// when the store counted more. Checked again under the file's lock, so a
/// game that starts meanwhile is left alone. Returns the games filled in.
pub fn import_store_playtime(paths: &DataPaths, items: &[StorePlaytime]) -> Result<Vec<String>> {
    let items: Vec<&StorePlaytime> = items.iter().filter(|i| i.minutes > 0 && !i.app_name.is_empty()).collect();
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let sessions: SessionsFile = json_store::read(&paths.sessions())?;
    let fillable = |app: &str, current: &Value| sessions.sessions.get(app).is_none_or(|s| s.is_empty()) && unplayed(current.get(app));
    // Nothing to fill: the file is not rewritten.
    let current = json_store::read_value(&paths.timestamps())?;
    if !items.iter().any(|i| fillable(&i.app_name, &current)) {
        return Ok(Vec::new());
    }
    json_store::update_value(&paths.timestamps(), |root| {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        let mut filled = Vec::new();
        for item in &items {
            if !fillable(&item.app_name, root) {
                continue;
            }
            let map = root.as_object_mut().expect("object");
            match map.get_mut(&item.app_name).and_then(Value::as_object_mut) {
                Some(entry) => {
                    entry.insert("totalPlayed".into(), json!(item.minutes));
                }
                None => {
                    map.insert(item.app_name.clone(), json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": item.minutes}));
                }
            }
            filled.push(item.app_name.clone());
        }
        Ok(filled)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup(content: Option<&str>) -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        if let Some(c) = content {
            fs::create_dir_all(paths.timestamps().parent().unwrap()).unwrap();
            fs::write(paths.timestamps(), c).unwrap();
        }
        (dir, paths)
    }

    fn write(path: std::path::PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn store(app: &str, minutes: u64) -> StorePlaytime {
        StorePlaytime { app_name: app.into(), minutes }
    }

    #[test]
    fn store_playtime_only_fills_games_never_played_here() {
        let (_d, paths) = setup(Some(
            r#"{
              "played": {"firstPlayed": "2025-01-01T00:00:00.000Z", "lastPlayed": "2025-01-02T00:00:00.000Z", "totalPlayed": 2},
              "started": {"firstPlayed": "2026-10-18T10:00:00.000Z"},
              "zero": {"firstPlayed": "", "lastPlayed": "", "totalPlayed": 0, "x": 1},
              "sessions_only": {"totalPlayed": 0}
            }"#,
        ));
        write(
            paths.sessions(),
            r#"{"sessions": {"sessions_only": [{"startedAt": "2026-01-01T00:00:00.000Z", "endedAt": "2026-01-01T01:00:00.000Z", "elapsedSeconds": 3600, "source": "heroic"}]}}"#,
        );
        let filled = import_store_playtime(
            &paths,
            &[
                store("played", 999),
                store("started", 50),
                store("zero", 30),
                store("sessions_only", 70),
                store("new", 1175),
                store("none", 0),
            ],
        )
        .unwrap();
        assert_eq!(filled, ["zero", "new"]);
        let v = json_store::read_value(&paths.timestamps()).unwrap();
        // Played here: the local time stays, even below the store's.
        assert_eq!(v["played"]["totalPlayed"], 2);
        // Started here a moment ago (no minutes yet): left alone.
        assert!(v["started"].get("totalPlayed").is_none());
        assert_eq!(v["sessions_only"]["totalPlayed"], 0);
        assert_eq!(v["zero"], json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": 30, "x": 1}));
        assert_eq!(v["new"], json!({"firstPlayed": "", "lastPlayed": "", "totalPlayed": 1175}));
        assert!(v.get("none").is_none());

        // A second import changes nothing: the time is a record now.
        let before = fs::read_to_string(paths.timestamps()).unwrap();
        assert!(import_store_playtime(&paths, &[store("new", 2000), store("zero", 40)]).unwrap().is_empty());
        assert_eq!(fs::read_to_string(paths.timestamps()).unwrap(), before);
    }

    #[test]
    fn store_playtime_with_nothing_to_fill_writes_nothing() {
        let (_d, paths) = setup(None);
        assert!(import_store_playtime(&paths, &[]).unwrap().is_empty());
        assert!(import_store_playtime(&paths, &[store("a", 0)]).unwrap().is_empty());
        assert!(!paths.timestamps().exists());
    }

    #[test]
    fn unplayed_games_are_the_runners_games_without_play() {
        let (_d, paths) = setup(Some(r#"{"played": {"totalPlayed": 5}, "dated": {"lastPlayed": "2026-01-01T00:00:00.000Z"}}"#));
        write(
            paths.legendary_library(),
            r#"{"library": [
              {"runner": "legendary", "app_name": "fresh", "title": "Fresh", "install": {}},
              {"runner": "legendary", "app_name": "played", "title": "Played", "install": {}},
              {"runner": "legendary", "app_name": "dated", "title": "Dated", "install": {}},
              {"runner": "legendary", "app_name": "with_session", "title": "With session", "install": {}},
              {"runner": "legendary", "app_name": "dlc", "title": "A DLC", "install": {"is_dlc": true}}
            ], "__timestamp": {"library": "x"}}"#,
        );
        write(
            paths.gog_library(),
            r#"{"games": [{"runner": "gog", "app_name": "g", "title": "G", "install": {}}], "__timestamp": {"games": "x"}}"#,
        );
        write(
            paths.sessions(),
            r#"{"sessions": {"with_session": [{"startedAt": "2026-01-01T00:00:00.000Z", "endedAt": "2026-01-01T00:10:00.000Z", "elapsedSeconds": 600, "source": "heroic"}]}}"#,
        );
        assert_eq!(unplayed_games(&paths, "legendary").unwrap(), HashSet::from(["fresh".to_string()]));
        assert_eq!(unplayed_games(&paths, "gog").unwrap(), HashSet::from(["g".to_string()]));
    }

    #[test]
    fn split_and_clamp_follow_the_dialog() {
        assert_eq!(split_minutes(125), (2, 5));
        assert_eq!(split_minutes(0), (0, 0));
        assert_eq!(clamp_number("12", None), 12);
        assert_eq!(clamp_number(" 7x", None), 7);
        assert_eq!(clamp_number("-3", None), 0);
        assert_eq!(clamp_number("", None), 0);
        assert_eq!(clamp_number("abc", None), 0);
        assert_eq!(clamp_number("99", Some(59)), 59);
        assert_eq!(clamp_number("59", Some(59)), 59);
    }

    #[test]
    fn keeps_first_and_last_played_and_unknown_fields() {
        let (_d, paths) = setup(Some(
            "{\n\t\"a\": {\n\t\t\"firstPlayed\": \"2025-01-01T00:00:00.000Z\",\n\t\t\"lastPlayed\": \"2025-02-01T00:00:00.000Z\",\n\t\t\"totalPlayed\": 10,\n\t\t\"x\": 1\n\t},\n\t\"b\": {\n\t\t\"totalPlayed\": 5\n\t}\n}",
        ));
        set_playtime_minutes(&paths, "a", 125.4).unwrap();
        let v = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(v["a"]["firstPlayed"], "2025-01-01T00:00:00.000Z");
        assert_eq!(v["a"]["lastPlayed"], "2025-02-01T00:00:00.000Z");
        assert_eq!(v["a"]["totalPlayed"], 125);
        assert_eq!(v["a"]["x"], 1);
        assert_eq!(v["b"]["totalPlayed"], 5);
        assert_eq!(get_playtime_minutes(&paths, "a"), 125);
    }

    #[test]
    fn new_entry_gets_empty_dates_and_negative_becomes_zero() {
        let (_d, paths) = setup(None);
        set_playtime_minutes(&paths, "n", 90.0).unwrap();
        let text = fs::read_to_string(paths.timestamps()).unwrap();
        assert_eq!(text, "{\n\t\"n\": {\n\t\t\"firstPlayed\": \"\",\n\t\t\"lastPlayed\": \"\",\n\t\t\"totalPlayed\": 90\n\t}\n}");
        set_playtime_minutes(&paths, "n", -4.0).unwrap();
        assert_eq!(get_playtime_minutes(&paths, "n"), 0);
        assert_eq!(get_playtime_minutes(&paths, "missing"), 0);
    }
}
