//! The journal of each game (`local_library/journal.json`): notes about the
//! moment they happened, placed on the timeline's days by the rule the
//! screenshots follow, optionally about one screenshot, and the game's
//! "where I left off" line.
//!
//! ```json
//! { "entries": { "<game key>": [ { "id": "…", "at": "2026-07-05T23:41:00.000Z",
//!       "text": "…", "shot": "/captures/Lies of P/… .webp", "source": "overlay",
//!       "createdAt": "…", "updatedAt": "…" } ] },
//!   "leftOff": { "<game key>": { "text": "…", "at": "…" } } }
//! ```
//!
//! Keys are `CollectionGame.key`. An entry with no text is a moment marked
//! in-game, waiting for its words. Entries stay raw JSON on disk: a broken
//! one never hides the others, and fields this app does not know survive a
//! write.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json_store;
use crate::models::{Extra, LocalGameSession};
use crate::paths::DataPaths;
use crate::timeline::{days_of_moments, TimelineDay};

/// `source` of an entry written in the Collection.
pub const SOURCE_APP: &str = "app";
/// `source` of an entry written (or marked) in the in-game overlay.
pub const SOURCE_OVERLAY: &str = "overlay";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// The moment the note is about (RFC 3339).
    pub at: String,
    #[serde(default)]
    pub text: String,
    /// Path of the screenshot it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shot: Option<String>,
    /// Path of the replay (a video of the seconds before `at`) it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Entry {
    /// A new entry about `at` (now when `None`).
    pub fn new(at: Option<DateTime<Utc>>, text: &str, shot: Option<String>, source: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let now = Utc::now();
        let stamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        let n = NEXT.fetch_add(1, Ordering::Relaxed) & 0xffff;
        Self {
            id: format!("{:x}-{:04x}-{:x}", now.timestamp_millis(), n, std::process::id() & 0xfff),
            at: at.unwrap_or(now).to_rfc3339_opts(SecondsFormat::Millis, true),
            text: text.trim().to_owned(),
            shot,
            clip: None,
            source: source.to_owned(),
            created_at: stamp.clone(),
            updated_at: stamp,
            extra: Extra::new(),
        }
    }

    /// The entry about a saved replay.
    pub fn with_clip(mut self, clip: String) -> Self {
        self.clip = Some(clip);
        self
    }

    /// `at` in unix milliseconds.
    pub fn at_ms(&self) -> Option<i64> {
        DateTime::parse_from_rfc3339(&self.at).ok().map(|d| d.timestamp_millis())
    }

    /// A moment marked in-game with no words yet.
    pub fn is_moment(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftOff {
    pub text: String,
    /// When it was written (RFC 3339).
    #[serde(default)]
    pub at: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One game's journal.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameJournal {
    /// Oldest moment first.
    pub entries: Vec<Entry>,
    pub left_off: Option<LeftOff>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalFile {
    #[serde(default)]
    entries: IndexMap<String, Vec<Value>>,
    #[serde(default)]
    left_off: IndexMap<String, Value>,
    #[serde(flatten)]
    extra: Extra,
}

/// The journal's key of a game: its `CollectionGame.key` (`runner_appName`).
pub fn collection_key(runner: &str, app_name: &str) -> String {
    format!("{runner}_{app_name}")
}

fn parse_entry(v: &Value) -> Option<Entry> {
    serde_json::from_value::<Entry>(v.clone()).ok().filter(|e| !e.id.is_empty() && e.at_ms().is_some())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The journal of `key`; a missing or broken file gives an empty one.
pub fn load(paths: &DataPaths, key: &str) -> GameJournal {
    let file: JournalFile = json_store::read(&paths.journal()).unwrap_or_default();
    let mut entries: Vec<Entry> = file.entries.get(key).map(|list| list.iter().filter_map(parse_entry).collect()).unwrap_or_default();
    entries.sort_by_key(|e| e.at_ms().unwrap_or(0));
    let left_off =
        file.left_off.get(key).and_then(|v| serde_json::from_value::<LeftOff>(v.clone()).ok()).filter(|l| !l.text.trim().is_empty());
    GameJournal { entries, left_off }
}

/// Adds `entry` to the journal of `key`.
pub fn add(paths: &DataPaths, key: &str, entry: &Entry) -> Result<()> {
    let value = serde_json::to_value(entry)?;
    json_store::update::<JournalFile, _>(&paths.journal(), |file| {
        file.entries.entry(key.to_owned()).or_default().push(value);
        Ok(())
    })
}

/// Changes the text of entry `id` (an empty text makes it a marked moment
/// again). `false` when there is no such entry.
pub fn set_text(paths: &DataPaths, key: &str, id: &str, text: &str) -> Result<bool> {
    json_store::update::<JournalFile, _>(&paths.journal(), |file| {
        let Some(list) = file.entries.get_mut(key) else { return Ok(false) };
        let Some(obj) = list.iter_mut().filter_map(Value::as_object_mut).find(|o| o.get("id").and_then(Value::as_str) == Some(id)) else {
            return Ok(false);
        };
        obj.insert("text".into(), Value::from(text.trim()));
        obj.insert("updatedAt".into(), Value::from(now()));
        Ok(true)
    })
}

/// Removes entry `id`; gives it back (for undo).
pub fn remove(paths: &DataPaths, key: &str, id: &str) -> Result<Option<Entry>> {
    json_store::update::<JournalFile, _>(&paths.journal(), |file| {
        let Some(list) = file.entries.get_mut(key) else { return Ok(None) };
        let Some(i) = list.iter().position(|v| v.get("id").and_then(Value::as_str) == Some(id)) else { return Ok(None) };
        let removed = parse_entry(&list.remove(i));
        if list.is_empty() {
            file.entries.shift_remove(key);
        }
        Ok(removed)
    })
}

/// Sets the "where I left off" line of `key` (empty removes it).
pub fn set_left_off(paths: &DataPaths, key: &str, text: &str) -> Result<()> {
    let text = text.trim();
    json_store::update::<JournalFile, _>(&paths.journal(), |file| {
        if text.is_empty() {
            file.left_off.shift_remove(key);
        } else {
            let left = LeftOff { text: text.to_owned(), at: now(), extra: Extra::new() };
            file.left_off.insert(key.to_owned(), serde_json::to_value(left)?);
        }
        Ok(())
    })
}

/// Puts back a "where I left off" line as it was (its `at` included).
pub fn restore_left_off(paths: &DataPaths, key: &str, left: &LeftOff) -> Result<()> {
    json_store::update::<JournalFile, _>(&paths.journal(), |file| {
        file.left_off.insert(key.to_owned(), serde_json::to_value(left)?);
        Ok(())
    })
}

/// Whether the stored `shot` is the screenshot at `path`: the same file, or
/// the same name in the same folder with another extension (the background
/// compression turns `.png` captures into `.webp`).
pub fn is_shot(stored: &str, path: &Path) -> bool {
    let stored = Path::new(stored);
    stored == path || (stored.parent() == path.parent() && stored.file_stem().is_some() && stored.file_stem() == path.file_stem())
}

/// A day of the journal: a timeline day, a day with entries, or both.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalDay {
    pub date: NaiveDate,
    /// Index into the timeline days.
    pub timeline: Option<usize>,
    /// Indices into the entries, oldest moment first.
    pub entries: Vec<usize>,
}

/// The days of the timeline and the days the entries fall on (by the
/// screenshots' rule), newest first.
pub fn journal_days<Tz: TimeZone>(timeline: &[TimelineDay], entries: &[Entry], sessions: &[LocalGameSession], tz: &Tz) -> Vec<JournalDay> {
    let mut days: std::collections::BTreeMap<NaiveDate, JournalDay> = std::collections::BTreeMap::new();
    for (i, day) in timeline.iter().enumerate() {
        days.insert(day.date, JournalDay { date: day.date, timeline: Some(i), entries: Vec::new() });
    }
    let moments: Vec<(usize, i64)> = entries.iter().enumerate().filter_map(|(i, e)| e.at_ms().map(|ms| (i, ms))).collect();
    let dates = days_of_moments(sessions, &moments.iter().map(|(_, ms)| *ms).collect::<Vec<_>>(), tz);
    let mut placed: Vec<(i64, usize, NaiveDate)> =
        moments.iter().zip(dates).filter_map(|((i, ms), date)| date.map(|d| (*ms, *i, d))).collect();
    placed.sort_unstable();
    for (_, i, date) in placed {
        days.entry(date).or_insert_with(|| JournalDay { date, timeline: None, entries: Vec::new() }).entries.push(i);
    }
    days.into_values().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::JsNum;
    use chrono::FixedOffset;

    fn paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    fn at(s: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    #[test]
    fn missing_or_broken_file_is_empty() {
        let (_dir, paths) = paths();
        assert_eq!(load(&paths, "a"), GameJournal::default());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.journal(), "{ not json").unwrap();
        assert_eq!(load(&paths, "a"), GameJournal::default());
        assert!(add(&paths, "a", &Entry::new(None, "x", None, SOURCE_APP)).is_err(), "a broken file is not overwritten");
    }

    #[test]
    fn add_edit_remove_sorted_by_moment() {
        let (_dir, paths) = paths();
        let late = Entry::new(at("2026-07-06T20:00:00Z"), "  late  ", None, SOURCE_APP);
        let early = Entry::new(at("2026-07-05T20:00:00Z"), "", Some("/c/a.png".into()), SOURCE_OVERLAY);
        add(&paths, "g", &late).unwrap();
        add(&paths, "g", &early).unwrap();
        add(&paths, "other", &Entry::new(None, "x", None, SOURCE_APP)).unwrap();
        let j = load(&paths, "g");
        assert_eq!(j.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [early.id.as_str(), late.id.as_str()]);
        assert_eq!(j.entries[1].text, "late");
        assert!(j.entries[0].is_moment());

        assert!(set_text(&paths, "g", &early.id, " found it ").unwrap());
        assert!(!set_text(&paths, "g", "nope", "x").unwrap());
        assert_eq!(load(&paths, "g").entries[0].text, "found it");

        let gone = remove(&paths, "g", &late.id).unwrap().unwrap();
        assert_eq!(gone.text, "late");
        assert_eq!(load(&paths, "g").entries.len(), 1);
        add(&paths, "g", &gone).unwrap();
        assert_eq!(load(&paths, "g").entries.len(), 2, "a removed entry comes back (undo)");
        assert_eq!(load(&paths, "other").entries.len(), 1);
    }

    #[test]
    fn left_off_set_and_cleared() {
        let (_dir, paths) = paths();
        set_left_off(&paths, "g", " Chapter 8, by the lift ").unwrap();
        assert_eq!(load(&paths, "g").left_off.unwrap().text, "Chapter 8, by the lift");
        let written = load(&paths, "g").left_off.unwrap();
        set_left_off(&paths, "g", "  ").unwrap();
        assert!(load(&paths, "g").left_off.is_none());
        restore_left_off(&paths, "g", &written).unwrap();
        assert_eq!(load(&paths, "g").left_off, Some(written), "back as it was, date included");
    }

    #[test]
    fn broken_entries_are_skipped_and_unknown_fields_survive() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(
            paths.journal(),
            r#"{"version":2,"entries":{"g":[{"id":"1","at":"2026-07-05T20:00:00Z","text":"ok","mood":"good"},{"id":"2","at":"later"},7]}}"#,
        )
        .unwrap();
        let j = load(&paths, "g");
        assert_eq!(j.entries.len(), 1);
        assert_eq!(j.entries[0].extra["mood"], "good");
        set_text(&paths, "g", "1", "still ok").unwrap();
        let v = json_store::read_value(&paths.journal()).unwrap();
        assert_eq!(v["version"], 2);
        assert_eq!(v["entries"]["g"][0]["mood"], "good");
        assert_eq!(v["entries"]["g"][2], 7);
    }

    #[test]
    fn a_replay_is_kept_with_its_moment() {
        let (_d, p) = paths();
        let e = Entry::new(at("2026-07-05T23:41:00Z"), "", None, SOURCE_OVERLAY).with_clip("/c/G/replays/a.mp4".into());
        add(&p, "k", &e).unwrap();
        let j = load(&p, "k");
        assert_eq!(j.entries[0].clip.as_deref(), Some("/c/G/replays/a.mp4"));
        assert!(j.entries[0].is_moment());
        let raw = std::fs::read_to_string(p.journal()).unwrap();
        assert!(raw.contains("\"clip\"") && !raw.contains("\"shot\""));
    }

    #[test]
    fn a_compressed_capture_is_the_same_shot() {
        assert!(is_shot("/c/g/a.png", Path::new("/c/g/a.webp")));
        assert!(is_shot("/c/g/a.png", Path::new("/c/g/a.png")));
        assert!(!is_shot("/c/g/a.png", Path::new("/c/h/a.webp")));
        assert!(!is_shot("/c/g/a.png", Path::new("/c/g/b.png")));
    }

    #[test]
    fn days_join_the_timeline_and_the_entries() {
        let tz = FixedOffset::west_opt(3 * 3600).unwrap();
        let session = LocalGameSession {
            started_at: "2026-10-08T02:30:00Z".into(), // 23:30 local on the 7th
            ended_at: "2026-10-08T04:30:00Z".into(),
            elapsed_seconds: JsNum(7200.),
            source: "heroic".into(),
            ..Default::default()
        };
        let timeline = crate::timeline::build_timeline(std::slice::from_ref(&session), &[], &tz);
        let entries = vec![
            Entry::new(at("2026-10-08T04:00:00Z"), "after midnight, same session", None, SOURCE_APP),
            Entry::new(at("2026-10-10T15:00:00Z"), "a day without play", None, SOURCE_APP),
        ];
        let days = journal_days(&timeline, &entries, &[session], &tz);
        assert_eq!(days.len(), 2);
        assert_eq!(
            (days[0].date, days[0].timeline, days[0].entries.clone()),
            (NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(), None, vec![1])
        );
        assert_eq!(
            (days[1].date, days[1].timeline, days[1].entries.clone()),
            (NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(), Some(0), vec![0])
        );
    }
}
