//! Locations of this app's data.
//!
//! The data folder keeps the layout and the JSON formats of Heroic's config
//! folder (this app grew out of a Heroic fork), but it is this app's own:
//! nothing here is shared with Heroic or Heronite. `migrate` copies an
//! existing Heroic folder in once.

use std::path::{Path, PathBuf};

/// Folder name of this app under the XDG base directories.
pub const APP_DIR: &str = "heronite";

#[derive(Debug, Clone)]
pub struct DataPaths {
    /// `~/.local/share/heronite` on Linux.
    pub root: PathBuf,
}

impl DataPaths {
    /// The data folder. `HERONITE_DATA_DIR` overrides it (used by tests and
    /// for pointing at a copy of real data).
    pub fn detect() -> Self {
        if let Some(dir) = std::env::var_os("HERONITE_DATA_DIR") {
            return Self { root: PathBuf::from(dir) };
        }
        Self { root: Self::default_root().unwrap_or_else(|| PathBuf::from(APP_DIR)) }
    }

    /// `$XDG_DATA_HOME/heronite`.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join(APP_DIR))
    }

    /// Whether this is the real data folder (not a fixture or a copy).
    pub fn is_default(&self) -> bool {
        Self::default_root().as_deref() == Some(self.root.as_path())
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn j(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    // Overlay (Collection) sidecar stores.
    pub fn local_library(&self) -> PathBuf {
        self.j("local_library")
    }
    pub fn local_meta(&self) -> PathBuf {
        self.j("local_library/library.json")
    }
    pub fn sessions(&self) -> PathBuf {
        self.j("local_library/sessions.json")
    }
    pub fn statuses(&self) -> PathBuf {
        self.j("local_library/statuses.json")
    }
    pub fn collection_settings(&self) -> PathBuf {
        self.j("local_library/settings.json")
    }
    pub fn metadata(&self) -> PathBuf {
        self.j("local_library/metadata.json")
    }
    pub fn art(&self) -> PathBuf {
        self.j("local_library/art.json")
    }
    pub fn art_dir(&self) -> PathBuf {
        self.j("local_library/art")
    }
    pub fn heroes_dir(&self) -> PathBuf {
        self.j("local_library/heroes")
    }
    pub fn metadata_art_dir(&self) -> PathBuf {
        self.j("local_library/metadata-art")
    }
    pub fn steam_details_dir(&self) -> PathBuf {
        self.j("local_library/steam-details")
    }
    pub fn emulators(&self) -> PathBuf {
        self.j("local_library/emulators.json")
    }
    pub fn rom_scanners(&self) -> PathBuf {
        self.j("local_library/rom-scanners.json")
    }
    pub fn screenshot_thumbnails_dir(&self) -> PathBuf {
        self.j("local_library/screenshot-thumbnails")
    }
    pub fn ratings(&self) -> PathBuf {
        self.j("local_library/ratings.json")
    }

    // Core stores (Heroic's layout and formats).
    pub fn sideload_library(&self) -> PathBuf {
        self.j("sideload_apps/library.json")
    }
    pub fn legendary_library(&self) -> PathBuf {
        self.j("store_cache/legendary_library.json")
    }
    pub fn gog_library(&self) -> PathBuf {
        self.j("store_cache/gog_library.json")
    }
    pub fn nile_library(&self) -> PathBuf {
        self.j("store_cache/nile_library.json")
    }
    pub fn timestamps(&self) -> PathBuf {
        self.j("store/timestamp.json")
    }
    /// The electron-store style config (`settings`, `games`, `userInfo`...).
    pub fn store_config(&self) -> PathBuf {
        self.j("store/config.json")
    }
    /// The global settings (`defaultSettings`, 2-space JSON).
    pub fn global_config(&self) -> PathBuf {
        self.j("config.json")
    }
    pub fn download_manager(&self) -> PathBuf {
        self.j("store/download-manager.json")
    }
    pub fn wine_downloader_info(&self) -> PathBuf {
        self.j("store/wine-downloader-info.json")
    }
    /// When each Wine Manager source was last checked and GitHub's API budget.
    pub fn wine_release_checks(&self) -> PathBuf {
        self.j("store/wine-release-checks.json")
    }
    /// `store_cache/<name>.json`.
    pub fn store_cache(&self, name: &str) -> PathBuf {
        self.j("store_cache").join(format!("{name}.json"))
    }
    /// `gog_store/<name>.json`.
    pub fn gog_store(&self, name: &str) -> PathBuf {
        self.j("gog_store").join(format!("{name}.json"))
    }
    pub fn games_config_dir(&self) -> PathBuf {
        self.j("GamesConfig")
    }
    pub fn game_config(&self, app_name: &str) -> PathBuf {
        self.j("GamesConfig").join(format!("{app_name}.json"))
    }
    pub fn zoom_library(&self) -> PathBuf {
        self.j("store_cache/zoom_library.json")
    }
    pub fn gog_installed(&self) -> PathBuf {
        self.j("gog_store/installed.json")
    }
    pub fn zoom_installed(&self) -> PathBuf {
        self.j("zoom_store/installed.json")
    }
    pub fn game_overrides(&self) -> PathBuf {
        self.j("store/game-overrides.json")
    }
    /// `{appId}.{lang}.json` (Steam language name, e.g. `english`).
    pub fn steam_details(&self, steam_app_id: &str, steam_language: &str) -> PathBuf {
        self.j("local_library/steam-details").join(format!("{steam_app_id}.{steam_language}.json"))
    }
    /// Cached Steam hero: `local_library/heroes/{appId}.jpg`.
    pub fn steam_hero(&self, steam_app_id: &str) -> PathBuf {
        self.j("local_library/heroes").join(format!("{steam_app_id}.jpg"))
    }
    pub fn wiki_game_info(&self) -> PathBuf {
        self.j("store_cache/wikigameinfo.json")
    }
    pub fn images_cache_dir(&self) -> PathBuf {
        self.j("images-cache")
    }
    pub fn legendary_config_dir(&self) -> PathBuf {
        self.j("legendaryConfig/legendary")
    }
    pub fn tools_dir(&self) -> PathBuf {
        self.j("tools")
    }
    /// The legendary binary this app runs (copied in by `migrate`).
    pub fn legendary_bin(&self) -> PathBuf {
        self.j("tools/bin/legendary")
    }
    pub fn known_fixes(&self) -> PathBuf {
        self.j("known_fixes.json")
    }
    /// Icons of the `.desktop` shortcuts (`<appName>.(jpg|png)`).
    pub fn icons_dir(&self) -> PathBuf {
        self.j("icons")
    }
    /// Launcher, runner and game logs.
    pub fn logs_dir(&self) -> PathBuf {
        self.j("logs")
    }
    /// Copy of areweanticheatyet.com.
    pub fn anticheat(&self) -> PathBuf {
        self.j("areweanticheatyet.json")
    }

    /// Resolves a `localart://<kind>/<rest>` URL to a file, refusing traversal.
    /// `screenshots` URLs need the configured screenshot folder.
    pub fn resolve_localart(&self, url: &str, screenshots_folder: Option<&Path>) -> Option<PathBuf> {
        let rest = url.strip_prefix("localart://")?;
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let (kind, rel) = rest.split_once('/')?;
        let rel = percent_decode(rel);
        if rel.split('/').any(|seg| seg == "..") || rel.starts_with('/') {
            return None;
        }
        let base = match kind {
            "heroes" => self.heroes_dir(),
            "art" => self.art_dir(),
            "metadata-art" => self.metadata_art_dir(),
            "screenshots" => screenshots_folder?.to_path_buf(),
            _ => return None,
        };
        Some(base.join(rel))
    }
}

/// UI state and caches of this app, next to the data folder.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config: PathBuf,
    pub cache: PathBuf,
}

