//! Effective Collection settings: port of `src/local-library/settings.ts`
//! (`getCollectionSettings` and the setters), plus the few Heroic settings the
//! Collection screen reads (`theme`, `language`, `allTilesInColor`).
//!
//! Reading applies the same defaults/normalisation as Heronite. Writing is a
//! read-modify-write of `local_library/settings.json` that touches only the
//! key being saved, like electron-store's `set(key, value)`.

use indexmap::IndexMap;

use anyhow::Result;
use serde_json::json;

use crate::json_store;
use crate::models::{
    BackgroundSettings, BackupSettings, CollectionSettingsFile, LudusaviSettings, MetadataSettings, ScreenshotsSettings, StoreConfigFile,
};
use crate::num::{js_round, JsNum};
use crate::paths::DataPaths;

pub const DEFAULT_SCREENSHOT_CACHE_LIMIT_MB: f64 = 500.0;
const MIN_SCREENSHOT_CACHE_LIMIT_MB: f64 = 50.0;
const MAX_SCREENSHOT_CACHE_LIMIT_MB: f64 = 10_000.0;
pub const DEFAULT_THEME: &str = "midnightMirage";
pub const DEFAULT_CAPTURE_ACCELERATOR: &str = "F10";

pub const METADATA_FIELDS: [&str; 13] = [
    "description",
    "releaseDate",
    "developers",
    "publishers",
    "genres",
    "themes",
    "gameModes",
    "platforms",
    "series",
    "features",
    "criticScore",
    "cover",
    "hero",
];

const SOURCE_IDS: [&str; 6] = ["igdb", "steam", "store", "playnite", "lutris", "manual"];

/// `DEFAULT_FIELD_PRIORITY`.
pub fn default_field_priority(field: &str) -> Vec<String> {
    let list: &[&str] = match field {
        "description" | "releaseDate" => &["igdb", "steam", "store", "lutris", "playnite"],
        "developers" | "genres" => &["igdb", "steam", "store", "playnite", "lutris"],
        "publishers" => &["igdb", "steam", "store", "playnite"],
        "themes" | "series" => &["igdb", "playnite"],
        "gameModes" => &["igdb"],
        "platforms" => &["igdb", "store", "playnite", "lutris"],
        "features" => &["igdb", "steam", "playnite"],
        "criticScore" => &["igdb", "steam"],
        "cover" => &["igdb", "steam", "lutris", "playnite", "store"],
        "hero" => &["igdb", "steam", "lutris", "playnite"],
        _ => &[],
    };
    list.iter().map(|s| s.to_string()).collect()
}

/// Cover aspect preset of the card grid (`data-cover-aspect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverAspect {
    #[default]
    Steam,
    Igdb,
    Gog,
    Square,
    Dvd,
    Banner,
}

impl CoverAspect {
    /// `asCoverAspect`: unknown values mean `steam`.
    pub fn parse(value: &str) -> Self {
        match value {
            "igdb" => Self::Igdb,
            "gog" => Self::Gog,
            "square" => Self::Square,
            "dvd" => Self::Dvd,
            "banner" => Self::Banner,
            _ => Self::Steam,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Igdb => "igdb",
            Self::Gog => "gog",
            Self::Square => "square",
            Self::Dvd => "dvd",
            Self::Banner => "banner",
        }
    }
}

/// Normalised `settings.json` plus the Heroic values the screen reads.
#[derive(Debug, Clone)]
pub struct EffectiveSettings {
    pub grey_uninstalled_games: bool,
    pub background: BackgroundSettings,
    pub screenshots: ScreenshotsSettings,
    pub metadata: MetadataSettings,
    /// Parsed `metadata.coverAspect`.
    pub cover_aspect: CoverAspect,
    pub backup: BackupSettings,
    pub ludusavi: LudusaviSettings,
    /// Heroic `store/config.json` `theme` (default `midnightMirage`).
    pub theme: String,
    /// The interface language (`heronite_i18n`), which also picks the Steam
    /// texts and date formats (Heroic's `store/config.json` `language` is not
    /// used: this app has its own setting).
    pub language: String,
    /// Heroic `store/config.json` `allTilesInColor` (default false).
    pub all_tiles_in_color: bool,
}

