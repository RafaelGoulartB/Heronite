//! The games hidden in Steam, read from the client's files: for a library
//! read through the account ([`crate::account`]), which Steam's Web API
//! lists without saying which games are hidden. A read of the running
//! client asks the client itself (`collectionStore.BIsHidden`).
//!
//! Steam keeps its collections in
//! `userdata/<account id>/config/cloudstorage/cloud-storage-namespace-1.json`,
//! a list of `[key, entry]` pairs. The hidden games are the entry
//! `user-collections.hidden`, whose `value` is a JSON text:
//! `{"id":"hidden","added":[620,…],"removed":[…]}`. The file is Steam's own
//! and undocumented: anything unexpected reads as no hidden game.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use heronite_data::steam_owned::account_id;
use serde_json::Value;

const KEY: &str = "user-collections.hidden";

/// The app ids hidden in Steam by the account `steam_id` (SteamID64), from
/// the first of `installs` (Steam folders, the chosen one first) that has
/// the account's collections. Empty when none has them.
pub fn hidden_app_ids(installs: &[PathBuf], steam_id: &str) -> HashSet<String> {
    let Some(account) = account_id(steam_id) else { return HashSet::new() };
    installs
        .iter()
        .map(|root| collections_file(root, account))
        .find_map(|file| std::fs::read_to_string(&file).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn collections_file(root: &Path, account: u32) -> PathBuf {
    root.join("userdata").join(account.to_string()).join("config/cloudstorage/cloud-storage-namespace-1.json")
}

fn parse(text: &str) -> HashSet<String> {
    let Ok(Value::Array(pairs)) = serde_json::from_str::<Value>(text) else { return HashSet::new() };
    let Some(entry) = pairs.iter().find_map(|p| (p.get(0)?.as_str()? == KEY).then(|| p.get(1)).flatten()) else {
        return HashSet::new();
    };
    if entry.get("is_deleted").and_then(Value::as_bool) == Some(true) {
        return HashSet::new();
    }
    let Some(collection) = entry.get("value").and_then(Value::as_str).and_then(|v| serde_json::from_str::<Value>(v).ok()) else {
        return HashSet::new();
    };
    let ids = |key: &str| -> HashSet<String> {
        collection.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).map(|id| id.to_string()).collect()
    };
    let removed = ids("removed");
    ids("added").into_iter().filter(|id| !removed.contains(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "76561197960287930";

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn file(value: &str) -> String {
        serde_json::json!([
            ["showcases-version", {"key": "showcases-version", "timestamp": 1, "is_deleted": true, "version": "96"}],
            [KEY, {"key": KEY, "timestamp": 1, "value": value, "version": "367"}]
        ])
        .to_string()
    }

    #[test]
    fn reads_the_hidden_collection_of_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let (other, steam) = (dir.path().join("other"), dir.path().join("steam"));
        let path = collections_file(&steam, account_id(ID).unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, file(r#"{"id":"hidden","name":"Hidden","added":[620,400,10],"removed":[10]}"#)).unwrap();
        assert!(path.ends_with("userdata/22202/config/cloudstorage/cloud-storage-namespace-1.json"));
        // The first install with the file counts.
        assert_eq!(hidden_app_ids(&[other.clone(), steam.clone()], ID), ids(&["620", "400"]));
        // Another account's folder, or none at all: nothing hidden.
        assert!(hidden_app_ids(std::slice::from_ref(&steam), "76561197960265770").is_empty());
        assert!(hidden_app_ids(&[other], ID).is_empty());
        assert!(hidden_app_ids(&[steam], "not an id").is_empty());
    }

    #[test]
    fn anything_unexpected_hides_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("{}").is_empty());
        assert!(parse(&file("not json")).is_empty());
        assert!(parse(&file(r#"{"id":"hidden"}"#)).is_empty());
        let deleted = serde_json::json!([[KEY, {"key": KEY, "is_deleted": true, "value": r#"{"added":[1]}"#}]]).to_string();
        assert!(parse(&deleted).is_empty());
        assert_eq!(parse(&file(r#"{"added":[1,"2",3.5,4]}"#)), ids(&["1", "4"]));
    }
}
