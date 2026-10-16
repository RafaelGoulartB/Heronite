//! This app's own UI state, kept in `AppPaths::ui_state()`.
//!
//! It mirrors what Heronite kept in Chromium's localStorage (imported once by
//! `migrate`): `collection_install_filter`, `collection_facet_filter`,
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
    /// The sort turned around (View menu's arrow).
    pub sort_descending: bool,
    /// The View menu's "Only" switches.
    pub only: OnlyFilters,
    /// Selected custom categories (`heronite_data::categories`;
    /// `categories::UNCATEGORIZED` for the games in none). Empty: all games.
    pub categories: BTreeSet<String>,
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
    /// Ids of the command palette items run lately, most recent first.
    pub palette_recent: Vec<String>,
    /// The Collection shows the full-window grid instead of the list next to
    /// the focus panel.
    pub grid_view: bool,
    /// Smallest cover width of the grid, in logical px ([`GRID_COVER_W`]).
    pub grid_cover_w: f32,
    /// The grid shows each game's title under its cover.
    pub grid_titles: bool,
}

/// Range and default of [`UiState::grid_cover_w`].
pub const GRID_COVER_W: (f32, f32, f32) = (110., 240., 150.);

impl Default for UiState {
    fn default() -> Self {
        Self {
            install_filter: InstallFilter::All,
            facet_filters: FacetFilters::default(),
            sort: SortMode::Title,
            sort_descending: false,
            only: OnlyFilters::default(),
            categories: BTreeSet::new(),
            sidebar_hidden: false,
            collapsed_groups: BTreeSet::new(),
            focused_game: None,
            group_by_status: true,
            show_hidden: false,
            console_mode: false,
            palette_recent: Vec::new(),
            grid_view: false,
            grid_cover_w: GRID_COVER_W.2,
            grid_titles: false,
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
            let text = |kind: FacetKind| obj.get(kind.as_str()).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            state.facet_filters = FacetFilters {
                series: text(FacetKind::Series),
                developer: text(FacetKind::Developer),
                publisher: text(FacetKind::Publisher),
                genre: text(FacetKind::Genre),
                platform: text(FacetKind::Platform),
                store: text(FacetKind::Store),
            };
        }
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        state.sort_descending = flag("sortDescending");
        state.only = OnlyFilters { favourites: flag("onlyFavourites"), updates: flag("onlyUpdates"), offline: flag("onlyOffline") };
        if let Some(list) = value.get("categories").and_then(Value::as_array) {
            state.categories = list.iter().filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)).collect();
        }
        state.sidebar_hidden = value.get("sidebarHidden").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("collapsedGroups").and_then(Value::as_array) {
            state.collapsed_groups = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.focused_game = value.get("focusedGame").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        state.group_by_status = value.get("groupByStatus").and_then(Value::as_bool).unwrap_or(true);
        state.show_hidden = value.get("showHidden").and_then(Value::as_bool).unwrap_or(false);
        state.console_mode = value.get("consoleMode").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("paletteRecent").and_then(Value::as_array) {
            state.palette_recent = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.grid_view = flag("gridView");
        if let Some(w) = value.get("gridCoverWidth").and_then(Value::as_f64).filter(|w| w.is_finite()) {
            state.grid_cover_w = (w as f32).clamp(GRID_COVER_W.0, GRID_COVER_W.1);
        }
        state.grid_titles = flag("gridTitles");
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
                "store": facets.store,
            },
            "sort": self.sort,
            "sortDescending": self.sort_descending,
            "onlyFavourites": self.only.favourites,
            "onlyUpdates": self.only.updates,
            "onlyOffline": self.only.offline,
            "categories": self.categories,
            "sidebarHidden": self.sidebar_hidden,
            "collapsedGroups": self.collapsed_groups,
            "focusedGame": self.focused_game,
            "groupByStatus": self.group_by_status,
            "showHidden": self.show_hidden,
            "consoleMode": self.console_mode,
            "paletteRecent": self.palette_recent,
            "gridView": self.grid_view,
            "gridCoverWidth": self.grid_cover_w,
            "gridTitles": self.grid_titles,
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

    /// Selects or clears one category of the filter.
    pub fn toggle_category(&mut self, name: &str) {
        if !self.categories.remove(name) {
            self.categories.insert(name.to_owned());
        }
    }
}

/// The View menu's "Only" switches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OnlyFilters {
    pub favourites: bool,
    /// Installed games with an update waiting.
    pub updates: bool,
    /// Games that run without a connection.
    pub offline: bool,
}

impl OnlyFilters {
    pub fn is_active(&self) -> bool {
        self.favourites || self.updates || self.offline
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
        std::fs::write(paths.ui_state(), r#"{"sort":"bogus","installFilter":7,"collapsedGroups":"x","facetFilters":{"genre":""}}"#)
            .unwrap();
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
            palette_recent: vec!["go:downloads".into(), "game:sideload_steam_400:primary".into()],
            sort_descending: true,
            only: OnlyFilters { favourites: true, updates: false, offline: true },
            grid_view: true,
            grid_cover_w: 180.,
            grid_titles: true,
            ..Default::default()
        };
        state.toggle_category("RPG");
        state.toggle_category(crate::categories::UNCATEGORIZED);
        state.toggle_category("Gone");
        state.toggle_category("Gone");
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
        assert_eq!(on_disk["onlyOffline"], true);
        assert_eq!(on_disk["gridView"], true);
        assert_eq!(on_disk["categories"], json!([crate::categories::UNCATEGORIZED, "RPG"]));
        state.clear_facets();
        assert!(!state.facet_filters.is_active());
    }

    #[test]
    fn grid_cover_width_is_clamped() {
        let state = UiState::from_value(&json!({ "gridCoverWidth": 9000 }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.1);
        let state = UiState::from_value(&json!({ "gridCoverWidth": "big" }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.2);
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
