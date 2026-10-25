//! The local copy: one file per game in `local_library/achievements/<runner>/`
//! plus `index.json`, the counts of every game (for lists that must not
//! open a file per game). It lives in the data folder, so backups carry it
//! and a new system starts with the whole history.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use serde::{Deserialize, Serialize};

use crate::model::{GameAchievements, Source};

/// A game's file: the app name with anything that is not safe in a file
/// name replaced.
pub fn game_file(paths: &DataPaths, runner: &str, app_name: &str) -> PathBuf {
    let safe = |s: &str| -> String {
        s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect()
    };
    paths.achievements_dir().join(safe(runner)).join(format!("{}.json", safe(app_name)))
}

fn index_file(paths: &DataPaths) -> PathBuf {
    paths.achievements_dir().join("index.json")
}

/// The copy of a game (empty when there is none or it cannot be read).
pub fn read(paths: &DataPaths, runner: &str, app_name: &str) -> GameAchievements {
    let file = game_file(paths, runner, app_name);
    if !file.exists() {
        return GameAchievements::default();
    }
    json_store::read(&file).unwrap_or_else(|e| {
        log::warn!("achievements of {runner}_{app_name}: {e:#}");
        GameAchievements::default()
    })
}

/// Writes a game's copy and its line of the index.
pub fn write(paths: &DataPaths, runner: &str, app_name: &str, game: &GameAchievements, order: &[Source]) -> Result<()> {
    let file = game_file(paths, runner, app_name);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    json_store::write(&file, game)?;
    let key = format!("{runner}_{app_name}");
    let summary = Summary::of(game, order);
    json_store::update_if_changed(&index_file(paths), |index: &mut Index| {
        match summary {
            Some(s) => index.games.insert(key, s),
            None => index.games.remove(&key),
        };
        Ok(())
    })
}

/// The counts of a game, as the index keeps them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub source: Source,
    pub total: usize,
    pub unlocked: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_unlocked_at: Option<DateTime<Utc>>,
}

impl Summary {
    pub fn of(game: &GameAchievements, order: &[Source]) -> Option<Summary> {
        let set = game.primary(order)?;
        Some(Summary {
            source: set.source,
            total: set.total(),
            unlocked: set.unlocked(),
            points: set.points(),
            last_unlocked_at: set.last_unlocked_at(),
        })
    }
}

/// `index.json`: `runner_appName` → counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub games: BTreeMap<String, Summary>,
}

pub fn read_index(paths: &DataPaths) -> Index {
    json_store::read(&index_file(paths)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Achievement, AchievementSet};

    #[test]
    fn write_then_read_keeps_the_game_and_its_counts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(read(&paths, "gog", "1"), GameAchievements::default());
        let set = AchievementSet {
            source: Source::Gog,
            external_id: "1".into(),
            title: None,
            fetched_at: Utc::now(),
            achievements: vec![
                Achievement { id: "a".into(), unlocked: true, ..Default::default() },
                Achievement { id: "b".into(), ..Default::default() },
            ],
        };
        let game = GameAchievements { sets: vec![set], retro_game_id: None };
        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();
        assert_eq!(read(&paths, "gog", "1"), game);
        let s = &read_index(&paths).games["gog_1"];
        assert_eq!((s.source, s.total, s.unlocked), (Source::Gog, 2, 1));

        write(&paths, "gog", "1", &GameAchievements::default(), &[Source::Gog]).unwrap();
        assert!(read_index(&paths).games.is_empty());
    }

    #[test]
    fn file_names_are_safe() {
        let paths = DataPaths::at("/d");
        let f = game_file(&paths, "sideload", "a/b c:é");
        assert_eq!(f, PathBuf::from("/d/local_library/achievements/sideload/a_b_c__.json"));
    }
}
