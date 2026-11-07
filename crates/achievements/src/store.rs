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
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

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
            Some(s) => {
                // This version's view of the game replaces a newer one's.
                index.unknown.remove(&key);
                index.games.insert(key, s);
            }
            None => {
                index.games.remove(&key);
            }
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    pub games: BTreeMap<String, Summary>,
    /// Lines whose source this version does not know (a newer Heronite
    /// wrote them): kept as they are.
    unknown: Map<String, Value>,
    /// Fields a newer Heronite added.
    fields: Map<String, Value>,
}

#[derive(Deserialize)]
struct StoredIndex {
    #[serde(default)]
    games: Map<String, Value>,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Index {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredIndex::deserialize(deserializer)?;
        let mut index = Index { fields: stored.fields, ..Default::default() };
        for (key, line) in stored.games {
            if line.get("source").is_some_and(|s| Source::deserialize(s).is_ok()) {
                index.games.insert(key, Summary::deserialize(line).map_err(de::Error::custom)?);
            } else {
                index.unknown.insert(key, line);
            }
        }
        Ok(index)
    }
}

impl Serialize for Index {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(untagged)]
        enum Line<'a> {
            Known(&'a Summary),
            Unknown(&'a Value),
        }

        struct Games<'a>(&'a Index);
        impl Serialize for Games<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let known = self.0.games.iter().map(|(k, v)| (k, Line::Known(v)));
                let unknown = self.0.unknown.iter().map(|(k, v)| (k, Line::Unknown(v)));
                serializer.collect_map(known.chain(unknown).collect::<BTreeMap<_, _>>())
            }
        }

        #[derive(Serialize)]
        struct Out<'a> {
            games: Games<'a>,
            #[serde(flatten)]
            fields: &'a Map<String, Value>,
        }

        Out { games: Games(self), fields: &self.fields }.serialize(serializer)
    }
}

