//! This app's own UI state, kept in `AppPaths::ui_state()`.
//!
//! It mirrors what Heronite keeps in Chromium's localStorage (so the two apps
//! cannot share it): `collection_install_filter`, `collection_facet_filter`,
//! `collection_sort`, `collection_sidebar_hidden`, `collection_collapsed_groups`,
//! `collection_focused_game`. Loading is as lenient as the `readX` helpers of
//! `index.tsx`: anything missing or invalid falls back to its default.

use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{json, Value};

use crate::collection::{FacetFilters, FacetKind, InstallFilter, SortMode};
use crate::json_store;
use crate::paths::AppPaths;

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub install_filter: InstallFilter,
    pub facet_filters: FacetFilters,
    pub sort: SortMode,
    pub sidebar_hidden: bool,
    /// Ids of the collapsed status groups (`ungrouped` for "Other").
    pub collapsed_groups: BTreeSet<String>,
    /// `runner_appName` of the focused game.
    pub focused_game: Option<String>,
    /// "Group by status" toggle. Heronite does not persist it (always on at
    /// start); this app does.
    pub group_by_status: bool,
    /// "Show hidden games" (Heroic keeps it in the renderer's localStorage
    /// `show_hidden`, so it cannot be shared): hidden games stay in the
    /// Collection, flagged.
    pub show_hidden: bool,
    /// The app was left in console mode (it reopens in it).
    pub console_mode: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            install_filter: InstallFilter::All,
            facet_filters: FacetFilters::default(),
            sort: SortMode::Title,
            sidebar_hidden: false,
            collapsed_groups: BTreeSet::new(),
            focused_game: None,
            group_by_status: true,
            show_hidden: false,
            console_mode: false,
        }
    }
}

impl UiState {
    /// Parses the stored document leniently.
    pub fn from_value(value: &Value) -> Self {
        let mut state = Self::default();
        if let Some(v) = value.get("installFilter").and_then(Value::as_str).and_then(InstallFilter::parse) {
            state.install_filter = v;
        }
        if let Some(v) = value.get("sort").and_then(Value::as_str).and_then(SortMode::parse) {
            state.sort = v;
        }
        if let Some(obj) = value.get("facetFilters").and_then(Value::as_object) {
            let text = |kind: FacetKind| {
                obj.get(kind.as_str()).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
            };
            state.facet_filters = FacetFilters {
                series: text(FacetKind::Series),
                developer: text(FacetKind::Developer),
                publisher: text(FacetKind::Publisher),
                genre: text(FacetKind::Genre),
                platform: text(FacetKind::Platform),
            };
        }
        state.sidebar_hidden = value.get("sidebarHidden").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("collapsedGroups").and_then(Value::as_array) {
            state.collapsed_groups = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.focused_game = value
            .get("focusedGame")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        state.group_by_status = value.get("groupByStatus").and_then(Value::as_bool).unwrap_or(true);
        state.show_hidden = value.get("showHidden").and_then(Value::as_bool).unwrap_or(false);
        state.console_mode = value.get("consoleMode").and_then(Value::as_bool).unwrap_or(false);
        state
    }

    pub fn to_value(&self) -> Value {
        let facets = &self.facet_filters;
        json!({
            "installFilter": self.install_filter,
            "facetFilters": {
                "series": facets.series,
                "developer": facets.developer,
                "publisher": facets.publisher,
                "genre": facets.genre,
                "platform": facets.platform,
            },
            "sort": self.sort,
            "sidebarHidden": self.sidebar_hidden,
            "collapsedGroups": self.collapsed_groups,
            "focusedGame": self.focused_game,
            "groupByStatus": self.group_by_status,
            "showHidden": self.show_hidden,
            "consoleMode": self.console_mode,
        })
    }

    /// Loads the state; a missing or unreadable file gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        match json_store::read_value(&paths.ui_state()) {
            Ok(value) if value.is_object() => Self::from_value(&value),
            _ => Self::default(),
        }
    }

    /// Writes the state atomically (creating the folder if needed). Keys of
    /// the file this struct does not model are kept.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        let Value::Object(known) = self.to_value() else { unreachable!("to_value builds an object") };
        json_store::update_value(&paths.ui_state(), |doc| {
            if !doc.is_object() {
                *doc = Value::Object(Default::default());
            }
            let obj = doc.as_object_mut().expect("object");
            for (k, v) in known {
                obj.insert(k, v);
            }
            Ok(())
        })
    }

    /// `toggleGroup`; returns whether the group is now collapsed.
    pub fn toggle_group(&mut self, id: &str) -> bool {
        if !self.collapsed_groups.remove(id) {
            self.collapsed_groups.insert(id.to_owned());
            true
        } else {
            false
        }
    }

    /// `handleFacetFilter`.
    pub fn toggle_facet(&mut self, kind: FacetKind, value: &str) {
        self.facet_filters = self.facet_filters.toggled(kind, value);
    }

    /// `handleClearFacets`.
    pub fn clear_facets(&mut self) {
        self.facet_filters = FacetFilters::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_paths(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("cfg"), cache: dir.join("cache") }
    }

    #[test]
    fn defaults_when_missing_or_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert_eq!(UiState::load(&paths), UiState::default());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.ui_state(), "{not json").unwrap();
        assert_eq!(UiState::load(&paths), UiState::default());
        std::fs::write(paths.ui_state(), r#"{"sort":"bogus","installFilter":7,"collapsedGroups":"x","facetFilters":{"genre":""}}"#).unwrap();
        assert_eq!(UiState::load(&paths), UiState::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState {
            install_filter: InstallFilter::Installed,
            sort: SortMode::LastPlayed,
            sidebar_hidden: true,
            focused_game: Some("sideload_steam_400".into()),
            group_by_status: false,
            show_hidden: true,
            console_mode: true,
            ..Default::default()
        };
        state.toggle_facet(FacetKind::Genre, "RPG");
        assert!(state.toggle_group("beaten"));
        assert!(state.toggle_group("ungrouped"));
        assert!(!state.toggle_group("beaten"));
        state.save(&paths).unwrap();
        let loaded = UiState::load(&paths);
        assert_eq!(loaded, state);
        assert_eq!(loaded.facet_filters.genre.as_deref(), Some("RPG"));
        assert_eq!(loaded.collapsed_groups.iter().collect::<Vec<_>>(), ["ungrouped"]);
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["sort"], "lastPlayed");
        assert_eq!(on_disk["installFilter"], "installed");
        assert_eq!(on_disk["showHidden"], true);
        assert_eq!(on_disk["consoleMode"], true);
        state.clear_facets();
        assert!(!state.facet_filters.is_active());
    }

    #[test]
    fn save_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.ui_state(), "{\n\t\"futureKey\": [1, 2],\n\t\"sort\": \"title\"\n}").unwrap();
        let mut state = UiState::load(&paths);
        assert!(!state.show_hidden);
        state.show_hidden = true;
        state.save(&paths).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["futureKey"], json!([1, 2]));
        assert_eq!(on_disk["showHidden"], true);
        assert!(UiState::load(&paths).show_hidden);
    }
}
