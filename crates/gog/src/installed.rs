//! `gog_store/installed.json`: `{"installed": [ {...}, ... ]}`, one entry per
//! installed game. Keys the app does not model are kept on every rewrite.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// One installed game. `executable` (always `""`) and `is_dlc` live in
/// `extra` with whatever else the file holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    #[serde(rename = "appName", default)]
    pub app_name: String,
    /// `windows`, `linux` or `osx`.
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub install_path: String,
    /// Text such as `1.5 GiB`.
    #[serde(default)]
    pub install_size: String,
    /// gogdl's `versionName`.
    #[serde(default)]
    pub version: String,
    /// Language code, e.g. `en-US`.
    #[serde(default)]
    pub language: String,
    #[serde(rename = "installedDLCs", default)]
    pub installed_dlcs: Vec<String>,
    /// `ETag` of the build's manifest; compared by the update check.
    #[serde(rename = "versionEtag", default)]
    pub version_etag: String,
    #[serde(rename = "buildId", default)]
    pub build_id: String,
    /// Games pinned to a version are never offered updates.
    #[serde(rename = "pinnedVersion", default)]
    pub pinned_version: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Installed {
    /// A new entry with the fixed keys filled in.
    pub fn new(app_name: &str) -> Self {
        let mut extra = Map::new();
        extra.insert("executable".into(), json!(""));
        extra.insert("is_dlc".into(), json!(false));
        Self { app_name: app_name.to_string(), extra, ..Default::default() }
    }

    /// The `install` object of a `GameInfo`.
    pub fn game_install(&self) -> Value {
        json!({
            "executable": self.extra.get("executable").and_then(Value::as_str).unwrap_or(""),
            "platform": self.platform,
            "is_dlc": false,
            "install_path": self.install_path,
            "install_size": self.install_size,
            "version": self.version,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    installed: Vec<Installed>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub fn load(paths: &DataPaths) -> Vec<Installed> {
    match json_store::read::<File>(&paths.gog_installed()) {
        Ok(f) => f.installed,
        Err(err) => {
            log::warn!("reading installed GOG games: {err:#}");
            Vec::new()
        }
    }
}

pub fn get(paths: &DataPaths, app_name: &str) -> Option<Installed> {
    load(paths).into_iter().find(|i| i.app_name == app_name)
}

/// Adds the entry or replaces the one with the same app name (its unknown
/// keys carry over unless the new entry sets them).
pub fn upsert(paths: &DataPaths, entry: &Installed) -> Result<()> {
    json_store::update::<File, _>(&paths.gog_installed(), |file| {
        match file.installed.iter_mut().find(|i| i.app_name == entry.app_name) {
            Some(old) => {
                let mut merged = entry.clone();
                for (k, v) in std::mem::take(&mut old.extra) {
                    merged.extra.entry(k).or_insert(v);
                }
                *old = merged;
            }
            None => file.installed.push(entry.clone()),
        }
        Ok(())
    })
}

pub fn remove(paths: &DataPaths, app_name: &str) -> Result<()> {
    if !paths.gog_installed().exists() {
        return Ok(());
    }
    json_store::update::<File, _>(&paths.gog_installed(), |file| {
        file.installed.retain(|i| i.app_name != app_name);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_with_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("gog_store")).unwrap();
        std::fs::write(
            paths.gog_installed(),
            r#"{"installed":[{"appName":"1","platform":"windows","install_path":"/g/one","executable":"","install_size":"1 GiB","is_dlc":false,"version":"1.0","language":"en-US","installedDLCs":["9"],"versionEtag":"\"e\"","buildId":"77","pinnedVersion":true,"somethingNew":5}],"other":1}"#,
        )
        .unwrap();
        let got = get(&paths, "1").unwrap();
        assert_eq!((got.build_id.as_str(), got.pinned_version, got.installed_dlcs.clone()), ("77", true, vec!["9".to_string()]));
        assert_eq!(got.version_etag, "\"e\"");

        let mut changed = got.clone();
        changed.version = "2.0".into();
        upsert(&paths, &changed).unwrap();
        let mut second = Installed::new("2");
        second.install_path = "/g/two".into();
        upsert(&paths, &second).unwrap();

        let doc = json_store::read_value(&paths.gog_installed()).unwrap();
        assert_eq!(doc["other"], 1);
        assert_eq!(doc["installed"][0]["version"], "2.0");
        assert_eq!(doc["installed"][0]["somethingNew"], 5);
        assert_eq!(doc["installed"][1]["executable"], "");
        assert_eq!(doc["installed"][1]["is_dlc"], false);
        assert_eq!(doc["installed"][1]["pinnedVersion"], false);
        assert_eq!(load(&paths).len(), 2);

        remove(&paths, "1").unwrap();
        assert_eq!(load(&paths).iter().map(|i| i.app_name.as_str()).collect::<Vec<_>>(), ["2"]);
        assert!(get(&paths, "1").is_none());
    }

    #[test]
    fn missing_files_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(load(&paths).is_empty());
        remove(&paths, "x").unwrap();
        assert!(!paths.gog_installed().exists());
    }
}