pub fn read_index(paths: &DataPaths) -> Index {
    json_store::read(&index_file(paths)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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
            groups: Vec::new(),
            unknown: Default::default(),
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

    /// A game's copy as a newer Heronite writes it: a set and an "empty"
    /// mark from a source this version does not know, and a new field.
    const FROM_A_NEWER_VERSION: &str = r#"{
        "sets": [
            {"source": "gog", "externalId": "1", "fetchedAt": "2026-10-01T10:00:00Z",
             "achievements": [{"id": "a", "name": "A", "description": "", "icon": "", "iconLocked": "", "unlocked": true}]},
            {"source": "future", "externalId": "x", "fetchedAt": "2026-10-02T10:00:00Z",
             "achievements": [{"id": "b", "name": "B", "unlocked": true, "somethingNew": 1}]}
        ],
        "retroGameId": 7,
        "emptyAt": {"epic": "2026-10-03T10:00:00Z", "future2": "2026-10-04T10:00:00Z"},
        "pin": {"folder": "/games/x"}
    }"#;

    fn json_of(file: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
    }

    #[test]
    fn what_a_newer_version_wrote_survives_a_read_and_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = game_file(&paths, "gog", "1");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, FROM_A_NEWER_VERSION).unwrap();

        let game = read(&paths, "gog", "1");
        assert_eq!(game.sets.len(), 1);
        assert_eq!(game.set(Source::Gog).unwrap().unlocked(), 1);
        assert_eq!(game.retro_game_id, Some(7));
        assert_eq!(game.empty_at.keys().collect::<Vec<_>>(), [&Source::Epic]);
        assert!(!game.unknown.is_empty());

        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();
        let before: Value = serde_json::from_str(FROM_A_NEWER_VERSION).unwrap();
        assert_eq!(json_of(&file), before);
    }

    #[test]
    fn a_refresh_keeps_what_it_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = game_file(&paths, "gog", "1");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, FROM_A_NEWER_VERSION).unwrap();

        let mut game = read(&paths, "gog", "1");
        game.sets[0].achievements.push(Achievement { id: "c".into(), ..Default::default() });
        game.empty_at.remove(&Source::Epic);
        game.retro_game_id = None;
        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();

        let doc = json_of(&file);
        assert_eq!(doc["sets"].as_array().unwrap().len(), 2);
        assert_eq!(doc["sets"][0]["achievements"].as_array().unwrap().len(), 2);
        assert_eq!(doc["sets"][1]["achievements"][0]["somethingNew"], 1);
        assert_eq!(doc["emptyAt"], serde_json::json!({"future2": "2026-10-04T10:00:00Z"}));
        assert_eq!(doc["pin"]["folder"], "/games/x");
        assert!(doc.get("retroGameId").is_none());
        assert_eq!(read(&paths, "gog", "1"), game);
    }

    #[test]
    fn the_index_keeps_lines_of_sources_it_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.achievements_dir()).unwrap();
        let line = serde_json::json!({"source": "future", "total": 4, "unlocked": 1});
        let doc = serde_json::json!({"games": {"sideload_x": line, "gog_1": line}, "version": 2});
        std::fs::write(index_file(&paths), doc.to_string()).unwrap();
        assert!(read_index(&paths).games.is_empty());

        let set = AchievementSet {
            source: Source::Gog,
            external_id: "1".into(),
            title: None,
            fetched_at: Utc::now(),
            groups: Vec::new(),
            unknown: Default::default(),
            achievements: vec![Achievement { id: "a".into(), unlocked: true, ..Default::default() }],
        };
        let game = GameAchievements { sets: vec![set], ..Default::default() };
        write(&paths, "gog", "1", &game, &[Source::Gog]).unwrap();
        write(&paths, "gog", "2", &GameAchievements::default(), &[Source::Gog]).unwrap();

        let doc = json_of(&index_file(&paths));
        assert_eq!(doc["games"]["sideload_x"], line);
        assert_eq!(doc["games"]["gog_1"]["source"], "gog");
        assert_eq!(doc["version"], 2);
        assert_eq!(read_index(&paths).games["gog_1"].unlocked, 1);
    }

    #[test]
    fn the_game_files_choice_and_set_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let set = AchievementSet {
            source: Source::Local,
            external_id: "480".into(),
            title: None,
            fetched_at: Utc::now(),
            achievements: vec![Achievement { id: "a".into(), unlocked: true, ..Default::default() }],
            groups: Vec::new(),
            unknown: Default::default(),
        };
        let pin = crate::model::LocalPin { folder: Some("/games/x".into()), app_id: Some("480".into()), off: false };
        let game = GameAchievements { sets: vec![set], local: Some(pin), ..Default::default() };
        write(&paths, "sideload", "x", &game, &[Source::Local]).unwrap();
        assert_eq!(read(&paths, "sideload", "x"), game);
        let doc = json_of(&game_file(&paths, "sideload", "x"));
        assert_eq!(doc["sets"][0]["source"], "local");
        assert_eq!(doc["local"], serde_json::json!({"folder": "/games/x", "appId": "480"}));
        assert_eq!(read_index(&paths).games["sideload_x"].source, Source::Local);
    }

    #[test]
    fn a_copy_with_nothing_unknown_is_written_as_before() {
        let game: GameAchievements = serde_json::from_str(
            r#"{"sets": [{"source": "steam", "externalId": "1", "fetchedAt": "2026-10-01T10:00:00Z", "achievements": []}]}"#,
        )
        .unwrap();
        assert!(game.unknown.is_empty());
        let out = serde_json::to_value(&game).unwrap();
        assert_eq!(out.as_object().unwrap().keys().collect::<Vec<_>>(), ["sets"]);
        assert_eq!(serde_json::to_value(GameAchievements::default()).unwrap(), serde_json::json!({"sets": []}));
    }

    #[test]
    fn copies_with_and_without_groups_read_and_write_back_unchanged() {
        let legacy = serde_json::json!({"sets": [{
            "source": "steam", "externalId": "620", "fetchedAt": "2026-10-01T10:00:00Z",
            "achievements": [{"id": "ACH.WAKE_UP", "name": "You Monster", "description": "", "icon": "", "iconLocked": "", "unlocked": true}]
        }]});
        let game: GameAchievements = serde_json::from_value(legacy.clone()).unwrap();
        assert!(game.sets[0].groups.is_empty() && game.sets[0].achievements[0].group.is_none());
        assert!(game.sets[0].sections().is_empty());
        assert_eq!(serde_json::to_value(&game).unwrap(), legacy);

        let grouped = serde_json::json!({"sets": [{
            "source": "steam", "externalId": "620", "fetchedAt": "2026-10-01T10:00:00Z",
            "achievements": [
                {"id": "ACH.WAKE_UP", "name": "You Monster", "description": "", "icon": "", "iconLocked": "", "unlocked": true, "group": "2"},
                {"id": "ACH.TEAM_BUILDING", "name": "Team Building", "description": "", "icon": "", "iconLocked": "", "group": "1"}
            ],
            "groups": [
                {"id": "2", "name": "Single Player", "order": 0, "dlcAppId": 0},
                {"id": "1", "name": "Cooperative", "order": 1, "archived": true, "futureGroup": "kept"}
            ]
        }]});
        let game: GameAchievements = serde_json::from_value(grouped.clone()).unwrap();
        let set = &game.sets[0];
        assert_eq!(set.groups[1].dlc_app_id, None);
        assert_eq!(set.groups[1].unknown["futureGroup"], "kept");
        assert_eq!(set.sections().len(), 2);
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write(&paths, "steam", "620", &game, &[Source::Steam]).unwrap();
        assert_eq!(read(&paths, "steam", "620"), game);
        assert_eq!(json_of(&game_file(&paths, "steam", "620")), grouped);
    }

    #[test]
    fn known_sources_keep_unknown_nested_fields() {
        let doc = serde_json::json!({
            "sets": [{
                "source": "steam", "externalId": "1", "fetchedAt": "2026-10-01T10:00:00Z",
                "futureSet": {"version": 2},
                "achievements": [{
                    "id": "a", "name": "A", "description": "", "icon": "", "iconLocked": "",
                    "unlocked": true, "unlockedAt": "2026-09-01T10:00:00Z",
                    "futureAchievement": ["kept"]
                }]
            }],
            "futureGame": true
        });
        let game: GameAchievements = serde_json::from_value(doc.clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write(&paths, "steam", "1", &game, &[Source::Steam]).unwrap();
        assert_eq!(read(&paths, "steam", "1"), game);
        assert_eq!(json_of(&game_file(&paths, "steam", "1")), doc);
    }

    #[test]
    fn file_names_are_safe() {
        let paths = DataPaths::at("/d");
        let f = game_file(&paths, "sideload", "a/b c:é");
        assert_eq!(f, PathBuf::from("/d/local_library/achievements/sideload/a_b_c__.json"));
    }
}
