use heronite_data::{json_store, HeroicPaths};
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
        self.is_installed == Some(true)
            && self.install_dir.as_deref().map_or(false, |d| !d.is_empty())
    }

    pub fn has_update(&self) -> bool {
        self.has_update == Some(true)
    }

    /// Releases without a download asset cannot be installed (Heroic hides them).
    pub fn downloadable(&self) -> bool {
        self.downsize.unwrap_or(0) > 0 && self.download.as_deref().map_or(false, |d| !d.is_empty())
    }
}

/// Progress of one install, mirrors `WineManagerStatus`.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Downloading { percentage: f64, downloaded: u64, total: u64, speed: f64, eta_secs: Option<u64> },
    Unzipping,
}

/// The few Heroic global settings the manager reads (`config.json` ->
/// `defaultSettings`).
#[derive(Clone, Debug, Default)]
pub struct WineConfig {
    pub default_steam_path: String,
    pub custom_wine_paths: Vec<String>,
    pub show_valve_proton: bool,
    pub download_proton_to_steam: bool,
}

impl WineConfig {
    /// Reads `<heroic>/config.json` (`defaultSettings`), falling back to the
    /// `store/config.json` copy; missing keys get Heroic's defaults.
    pub fn load(heroic: &HeroicPaths) -> Self {
        let mut cfg = Self::default();
        let from_default = json_store::read_value(&heroic.root.join("config.json"))
            .ok()
            .and_then(|v| v.get("defaultSettings").cloned());
        let doc = match from_default {
            Some(v) if v.is_object() => Some(v),
            _ => json_store::read_value(&heroic.config()).ok().filter(Value::is_object),
        };
        if let Some(Value::Object(m)) = doc {
            if let Some(s) = m.get("defaultSteamPath").and_then(Value::as_str) {
                cfg.default_steam_path = s.replace('\'', "");
            }
            if let Some(a) = m.get("customWinePaths").and_then(Value::as_array) {
                cfg.custom_wine_paths = a.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            cfg.show_valve_proton = m.get("showValveProton").and_then(Value::as_bool).unwrap_or(false);
            cfg.download_proton_to_steam =
                m.get("downloadProtonToSteam").and_then(Value::as_bool).unwrap_or(false);
        }
        if cfg.default_steam_path.is_empty() {
            let home = dirs::home_dir().unwrap_or_default();
            cfg.default_steam_path = home.join(".steam/steam").to_string_lossy().into_owned();
        }
        cfg
    }
}
