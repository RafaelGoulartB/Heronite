//! Preview / result / export shapes (`PlaynitePreview*`, `PlayniteImport*`,
//! `PlayniteMerge*`, `PlayniteExport*` of `common/types/local-library.ts`).
//! `camelCase` and `skip_serializing_if` match what the TS puts on the wire.

use heronite_data::models::{CompletionStatus, LocalGameSession};
use heronite_data::JsNum;
use serde::{Deserialize, Serialize};

pub use crate::remap::DriveRemap;

/// `PlaynitePreviewGame.destination` / `MappedDestination`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Destination {
    Sideload,
    Legendary,
    Gog,
    Nile,
    Skip,
}

impl Destination {
    pub fn as_str(self) -> &'static str {
        match self {
            Destination::Sideload => "sideload",
            Destination::Legendary => "legendary",
            Destination::Gog => "gog",
            Destination::Nile => "nile",
            Destination::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaynitePreviewGame {
    pub playnite_id: String,
    pub title: String,
    /// `LocalGameSource`.
    pub source: String,
    pub destination: Destination,
    pub playtime_minutes: JsNum,
    pub session_count: usize,
    pub already_imported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_app_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteImportCounts {
    pub steam: usize,
    pub emulator: usize,
    pub manual: usize,
    pub xbox: usize,
    pub amazon: usize,
    pub other: usize,
    pub epic_match: usize,
    pub gog_match: usize,
    pub skipped_store: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteImportPreview {
    pub library_path: String,
    pub playnite_root: String,
    pub game_count: usize,
    pub session_file_count: usize,
    pub detected_drives: Vec<String>,
    pub games: Vec<PlaynitePreviewGame>,
    pub counts: PlayniteImportCounts,
    pub errors: Vec<String>,
}

/// `PlayniteImportArgs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteImportArgs {
    pub library_path: String,
    #[serde(default)]
    pub drive_map: Vec<DriveRemap>,
    /// `None` means true (only an explicit `false` disables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_covers: Option<bool>,
    #[serde(default)]
    pub merge_existing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteImportResult {
    pub imported: usize,
    pub updated: usize,
    pub matched_store: usize,
    pub skipped: usize,
    pub covers_fetched: usize,
    pub sessions_imported: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteMergePreviewItem {
    pub title: String,
    pub playnite_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heroic_minutes: Option<JsNum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_minutes: Option<JsNum>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heroic_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_status: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteMergePreview {
    pub library_path: String,
    pub new_games: Vec<PlayniteMergePreviewItem>,
    pub playtime_updates: Vec<PlayniteMergePreviewItem>,
    pub status_from_playnite: Vec<PlayniteMergePreviewItem>,
    pub status_conflicts: Vec<PlayniteMergePreviewItem>,
    pub new_sessions: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteExportStatus {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playnite_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteExportGame {
    pub playnite_id: String,
    pub heroic_app_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_game_id: Option<String>,
    pub title: String,
    pub source: String,
    pub playtime_minutes: JsNum,
    pub playtime_seconds: JsNum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_played: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_played: Option<String>,
    pub completion_status: PlayniteExportStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_playnite_completion_status_id: Option<String>,
    pub sessions: Vec<LocalGameSession>,
}

/// `format: 'heroic-playnite-export'`, `version: 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteExportFile {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub comment: String,
    pub statuses: Vec<CompletionStatus>,
    pub games: Vec<PlayniteExportGame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayniteExportResult {
    pub path: String,
    pub game_count: usize,
}
