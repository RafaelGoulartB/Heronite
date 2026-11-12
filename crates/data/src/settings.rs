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
    BackgroundSettings, BackupSettings, CollectionSettingsFile, LudusaviSettings, MetadataSettings, ScreenshotsSettings, SourcePrefs,
    StoreConfigFile,
};
use crate::num::{js_round, JsNum};
use crate::paths::DataPaths;

pub const DEFAULT_SCREENSHOT_CACHE_LIMIT_MB: f64 = 500.0;
const MIN_SCREENSHOT_CACHE_LIMIT_MB: f64 = 50.0;
const MAX_SCREENSHOT_CACHE_LIMIT_MB: f64 = 10_000.0;
pub const DEFAULT_THEME: &str = "midnightMirage";
pub const DEFAULT_CAPTURE_ACCELERATOR: &str = "F10";

pub const METADATA_FIELDS: [&str; 14] = [
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
    "ageRatings",
    "cover",
    "hero",
];

/// The fields of the cover families beside the tall one (`cover`), see
/// [`crate::cover_family`]. They are merged like [`METADATA_FIELDS`], but
/// only for the families an installation keeps; the screens that list
/// `METADATA_FIELDS` leave them out.
pub const FAMILY_COVER_FIELDS: [&str; 2] = ["coverSquare", "coverWide"];

/// Every field the merge knows: [`METADATA_FIELDS`], then [`FAMILY_COVER_FIELDS`].
pub const ALL_FIELDS: [&str; 16] = [
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
    "ageRatings",
    "cover",
    "hero",
    "coverSquare",
    "coverWide",
];

/// `MetadataSourceId`: the built-in ids of a `fieldPriority`. Every one but
/// `manual` is a details source of `heronite_metadata::sources` (its tests
/// check the two agree). A saved list keeps other ids too.
pub const SOURCE_IDS: [&str; 7] = ["igdb", "steam", "store", "playnite", "lutris", "launchbox", "manual"];

/// Sources added after `fieldPriority` lists were already saved in
/// every settings file (the lists are written whole), to some fields or
/// later to more fields (LaunchBox to the wide cover after libretro). A
/// saved list that is the default without some of them still reads as the
/// default, and a list the user set that lacks one gets it at the end
/// where the default names it (see `heronite_metadata::sources::order`),
/// so a new source fills those fields without a migration of the file.
pub const ADDED_SOURCES: [&str; 2] = ["launchbox", "libretro"];

/// Ids of extensions that the default lists of some saved files name, as
/// the builds that wrote them had the source built in (VNDB): last of the
/// sources of their time, followed only by [`ADDED_SOURCES`]. A saved list that is a default but for one of
/// them, at that place, still reads as the default; the extension of that
/// id, once installed, joins its end like any extension.
pub const DEFAULT_LISTED_EXTENSIONS: [&str; 1] = ["vndb"];