impl AppPaths {
    pub fn detect() -> Self {
        let config = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR);
        let cache = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR);
        Self { config, cache }
    }
    /// The app state that belongs to a data folder: the real one for the
    /// default folder, otherwise (fixtures, `HERONITE_DATA_DIR`, tests) a
    /// sibling `<folder>.heronite/` so state that points into a copy
    /// never leaks into the real config.
    pub fn for_data(data_paths: &DataPaths) -> Self {
        if data_paths.is_default() {
            return Self::detect();
        }
        let name = data_paths.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "data".into());
        let base = data_paths.root.with_file_name(format!("{name}.heronite"));
        Self { config: base.join("config"), cache: base.join("cache") }
    }
    pub fn ui_state(&self) -> PathBuf {
        self.config.join("ui-state.json")
    }
    /// Appearance settings specific to this app (see `heronite_data::appearance`).
    pub fn appearance(&self) -> PathBuf {
        self.config.join("appearance.json")
    }
    /// Screenshot options specific to this app (see `heronite_data::screenshot_prefs`).
    pub fn screenshot_prefs(&self) -> PathBuf {
        self.config.join("screenshots.json")
    }
    /// Update switches and the last update check (see `heronite_update::prefs`).
    pub fn update_state(&self) -> PathBuf {
        self.config.join("update.json")
    }
    /// Journal of an Epic install move in progress (see `heronite_epic::moves`).
    pub fn epic_move_journal(&self) -> PathBuf {
        self.config.join("epic-move.json")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.cache.join("thumbs")
    }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localart_resolves_and_blocks_traversal() {
        let p = DataPaths::at("/cfg");
        assert_eq!(
            p.resolve_localart("localart://art/sideload_x/hero-1.png", None).unwrap(),
            PathBuf::from("/cfg/local_library/art/sideload_x/hero-1.png")
        );
        assert_eq!(
            p.resolve_localart("localart://heroes/My%20Game.jpg?x=1", None).unwrap(),
            PathBuf::from("/cfg/local_library/heroes/My Game.jpg")
        );
        assert!(p.resolve_localart("localart://art/../../etc/passwd", None).is_none());
        assert!(p.resolve_localart("localart://screenshots/a.png", None).is_none());
    }
}
