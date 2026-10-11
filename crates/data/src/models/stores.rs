//! Heroic core files that Collection reads.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Extra;
use crate::num::JsNum;

// store/timestamp.json  (Heroic core: playtime, keyed by appName)
pub type TimestampFile = IndexMap<String, TimestampEntry>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_played: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_played: Option<String>,
    /// Minutes.
    #[serde(default)]
    pub total_played: JsNum,
    #[serde(flatten)]
    pub extra: Extra,
}

// store/config.json  (Heroic core; only the parts Collection uses)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_tiles_in_color: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub games: Option<ConfigGames>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigGames {
    #[serde(default)]
    pub recent: Vec<GameRef>,
    #[serde(default)]
    pub hidden: Vec<GameRef>,
    #[serde(default)]
    pub favourites: Vec<GameRef>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameRef {
    pub app_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(flatten)]
    pub extra: Extra,
}

// Heroic GameInfo (sideload_apps/library.json, store_cache/*_library.json)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideloadLibraryFile {
    #[serde(default)]
    pub games: Vec<GameInfo>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `store_cache/legendary_library.json` / `nile_library.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreLibraryFile {
    #[serde(default)]
    pub library: Vec<GameInfo>,
    /// gog_library.json uses `games`.
    #[serde(default)]
    pub games: Vec<GameInfo>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Heroic's GameInfo. Snake_case keys are Heroic's own naming.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameInfo {
    pub runner: String,
    pub app_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_cover: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_square: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer: Option<String>,
    #[serde(default)]
    pub is_installed: bool,
    #[serde(default)]
    pub install: InstallInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_linux_native: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    #[serde(flatten)]
    pub rest: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_dlc: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_size: Option<String>,
    #[serde(flatten)]
    pub rest: Extra,
}

/// Collection's key for a game across overlay stores: `"runner:appName"`.
pub fn game_key(runner: &str, app_name: &str) -> String {
    format!("{runner}:{app_name}")
}

// store/game-overrides.json  (Heroic core: custom title/art per appName)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameOverridesFile {
    #[serde(default)]
    pub overrides: IndexMap<String, GameOverride>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Heroic's `GameInfo.overrides`, attached in memory by the frontend.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GameOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_cover: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_square: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

// gog_store/installed.json, zoom_store/installed.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledGamesFile {
    #[serde(default)]
    pub installed: Vec<InstalledGame>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Heroic's `InstalledInfo`; becomes `GameInfo.install` for GOG/Zoom games.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    #[serde(default)]
    pub app_name: String,
    #[serde(flatten)]
    pub info: InstallInfo,
}

