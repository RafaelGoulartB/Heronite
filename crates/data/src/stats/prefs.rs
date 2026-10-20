//! What the Statistics page remembers: its last tab, the goal of beaten
//! games of each year, and the discoveries pinned or hidden. They live in the
//! app's `ui-state.json` under keys of their own (`statsTab`, `statsGoals`,
//! `statsPinned`, `statsHidden`), which [`crate::ui_state::UiState`] keeps
//! when it saves, so the page writes them without touching the Collection's.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatsPrefs {
    /// Id of the tab shown last.
    pub tab: Option<String>,
    /// Games to beat in each year.
    pub goals: BTreeMap<i32, u32>,
    /// Kinds of discoveries kept first.
    pub pinned: Vec<String>,
    /// Kinds of discoveries not shown.
    pub hidden: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

impl StatsPrefs {
    pub fn from_value(value: &Value) -> Self {
        let goals = value
            .get("statsGoals")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(y, n)| Some((y.parse().ok()?, n.as_u64().filter(|n| (1..=999).contains(n))? as u32))).collect())
            .unwrap_or_default();
        Self {
            tab: value.get("statsTab").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned),
            goals,
            pinned: strings(value.get("statsPinned")),
            hidden: strings(value.get("statsHidden")),
        }
    }

    pub fn load(paths: &AppPaths) -> Self {
        json_store::read_value(&paths.ui_state()).map(|v| Self::from_value(&v)).unwrap_or_default()
    }

    /// Writes these keys only; the rest of the file stays as it is.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let goals: serde_json::Map<String, Value> = self.goals.iter().map(|(y, n)| (y.to_string(), json!(n))).collect();
        json_store::update_value(&paths.ui_state(), |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Default::default());
            }
            let obj = doc.as_object_mut().expect("object");
            obj.insert("statsTab".into(), json!(self.tab));
            obj.insert("statsGoals".into(), Value::Object(goals));
            obj.insert("statsPinned".into(), json!(self.pinned));
            obj.insert("statsHidden".into(), json!(self.hidden));
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_state::UiState;

    #[test]
    fn prefs_round_trip_and_survive_the_collection_saving() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let prefs = StatsPrefs {
            tab: Some("backlog".into()),
            goals: [(2026, 12)].into_iter().collect(),
            pinned: vec!["streak".into()],
            hidden: vec!["owl".into()],
        };
        prefs.save(&paths).unwrap();
        UiState::load(&paths).save(&paths).unwrap();
        assert_eq!(StatsPrefs::load(&paths), prefs);
        let odd = StatsPrefs::from_value(&json!({ "statsGoals": { "x": 3, "2025": 0, "2024": 8 } }));
        assert_eq!(odd.goals, [(2024, 8)].into_iter().collect());
    }
}
