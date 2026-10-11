use heronite_data::{json_store, DataPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Heroic's `WineInstallation` (`common/types.ts`), the value stored as
/// `wineVersion` in `config.json` / `GamesConfig/<app>.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WineInstallation {
    pub bin: String,
    pub name: String,
    /// `wine` | `proton` | `crossover` | `toolkit`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib32: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wineserver: Option<String>,
}

/// Heroic's `WineVersionInfo` (an entry of `wine-releases` in
/// `store/wine-downloader-info.json`). Absent fields stay absent when written
/// back (Heroic leaves them `undefined`); unknown fields are preserved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WineVersionInfo {
    pub version: String,
    /// `GE-Proton`, `Proton-CachyOS`, `Proton`, `Wine-Lutris`, ...
    #[serde(rename = "type", default)]
    pub kind: String,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downsize: Option<u64>,
    #[serde(default)]
    pub disksize: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default)]
    pub release_notes_link: String,
    #[serde(rename = "installDir", default, skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    #[serde(rename = "isInstalled", default, skip_serializing_if = "Option::is_none")]
    pub is_installed: Option<bool>,
    #[serde(rename = "hasUpdate", default, skip_serializing_if = "Option::is_none")]
    pub has_update: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl WineVersionInfo {
    pub fn installed(&self) -> bool {
        self.is_installed == Some(true) && self.install_dir.as_deref().is_some_and(|d| !d.is_empty())
    }

    pub fn has_update(&self) -> bool {
        self.has_update == Some(true)
    }

    /// Releases without a download asset cannot be installed (Heroic hides them).
    pub fn downloadable(&self) -> bool {
        self.downsize.unwrap_or(0) > 0 && self.download.as_deref().is_some_and(|d| !d.is_empty())
    }
}

/// Progress of one install, mirrors `WineManagerStatus`.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Downloading { percentage: f64, downloaded: u64, total: u64, speed: f64, eta_secs: Option<u64> },
    Unzipping,
}

/// The few global settings the manager reads (`config.json` ->
/// `defaultSettings`).
#[derive(Clone, Debug, Default)]
pub struct WineConfig {
    pub default_steam_path: String,
    pub custom_wine_paths: Vec<String>,
    pub show_valve_proton: bool,
    pub download_proton_to_steam: bool,
}

impl WineConfig {
    /// Reads `<data>/config.json` (`defaultSettings`), falling back to the
    /// `store/config.json` copy; missing keys get Heroic's defaults.
    pub fn load(data_paths: &DataPaths) -> Self {
        let mut cfg = Self::default();
        let from_default = json_store::read_value(&data_paths.global_config()).ok().and_then(|v| v.get("defaultSettings").cloned());
        let doc = match from_default {
            Some(v) if v.is_object() => Some(v),
            _ => json_store::read_value(&data_paths.store_config()).ok().filter(Value::is_object),
        };
        if let Some(Value::Object(m)) = doc {
            if let Some(s) = m.get("defaultSteamPath").and_then(Value::as_str) {
                cfg.default_steam_path = s.replace('\'', "");
            }
            if let Some(a) = m.get("customWinePaths").and_then(Value::as_array) {
                cfg.custom_wine_paths = a.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            cfg.show_valve_proton = m.get("showValveProton").and_then(Value::as_bool).unwrap_or(false);
            cfg.download_proton_to_steam = m.get("downloadProtonToSteam").and_then(Value::as_bool).unwrap_or(false);
        }
        if cfg.default_steam_path.is_empty() {
            let home = dirs::home_dir().unwrap_or_default();
            cfg.default_steam_path = home.join(".steam/steam").to_string_lossy().into_owned();
        }
        cfg
    }
}

impl WineConfig {
    /// Writes `downloadProtonToSteam` like Heroic's `writeConfig('default')`:
    /// `config.json` (`defaultSettings`, 2-space JSON) and, when present, the
    /// copy in `store/config.json` (`settings`, electron-store format).
    pub fn set_download_proton_to_steam(data_paths: &DataPaths, value: bool) -> anyhow::Result<()> {
        // Heroic's config.json is `JSON.stringify(config, null, 2)`; the store
        // helper adds the path lock, a unique temp file and the fsync.
        json_store::update_value_indented(&data_paths.global_config(), json_store::INDENT_2, |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Map::new());
            }
            let settings = doc.as_object_mut().unwrap().entry("defaultSettings").or_insert_with(|| Value::Object(Map::new()));
            if let Some(m) = settings.as_object_mut() {
                m.insert("downloadProtonToSteam".into(), Value::Bool(value));
            }
            Ok(())
        })?;

        if data_paths.store_config().exists() {
            json_store::update_value(&data_paths.store_config(), |doc| {
                if let Some(m) = doc.get_mut("settings").and_then(Value::as_object_mut) {
                    m.insert("downloadProtonToSteam".into(), Value::Bool(value));
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_and_download_to_steam_write() {
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path());
        assert!(!WineConfig::load(&data_paths).download_proton_to_steam);
        std::fs::write(
            tmp.path().join("config.json"),
            "{\n  \"defaultSettings\": {\n    \"defaultSteamPath\": \"/s/steam\",\n    \"customWinePaths\": [\"/w/bin/wine\"],\n    \"showValveProton\": true,\n    \"other\": 1\n  },\n  \"version\": \"v0\"\n}",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("store")).unwrap();
        std::fs::write(tmp.path().join("store/config.json"), "{\n\t\"settings\": {\n\t\t\"a\": 1\n\t},\n\t\"theme\": \"x\"\n}").unwrap();
        let cfg = WineConfig::load(&data_paths);
        assert_eq!(cfg.default_steam_path, "/s/steam");
        assert_eq!(cfg.custom_wine_paths, ["/w/bin/wine"]);
        assert!(cfg.show_valve_proton && !cfg.download_proton_to_steam);

        WineConfig::set_download_proton_to_steam(&data_paths, true).unwrap();
        assert!(WineConfig::load(&data_paths).download_proton_to_steam);
        let text = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        assert!(text.contains("\n    \"other\": 1,"), "{text}");
        assert!(text.contains("\"version\": \"v0\""));
        let store = std::fs::read_to_string(tmp.path().join("store/config.json")).unwrap();
        assert!(store.contains("\"downloadProtonToSteam\": true") && store.contains("\"theme\": \"x\""), "{store}");
        // Same store helper as every other write: no temp file is left behind.
        let leftovers: Vec<_> =
            std::fs::read_dir(tmp.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn download_to_steam_creates_a_missing_config_with_two_space_indent() {
        let tmp = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(tmp.path());
        WineConfig::set_download_proton_to_steam(&data_paths, true).unwrap();
        let text = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        assert_eq!(text, "{\n  \"defaultSettings\": {\n    \"downloadProtonToSteam\": true\n  }\n}");
    }
}
