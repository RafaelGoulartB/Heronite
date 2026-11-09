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
    /// What the notices over the game show besides the name.
    pub notice: NoticeFields,
}

/// The optional lines of an achievement's notice over the game; each one
/// shows only when the source gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoticeFields {
    /// The description, on one line.
    pub description: bool,
    /// The share of players that have it (and its tier's colour).
    pub rarity: bool,
    /// RetroAchievements points, Epic Games XP.
    pub points: bool,
    /// Steam's Achievement Set (base game or a DLC's), for a game with more
    /// than one.
    pub group: bool,
    /// RetroAchievements' type (Progression, Win condition, Missable).
    pub kind: bool,
    /// A RetroAchievements unlock in hardcore mode.
    pub hardcore: bool,
}

impl Default for NoticeFields {
    fn default() -> Self {
        Self { description: true, rarity: true, points: true, group: true, kind: true, hardcore: true }
    }
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
            notice: NoticeFields::default(),
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

    #[test]
    fn every_notice_line_is_on_by_default() {
        assert_eq!(Prefs::default().notice, NoticeFields::default());
        let n = NoticeFields::default();
        assert!(n.description && n.rarity && n.points && n.group && n.kind && n.hardcore);
        // An older file without them, or with only some.
        let p: Prefs = serde_json::from_str(r#"{"overlayNotices":true}"#).unwrap();
        assert_eq!(p.notice, NoticeFields::default());
        let p: Prefs = serde_json::from_str(r#"{"notice":{"rarity":false}}"#).unwrap();
        assert_eq!(p.notice, NoticeFields { rarity: false, ..NoticeFields::default() });
    }

    #[test]
    fn notice_lines_survive_a_write_and_a_read() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let notice = NoticeFields { description: false, points: false, hardcore: false, ..NoticeFields::default() };
        let prefs = Prefs { notice, ..Prefs::default() };
        write(&app, &prefs).unwrap();
        assert_eq!(read(&app).notice, notice);
        let raw = std::fs::read_to_string(app.achievement_prefs()).unwrap();
        assert!(raw.contains("\"description\": false") || raw.contains("\"description\":false"), "{raw}");
    }
}
