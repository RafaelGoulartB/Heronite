//! Manual playtime edit (`playtime.ts` of the Collection screen +
//! `CollectionPlaytimeDialog.tsx`): the total time played is stored in
//! `store/timestamp.json` as `totalPlayed` minutes, next to `firstPlayed` /
//! `lastPlayed`.

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::json_store;
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
        let text = |key: &str| {
            current.and_then(|c| c.get(key)).and_then(Value::as_str).unwrap_or("").to_string()
        };
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
        assert_eq!(
            text,
            "{\n\t\"n\": {\n\t\t\"firstPlayed\": \"\",\n\t\t\"lastPlayed\": \"\",\n\t\t\"totalPlayed\": 90\n\t}\n}"
        );
        set_playtime_minutes(&paths, "n", -4.0).unwrap();
        assert_eq!(get_playtime_minutes(&paths, "n"), 0);
        assert_eq!(get_playtime_minutes(&paths, "missing"), 0);
    }
}
