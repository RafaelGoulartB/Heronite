//! Collection overlay files in `local_library/`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Extra;
use crate::num::JsNum;

// local_library/library.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalMetaFile {
    #[serde(default)]
    pub games: IndexMap<String, LocalGameMeta>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalGameRom {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalGameMeta {
    pub app_name: String,
    /// 'sideload' | 'legendary' | 'gog' | 'nile'
    pub runner: String,
    #[serde(default)]
    pub playnite_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// 'steam' | 'epic' | 'gog' | 'amazon' | 'xbox' | 'emulator' | 'manual' | 'other'
    #[serde(default)]
    pub source: String,
    /// 'steam-uri' | 'executable' | 'emulator' | 'unavailable'
    #[serde(default)]
    pub launch_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_game_id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_install_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remapped_executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launcher_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roms: Option<Vec<LocalGameRom>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_rom_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_state_keys: Option<IndexMap<String, String>>,
    /// 'auto' | 'linux' | 'windows'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_status_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_completion_status_id: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

// local_library/sessions.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsFile {
    #[serde(default)]
    pub sessions: IndexMap<String, Vec<LocalGameSession>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalGameSession {
    pub started_at: String,
    pub ended_at: String,
    pub elapsed_seconds: JsNum,
    /// 'playnite' | 'heroic'
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_action_name: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// At most this many sessions are kept per game (Heronite kept 200: a game
/// played every day lost its older months to Statistics and the year in
/// review). About 190 bytes each on disk.
pub const MAX_SESSIONS_PER_GAME: usize = 5000;

// local_library/statuses.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusesFile {
    #[serde(default)]
    pub statuses: Vec<CompletionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_playnite_library_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_playnite_drive_map: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionStatus {
    pub id: String,
    pub name: String,
    /// 'playing' | 'plan-to-play' | 'on-hold' | 'played' | 'endless' | 'beaten'
    /// | 'completed' | 'abandoned' | 'not-played' | 'custom'
    pub slug: String,
    pub sort_order: JsNum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_ids: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

// local_library/metadata.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFile {
    /// Keyed by `"runner:appName"`.
    #[serde(default)]
    pub games: IndexMap<String, CollectionGameMetadata>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionGameMetadata {
    pub app_name: String,
    pub runner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub igdb_id: Option<JsNum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub igdb_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lutris_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default)]
    pub developers: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub themes: Vec<String>,
    #[serde(default)]
    pub game_modes: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critic_score: Option<JsNum>,
    #[serde(default)]
    pub websites: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_url: Option<String>,
    #[serde(default)]
    pub field_sources: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_error: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

// local_library/art.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtFile {
    /// Keyed by `"runner:appName"`.
    #[serde(default)]
    pub games: IndexMap<String, CollectionGameArt>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionGameArt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_url: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