impl Default for EffectiveSettings {
    fn default() -> Self {
        Self::from_files(&CollectionSettingsFile::default(), &StoreConfigFile::default())
    }
}

impl EffectiveSettings {
    pub fn from_files(file: &CollectionSettingsFile, config: &StoreConfigFile) -> Self {
        let backup = file.backup.clone().unwrap_or_else(fallback_backup);
        let interval = match backup.interval.as_str() {
            "daily" | "weekly" | "monthly" => backup.interval.clone(),
            _ => "weekly".to_owned(),
        };
        let ludusavi = file.ludusavi.clone().unwrap_or_else(fallback_ludusavi);
        let screenshots = file.screenshots.clone().unwrap_or_default();
        let metadata = normalize_metadata(file.metadata.as_ref());
        Self {
            grey_uninstalled_games: file.grey_uninstalled_games != Some(false),
            background: normalize_background(file.background.as_ref().map(PartialBackground::from).as_ref()),
            screenshots: ScreenshotsSettings {
                folder: screenshots.folder.trim().to_owned(),
                cache_limit_mb: JsNum(as_screenshot_cache_limit(Some(screenshots.cache_limit_mb.get()))),
                capture_enabled: screenshots.capture_enabled,
                capture_accelerator: non_empty_trimmed(&screenshots.capture_accelerator).unwrap_or(DEFAULT_CAPTURE_ACCELERATOR).to_owned(),
                extra: Default::default(),
            },
            cover_aspect: CoverAspect::parse(&metadata.cover_aspect),
            metadata,
            backup: BackupSettings {
                folder: backup.folder,
                interval,
                last_backup_at: backup.last_backup_at,
                last_backup_path: backup.last_backup_path,
                last_error: backup.last_error,
                extra: Default::default(),
            },
            ludusavi: LudusaviSettings {
                enabled: ludusavi.enabled,
                use_installed_config: ludusavi.use_installed_config,
                binary_path: ludusavi.binary_path,
                backup_path: ludusavi.backup_path,
                format: if ludusavi.format == "simple" { "simple".into() } else { "zip".into() },
                compression: match ludusavi.compression.as_str() {
                    c @ ("none" | "bzip2" | "zstd") => c.to_owned(),
                    _ => "deflate".to_owned(),
                },
                restore_paths: ludusavi.restore_paths.into_iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect(),
                full_limit: ludusavi.full_limit.filter(|n| *n >= 1),
                differential_limit: ludusavi.differential_limit,
                last_backup_at: ludusavi.last_backup_at,
                last_backup_game: ludusavi.last_backup_game,
                last_backup_path: ludusavi.last_backup_path,
                last_error: ludusavi.last_error,
                extra: Default::default(),
            },
            theme: config.theme.clone().unwrap_or_else(|| DEFAULT_THEME.to_owned()),
            language: heronite_i18n::current().code().to_owned(),
            all_tiles_in_color: config.all_tiles_in_color.unwrap_or(false),
        }
    }

    /// Folder where screenshots live (`localart://screenshots/...`), if set.
    pub fn screenshots_folder(&self) -> Option<&std::path::Path> {
        let f = self.screenshots.folder.as_str();
        (!f.is_empty()).then(|| std::path::Path::new(f))
    }
}

