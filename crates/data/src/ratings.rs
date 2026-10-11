//! User star ratings (`local_library/ratings.json`), one per game.
//!
//! The file is `{ "ratings": { "<game key>": 4 } }`, keyed by
//! `CollectionGame.key`. Other fields in the file are kept on write.

use std::collections::HashMap;

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

/// `ratings.json`. Values stay raw JSON so an odd entry never blocks the
/// others; they are clamped on read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RatingsFile {
    #[serde(default)]
    ratings: IndexMap<String, Value>,
    #[serde(flatten)]
    extra: Extra,
}

/// `0` for anything that is not a positive number.
fn clamp(value: &Value) -> u8 {
    value.as_f64().map_or(0, |v| v.round().clamp(0., 5.) as u8)
}

/// All ratings by game key, each `1..=5`. A missing or broken file gives an
/// empty map; zeros are dropped.
pub fn load(paths: &DataPaths) -> HashMap<String, u8> {
    let file: RatingsFile = json_store::read(&paths.ratings()).unwrap_or_default();
    file.ratings.into_iter().map(|(key, stars)| (key, clamp(&stars))).filter(|(_, stars)| *stars > 0).collect()
}

/// Sets the rating of a game. `None` or `Some(0)` removes it; other values
/// are clamped to `1..=5`.
pub fn set_rating(paths: &DataPaths, key: &str, stars: Option<u8>) -> Result<()> {
    json_store::update::<RatingsFile, _>(&paths.ratings(), |file| {
        match stars.filter(|s| *s > 0) {
            Some(s) => {
                file.ratings.insert(key.to_owned(), Value::from(s.min(5)));
            }
            None => {
                file.ratings.shift_remove(key);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    #[test]
    fn missing_or_broken_file_is_empty() {
        let (_dir, paths) = paths();
        assert!(load(&paths).is_empty());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.ratings(), "{ not json").unwrap();
        assert!(load(&paths).is_empty());
    }

    #[test]
    fn set_and_remove() {
        let (_dir, paths) = paths();
        set_rating(&paths, "sideload_steam_1", Some(4)).unwrap();
        set_rating(&paths, "sideload_steam_2", Some(9)).unwrap();
        set_rating(&paths, "sideload_steam_3", Some(2)).unwrap();
        let map = load(&paths);
        assert_eq!(map["sideload_steam_1"], 4);
        assert_eq!(map["sideload_steam_2"], 5);
        set_rating(&paths, "sideload_steam_1", None).unwrap();
        set_rating(&paths, "sideload_steam_2", Some(0)).unwrap();
        let map = load(&paths);
        assert_eq!(map.len(), 1);
        assert_eq!(map["sideload_steam_3"], 2);
    }

    #[test]
    fn load_clamps_and_drops_zeros() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.ratings(), r#"{"ratings":{"a":0,"b":3,"c":99}}"#).unwrap();
        let map = load(&paths);
        assert_eq!(map.len(), 2);
        assert_eq!((map["b"], map["c"]), (3, 5));
    }

    #[test]
    fn unknown_fields_survive_a_write() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.ratings(), r#"{"version":2,"ratings":{"a":1}}"#).unwrap();
        set_rating(&paths, "b", Some(5)).unwrap();
        let v = json_store::read_value(&paths.ratings()).unwrap();
        assert_eq!(v["version"], 2);
        assert_eq!(v["ratings"]["a"], 1);
        assert_eq!(v["ratings"]["b"], 5);
    }
}
