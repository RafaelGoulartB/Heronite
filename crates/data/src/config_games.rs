//! Heroic's per-game lists in `store/config.json` (`games.favourites`,
//! `games.hidden`, `games.recent`), written the way `GlobalState.tsx` does:
//! `{ appName, title }` entries, read-modify-write of the raw document so
//! every other key (and key order) is preserved.

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::DataPaths;

fn list_mut<'a>(doc: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    if !doc.is_object() {
        *doc = Value::Object(Default::default());
    }
    let games = doc
        .as_object_mut()
        .expect("object")
        .entry("games")
        .or_insert_with(|| Value::Object(Default::default()));
    if !games.is_object() {
        *games = Value::Object(Default::default());
    }
    let list = games.as_object_mut().expect("object").entry(key).or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    list.as_array_mut().expect("array")
}

fn is_entry(v: &Value, app_name: &str) -> bool {
    v.get("appName").and_then(Value::as_str) == Some(app_name)
}

fn contains(path: &std::path::Path, key: &str, app_name: &str) -> Result<bool> {
    let doc = json_store::read_value(path)?;
    Ok(doc
        .get("games")
        .and_then(|g| g.get(key))
        .and_then(Value::as_array)
        .is_some_and(|l| l.iter().any(|v| is_entry(v, app_name))))
}

/// `addGameToFavourites`: any previous entry is dropped and the game is
/// appended.
pub fn add_favourite(paths: &DataPaths, app_name: &str, title: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        let list = list_mut(doc, "favourites");
        list.retain(|v| !is_entry(v, app_name));
        list.push(json!({ "appName": app_name, "title": title }));
        Ok(())
    })
}

/// `removeGameFromFavourites`.
pub fn remove_favourite(paths: &DataPaths, app_name: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        list_mut(doc, "favourites").retain(|v| !is_entry(v, app_name));
        Ok(())
    })
}

/// Toggles the favourite flag against what is on disk now (not a stale view);
/// returns the new state.
pub fn toggle_favourite(paths: &DataPaths, app_name: &str, title: &str) -> Result<bool> {
    if contains(&paths.store_config(), "favourites", app_name)? {
        remove_favourite(paths, app_name)?;
        Ok(false)
    } else {
        add_favourite(paths, app_name, title)?;
        Ok(true)
    }
}

/// `hideGame`: appended once (Heroic would allow duplicates, harmless to avoid).
pub fn hide_game(paths: &DataPaths, app_name: &str, title: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        let list = list_mut(doc, "hidden");
        if !list.iter().any(|v| is_entry(v, app_name)) {
            list.push(json!({ "appName": app_name, "title": title }));
        }
        Ok(())
    })
}

/// `unhideGame`.
pub fn unhide_game(paths: &DataPaths, app_name: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        list_mut(doc, "hidden").retain(|v| !is_entry(v, app_name));
        Ok(())
    })
}

/// Toggles hidden; returns the new state.
pub fn toggle_hidden(paths: &DataPaths, app_name: &str, title: &str) -> Result<bool> {
    if contains(&paths.store_config(), "hidden", app_name)? {
        unhide_game(paths, app_name)?;
        Ok(false)
    } else {
        hide_game(paths, app_name, title)?;
        Ok(true)
    }
}

/// The `{ appName, title }` entries of `games.<key>` (`favourites`, `hidden`
/// or `recent`) as stored, in order. Missing file or key: empty.
pub fn list_entries(paths: &DataPaths, key: &str) -> Vec<(String, String)> {
    let doc = json_store::read_value(&paths.store_config()).unwrap_or(Value::Null);
    doc.get("games")
        .and_then(|g| g.get(key))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|v| {
                    let app = v.get("appName")?.as_str()?.to_owned();
                    let title = v.get("title").and_then(Value::as_str).unwrap_or_default().to_owned();
                    Some((app, title))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Empties `games.<key>`; returns how many entries were dropped.
pub fn clear_list(paths: &DataPaths, key: &str) -> Result<usize> {
    json_store::update_value(&paths.store_config(), |doc| {
        let list = list_mut(doc, key);
        let n = list.len();
        list.clear();
        Ok(n)
    })
}

/// "Remove from recent".
pub fn remove_recent(paths: &DataPaths, app_name: &str) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        list_mut(doc, "recent").retain(|v| !is_entry(v, app_name));
        Ok(())
    })
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

    #[test]
    fn favourite_and_hidden_follow_heroic_shapes() {
        let (_d, paths) = setup(
            "{\n\t\"theme\": \"nord-dark\",\n\t\"games\": {\n\t\t\"recent\": [{\"appName\": \"r\", \"title\": \"R\"}],\n\t\t\"hidden\": [],\n\t\t\"favourites\": []\n\t},\n\t\"language\": \"en\"\n}",
        );
        assert!(toggle_favourite(&paths, "a", "Game A").unwrap());
        assert!(toggle_favourite(&paths, "b", "Game B").unwrap());
        add_favourite(&paths, "a", "Game A!").unwrap();
        let v = json_store::read_value(&paths.store_config()).unwrap();
        let favs = v["games"]["favourites"].as_array().unwrap();
        assert_eq!(favs.len(), 2);
        // re-adding moves it to the end with the new title
        assert_eq!(favs[1], json!({"appName": "a", "title": "Game A!"}));
        assert!(!toggle_favourite(&paths, "a", "x").unwrap());

        assert!(toggle_hidden(&paths, "h", "Hidden").unwrap());
        hide_game(&paths, "h", "Hidden").unwrap();
        let v = json_store::read_value(&paths.store_config()).unwrap();
        assert_eq!(v["games"]["hidden"].as_array().unwrap().len(), 1);
        assert!(!toggle_hidden(&paths, "h", "Hidden").unwrap());
        remove_recent(&paths, "r").unwrap();
        let v = json_store::read_value(&paths.store_config()).unwrap();
        assert!(v["games"]["hidden"].as_array().unwrap().is_empty());
        assert!(v["games"]["recent"].as_array().unwrap().is_empty());
        // untouched keys survive, in their order
        assert_eq!(v["theme"], "nord-dark");
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["theme", "games", "language"]);
    }

    #[test]
    fn missing_config_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(toggle_favourite(&paths, "a", "A").unwrap());
        let v = json_store::read_value(&paths.store_config()).unwrap();
        assert_eq!(v["games"]["favourites"][0]["appName"], "a");
    }

    #[test]
    fn list_and_clear() {
        let (_d, paths) = setup(
            "{\n\t\"games\": {\n\t\t\"hidden\": [{\"appName\": \"h\", \"title\": \"H\"}, {\"appName\": \"i\"}],\n\t\t\"recent\": [{\"appName\": \"r\", \"title\": \"R\"}]\n\t}\n}",
        );
        assert_eq!(list_entries(&paths, "hidden"), [("h".to_string(), "H".to_string()), ("i".to_string(), String::new())]);
        assert!(list_entries(&paths, "favourites").is_empty());
        assert_eq!(clear_list(&paths, "hidden").unwrap(), 2);
        assert!(list_entries(&paths, "hidden").is_empty());
        assert_eq!(list_entries(&paths, "recent").len(), 1);
    }
}
