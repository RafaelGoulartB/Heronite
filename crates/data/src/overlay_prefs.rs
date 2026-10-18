//! Options of the in-game overlay (the Guide button panel), kept in
//! `AppPaths::overlay_prefs()`: they belong to this machine, not to the
//! library.
//!
//! - `enabled`: the overlay is off until the user turns it on (beta).
//! - `screenshotShortcut`: Guide + RB takes a screenshot without opening it.
//! - `volumeShortcut`: Guide + ↑ / ↓ changes the volume without opening it.
//! - `volumeStep`: percent per press of the volume shortcut (2, 5 or 10).
//! - `side`: `left` (default) or `right` edge of the screen.

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// Volume steps offered in Settings.
pub const VOLUME_STEPS: [u8; 3] = [2, 5, 10];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Left,
    Right,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayPrefs {
    pub enabled: bool,
    pub screenshot_shortcut: bool,
    pub volume_shortcut: bool,
    /// One of [`VOLUME_STEPS`].
    pub volume_step: u8,
    pub side: Side,
}

impl Default for OverlayPrefs {
    fn default() -> Self {
        Self { enabled: false, screenshot_shortcut: true, volume_shortcut: true, volume_step: 5, side: Side::Left }
    }
}

impl OverlayPrefs {
    /// Loads the options; a missing or invalid file (or key) gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        let value = json_store::read_value(&paths.overlay_prefs()).unwrap_or(Value::Null);
        let d = Self::default();
        let flag = |key: &str, default: bool| value.get(key).and_then(Value::as_bool).unwrap_or(default);
        let step = value
            .get("volumeStep")
            .and_then(Value::as_u64)
            .and_then(|n| VOLUME_STEPS.iter().copied().find(|s| u64::from(*s) == n))
            .unwrap_or(d.volume_step);
        Self {
            enabled: flag("enabled", d.enabled),
            screenshot_shortcut: flag("screenshotShortcut", d.screenshot_shortcut),
            volume_shortcut: flag("volumeShortcut", d.volume_shortcut),
            volume_step: step,
            side: value.get("side").and_then(Value::as_str).and_then(Side::parse).unwrap_or(d.side),
        }
    }

    /// Writes the options atomically, keeping keys this struct does not model.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::update_value(&paths.overlay_prefs(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            doc["enabled"] = json!(self.enabled);
            doc["screenshotShortcut"] = json!(self.screenshot_shortcut);
            doc["volumeShortcut"] = json!(self.volume_shortcut);
            doc["volumeStep"] = json!(self.volume_step);
            doc["side"] = json!(self.side.as_str());
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_lenient_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_data(&crate::DataPaths::at(dir.path().join("heroic")));
        let d = OverlayPrefs::load(&paths);
        assert_eq!(d, OverlayPrefs::default());
        assert!(!d.enabled, "the overlay is off until the user turns it on");

        std::fs::create_dir_all(paths.overlay_prefs().parent().unwrap()).unwrap();
        std::fs::write(paths.overlay_prefs(), r#"{"enabled": "yes", "volumeStep": 7, "side": "top", "other": 1}"#).unwrap();
        assert_eq!(OverlayPrefs::load(&paths), OverlayPrefs::default());

        let p = OverlayPrefs { enabled: true, screenshot_shortcut: false, volume_shortcut: true, volume_step: 10, side: Side::Right };
        p.save(&paths).unwrap();
        assert_eq!(OverlayPrefs::load(&paths), p);
        assert_eq!(json_store::read_value(&paths.overlay_prefs()).unwrap()["other"], 1);
    }
}
