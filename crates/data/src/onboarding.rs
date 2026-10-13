//! The getting-started checklist, kept in `AppPaths::onboarding()`: whether
//! it opened by itself already (once per setup, new or updated), whether the
//! user asked not to see it again, and the steps that only count once done
//! here (the others are read from the settings each time).

use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Onboarding {
    /// It opened by itself once.
    pub shown: bool,
    /// "Don't show again": no header button, no opening by itself.
    pub hidden: bool,
    /// Ids of the steps done through the checklist (or the feature itself).
    pub done: BTreeSet<String>,
}

impl Onboarding {
    /// A missing or unreadable file gives a new checklist.
    pub fn load(paths: &AppPaths) -> Self {
        let Ok(value) = json_store::read_value(&paths.onboarding()) else { return Self::default() };
        Self {
            shown: value.get("shown").and_then(Value::as_bool).unwrap_or(false),
            hidden: value.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            done: value
                .get("done")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
        }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let value = json!({ "shown": self.shown, "hidden": self.hidden, "done": self.done });
        json_store::update_value(&paths.onboarding(), |doc| {
            *doc = value;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_a_new_checklist_and_saves_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert_eq!(Onboarding::load(&paths), Onboarding::default());
        let state = Onboarding { shown: true, hidden: true, done: ["palette".to_string()].into() };
        state.save(&paths).unwrap();
        assert_eq!(Onboarding::load(&paths), state);
    }
}