fn non_empty_trimmed(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

fn fallback_backup() -> BackupSettings {
    BackupSettings { folder: String::new(), interval: "weekly".into(), ..Default::default() }
}

fn fallback_ludusavi() -> LudusaviSettings {
    LudusaviSettings {
        enabled: false,
        use_installed_config: true,
        binary_path: String::new(),
        backup_path: String::new(),
        format: "zip".into(),
        compression: "deflate".into(),
        ..Default::default()
    }
}

/// Reads `settings.json` + the store config and applies defaults.
pub fn load(paths: &DataPaths) -> Result<EffectiveSettings> {
    let file: CollectionSettingsFile = json_store::read(&paths.collection_settings())?;
    let config: StoreConfigFile = json_store::read(&paths.store_config()).unwrap_or_default();
    Ok(EffectiveSettings::from_files(&file, &config))
}

// Normalisation (same clamps and defaults as the TS)

/// `Partial<CollectionBackgroundSettings>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartialBackground {
    pub enabled: Option<bool>,
    pub blur: Option<f64>,
    pub dimming: Option<f64>,
    pub saturation: Option<f64>,
    pub scrim: Option<f64>,
}

impl From<&BackgroundSettings> for PartialBackground {
    fn from(b: &BackgroundSettings) -> Self {
        Self {
            enabled: Some(b.enabled),
            blur: Some(b.blur.get()),
            dimming: Some(b.dimming.get()),
            saturation: Some(b.saturation.get()),
            scrim: Some(b.scrim.get()),
        }
    }
}

/// `normalizeBackground`: rounds and clamps (blur 0-40, dimming 0-90,
/// saturation 0-150, scrim 0-150); a missing value takes the default.
pub fn normalize_background(value: Option<&PartialBackground>) -> BackgroundSettings {
    let defaults = BackgroundSettings::default();
    let clamp = |raw: Option<f64>, min: f64, max: f64, default: JsNum| -> JsNum {
        match raw {
            Some(v) if v.is_finite() => JsNum(js_round(v).clamp(min, max)),
            _ => default,
        }
    };
    BackgroundSettings {
        enabled: value.and_then(|v| v.enabled) != Some(false),
        blur: clamp(value.and_then(|v| v.blur), 0.0, 40.0, defaults.blur),
        dimming: clamp(value.and_then(|v| v.dimming), 0.0, 90.0, defaults.dimming),
        saturation: clamp(value.and_then(|v| v.saturation), 0.0, 150.0, defaults.saturation),
        scrim: clamp(value.and_then(|v| v.scrim), 0.0, 150.0, defaults.scrim),
        extra: Default::default(),
    }
}

fn as_screenshot_cache_limit(value: Option<f64>) -> f64 {
    match value {
        Some(v) if v.is_finite() => js_round(v).clamp(MIN_SCREENSHOT_CACHE_LIMIT_MB, MAX_SCREENSHOT_CACHE_LIMIT_MB),
        _ => DEFAULT_SCREENSHOT_CACHE_LIMIT_MB,
    }
}

fn as_source_list(value: Option<&Vec<String>>, fallback: Vec<String>) -> Vec<String> {
    let Some(list) = value else { return fallback };
    let next: Vec<String> = list.iter().filter(|s| SOURCE_IDS.contains(&s.as_str())).cloned().collect();
    if next.is_empty() {
        fallback
    } else {
        next
    }
}

/// `normalizeMetadata`.
pub fn normalize_metadata(value: Option<&MetadataSettings>) -> MetadataSettings {
    let mut field_priority: IndexMap<String, Vec<String>> = IndexMap::new();
    for field in METADATA_FIELDS {
        let current = value.and_then(|v| v.field_priority.get(field));
        field_priority.insert(field.to_owned(), as_source_list(current, default_field_priority(field)));
    }
    MetadataSettings {
        igdb_client_id: value.map(|v| v.igdb_client_id.trim().to_owned()).unwrap_or_default(),
        igdb_client_secret: value.map(|v| v.igdb_client_secret.clone()).unwrap_or_default(),
        cover_aspect: CoverAspect::parse(value.map(|v| v.cover_aspect.as_str()).unwrap_or("")).as_str().to_owned(),
        download_images: value.is_some_and(|v| v.download_images),
        auto_fill_missing: value.map(|v| v.auto_fill_missing).unwrap_or(true),
        field_priority,
        extra: Default::default(),
    }
}

