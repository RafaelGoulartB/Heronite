//! Custom categories: user-made groups of games, kept under
//! `games.customCategories` in `store/config.json` as category name -> list
//! of `<appName>_<runner>` (the same JSON a migrated setup already has).
//! Edits rewrite only that key; the rest of the document is kept.
//!
//! Filtering by several categories shows the games of any of them (a
//! union); [`UNCATEGORIZED`] stands for the games in none.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use serde_json::Value;

use crate::json_store;
use crate::paths::DataPaths;

/// The filter's "Uncategorized" choice (kept with the selected names; a
/// category cannot take this name).
pub const UNCATEGORIZED: &str = ":uncategorized";

/// How a game is written in a category.
pub fn game_ref(runner: &str, app_name: &str) -> String {
    format!("{app_name}_{runner}")
}

/// The categories as loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Categories {
    map: BTreeMap<String, BTreeSet<String>>,
}

impl Categories {
    pub fn load(paths: &DataPaths) -> Self {
        let doc = json_store::read_value(&paths.store_config()).unwrap_or(Value::Null);
        Self::from_value(doc.pointer("/games/customCategories").unwrap_or(&Value::Null))
    }

    fn from_value(v: &Value) -> Self {
        let map = v
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(name, _)| !name.trim().is_empty())
                    .map(|(name, games)| {
                        let games = games.as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_owned)).collect());
                        (name.clone(), games.unwrap_or_default())
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { map }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    pub fn contains(&self, name: &str, runner: &str, app_name: &str) -> bool {
        self.map.get(name).is_some_and(|g| g.contains(&game_ref(runner, app_name)))
    }

    /// The categories of a game, sorted.
    pub fn of_game(&self, runner: &str, app_name: &str) -> Vec<&str> {
        let key = game_ref(runner, app_name);
        self.map.iter().filter(|(_, g)| g.contains(&key)).map(|(n, _)| n.as_str()).collect()
    }

    /// Games written in a category (some may no longer be in the library).
    pub fn len_of(&self, name: &str) -> usize {
        self.map.get(name).map_or(0, BTreeSet::len)
    }

    /// No selection shows every game; otherwise the game is in one of the
    /// `selected` categories, or in none with [`UNCATEGORIZED`] selected. Selected names that no longer exist match nothing.
    pub fn matches(&self, selected: &BTreeSet<String>, runner: &str, app_name: &str) -> bool {
        if selected.is_empty() {
            return true;
        }
        let key = game_ref(runner, app_name);
        selected.iter().any(|name| {
            if name == UNCATEGORIZED {
                !self.map.values().any(|g| g.contains(&key))
            } else {
                self.map.get(name).is_some_and(|g| g.contains(&key))
            }
        })
    }
}

/// Runs `f` on `games.customCategories` (created when missing).
fn update(paths: &DataPaths, f: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<()>) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        let games = doc.as_object_mut().expect("object").entry("games").or_insert_with(|| Value::Object(Default::default()));
        if !games.is_object() {
            *games = Value::Object(Default::default());
        }
        let cats = games.as_object_mut().expect("object").entry("customCategories").or_insert_with(|| Value::Object(Default::default()));
        if !cats.is_object() {
            *cats = Value::Object(Default::default());
        }
        f(cats.as_object_mut().expect("object"))
    })
}

fn clean(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("{}", heronite_i18n::tr!("The category needs a name"));
    }
    if name == UNCATEGORIZED {
        bail!("{}", heronite_i18n::tr!("That name is reserved"));
    }
    Ok(name.to_owned())
}

/// A new, empty category (an existing one is kept as is).
pub fn add_category(paths: &DataPaths, name: &str) -> Result<()> {
    let name = clean(name)?;
    update(paths, |cats| {
        cats.entry(name).or_insert_with(|| Value::Array(Vec::new()));
        Ok(())
    })
}

/// Renames a category, keeping its games. Renaming onto another category
/// merges them.
pub fn rename_category(paths: &DataPaths, old: &str, new: &str) -> Result<()> {
    let new = clean(new)?;
    if old == new {
        return Ok(());
    }
    update(paths, |cats| {
        let Some(Value::Array(games)) = cats.remove(old) else { return Ok(()) };
        let target = cats.entry(new).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(list) = target {
            for g in games {
                if !list.contains(&g) {
                    list.push(g);
                }
            }
        }
        Ok(())
    })
}

/// Deletes a category; its games stay in the library.
pub fn remove_category(paths: &DataPaths, name: &str) -> Result<()> {
    update(paths, |cats| {
        cats.remove(name);
        Ok(())
    })
}

/// Puts a game in a category (`on`) or takes it out. Adding to a category
/// that does not exist creates it.
pub fn set_in_category(paths: &DataPaths, name: &str, runner: &str, app_name: &str, on: bool) -> Result<()> {
    set_many_in_category(paths, name, &[(runner.to_owned(), app_name.to_owned(), on)])
}

/// [`set_in_category`] for many games in one write: `(runner, app_name, on)`.
pub fn set_many_in_category(paths: &DataPaths, name: &str, games: &[(String, String, bool)]) -> Result<()> {
    let name = clean(name)?;
    update(paths, |cats| {
        for (runner, app_name, on) in games {
            put_in_category(cats, &name, runner, app_name, *on);
        }
        Ok(())
    })
}

