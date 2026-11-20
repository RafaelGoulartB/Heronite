//! Options of the quick launcher (the command palette in a small window of
//! its own), kept in `AppPaths::quick_launch_prefs()`: they belong to this
//! machine, not to the library.
//!
//! - `keyboardShortcut`: the key combination that opens it from anywhere
//!   (Electron-style, `Super+Space`); empty, the default, for none.

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickLaunchPrefs {
    pub keyboard_accelerator: String,
}

impl QuickLaunchPrefs {
    pub fn load(paths: &AppPaths) -> Self {
        let value = json_store::read_value(&paths.quick_launch_prefs()).unwrap_or(Value::Null);
        let keyboard_accelerator = value.get("keyboardShortcut").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        Self { keyboard_accelerator }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::update_value(&paths.quick_launch_prefs(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            doc["keyboardShortcut"] = json!(self.keyboard_accelerator);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_shortcut_until_one_is_set_and_it_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_data(&crate::DataPaths::at(dir.path().join("heroic")));
        std::fs::create_dir_all(paths.quick_launch_prefs().parent().unwrap()).unwrap();
        assert_eq!(QuickLaunchPrefs::load(&paths), QuickLaunchPrefs::default());

        std::fs::write(paths.quick_launch_prefs(), r#"{"keyboardShortcut": 3, "other": 1}"#).unwrap();
        assert_eq!(QuickLaunchPrefs::load(&paths).keyboard_accelerator, "");

        let p = QuickLaunchPrefs { keyboard_accelerator: "Super+Space".into() };
        p.save(&paths).unwrap();
        assert_eq!(QuickLaunchPrefs::load(&paths), p);
        assert_eq!(json_store::read_value(&paths.quick_launch_prefs()).unwrap()["other"], 1);
    }
}
