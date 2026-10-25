//! Settings → Achievements options, in the app state (`achievements.json`).

use anyhow::Result;
use heronite_data::paths::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// A desktop notification when a refresh finds new unlocks.
    pub notify: bool,
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
}

impl Default for Prefs {
    fn default() -> Self {
        Self { notify: false, timeline: true, hide_secrets: true, emulators: true, hardcore: false }
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
}