// Setters (read-modify-write of local_library/settings.json)

/// `setCollectionUiSettings`.
pub fn save_ui(
    paths: &DataPaths,
    grey_uninstalled_games: Option<bool>,
    background: Option<&PartialBackground>,
) -> Result<EffectiveSettings> {
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        if let Some(grey) = grey_uninstalled_games {
            file.grey_uninstalled_games = Some(grey);
        }
        if let Some(patch) = background {
            let current = normalize_background(file.background.as_ref().map(PartialBackground::from).as_ref());
            let merged = PartialBackground {
                enabled: patch.enabled.or(Some(current.enabled)),
                blur: patch.blur.or(Some(current.blur.get())),
                dimming: patch.dimming.or(Some(current.dimming.get())),
                saturation: patch.saturation.or(Some(current.saturation.get())),
                scrim: patch.scrim.or(Some(current.scrim.get())),
            };
            file.background = Some(normalize_background(Some(&merged)));
        }
        Ok(())
    })?;
    load(paths)
}

/// `setCollectionScreenshotsSettings`.
pub fn save_screenshots(
    paths: &DataPaths,
    folder: &str,
    cache_limit_mb: f64,
    capture_enabled: bool,
    capture_accelerator: &str,
) -> Result<EffectiveSettings> {
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        file.screenshots = Some(ScreenshotsSettings {
            folder: folder.trim().to_owned(),
            cache_limit_mb: JsNum(as_screenshot_cache_limit(Some(cache_limit_mb))),
            capture_enabled,
            capture_accelerator: non_empty_trimmed(capture_accelerator).unwrap_or(DEFAULT_CAPTURE_ACCELERATOR).to_owned(),
            extra: Default::default(),
        });
        Ok(())
    })?;
    load(paths)
}

/// `setCollectionBackupSettings`.
pub fn save_backup(paths: &DataPaths, folder: &str, interval: &str) -> Result<EffectiveSettings> {
    let current = load(paths)?.backup;
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        file.backup = Some(BackupSettings { folder: folder.trim().to_owned(), interval: interval.to_owned(), ..current });
        Ok(())
    })?;
    load(paths)
}

/// `setCollectionMetadataSettings`: `patch` replaces the metadata section after
/// normalisation; `field_priority` entries are merged over the current ones.
pub fn save_metadata(paths: &DataPaths, patch: &MetadataSettings) -> Result<EffectiveSettings> {
    let current = load(paths)?.metadata;
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        let mut merged = patch.clone();
        let mut priority = current.field_priority.clone();
        for (k, v) in &patch.field_priority {
            priority.insert(k.clone(), v.clone());
        }
        merged.field_priority = priority;
        file.metadata = Some(normalize_metadata(Some(&merged)));
        Ok(())
    })?;
    load(paths)
}

/// Saves just the grey-uninstalled flag (the toggle in the settings dialog).
pub fn save_grey_uninstalled(paths: &DataPaths, grey: bool) -> Result<EffectiveSettings> {
    save_ui(paths, Some(grey), None)
}

