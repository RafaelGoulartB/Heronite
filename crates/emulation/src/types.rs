//! Data shapes shared with the UI (camelCase, same as
//! `common/types/local-library.ts`).

use heronite_data::models::{Extra, LocalGameRom};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use serde::{Deserialize, Serialize};

use crate::catalog::EmulatorDefinition;

// local_library/rom-scanners.json

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomScannersFile {
    #[serde(default)]
    pub scanners: Vec<RomScanner>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RomScanner {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub emulator_id: String,
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub scan_subfolders: bool,
    #[serde(default)]
    pub merge_related_files: bool,
    #[serde(default)]
    pub auto_scan_on_refresh: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `UpsertUserEmulatorArgs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertUserEmulatorArgs {
    #[serde(default)]
    pub id: Option<String>,
    pub definition_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub install_dir: Option<String>,
    /// 'native' | 'flatpak' | 'wine'
    pub launch_kind: String,
    #[serde(default)]
    pub flatpak_id: Option<String>,
    #[serde(default)]
    pub playnite_id: Option<String>,
    #[serde(default)]
    pub profiles: Option<Vec<UserEmulatorProfile>>,
}

/// `UpsertRomScannerArgs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRomScannerArgs {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub emulator_id: String,
    pub profile_id: String,
    pub directory: String,
    #[serde(default)]
    pub scan_subfolders: Option<bool>,
    #[serde(default)]
    pub merge_related_files: Option<bool>,
    #[serde(default)]
    pub auto_scan_on_refresh: Option<bool>,
}

/// Arguments of `previewRomScan`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomScanArgs {
    pub emulator_id: String,
    pub profile_id: String,
    pub directory: String,
    #[serde(default)]
    pub scan_subfolders: Option<bool>,
    #[serde(default)]
    pub merge_related_files: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomScanItem {
    pub title: String,
    pub roms: Vec<LocalGameRom>,
    #[serde(default)]
    pub already_imported: bool,
    #[serde(default)]
    pub import: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomScanPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanner_id: Option<String>,
    pub emulator_id: String,
    pub emulator_name: String,
    pub profile_id: String,
    pub profile_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_name: Option<String>,
    pub directory: String,
    pub games: Vec<RomScanItem>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomScanImportArgs {
    pub emulator_id: String,
    pub profile_id: String,
    pub directory: String,
    #[serde(default)]
    pub scanner_id: Option<String>,
    #[serde(default)]
    pub scan_subfolders: Option<bool>,
    #[serde(default)]
    pub merge_related_files: Option<bool>,
    #[serde(default)]
    pub auto_scan_on_refresh: Option<bool>,
    pub games: Vec<RomScanItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RomScanImportResult {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub scanner_id: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEmulatorProfile {
    pub id: String,
    pub name: String,
    pub platform_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_path: Option<String>,
    /// Overrides for profiles the catalog does not know (discovered forks
    /// such as yuzu); `None` falls back to the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEmulator {
    pub definition_id: String,
    pub name: String,
    pub executable: String,
    /// 'native' | 'flatpak'
    pub launch_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flatpak_id: Option<String>,
    pub profiles: Vec<DetectedEmulatorProfile>,
    pub already_added: bool,
    /// Where it was found: `PATH`, `Flatpak` or a folder label.
    #[serde(default)]
    pub source: String,
}

/// `EmulationState`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulationState {
    pub catalog: Vec<EmulatorDefinition>,
    pub emulators: Vec<UserEmulator>,
    pub scanners: Vec<RomScanner>,
    pub detected: Vec<DetectedEmulator>,
}