/// `DEFAULT_FIELD_PRIORITY`. Uses only [`SOURCE_IDS`], but for the square
/// and wide covers, which SteamGridDB gives too, and for the covers
/// libretro-thumbnails gives (a pictures source, like SteamGridDB: not in
/// [`SOURCE_IDS`]). The LaunchBox Games Database and libretro-thumbnails
/// come last: they fill what the others left empty (and move ahead for the
/// emulated games they find surely, see `heronite_metadata::sources::order`).
pub fn default_field_priority(field: &str) -> Vec<String> {
    let list: &[&str] = match field {
        "description" | "releaseDate" => &["igdb", "steam", "store", "lutris", "playnite", "launchbox"],
        "developers" | "genres" => &["igdb", "steam", "store", "playnite", "lutris", "launchbox"],
        "publishers" => &["igdb", "steam", "store", "playnite", "launchbox"],
        "themes" => &["igdb", "playnite"],
        "series" => &["igdb", "playnite"],
        "gameModes" => &["igdb", "launchbox"],
        "platforms" => &["igdb", "store", "playnite", "lutris", "launchbox"],
        "features" => &["igdb", "steam", "playnite"],
        "criticScore" => &["igdb", "steam", "launchbox"],
        // Both are read: the first fills the boards it has, the next the rest.
        "ageRatings" => &["igdb", "steam"],
        "cover" => &["igdb", "steam", "lutris", "playnite", "store", "launchbox", "libretro"],
        "hero" => &["igdb", "steam", "lutris", "playnite", "launchbox"],
        "coverSquare" => &["steamgriddb", "launchbox", "libretro"],
        // Official art first (Steam's header, the store's wide box art,
        // LaunchBox's landscape boxes), then SteamGridDB's.
        "coverWide" => &["steam", "store", "launchbox", "steamgriddb", "libretro"],
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
    /// `keep` is always `Some` here.
    pub backup: BackupSettings,
    pub ludusavi: LudusaviSettings,
    /// MB, 0 = no cap ([`DEFAULT_IMAGE_CACHE_LIMIT_MB`] when unset).
    pub image_cache_limit_mb: u32,
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
            "daily" | "weekly" | "monthly" | "never" => backup.interval.clone(),
            _ => "weekly".to_owned(),
        };
        let ludusavi = file.ludusavi.clone().unwrap_or_else(fallback_ludusavi);
        let screenshots = file.screenshots.clone().unwrap_or_default();
        let metadata = normalize_metadata(file.metadata.as_ref());
        Self {
            grey_uninstalled_games: file.grey_uninstalled_games == Some(true),
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
                keep: Some(backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP)),
                exclude: backup.exclude.into_iter().filter(|p| BACKUP_OPTIONAL_PARTS.contains(&p.as_str())).collect(),
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
                auto_backup_off: ludusavi.auto_backup_off,
                interval_minutes: as_ludusavi_interval(ludusavi.interval_minutes),
                last_backup_at: ludusavi.last_backup_at,
                last_backup_game: ludusavi.last_backup_game,
                last_backup_path: ludusavi.last_backup_path,
                last_error: ludusavi.last_error,
                extra: Default::default(),
            },
            image_cache_limit_mb: file.image_cache_limit_mb.unwrap_or(DEFAULT_IMAGE_CACHE_LIMIT_MB),
            theme: config.theme.clone().unwrap_or_else(|| DEFAULT_THEME.to_owned()),
            language: heronite_i18n::current().code().to_owned(),
            all_tiles_in_color: config.all_tiles_in_color.unwrap_or(false),
        }
    }

    /// [`Self::from_files`] for the data folder `paths`: an empty screenshots
    /// folder means [`DataPaths::default_screenshots`], so captures work
    /// before the user picks one (and a portable copy keeps finding its own
    /// after it moves).
    pub fn for_paths(paths: &DataPaths, file: &CollectionSettingsFile, config: &StoreConfigFile) -> Self {
        let mut s = Self::from_files(file, config);
        if s.screenshots.folder.is_empty() {
            if let Some(folder) = paths.default_screenshots() {
                s.screenshots.folder = folder.to_string_lossy().into_owned();
            }
        }
        s
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

/// Backups the folder keeps when `keep` is unset.
pub const DEFAULT_BACKUP_KEEP: u32 = 10;
/// Parts of a backup that can be left out (the Collection is always in).
pub const BACKUP_OPTIONAL_PARTS: [&str; 3] = ["settings", "art", "logins"];
/// Cap of the image caches when unset: about 4x what a library of a few
/// hundred games uses, so it only trims a cache that grew without bound.
pub const DEFAULT_IMAGE_CACHE_LIMIT_MB: u32 = 1024;

/// Shortest and longest time between save backups while a game runs, in
/// minutes: a hand-edited value is clamped into this range.
pub const LUDUSAVI_INTERVAL_RANGE: std::ops::RangeInclusive<u32> = 5..=24 * 60;

/// `ludusavi.intervalMinutes`: 0 or unset = off.
pub fn as_ludusavi_interval(minutes: Option<u32>) -> Option<u32> {
    minutes.filter(|m| *m > 0).map(|m| m.clamp(*LUDUSAVI_INTERVAL_RANGE.start(), *LUDUSAVI_INTERVAL_RANGE.end()))
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
    Ok(EffectiveSettings::for_paths(paths, &file, &config))
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

/// A saved list of source ids. An id this build does not know (an
/// extension's, or one a newer build added) is kept: it never answers here,
/// and the list stays the same for the build that knows it.
fn as_source_list(value: Option<&Vec<String>>, fallback: Vec<String>) -> Vec<String> {
    let Some(list) = value else { return fallback };
    let next = clean_ids(list);
    if next.is_empty() {
        fallback
    } else {
        next
    }
}

/// `ids` without blanks and repeats, in order.
fn clean_ids(ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == id) {
            out.push(id.to_owned());
        }
    }
    out
}

