//! `local_library/settings.json`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::Extra;
use crate::num::JsNum;

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
    /// More folders to restore backups from (a Ludusavi folder made on
    /// another PC or on Windows), after the backup folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restore_paths: Vec<String>,
    /// Full backups kept per game (1..=255); `None` = Ludusavi's own setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_limit: Option<u8>,
    /// Differential backups kept per full one (0..=255); `None` = Ludusavi's own setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub differential_limit: Option<u8>,
    /// Games (`runner:appName`, `game_key`) left out of the after-play
    /// backup; a backup by hand still runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_backup_off: Vec<String>,
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
        Self { enabled: true, blur: JsNum(6.0), dimming: JsNum(14.0), saturation: JsNum(94.0), scrim: JsNum(100.0), extra: Extra::new() }
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

impl LudusaviSettings {
    /// Whether a session of this game ends with a backup.
    pub fn auto_backup_for(&self, runner: &str, app_name: &str) -> bool {
        self.enabled && !self.auto_backup_off.contains(&super::game_key(runner, app_name))
    }
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
