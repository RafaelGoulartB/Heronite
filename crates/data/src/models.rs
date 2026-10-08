//! Typed views of the JSON files shared with Heronite.
//!
//! Conventions (keep them when adding fields):
//! - `#[serde(rename_all = "camelCase")]`, matching the TypeScript types in
//!   `HeroicGamesLauncher/src/common/types/local-library.ts`.
//! - Optional fields are `Option<_>` with `skip_serializing_if`, so a field
//!   absent on disk stays absent after a write.
//! - Every struct ends with `#[serde(flatten)] extra`, so fields this app
//!   does not model survive a read-modify-write.
//! - Numbers use [`JsNum`] so integers are written without `.0`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::num::JsNum;

pub type Extra = Map<String, Value>;

// ---------------------------------------------------------------------------
// local_library/library.json
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// local_library/sessions.json
// ---------------------------------------------------------------------------

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

/// Heronite keeps at most this many sessions per game.
pub const MAX_SESSIONS_PER_GAME: usize = 200;

// ---------------------------------------------------------------------------
// local_library/statuses.json
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// local_library/settings.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSettingsFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ludusavi: Option<LudusaviSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grey_uninstalled_games: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshots: Option<ScreenshotsSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataSettings>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    #[serde(default)]
    pub folder: String,
    /// 'daily' | 'weekly' | 'monthly' (Heronite also accepts 'never' in the UI)
    #[serde(default)]
    pub interval: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub use_installed_config: bool,
    #[serde(default)]
    pub binary_path: String,
    #[serde(default)]
    pub backup_path: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub compression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_game: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSettings {
    pub enabled: bool,
    /// Backdrop blur over the artwork, px.
    pub blur: JsNum,
    /// Dark tint over the artwork, 0-90 (%).
    pub dimming: JsNum,
    /// Artwork saturation, 0-150 (%).
    pub saturation: JsNum,
    /// Side/top/bottom gradient strength, 0-150 (%).
    pub scrim: JsNum,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            blur: JsNum(6.0),
            dimming: JsNum(14.0),
            saturation: JsNum(94.0),
            scrim: JsNum(100.0),
            extra: Extra::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotsSettings {
    #[serde(default)]
    pub folder: String,
    #[serde(default = "default_cache_limit")]
    pub cache_limit_mb: JsNum,
    #[serde(default)]
    pub capture_enabled: bool,
    #[serde(default)]
    pub capture_accelerator: String,
    #[serde(flatten)]
    pub extra: Extra,
}

fn default_true() -> bool {
    true
}

fn default_cache_limit() -> JsNum {
    JsNum(500.0)
}

impl Default for ScreenshotsSettings {
    fn default() -> Self {
        Self {
            folder: String::new(),
            cache_limit_mb: default_cache_limit(),
            capture_enabled: false,
            capture_accelerator: String::new(),
            extra: Extra::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSettings {
    #[serde(default)]
    pub igdb_client_id: String,
    #[serde(default)]
    pub igdb_client_secret: String,
    /// 'steam' | 'igdb' | 'gog' | 'square' | 'dvd' | 'banner'
    #[serde(default)]
    pub cover_aspect: String,
    #[serde(default)]
    pub download_images: bool,
    #[serde(default = "default_true")]
    pub auto_fill_missing: bool,
    #[serde(default)]
    pub field_priority: IndexMap<String, Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------------------
// local_library/metadata.json
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// local_library/art.json
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// store/timestamp.json  (Heroic core: playtime, keyed by appName)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// store/config.json  (Heroic core; only the parts Collection uses)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroicConfigFile {
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

// ---------------------------------------------------------------------------
// Heroic GameInfo (sideload_apps/library.json, store_cache/*_library.json)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// store/game-overrides.json  (Heroic core: custom title/art per appName)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// gog_store/installed.json, zoom_store/installed.json
// ---------------------------------------------------------------------------

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
