//! The getting-started checklist, kept in `AppPaths::onboarding()`: whether
//! it opened by itself already (once per setup, new or updated), whether it
//! was closed once, the steps that only count once done here (the others are read
//! from the settings each time) and the ones the user does not use.

use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Onboarding {
    /// It opened by itself once.
    pub shown: bool,
    /// Closed once (or "Don't show again" in older versions): no header
    /// button, no opening by itself; the command palette still opens it.
    pub hidden: bool,
    /// Ids of the steps done through the checklist (or the feature itself).
    pub done: BTreeSet<String>,
    /// Ids of the steps marked "Not used": they count as settled.
    pub skipped: BTreeSet<String>,
}

impl Onboarding {
    /// A missing or unreadable file gives a new checklist.
    pub fn load(paths: &AppPaths) -> Self {
        let Ok(value) = json_store::read_value(&paths.onboarding()) else { return Self::default() };
        Self {
            shown: value.get("shown").and_then(Value::as_bool).unwrap_or(false),
            hidden: value.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            done: ids(&value, "done"),
            skipped: ids(&value, "skipped"),
        }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let value = json!({ "shown": self.shown, "hidden": self.hidden, "done": self.done, "skipped": self.skipped });
        json_store::update_value(&paths.onboarding(), |doc| {
            *doc = value;
            Ok(())
        })
    }
}

fn ids(value: &Value, key: &str) -> BTreeSet<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_a_new_checklist_and_saves_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert_eq!(Onboarding::load(&paths), Onboarding::default());
        let state = Onboarding { shown: true, hidden: true, done: ["palette".to_string()].into(), skipped: ["saves".to_string()].into() };
        state.save(&paths).unwrap();
        assert_eq!(Onboarding::load(&paths), state);
    }

    #[test]
    fn a_file_from_before_not_used_reads_with_nothing_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        json_store::update_value(&paths.onboarding(), |doc| {
            *doc = json!({ "shown": true, "hidden": false, "done": ["stats"] });
            Ok(())
        })
        .unwrap();
        let state = Onboarding::load(&paths);
        assert_eq!(state.done, ["stats".to_string()].into());
        assert!(state.skipped.is_empty());
    }
}
