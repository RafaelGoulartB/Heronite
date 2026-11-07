//! Settings → Achievements options, in the app state (`achievements.json`).

use anyhow::Result;
use heronite_data::paths::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// A desktop notification when a refresh finds new unlocks (and, while
    /// a game runs, when its live check does).
    pub notify: bool,
    /// While a game runs, its game files are checked every few seconds and
    /// new unlocks show over the game (the overlay must be on).
    pub overlay_notices: bool,
    /// The live check also reads the Steam client's stats files (off by
    /// default: Steam's own overlay already announces them).
    pub live_steam: bool,
    /// The achievements of each day on the Timeline.
    pub timeline: bool,
    /// Secret achievements keep their name and description hidden until
    /// they are unlocked.
    pub hide_secrets: bool,
    /// The RetroAchievements login is written into the emulators that
    /// support it (`emulators`).
    pub emulators: bool,
    /// The emulators play in hardcore mode (no save states, rewind or
    /// cheats; hardcore points).
    pub hardcore: bool,
    /// Achievements that games keep in their own files (the open-source
    /// Steam API emulators' saves), for DRM-free and offline copies.
    pub game_files: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            notify: false,
            overlay_notices: true,
            live_steam: false,
            timeline: true,
            hide_secrets: true,
            emulators: true,
            hardcore: false,
            game_files: false,
        }
    }
}

pub fn read(app: &AppPaths) -> Prefs {
    heronite_data::json_store::read(&app.achievement_prefs()).unwrap_or_default()
}

pub fn write(app: &AppPaths, prefs: &Prefs) -> Result<()> {
    heronite_data::json_store::write(&app.achievement_prefs(), prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_take_the_defaults() {
        let p: Prefs = serde_json::from_str(r#"{"notify":true}"#).unwrap();
        assert_eq!(p, Prefs { notify: true, ..Prefs::default() });
    }

    #[test]
    fn live_notices_default_to_the_overlay_and_game_files_only() {
        let p = Prefs::default();
        assert!(p.overlay_notices && !p.live_steam && !p.notify);
    }

    #[test]
    fn live_options_survive_a_write_and_a_read() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let prefs = Prefs { overlay_notices: false, live_steam: true, notify: true, ..Prefs::default() };
        write(&app, &prefs).unwrap();
        assert_eq!(read(&app), prefs);
    }
}
