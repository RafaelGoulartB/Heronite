//! Screenshot options that belong to this app only, kept in
//! `AppPaths::screenshot_prefs()`: Heronite rewrites the whole `screenshots`
//! object of `local_library/settings.json`, so a key added there would not
//! survive a save from the other app.
//!
//! - `compressLossless`: convert PNG captures to lossless WebP in the
//!   background (see `heronite_media::compress`).

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScreenshotPrefs {
    pub compress_lossless: bool,
}

impl ScreenshotPrefs {
    /// Loads the options; a missing or invalid file gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        let value = json_store::read_value(&paths.screenshot_prefs()).unwrap_or(Value::Null);
        Self { compress_lossless: value.get("compressLossless").and_then(Value::as_bool).unwrap_or(false) }
    }

    /// Writes the options atomically, keeping keys this struct does not model.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::update_value(&paths.screenshot_prefs(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            doc["compressLossless"] = json!(self.compress_lossless);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_lenient_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_heroic(&crate::HeroicPaths::at(dir.path().join("heroic")));
        assert_eq!(ScreenshotPrefs::load(&paths), ScreenshotPrefs::default());
        std::fs::create_dir_all(paths.screenshot_prefs().parent().unwrap()).unwrap();
        std::fs::write(paths.screenshot_prefs(), r#"{"compressLossless": "yes", "other": 1}"#).unwrap();
        assert!(!ScreenshotPrefs::load(&paths).compress_lossless);
        ScreenshotPrefs { compress_lossless: true }.save(&paths).unwrap();
        assert!(ScreenshotPrefs::load(&paths).compress_lossless);
        let doc = json_store::read_value(&paths.screenshot_prefs()).unwrap();
        assert_eq!(doc["other"], 1);
    }
}
