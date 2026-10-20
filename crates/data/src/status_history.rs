//! When each game's completion status changed (`local_library/status-history.json`),
//! so the Statistics page can tell the day a game was beaten or abandoned
//! (before this file existed it can only guess from the last session).
//!
//! The file is `{ "changes": { "<appName>": [{ "status": "<status id>", "at":
//! "<RFC 3339>" }] } }`, keyed like the overlay entries of
//! [`crate::status`], oldest change first. Other fields are kept on write.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// Status id the game moved to.
    pub status: String,
    /// When, RFC 3339.
    pub at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    changes: IndexMap<String, Vec<Change>>,
    #[serde(flatten)]
    extra: Extra,
}

/// Every game's changes, by `appName`.
pub fn load(paths: &DataPaths) -> HashMap<String, Vec<Change>> {
    json_store::read::<HistoryFile>(&paths.status_history()).map(|f| f.changes.into_iter().collect()).unwrap_or_default()
}

/// Adds `(appName, status id)` changes made at `at` (RFC 3339).
pub fn record(paths: &DataPaths, changes: &[(String, String)], at: &str) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    json_store::update::<HistoryFile, _>(&paths.status_history(), |file| {
        for (app_name, status) in changes {
            file.changes.entry(app_name.clone()).or_default().push(Change { status: status.clone(), at: at.to_owned() });
        }
        Ok(())
    })
}

/// [`record`] stamped now; a failure is only logged (the status itself is saved).
pub(crate) fn record_now(paths: &DataPaths, changes: &[(String, String)]) {
    let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Err(err) = record(paths, changes, &at) {
        log::warn!("saving when statuses changed: {err:#}");
    }
}

/// Local day of the last change into one of `statuses` (ids), if the game
/// is still in it (`current` is its status id now).
pub fn day_of(changes: &[Change], current: &str, statuses: &[&str]) -> Option<NaiveDate> {
    if !statuses.contains(&current) {
        return None;
    }
    let last = changes.iter().rev().find(|c| statuses.contains(&c.status.as_str()))?;
    DateTime::parse_from_rfc3339(&last.at).ok().map(|d| d.with_timezone(&Local).date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_kept_in_order_and_the_last_one_counts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        record(&paths, &[("a".into(), "playing".into())], "2026-01-10T12:00:00.000Z").unwrap();
        record(&paths, &[("a".into(), "beaten".into()), ("b".into(), "abandoned".into())], "2026-03-05T12:00:00.000Z").unwrap();
        let all = load(&paths);
        assert_eq!(all["a"].len(), 2);
        assert_eq!(all["a"][1].status, "beaten");
        let day = day_of(&all["a"], "beaten", &["beaten", "completed"]).unwrap();
        assert_eq!(day.format("%m").to_string(), "03");
        // Moved on since: no "beaten" day.
        assert_eq!(day_of(&all["a"], "playing", &["beaten", "completed"]), None);
        assert!(day_of(&all["b"], "abandoned", &["abandoned"]).is_some());
    }

    #[test]
    fn setting_a_status_records_only_real_changes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        crate::status::set_game_status(&paths, "g", "beaten", &Default::default()).unwrap();
        crate::status::set_game_status(&paths, "g", "beaten", &Default::default()).unwrap();
        crate::status::set_games_status(
            &paths,
            &[("g".into(), "playing".into(), Default::default()), ("h".into(), "abandoned".into(), Default::default())],
        )
        .unwrap();
        let all = load(&paths);
        assert_eq!(all["g"].iter().map(|c| c.status.as_str()).collect::<Vec<_>>(), ["beaten", "playing"]);
        assert_eq!(all["h"].len(), 1);
    }
}