/// Puts each game back in exactly the categories listed for it:
/// `(runner, app_name, categories)`. Used to undo category changes.
pub fn restore_categories(paths: &DataPaths, games: &[(String, String, Vec<String>)]) -> Result<()> {
    update(paths, |cats| {
        let names: Vec<String> = cats.keys().cloned().collect();
        for (runner, app_name, keep) in games {
            for name in names.iter().chain(keep.iter().filter(|n| !names.contains(n))) {
                put_in_category(cats, name, runner, app_name, keep.contains(name));
            }
        }
        Ok(())
    })
}

fn put_in_category(cats: &mut serde_json::Map<String, Value>, name: &str, runner: &str, app_name: &str, on: bool) {
    let key = Value::String(game_ref(runner, app_name));
    if !on {
        if let Some(Value::Array(list)) = cats.get_mut(name) {
            list.retain(|g| *g != key);
        }
        return;
    }
    let list = cats.entry(name.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    let list = list.as_array_mut().expect("array");
    if !list.contains(&key) {
        list.push(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup(content: &str) -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.store_config().parent().unwrap()).unwrap();
        fs::write(paths.store_config(), content).unwrap();
        (dir, paths)
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_stored_categories() {
        let (_d, paths) = setup(r#"{"games":{"customCategories":{"RPG":["Quail_legendary","1_gog"],"Co-op":["1_gog"],"":["x_gog"]}}}"#);
        let c = Categories::load(&paths);
        assert_eq!(c.names().collect::<Vec<_>>(), ["Co-op", "RPG"], "sorted, nameless dropped");
        assert_eq!(c.of_game("gog", "1"), ["Co-op", "RPG"]);
        assert!(c.contains("RPG", "legendary", "Quail"));
        assert!(!c.contains("Co-op", "legendary", "Quail"));
        assert_eq!(c.len_of("RPG"), 2);
    }

    #[test]
    fn filter_is_a_union_with_uncategorized() {
        let (_d, paths) = setup(r#"{"games":{"customCategories":{"RPG":["Quail_legendary"],"Co-op":["1_gog"]}}}"#);
        let c = Categories::load(&paths);
        assert!(c.matches(&set(&[]), "sideload", "z"), "no selection: everything");
        assert!(c.matches(&set(&["RPG"]), "legendary", "Quail"));
        assert!(!c.matches(&set(&["RPG"]), "gog", "1"));
        assert!(c.matches(&set(&["RPG", "Co-op"]), "gog", "1"));
        assert!(c.matches(&set(&[UNCATEGORIZED]), "sideload", "z"));
        assert!(!c.matches(&set(&[UNCATEGORIZED]), "gog", "1"));
        assert!(c.matches(&set(&[UNCATEGORIZED, "Co-op"]), "gog", "1"));
        assert!(!c.matches(&set(&["Gone"]), "gog", "1"));
    }

    #[test]
    fn edits_keep_the_rest_of_the_document() {
        let (_d, paths) = setup(r#"{"theme":"nord","games":{"favourites":[{"appName":"a","title":"A"}]}}"#);
        add_category(&paths, "  RPG ").unwrap();
        set_in_category(&paths, "RPG", "legendary", "Quail", true).unwrap();
        set_in_category(&paths, "RPG", "legendary", "Quail", true).unwrap();
        set_in_category(&paths, "Co-op", "gog", "1", true).unwrap();
        let c = Categories::load(&paths);
        assert_eq!(c.len_of("RPG"), 1, "added once");
        assert!(c.contains("Co-op", "gog", "1"), "adding to a missing category creates it");

        rename_category(&paths, "RPG", "Co-op").unwrap();
        let c = Categories::load(&paths);
        assert!(!c.exists("RPG"));
        assert_eq!(c.len_of("Co-op"), 2, "renaming onto a category merges");

        set_in_category(&paths, "Co-op", "gog", "1", false).unwrap();
        remove_category(&paths, "Missing").unwrap();
        assert_eq!(Categories::load(&paths).of_game("legendary", "Quail"), ["Co-op"]);
        remove_category(&paths, "Co-op").unwrap();
        assert!(Categories::load(&paths).is_empty());

        let v: Value = serde_json::from_str(&fs::read_to_string(paths.store_config()).unwrap()).unwrap();
        assert_eq!(v["theme"], "nord");
        assert_eq!(v["games"]["favourites"][0]["appName"], "a");
        assert!(add_category(&paths, " ").is_err());
        assert!(add_category(&paths, UNCATEGORIZED).is_err());
    }

    #[test]
    fn many_games_and_restore() {
        let (_d, paths) = setup(r#"{"games":{"customCategories":{"RPG":["a_gog"],"Co-op":["b_gog"]}}}"#);
        let g = |app: &str, on: bool| ("gog".to_string(), app.to_string(), on);
        set_many_in_category(&paths, "RPG", &[g("a", true), g("b", true), g("c", false)]).unwrap();
        let c = Categories::load(&paths);
        assert_eq!(c.len_of("RPG"), 2);
        assert!(c.contains("RPG", "gog", "b"));
        restore_categories(
            &paths,
            &[("gog".into(), "a".into(), vec!["RPG".into()]), ("gog".into(), "b".into(), vec!["Co-op".into(), "New".into()])],
        )
        .unwrap();
        let c = Categories::load(&paths);
        assert_eq!(c.of_game("gog", "a"), ["RPG"]);
        assert_eq!(c.of_game("gog", "b"), ["Co-op", "New"]);
    }
}
