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
}

impl Default for Prefs {
    fn default() -> Self {
        Self { notify: true, timeline: true, hide_secrets: true }
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
        let p: Prefs = serde_json::from_str(r#"{"notify":false}"#).unwrap();
        assert_eq!(p, Prefs { notify: false, ..Prefs::default() });
    }
}
