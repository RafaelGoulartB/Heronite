//! The local copy: one file per game in `local_library/achievements/<runner>/`
//! plus `index.json`, the counts of every game (for lists that must not
//! open a file per game). It lives in the data folder, so backups carry it
//! and a new system starts with the whole history.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use serde::de::DeserializeOwned;
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

/// Clears the way for a write over `file`. A missing or valid file needs
/// nothing. One that is not valid JSON of type `T` is renamed to
/// `<name>.corrupt-<timestamp>` first, so the write never destroys data a
/// person could still recover. One that cannot be read at all (permissions, a
/// failing disk) is an error: the write must not replace what it did not see.
pub(crate) fn keep_unreadable<T: DeserializeOwned>(file: &Path) -> Result<()> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) || serde_json::from_slice::<Option<T>>(&bytes).is_ok() {
        return Ok(());
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let aside = file.with_file_name(format!("{name}.corrupt-{}", Utc::now().format("%Y%m%dT%H%M%S%.3fZ")));
    std::fs::rename(file, &aside).with_context(|| format!("setting aside {}", file.display()))?;
    log::warn!("{} is not valid; kept as {}", file.display(), aside.display());
    Ok(())
}

/// The copy of a game for display (empty when there is none or it cannot be
/// read). [`write`] never replaces a file this could not read.
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
    keep_unreadable::<GameAchievements>(&file)?;
    json_store::write(&file, game)?;
    let key = format!("{runner}_{app_name}");
    let summary = Summary::of(game, order);
    let index_file = index_file(paths);
    keep_unreadable::<Index>(&index_file)?;
    json_store::update_if_changed(&index_file, |index: &mut Index| {
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
        let game = GameAchievements { sets: vec![set], ..Default::default() };
        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();
        assert_eq!(read(&paths, "gog", "1"), game);
        let s = &read_index(&paths).games["gog_1"];
        assert_eq!((s.source, s.total, s.unlocked), (Source::Gog, 2, 1));

        write(&paths, "gog", "1", &GameAchievements::default(), &[Source::Gog]).unwrap();
        assert!(read_index(&paths).games.is_empty());
    }

    fn game_with_unlock() -> GameAchievements {
        let set = AchievementSet {
            source: Source::Gog,
            external_id: "1".into(),
            title: None,
            fetched_at: Utc::now(),
            achievements: vec![Achievement { id: "a".into(), unlocked: true, ..Default::default() }],
        };
        GameAchievements { sets: vec![set], ..Default::default() }
    }

    fn aside(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<_> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains(".corrupt-")).collect();
        found.sort();
        found
    }

    #[test]
    fn a_corrupt_game_file_is_kept_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = game_file(&paths, "gog", "1");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"sets\": [{\"source\": \"gog\"").unwrap();
        // Reading for display gives an empty copy, and changes nothing.
        assert_eq!(read(&paths, "gog", "1"), GameAchievements::default());
        assert!(aside(file.parent().unwrap()).is_empty());

        let game = game_with_unlock();
        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();
        assert_eq!(read(&paths, "gog", "1"), game);
        let kept = aside(file.parent().unwrap());
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "{\"sets\": [{\"source\": \"gog\"");
    }

    #[test]
    fn a_game_file_that_cannot_be_read_fails_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = game_file(&paths, "gog", "1");
        // A directory in place of the file: it exists, but reading it fails.
        std::fs::create_dir_all(&file).unwrap();
        assert!(write(&paths, "gog", "1", &game_with_unlock(), &[Source::Gog]).is_err());
        assert!(file.is_dir());
    }

    #[test]
    fn a_corrupt_index_is_set_aside_and_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(index_file(&paths).parent().unwrap()).unwrap();
        std::fs::write(index_file(&paths), "][").unwrap();
        write(&paths, "gog", "1", &game_with_unlock(), &[Source::Gog]).unwrap();
        assert_eq!(read_index(&paths).games["gog_1"].unlocked, 1);
        assert_eq!(aside(index_file(&paths).parent().unwrap()).len(), 1);
    }

    #[test]
    fn file_names_are_safe() {
        let paths = DataPaths::at("/d");
        let f = game_file(&paths, "sideload", "a/b c:é");
        assert_eq!(f, PathBuf::from("/d/local_library/achievements/sideload/a_b_c__.json"));
    }
}