fn https_only(urls: Vec<String>) -> Vec<String> {
    urls.into_iter().filter(|u| u.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))).collect()
}

fn normalize_source_prefs(value: Option<&IndexMap<String, SourcePrefs>>) -> IndexMap<String, SourcePrefs> {
    value
        .into_iter()
        .flatten()
        .filter(|(id, _)| !id.trim().is_empty())
        .map(|(id, p)| (id.clone(), SourcePrefs { fields_off: clean_ids(&p.fields_off), ..p.clone() }))
        .filter(|(_, p)| !p.is_empty())
        .collect()
}

/// `normalizeMetadata`.
pub fn normalize_metadata(value: Option<&MetadataSettings>) -> MetadataSettings {
    let mut field_priority: IndexMap<String, Vec<String>> = IndexMap::new();
    for field in ALL_FIELDS {
        let current = value.and_then(|v| v.field_priority.get(field));
        field_priority.insert(field.to_owned(), as_source_list(current, default_field_priority(field)));
    }
    MetadataSettings {
        igdb_client_id: value.map(|v| v.igdb_client_id.trim().to_owned()).unwrap_or_default(),
        igdb_client_secret: value.map(|v| v.igdb_client_secret.clone()).unwrap_or_default(),
        cover_aspect: CoverAspect::parse(value.map(|v| v.cover_aspect.as_str()).unwrap_or("")).as_str().to_owned(),
        download_images: value.is_some_and(|v| v.download_images),
        allow_adult_covers: value.is_some_and(|v| v.allow_adult_covers),
        auto_fill_missing: value.map(|v| v.auto_fill_missing).unwrap_or(true),
        show_age_ratings: value.map(|v| v.show_age_ratings).unwrap_or(true),
        steam_tags: value.map(|v| v.steam_tags).unwrap_or(true),
        steam_tag_limit: crate::steam_tags::normalize_limit(value.map(|v| v.steam_tag_limit).unwrap_or(0)),
        how_long_to_beat: value.map(|v| v.how_long_to_beat).unwrap_or(true),
        age_rating_system: value
            .map(|v| v.age_rating_system.as_str())
            .filter(|s| crate::age_rating::RatingSystem::parse(s).is_some())
            .unwrap_or(crate::age_rating::AUTO)
            .to_owned(),
        field_priority,
        sources: normalize_source_prefs(value.map(|v| &v.sources)),
        source_order: value.map(|v| clean_ids(&v.source_order)).unwrap_or_default(),
        keep_cover_families: value.map(|v| clean_ids(&v.keep_cover_families)).unwrap_or_default(),
        fill_cover_families: value.is_some_and(|v| v.fill_cover_families),
        // Addresses only: a list the user adds is https; a path here (a
        // hand-edited or synced file) would be read as a list on this computer.
        extension_lists: value.map(|v| https_only(clean_ids(&v.extension_lists))).unwrap_or_default(),
        // Keys a newer build added stay for it.
        extra: value.map(|v| v.extra.clone()).unwrap_or_default(),
    }
}

/// Read-modify-write of `settings.metadata` through `edit`, normalized
/// before and after; the rest of the file is untouched.
pub fn update_metadata(paths: &DataPaths, edit: impl FnOnce(&mut MetadataSettings)) -> Result<EffectiveSettings> {
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        let mut m = normalize_metadata(file.metadata.as_ref());
        edit(&mut m);
        file.metadata = Some(normalize_metadata(Some(&m)));
        Ok(())
    })?;
    load(paths)
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

/// `setCollectionScreenshotsSettings`. The default folder is stored as
/// empty, so it keeps following [`DataPaths::default_screenshots`].
pub fn save_screenshots(
    paths: &DataPaths,
    folder: &str,
    cache_limit_mb: f64,
    capture_enabled: bool,
    capture_accelerator: &str,
) -> Result<EffectiveSettings> {
    let folder = folder.trim();
    let is_default = paths.default_screenshots().is_some_and(|d| d.as_os_str() == folder);
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        file.screenshots = Some(ScreenshotsSettings {
            folder: if is_default { String::new() } else { folder.to_owned() },
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

/// Changes to the backup settings; `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct BackupPatch {
    pub folder: Option<String>,
    pub interval: Option<String>,
    pub keep: Option<u32>,
    pub exclude: Option<Vec<String>>,
}

pub fn save_backup_settings(paths: &DataPaths, patch: BackupPatch) -> Result<EffectiveSettings> {
    let current = load(paths)?.backup;
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        let mut next = current;
        if let Some(folder) = patch.folder {
            next.folder = folder.trim().to_owned();
        }
        if let Some(interval) = patch.interval {
            next.interval = interval;
        }
        if let Some(keep) = patch.keep {
            next.keep = Some(keep);
        }
        if let Some(exclude) = patch.exclude {
            next.exclude = exclude;
        }
        file.backup = Some(next);
        Ok(())
    })?;
    load(paths)
}

