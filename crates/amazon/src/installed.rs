//! nile's `installed.json`: an array of `{ id, version, path, size? }` where
//! `id` is the product id. nile writes it; the edits here only drop an entry
//! nile could not uninstall or rewrite a `path` after a move, and keep every
//! other key and the order of the array.

use std::path::PathBuf;

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::DataPaths;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// Product id (the `app_name` of the library).
    pub id: String,
    pub version: String,
    /// Install folder.
    pub path: String,
    /// Bytes on disk, when nile recorded it.
    pub size: Option<u64>,
}

pub(crate) fn file(paths: &DataPaths) -> PathBuf {
    paths.nile_config_dir().join("nile").join("installed.json")
}

fn parse(entry: &Value) -> Option<Installed> {
    let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let id = text("id");
    if id.is_empty() {
        return None;
    }
    let size = entry.get("size").and_then(|s| s.as_u64().or_else(|| s.as_f64().filter(|f| *f >= 0.).map(|f| f as u64)));
    Some(Installed { id, version: text("version"), path: text("path"), size })
}

/// Every installed game. A missing or unreadable file gives an empty list.
pub fn load(paths: &DataPaths) -> Vec<Installed> {
    match json_store::read_value(&file(paths)) {
        Ok(Value::Array(list)) => list.iter().filter_map(parse).collect(),
        Ok(_) => Vec::new(),
        Err(err) => {
            log::warn!("reading nile's installed games: {err:#}");
            Vec::new()
        }
    }
}

pub fn get(paths: &DataPaths, app_name: &str) -> Option<Installed> {
    load(paths).into_iter().find(|g| g.id == app_name)
}

/// Read-modify-write of the array; the closure returns whether it changed
/// anything. A missing file is left alone.
fn edit(paths: &DataPaths, f: impl FnOnce(&mut Vec<Value>) -> bool) -> Result<bool> {
    let path = file(paths);
    if !path.exists() {
        return Ok(false);
    }
    json_store::update_value(&path, |doc| {
        let Value::Array(list) = doc else { return Ok(false) };
        Ok(f(list))
    })
}

fn is_entry(entry: &Value, app_name: &str) -> bool {
    entry.get("id").and_then(Value::as_str) == Some(app_name)
}

/// Rewrites the `path` of one game. `false` when it is not in the file.
pub(crate) fn set_path(paths: &DataPaths, app_name: &str, new_path: &str) -> Result<bool> {
    edit(paths, |list| match list.iter_mut().find(|e| is_entry(e, app_name)).and_then(Value::as_object_mut) {
        Some(entry) => {
            entry.insert("path".into(), Value::String(new_path.into()));
            true
        }
        None => false,
    })
}

/// Drops the entry of one game. `false` when it was not in the file.
pub(crate) fn remove(paths: &DataPaths, app_name: &str) -> Result<bool> {
    edit(paths, |list| {
        let before = list.len();
        list.retain(|e| !is_entry(e, app_name));
        list.len() != before
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::Env;

    const FILE: &str = r#"[
        {"id": "amzn1.adg.product.AAA", "version": "v1", "path": "/games/A", "size": 1500},
        {"id": "amzn1.adg.product.BBB", "version": "v2", "path": "/games/B", "extra": [1, 2]},
        {"version": "no id"}
    ]"#;

    #[test]
    fn entries_are_read_and_the_ones_without_an_id_skipped() {
        let env = Env::new();
        assert!(load(&env.paths).is_empty());
        env.write_nile_file("installed.json", FILE);
        let list = load(&env.paths);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0],
            Installed { id: "amzn1.adg.product.AAA".into(), version: "v1".into(), path: "/games/A".into(), size: Some(1500) }
        );
        assert_eq!(list[1].size, None);
        assert_eq!(get(&env.paths, "amzn1.adg.product.BBB").unwrap().path, "/games/B");
        assert!(get(&env.paths, "nope").is_none());
    }

    #[test]
    fn broken_file_gives_an_empty_list() {
        let env = Env::new();
        env.write_nile_file("installed.json", "{not json");
        assert!(load(&env.paths).is_empty());
    }

    #[test]
    fn edits_keep_other_keys_and_the_order() {
        let env = Env::new();
        env.write_nile_file("installed.json", FILE);
        assert!(set_path(&env.paths, "amzn1.adg.product.BBB", "/new/B").unwrap());
        assert!(!set_path(&env.paths, "missing", "/x").unwrap());
        let doc = json_store::read_value(&file(&env.paths)).unwrap();
        assert_eq!(doc[0]["path"], "/games/A");
        assert_eq!(doc[1]["path"], "/new/B");
        assert_eq!(doc[1]["extra"][1], 2);
        assert_eq!(doc[2]["version"], "no id");
        assert!(remove(&env.paths, "amzn1.adg.product.AAA").unwrap());
        assert!(!remove(&env.paths, "amzn1.adg.product.AAA").unwrap());
        assert_eq!(load(&env.paths).len(), 1);
    }

    #[test]
    fn editing_a_missing_file_does_not_create_it() {
        let env = Env::new();
        assert!(!remove(&env.paths, "x").unwrap());
        assert!(!file(&env.paths).exists());
    }
}