/// Heroic's `allTilesInColor` toggle (`store/config.json`), written like
/// `configStore.set('allTilesInColor', value)`.
pub fn save_all_tiles_in_color(paths: &DataPaths, value: bool) -> Result<()> {
    json_store::update_value(&paths.store_config(), |doc| {
        doc["allTilesInColor"] = json!(value);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    #[test]
    fn defaults_match_the_ts() {
        let s = EffectiveSettings::default();
        assert!(s.grey_uninstalled_games);
        assert_eq!(s.background, BackgroundSettings::default());
        assert_eq!(s.screenshots.cache_limit_mb.get(), 500.0);
        assert_eq!(s.screenshots.capture_accelerator, "F10");
        assert!(!s.screenshots.capture_enabled);
        assert_eq!(s.cover_aspect, CoverAspect::Steam);
        assert!(s.metadata.auto_fill_missing);
        assert!(!s.metadata.download_images);
        assert_eq!(s.metadata.field_priority["cover"], ["igdb", "steam", "lutris", "playnite", "store"]);
        assert_eq!(s.backup.interval, "weekly");
        assert_eq!(s.ludusavi.format, "zip");
        assert_eq!(s.ludusavi.compression, "deflate");
        assert!(s.ludusavi.use_installed_config);
        assert_eq!(s.theme, "midnightMirage");
        assert_eq!(s.language, "en");
        assert!(!s.all_tiles_in_color);
    }

    #[test]
    fn background_is_clamped_and_rounded() {
        let b = normalize_background(Some(&PartialBackground {
            enabled: Some(false),
            blur: Some(99.4),
            dimming: Some(-3.0),
            saturation: Some(10.5),
            scrim: None,
        }));
        assert!(!b.enabled);
        assert_eq!(b.blur.get(), 40.0);
        assert_eq!(b.dimming.get(), 0.0);
        assert_eq!(b.saturation.get(), 11.0);
        assert_eq!(b.scrim.get(), 100.0);
    }

    #[test]
    fn save_ui_touches_only_its_keys() {
        let (_d, paths) = temp();
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(
            paths.collection_settings(),
            "{\n\t\"zzz\": {\"keep\": 1},\n\t\"greyUninstalledGames\": true,\n\t\"background\": {\"enabled\": true, \"blur\": 4, \"dimming\": 10, \"saturation\": 96, \"scrim\": 100}\n}",
        )
        .unwrap();
        let s = save_ui(&paths, Some(false), Some(&PartialBackground { blur: Some(8.0), ..Default::default() })).unwrap();
        assert!(!s.grey_uninstalled_games);
        assert_eq!(s.background.blur.get(), 8.0);
        assert_eq!(s.background.dimming.get(), 10.0);
        let v = json_store::read_value(&paths.collection_settings()).unwrap();
        assert_eq!(v["zzz"]["keep"], 1);
        assert_eq!(v["background"]["blur"], 8);
    }

    #[test]
    fn screenshots_and_backup_round_trip() {
        let (_d, paths) = temp();
        let s = save_screenshots(&paths, " /shots ", 5.0, true, "  ").unwrap();
        assert_eq!(s.screenshots.folder, "/shots");
        assert_eq!(s.screenshots.cache_limit_mb.get(), 50.0);
        assert_eq!(s.screenshots.capture_accelerator, "F10");
        assert_eq!(s.screenshots_folder(), Some(std::path::Path::new("/shots")));
        let s = save_backup(&paths, "/b", "monthly").unwrap();
        assert_eq!(s.backup.interval, "monthly");
        let s = save_backup(&paths, "/b", "yearly").unwrap();
        // An invalid interval is stored but read back as weekly.
        assert_eq!(s.backup.interval, "weekly");
        let mut md = MetadataSettings { igdb_client_id: " id ".into(), cover_aspect: "dvd".into(), ..Default::default() };
        md.field_priority.insert("cover".into(), vec!["steam".into(), "bogus".into()]);
        let s = save_metadata(&paths, &md).unwrap();
        assert_eq!(s.metadata.igdb_client_id, "id");
        assert_eq!(s.cover_aspect, CoverAspect::Dvd);
        assert_eq!(s.metadata.field_priority["cover"], ["steam"]);
        assert_eq!(s.metadata.field_priority["hero"], ["igdb", "steam", "lutris", "playnite"]);
    }

    #[test]
    fn reads_real_settings() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE") else { return };
        let s = load(&DataPaths::at(root)).unwrap();
        eprintln!(
            "theme={} language={} grey={} bg={:?} shots={:?}",
            s.theme, s.language, s.grey_uninstalled_games, s.background, s.screenshots.folder
        );
        assert!(!s.grey_uninstalled_games);
        assert_eq!(s.background.blur.get(), 4.0);
        assert_eq!(s.theme, "nord-dark");
    }
}