pub fn save_image_cache_limit(paths: &DataPaths, limit_mb: u32) -> Result<EffectiveSettings> {
    json_store::update::<CollectionSettingsFile, _>(&paths.collection_settings(), |file| {
        file.image_cache_limit_mb = Some(limit_mb);
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

/// Keeps the cover `family` (`square` or `wide`) even while no view uses it,
/// or stops keeping it. The tall one is always kept.
pub fn set_cover_family_kept(paths: &DataPaths, family: crate::cover_family::CoverFamily, keep: bool) -> Result<EffectiveSettings> {
    update_metadata(paths, |m| {
        m.keep_cover_families.retain(|id| id != family.id());
        if keep && family != crate::cover_family::CoverFamily::Tall {
            m.keep_cover_families.push(family.id().to_owned());
        }
    })
}

/// Whether a family a view starts using is fetched for the games lacking it.
pub fn set_fill_cover_families(paths: &DataPaths, on: bool) -> Result<EffectiveSettings> {
    update_metadata(paths, |m| m.fill_cover_families = on)
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
        // Colour covers until the user asks for grey ones.
        assert!(!s.grey_uninstalled_games);
        assert_eq!(s.background, BackgroundSettings::default());
        assert_eq!(s.screenshots.cache_limit_mb.get(), 500.0);
        assert_eq!(s.screenshots.capture_accelerator, "F10");
        assert!(!s.screenshots.capture_enabled);
        assert_eq!(s.cover_aspect, CoverAspect::Steam);
        assert!(s.metadata.auto_fill_missing);
        assert!(!s.metadata.download_images);
        assert_eq!(s.metadata.field_priority["cover"], ["igdb", "steam", "lutris", "playnite", "store", "launchbox", "libretro"]);
        assert!(!s.metadata.allow_adult_covers);
        assert_eq!(s.metadata.field_priority["coverSquare"], ["steamgriddb", "launchbox", "libretro"]);
        assert!(s.metadata.keep_cover_families.is_empty() && !s.metadata.fill_cover_families);
        assert_eq!(s.backup.interval, "weekly");
        assert_eq!(s.ludusavi.format, "zip");
        assert_eq!(s.ludusavi.compression, "deflate");
        assert!(s.ludusavi.use_installed_config);
        assert_eq!(s.ludusavi.interval_minutes, None, "no backups during play unless asked for");
        assert_eq!(s.theme, "midnightMirage");
        assert_eq!(s.language, "en");
        assert!(!s.all_tiles_in_color);
    }

    #[test]
    fn only_https_addresses_are_kept_as_lists_of_extensions() {
        let m = MetadataSettings {
            extension_lists: [
                "https://a.example.org/x.json",
                "/home/me/index.json",
                "file:///tmp/index.json",
                "http://b.example.org",
                " HTTPS://C.example.org/y.json ",
            ]
            .map(String::from)
            .to_vec(),
            ..Default::default()
        };
        assert_eq!(normalize_metadata(Some(&m)).extension_lists, ["https://a.example.org/x.json", "HTTPS://C.example.org/y.json"]);
    }

    #[test]
    fn ludusavi_interval_is_off_unless_set_and_clamped() {
        let read = |json: &str| {
            let file: CollectionSettingsFile = serde_json::from_str(json).unwrap();
            EffectiveSettings::from_files(&file, &StoreConfigFile::default()).ludusavi
        };
        assert_eq!(read(r#"{"ludusavi":{"enabled":true}}"#).interval_minutes, None);
        assert_eq!(read(r#"{"ludusavi":{"enabled":true,"intervalMinutes":0}}"#).interval_minutes, None);
        assert_eq!(read(r#"{"ludusavi":{"enabled":true,"intervalMinutes":30}}"#).interval_minutes, Some(30));
        assert_eq!(read(r#"{"ludusavi":{"enabled":true,"intervalMinutes":1}}"#).interval_minutes, Some(5));
        assert_eq!(read(r#"{"ludusavi":{"enabled":true,"intervalMinutes":99999}}"#).interval_minutes, Some(1440));
        let l = read(r#"{"ludusavi":{"enabled":true,"intervalMinutes":30,"autoBackupOff":["sideload:g"]}}"#);
        assert_eq!(l.periodic_interval_for("sideload", "h"), Some(std::time::Duration::from_secs(30 * 60)));
        assert_eq!(l.periodic_interval_for("sideload", "g"), None, "a game left out of the after-play backup is left out too");
        assert_eq!(read(r#"{"ludusavi":{"enabled":false,"intervalMinutes":30}}"#).periodic_interval_for("sideload", "h"), None);
        assert_eq!(read(r#"{"ludusavi":{"enabled":true}}"#).periodic_interval_for("sideload", "h"), None);
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
        assert_eq!(b.scrim.get(), 95.0);
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
        assert_eq!(s.backup.keep, Some(DEFAULT_BACKUP_KEEP));
        let patch = BackupPatch {
            interval: Some("never".into()),
            keep: Some(0),
            exclude: Some(vec!["logins".into(), "bogus".into()]),
            ..Default::default()
        };
        let s = save_backup_settings(&paths, patch).unwrap();
        assert_eq!((s.backup.folder.as_str(), s.backup.interval.as_str(), s.backup.keep), ("/b", "never", Some(0)));
        assert_eq!(s.backup.exclude, ["logins"]);
        assert_eq!(s.image_cache_limit_mb, DEFAULT_IMAGE_CACHE_LIMIT_MB);
        assert_eq!(save_image_cache_limit(&paths, 0).unwrap().image_cache_limit_mb, 0);
        let mut md = MetadataSettings { igdb_client_id: " id ".into(), cover_aspect: "dvd".into(), ..Default::default() };
        md.field_priority.insert("cover".into(), vec!["steam".into(), "bogus".into()]);
        let s = save_metadata(&paths, &md).unwrap();
        assert_eq!(s.metadata.igdb_client_id, "id");
        assert_eq!(s.cover_aspect, CoverAspect::Dvd);
        // An id this build does not know is kept (it never answers).
        assert_eq!(s.metadata.field_priority["cover"], ["steam", "bogus"]);
        assert_eq!(s.metadata.field_priority["hero"], ["igdb", "steam", "lutris", "playnite", "launchbox"]);
    }

    #[test]
    fn cover_family_settings_are_additive() {
        let (_d, paths) = temp();
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), r#"{"metadata":{"coverAspect":"square","igdbClientId":"x"}}"#).unwrap();
        let s = load(&paths).unwrap();
        assert!(s.metadata.keep_cover_families.is_empty() && !s.metadata.fill_cover_families);
        let s = set_cover_family_kept(&paths, crate::cover_family::CoverFamily::Wide, true).unwrap();
        assert_eq!(s.metadata.keep_cover_families, ["wide"]);
        set_cover_family_kept(&paths, crate::cover_family::CoverFamily::Tall, true).unwrap();
        let s = set_fill_cover_families(&paths, true).unwrap();
        assert!(s.metadata.fill_cover_families);
        let v = json_store::read_value(&paths.collection_settings()).unwrap();
        assert_eq!(v["metadata"]["keepCoverFamilies"], json!(["wide"]));
        assert_eq!(v["metadata"]["fillCoverFamilies"], true);
        assert_eq!(v["metadata"]["fieldPriority"]["coverWide"], json!(["steam", "store", "launchbox", "steamgriddb", "libretro"]));
        let s = set_fill_cover_families(&paths, false).unwrap();
        assert!(!s.metadata.fill_cover_families);
        let v = json_store::read_value(&paths.collection_settings()).unwrap();
        assert!(v["metadata"].get("fillCoverFamilies").is_none(), "off is the default: not written");
        set_cover_family_kept(&paths, crate::cover_family::CoverFamily::Wide, false).unwrap();
        let v = json_store::read_value(&paths.collection_settings()).unwrap();
        assert!(v["metadata"].get("keepCoverFamilies").is_none());
    }

    #[test]
    fn a_copy_of_the_data_has_no_default_screenshots_folder() {
        let (_d, paths) = temp();
        assert_eq!(paths.default_screenshots(), None);
        assert_eq!(load(&paths).unwrap().screenshots_folder(), None);
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
