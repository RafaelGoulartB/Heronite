//! When each game entered the library (`local_library/added.json`), for the
//! Collection's "Date added" sort.
//!
//! The file is `{ "added": { "<game key>": "2026-10-17" } }`, keyed by
//! `CollectionGame.key`. Heroic keeps no such date, so a game gets one the
//! first time a load sees it: the day it was first played when there is a
//! play history (it was in the library by then), else the day of that load.
//! Other fields in the file are kept on write.

use std::collections::HashMap;

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::library::CollectionGame;
use crate::models::Extra;
use crate::paths::DataPaths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AddedFile {
    #[serde(default)]
    added: IndexMap<String, String>,
    #[serde(flatten)]
    extra: Extra,
}

/// The `YYYY-MM-DD` start of an ISO date or instant.
fn day(value: &str) -> Option<&str> {
    let d = value.trim().get(..10)?;
    let b = d.as_bytes();
    (b[4] == b'-' && b[7] == b'-' && d.bytes().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())).then_some(d)
}

/// The day a game with no date yet entered the library: its first play, else `today`.
fn first_day(game: &CollectionGame, today: &str) -> String {
    let sessions = game.sessions.iter().flat_map(|s| s.iter()).filter_map(|s| day(&s.started_at));
    let first = game.first_played.as_deref().and_then(day).into_iter().chain(sessions).min();
    first.unwrap_or(today).to_owned()
}

/// The stored dates, by game key (`YYYY-MM-DD`), without adding any.
pub fn load(paths: &DataPaths) -> HashMap<String, String> {
    json_store::read::<AddedFile>(&paths.added()).map(|f| f.added.into_iter().collect()).unwrap_or_default()
}

/// The dates of every game, giving one to the games that had none (written
/// back when any was new). A file that cannot be written still gives the dates.
pub fn sync(paths: &DataPaths, games: &[std::sync::Arc<CollectionGame>]) -> HashMap<String, String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    sync_on(paths, games, &today)
}

fn sync_on(paths: &DataPaths, games: &[std::sync::Arc<CollectionGame>], today: &str) -> HashMap<String, String> {
    let file: AddedFile = json_store::read(&paths.added()).unwrap_or_default();
    let mut dates: HashMap<String, String> = file.added.into_iter().collect();
    let new: Vec<(String, String)> =
        games.iter().filter(|g| !dates.contains_key(&g.key)).map(|g| (g.key.clone(), first_day(g, today))).collect();
    if !new.is_empty() {
        if let Err(err) = record(paths, &new) {
            log::warn!("saving the dates games were added: {err:#}");
        }
        dates.extend(new);
    }
    dates
}

fn record(paths: &DataPaths, new: &[(String, String)]) -> Result<()> {
    json_store::update::<AddedFile, _>(&paths.added(), |file| {
        for (key, date) in new {
            file.added.entry(key.clone()).or_insert_with(|| date.clone());
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game;

    #[test]
    fn new_games_get_their_first_play_or_today_and_keep_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let played = game("p", "Played", |s| {
            s.first_played = Some("2025-12-24T22:00:00.000Z".into());
            s.last_played = Some("2026-03-01T10:00:00.000Z".into());
        });
        let games = vec![played, game("n", "Never", |_| {})];
        assert!(load(&paths).is_empty());
        let dates = sync_on(&paths, &games, "2026-10-17");
        assert_eq!(load(&paths), dates);
        assert_eq!(dates["sideload_p"], "2025-12-24");
        assert_eq!(dates["sideload_n"], "2026-10-17");
        // a later load keeps the stored day
        let dates = sync_on(&paths, &games, "2027-01-01");
        assert_eq!(dates["sideload_n"], "2026-10-17");
        assert_eq!(day("2026-1-5"), None);
    }
}
