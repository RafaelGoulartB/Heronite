//! This app's own UI state, kept in `AppPaths::ui_state()`.
//!
//! It mirrors what Heronite kept in Chromium's localStorage (imported once by
//! `migrate`): `collection_install_filter`, `collection_facet_filter`,
//! `collection_sort`, `collection_sidebar_hidden`, `collection_collapsed_groups`,
//! `collection_focused_game`. Loading is as lenient as the `readX` helpers of
//! `index.tsx`: anything missing or invalid falls back to its default.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::age_rating::AgeFilter;
use crate::collection::{FacetFilters, FacetKind, InstallFilter, SortMode, FACET_KINDS};
use crate::console_theme::{AutoConsole, ConsoleButton, OpenIn};
use crate::json_store;
use crate::organize::GroupBy;
use crate::paths::AppPaths;

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub install_filter: InstallFilter,
    pub facet_filters: FacetFilters,
    /// Facet kinds of `facetFilters` this version does not know, kept as read.
    pub unknown_facets: Map<String, Value>,
    pub sort: SortMode,
    /// The sort turned around (View menu's arrow).
    pub sort_descending: bool,
    /// The View menu's "Only" switches.
    pub only: OnlyFilters,
    /// The View menu's "Ages up to".
    pub age: AgeFilter,
    /// Selected custom categories (`heronite_data::categories`;
    /// `categories::UNCATEGORIZED` for the games in none). Empty: all games.
    pub categories: BTreeSet<String>,
    /// Ids of the selected completion statuses. Empty: all games.
    pub statuses: BTreeSet<String>,
    /// Ids of the selected smart groups (`heronite_data::smart_groups`),
    /// the games of any of them. Empty: all games.
    pub smart_groups: BTreeSet<String>,
    /// Colour and pin of each manual category, by name (`categoryLooks`).
    /// Smart categories keep theirs in `groups.json`; these stay on this PC.
    pub category_looks: BTreeMap<String, CategoryLook>,
    /// The View menu's views, in menu order.
    pub saved_views: Vec<SavedView>,
    /// The built-in views were added to [`saved_views`](Self::saved_views)
    /// once (after that they are the user's to edit or delete).
    pub views_seeded: bool,
    pub sidebar_hidden: bool,
    /// Ids of the collapsed status groups (`ungrouped` for "Other").
    pub collapsed_groups: BTreeSet<String>,
    /// `runner_appName` of the focused game.
    pub focused_game: Option<String>,
    /// What the list is split into sections by (Heronite only had a "Group
    /// by status" toggle, not persisted; this app keeps it).
    pub group_by: GroupBy,
    /// "Show hidden games" (Heroic keeps it in the renderer's localStorage
    /// `show_hidden`, so it cannot be shared): hidden games stay in the
    /// Collection, flagged.
    pub show_hidden: bool,
    /// Copies of one game from several stores show as one card
    /// ([`crate::copies`]); off, every copy has its own.
    pub group_copies: bool,
    /// The app was left in console mode (it reopens in it).
    pub console_mode: bool,
    /// Ids of the command palette items run lately, most recent first.
    pub palette_recent: Vec<String>,
    /// The Collection shows the full-window grid instead of the list next to
    /// the focus panel.
    pub grid_view: bool,
    /// Smallest cover width of the grid, in logical px ([`GRID_COVER_W`]).
    pub grid_cover_w: f32,
    /// What the grid shows under each cover.
    pub grid_label: GridLabel,
    /// Selecting a game in the grid opens its drawer; off, the drawer opens
    /// on a double click or Enter and the grid keeps the whole width.
    pub grid_open_on_select: bool,
    /// The Focus list shows one game per row (thumbnail, title, playtime)
    /// instead of its covers.
    pub focus_list: bool,
    /// How big the Focus list's covers are.
    pub focus_cover_size: FocusCoverSize,
    /// The focus panel shows the Journal instead of the Timeline.
    pub journal_view: bool,
    /// Rows of the focus panel's "Details" the user turned off, by key
    /// (`"ageRating"`, `"notes"`...). The age ratings start off.
    pub detail_rows_hidden: BTreeSet<String>,
    /// Width the Focus list column was dragged to, in logical px. `None`: the
    /// default for the window, which is also the narrowest it gets.
    pub list_column_w: Option<f32>,
    /// Width the grid's drawer was dragged to, like
    /// [`list_column_w`](Self::list_column_w).
    pub grid_drawer_w: Option<f32>,
    /// `openIn` of `console.json`, mirrored here so the start does not read
    /// that file for users who never changed it. Read by [`UiState::load`];
    /// only [`set_console_open_in`] writes it ([`UiState::save`] does not, so
    /// a copy of the state held elsewhere cannot write a stale value back).
    pub console_open_in: OpenIn,
    /// `autoConsole` of `console.json`, mirrored like
    /// [`console_open_in`](Self::console_open_in) ([`set_console_auto`]).
    pub console_auto: AutoConsole,
    /// `consoleButton` of `console.json`, mirrored like
    /// [`console_open_in`](Self::console_open_in) ([`set_console_button`]).
    pub console_button: ConsoleButton,
    /// `fullscreen` of `console.json`, mirrored like
    /// [`console_open_in`](Self::console_open_in) ([`set_console_fullscreen`]):
    /// a window that opens in console mode opens fullscreen.
    pub console_fullscreen: bool,
}

/// Range and default of [`UiState::grid_cover_w`].
/// The "Details" rows hidden until the user turns them on.
pub const DETAIL_ROWS_HIDDEN_DEFAULT: &[&str] = &["ageRating"];

pub const GRID_COVER_W: (f32, f32, f32) = (110., 240., 150.);

/// What the grid shows under each cover: the title, the info line of
/// Settings → Appearance → "Under the cover" (playtime, last played,
/// status...), both or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridLabel {
    Nothing,
    Title,
    #[default]
    Info,
    TitleInfo,
}

impl GridLabel {
    pub const ALL: [GridLabel; 4] = [Self::Nothing, Self::Title, Self::Info, Self::TitleInfo];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nothing => "none",
            Self::Title => "title",
            Self::Info => "info",
            Self::TitleInfo => "titleInfo",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == value)
    }

    pub fn title(self) -> bool {
        matches!(self, Self::Title | Self::TitleInfo)
    }

    pub fn info(self) -> bool {
        matches!(self, Self::Info | Self::TitleInfo)
    }
}

/// Size of the Focus list's covers: Medium is the list's own width
/// (`theme::CARD_W`); Small fits a cover more on each row, Large one less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusCoverSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl FocusCoverSize {
    pub const ALL: [FocusCoverSize; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == value)
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            install_filter: InstallFilter::All,
            facet_filters: FacetFilters::default(),
            unknown_facets: Map::new(),
            sort: SortMode::Title,
            sort_descending: false,
            only: OnlyFilters::default(),
            age: AgeFilter::default(),
            categories: BTreeSet::new(),
            statuses: BTreeSet::new(),
            smart_groups: BTreeSet::new(),
            category_looks: BTreeMap::new(),
            saved_views: Vec::new(),
            views_seeded: false,
            sidebar_hidden: false,
            collapsed_groups: BTreeSet::new(),
            focused_game: None,
            group_by: GroupBy::Status,
            show_hidden: false,
            group_copies: true,
            console_mode: false,
            palette_recent: Vec::new(),
            grid_view: true,
            grid_cover_w: GRID_COVER_W.2,
            grid_label: GridLabel::default(),
            grid_open_on_select: true,
            focus_list: false,
            focus_cover_size: FocusCoverSize::default(),
            journal_view: false,
            detail_rows_hidden: DETAIL_ROWS_HIDDEN_DEFAULT.iter().map(|k| (*k).to_owned()).collect(),
            list_column_w: None,
            grid_drawer_w: None,
            console_open_in: OpenIn::default(),
            console_auto: AutoConsole::default(),
            console_button: ConsoleButton::default(),
            console_fullscreen: true,
        }
    }
}

impl UiState {
    /// Parses the stored document leniently.
    pub fn from_value(value: &Value) -> Self {
        let mut state = Self::default();
        state.apply_view(&ViewFilters::from_value(value));
        if let Some(list) = value.get("savedViews").and_then(Value::as_array) {
            state.saved_views = list
                .iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                    Some(SavedView { name: name.to_owned(), filters: ViewFilters::from_value(v), extra: SavedView::extra_of(v) })
                })
                .collect();
        }
        if let Some(looks) = value.get(CATEGORY_LOOKS).and_then(Value::as_object) {
            state.category_looks = looks
                .iter()
                .filter(|(name, _)| !name.trim().is_empty())
                .filter_map(|(name, v)| Some((name.clone(), CategoryLook::from_value(v).filter(|l| !l.is_plain())?)))
                .collect();
        }
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        state.views_seeded = flag("viewsSeeded");
        state.sidebar_hidden = value.get("sidebarHidden").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("collapsedGroups").and_then(Value::as_array) {
            state.collapsed_groups = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        state.focused_game = value.get("focusedGame").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        state.show_hidden = value.get("showHidden").and_then(Value::as_bool).unwrap_or(false);
        state.group_copies = value.get("groupCopies").and_then(Value::as_bool).unwrap_or(true);
        state.console_mode = value.get("consoleMode").and_then(Value::as_bool).unwrap_or(false);
        if let Some(list) = value.get("paletteRecent").and_then(Value::as_array) {
            state.palette_recent = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        // The grid until the user picks the Focus view.
        state.grid_view = value.get("gridView").and_then(Value::as_bool).unwrap_or(true);
        if let Some(w) = value.get("gridCoverWidth").and_then(Value::as_f64).filter(|w| w.is_finite()) {
            state.grid_cover_w = (w as f32).clamp(GRID_COVER_W.0, GRID_COVER_W.1);
        }
        // `gridTitles` came first: the info line always showed, the title on request.
        state.grid_label = match value.get("gridLabel").and_then(Value::as_str).and_then(GridLabel::parse) {
            Some(label) => label,
            None if flag("gridTitles") => GridLabel::TitleInfo,
            None => GridLabel::Info,
        };
        state.grid_open_on_select = value.get("gridOpenOnSelect").and_then(Value::as_bool).unwrap_or(true);
        state.focus_list = flag("focusList");
        state.focus_cover_size = value.get("focusCoverSize").and_then(Value::as_str).and_then(FocusCoverSize::parse).unwrap_or_default();
        state.journal_view = flag("journalView");
        if let Some(list) = value.get("detailRowsHidden").and_then(Value::as_array) {
            state.detail_rows_hidden = list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        }
        let width = |key: &str| value.get(key).and_then(Value::as_f64).filter(|w| w.is_finite() && *w > 0.).map(|w| w as f32);
        state.list_column_w = width("listColumnWidth");
        state.grid_drawer_w = width("gridDrawerWidth");
        state.console_open_in = value.get(CONSOLE_OPEN_IN).and_then(Value::as_str).and_then(OpenIn::parse).unwrap_or_default();
        state.console_auto = value.get(CONSOLE_AUTO).and_then(Value::as_str).and_then(AutoConsole::parse).unwrap_or_default();
        state.console_button = value.get(CONSOLE_BUTTON).and_then(Value::as_str).and_then(ConsoleButton::parse).unwrap_or_default();
        state.console_fullscreen = value.get(CONSOLE_FULLSCREEN).and_then(Value::as_bool).unwrap_or(true);
        state
    }

    pub fn to_value(&self) -> Value {
        let mut doc = json!({
            "savedViews": self.saved_views.iter().map(SavedView::to_value).collect::<Vec<_>>(),
            "viewsSeeded": self.views_seeded,
            "sidebarHidden": self.sidebar_hidden,
            "collapsedGroups": self.collapsed_groups,
            "focusedGame": self.focused_game,
            "showHidden": self.show_hidden,
            "groupCopies": self.group_copies,
            "consoleMode": self.console_mode,
            "paletteRecent": self.palette_recent,
            "gridView": self.grid_view,
            "gridCoverWidth": self.grid_cover_w,
            "gridLabel": self.grid_label.as_str(),
            "gridOpenOnSelect": self.grid_open_on_select,
            "focusList": self.focus_list,
            "focusCoverSize": self.focus_cover_size.as_str(),
            "journalView": self.journal_view,
            "detailRowsHidden": self.detail_rows_hidden,
            "listColumnWidth": self.list_column_w,
            "gridDrawerWidth": self.grid_drawer_w,
        });
        if let (Value::Object(doc), Value::Object(view)) = (&mut doc, self.view_filters().to_value()) {
            doc.extend(view);
        }
        let looks: Map<String, Value> =
            self.category_looks.iter().filter(|(_, l)| !l.is_plain()).map(|(name, l)| (name.clone(), l.to_value())).collect();
        if !looks.is_empty() {
            doc[CATEGORY_LOOKS] = Value::Object(looks);
        }
        doc
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
            // Not written when no group is picked (see `ViewFilters::to_value`):
            // the groups picked before would come back on the next start.
            if !known.contains_key("smartGroups") {
                obj.remove("smartGroups");
            }
            // Likewise: the last look taken off leaves no key behind.
            if !known.contains_key(CATEGORY_LOOKS) {
                obj.remove(CATEGORY_LOOKS);
            }
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

    /// Follows a rename, merge (`to`) or removal (`None`) of names in the
    /// filters on screen and in the saved views. True when one changed.
    pub fn rename_facet_values(&mut self, kinds: &[FacetKind], from: &[String], to: Option<&str>) -> bool {
        let mut changed = false;
        for kind in kinds {
            changed |= self.facet_filters.rename_values(*kind, from, to);
            for view in &mut self.saved_views {
                changed |= view.filters.facet_filters.rename_values(*kind, from, to);
            }
        }
        changed
    }

    /// Drops the statuses `keep` rejects (removed ones) from the filter on
    /// screen and from the saved views. True when one changed.
    pub fn prune_statuses(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.statuses.len();
        self.statuses.retain(|id| keep(id));
        let mut changed = self.statuses.len() != before;
        for view in &mut self.saved_views {
            let before = view.filters.statuses.len();
            view.filters.statuses.retain(|id| keep(id));
            changed |= view.filters.statuses.len() != before;
        }
        changed
    }

    /// [`prune_statuses`](Self::prune_statuses) for categories. Their looks
    /// stay (a category that is back, from a sync or an unreadable file,
    /// keeps its colour); [`forget_category`](Self::forget_category) drops one.
    pub fn prune_categories(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.categories.len();
        self.categories.retain(|name| keep(name));
        let mut changed = self.categories.len() != before;
        for view in &mut self.saved_views {
            let before = view.filters.categories.len();
            view.filters.categories.retain(|name| keep(name));
            changed |= view.filters.categories.len() != before;
        }
        changed
    }

    /// Follows a category rename (or merge) in the filter on screen and in
    /// the saved views. True when one changed.
    pub fn rename_category(&mut self, old: &str, new: &str) -> bool {
        let rename = |set: &mut BTreeSet<String>| {
            let had = set.remove(old);
            if had {
                set.insert(new.to_owned());
            }
            had
        };
        let mut changed = rename(&mut self.categories);
        for view in &mut self.saved_views {
            changed |= rename(&mut view.filters.categories);
        }
        // A merge keeps the colour of the category merged into (the merged
        // one's when it has none), and its pin when either was pinned.
        if let Some(look) = self.category_looks.remove(old) {
            match self.category_looks.get_mut(new) {
                Some(into) => {
                    into.pinned |= look.pinned;
                    if into.colour.is_none() {
                        into.colour = look.colour;
                    }
                }
                None => {
                    self.category_looks.insert(new.to_owned(), look);
                }
            }
            changed = true;
        }
        changed
    }

    /// The user removed the category `name`: its look goes too.
    pub fn forget_category(&mut self, name: &str) -> bool {
        self.category_looks.remove(name).is_some()
    }

    /// The look of the manual category `name` (plain when it has none).
    pub fn category_look(&self, name: &str) -> CategoryLook {
        self.category_looks.get(name).cloned().unwrap_or_default()
    }

    /// Changes the look of `name`; a plain look is dropped (its unknown
    /// keys are kept).
    pub fn set_category_look(&mut self, name: &str, f: impl FnOnce(&mut CategoryLook)) {
        let mut look = self.category_look(name);
        f(&mut look);
        if look.is_plain() {
            self.category_looks.remove(name);
        } else {
            self.category_looks.insert(name.to_owned(), look);
        }
    }

    /// `handleClearFacets`.
    pub fn clear_facets(&mut self) {
        self.facet_filters = FacetFilters::default();
        self.unknown_facets.clear();
    }

    /// Selects or clears one category of the filter.
    pub fn toggle_category(&mut self, name: &str) {
        if !self.categories.remove(name) {
            self.categories.insert(name.to_owned());
        }
    }

    /// Drops the smart groups `keep` rejects (removed ones) from the filter
    /// on screen and from the saved views. True when one changed.
    pub fn prune_smart_groups(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.smart_groups.len();
        self.smart_groups.retain(|id| keep(id));
        let mut changed = self.smart_groups.len() != before;
        for view in &mut self.saved_views {
            let before = view.filters.smart_groups.len();
            view.filters.smart_groups.retain(|id| keep(id));
            changed |= view.filters.smart_groups.len() != before;
        }
        changed
    }

    /// Selects or clears one smart group of the filter.
    pub fn toggle_smart_group(&mut self, id: &str) {
        if !self.smart_groups.remove(id) {
            self.smart_groups.insert(id.to_owned());
        }
    }

    /// Selects or clears one completion status of the filter.
    pub fn toggle_status(&mut self, id: &str) {
        if !self.statuses.remove(id) {
            self.statuses.insert(id.to_owned());
        }
    }

    /// Everything a view stores: the filters, the sort, the grouping and
    /// "Show hidden games".
    pub fn view_filters(&self) -> ViewFilters {
        ViewFilters {
            install_filter: self.install_filter,
            sort: self.sort,
            sort_descending: self.sort_descending,
            only: self.only,
            age: self.age,
            facet_filters: self.facet_filters.clone(),
            unknown_facets: self.unknown_facets.clone(),
            categories: self.categories.clone(),
            statuses: self.statuses.clone(),
            smart_groups: self.smart_groups.clone(),
            group_by: Some(self.group_by),
            show_hidden: Some(self.show_hidden),
        }
    }

    /// Sets what `view` stores (a view saved without the grouping or
    /// "Show hidden games" leaves them as they are).
    pub fn apply_view(&mut self, view: &ViewFilters) {
        self.install_filter = view.install_filter;
        self.sort = view.sort;
        self.sort_descending = view.sort_descending;
        self.only = view.only;
        self.age = view.age;
        self.facet_filters = view.facet_filters.clone();
        self.unknown_facets = view.unknown_facets.clone();
        self.categories = view.categories.clone();
        self.statuses = view.statuses.clone();
        self.smart_groups = view.smart_groups.clone();
        if let Some(by) = view.group_by {
            self.group_by = by;
        }
        if let Some(on) = view.show_hidden {
            self.show_hidden = on;
        }
    }

    /// Whether the Collection shows exactly `view` now.
    pub fn shows_view(&self, view: &ViewFilters) -> bool {
        self.install_filter == view.install_filter
            && self.sort == view.sort
            && self.sort_descending == view.sort_descending
            && self.only == view.only
            && self.age == view.age
            && self.categories == view.categories
            && self.statuses == view.statuses
            && self.smart_groups == view.smart_groups
            && self.facet_filters.same_as(&view.facet_filters)
            && view.group_by.is_none_or(|by| by == self.group_by)
            && view.show_hidden.is_none_or(|on| on == self.show_hidden)
    }

    /// Filters that hide games (install, "Only" switches, categories,
    /// statuses, smart groups and facet values), counted one per value.
    pub fn active_filters(&self) -> usize {
        usize::from(self.install_filter != InstallFilter::All)
            + usize::from(self.only.favourites)
            + usize::from(self.only.updates)
            + usize::from(self.only.offline)
            + usize::from(self.age.is_active())
            + self.categories.len()
            + self.statuses.len()
            + self.smart_groups.len()
            + self.facet_filters.len()
    }

    /// Clears every filter of [`active_filters`](Self::active_filters); the sort stays.
    pub fn clear_filters(&mut self) {
        self.install_filter = InstallFilter::All;
        self.only = OnlyFilters::default();
        self.age = AgeFilter::default();
        self.facet_filters = FacetFilters::default();
        self.unknown_facets.clear();
        self.categories.clear();
        self.statuses.clear();
        self.smart_groups.clear();
    }
}

/// What a view of the View menu sets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewFilters {
    pub install_filter: InstallFilter,
    pub sort: SortMode,
    pub sort_descending: bool,
    pub only: OnlyFilters,
    pub age: AgeFilter,
    pub facet_filters: FacetFilters,
    /// Facet kinds of `facetFilters` this version does not know (saved by a
    /// newer one), written back unchanged.
    pub unknown_facets: Map<String, Value>,
    pub categories: BTreeSet<String>,
    pub statuses: BTreeSet<String>,
    pub smart_groups: BTreeSet<String>,
    /// `None`: the view leaves it as it is.
    pub group_by: Option<GroupBy>,
    /// `None`: the view leaves it as it is.
    pub show_hidden: Option<bool>,
}

impl ViewFilters {
    /// Reads the keys [`UiState`] stores them under, leniently. A facet kind
    /// may hold one value (as Heronite stored it) or a list.
    pub fn from_value(value: &Value) -> Self {
        let mut v = Self::default();
        if let Some(f) = value.get("installFilter").and_then(Value::as_str).and_then(InstallFilter::parse) {
            v.install_filter = f;
        }
        if let Some(sort) = value.get("sort").and_then(Value::as_str).and_then(SortMode::parse) {
            v.sort = sort;
        }
        if let Some(obj) = value.get("facetFilters").and_then(Value::as_object) {
            v.unknown_facets = obj.iter().filter(|(k, _)| FacetKind::parse(k).is_none()).map(|(k, v)| (k.clone(), v.clone())).collect();
            for kind in FACET_KINDS {
                let values: Vec<&str> = match obj.get(kind.as_str()) {
                    Some(Value::String(s)) => vec![s.as_str()],
                    Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
                    _ => Vec::new(),
                };
                for value in values.into_iter().filter(|s| !s.trim().is_empty()) {
                    if !v.facet_filters.contains(kind, value) {
                        v.facet_filters = v.facet_filters.toggled(kind, value);
                    }
                }
            }
        }
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        v.sort_descending = flag("sortDescending");
        v.only = OnlyFilters { favourites: flag("onlyFavourites"), updates: flag("onlyUpdates"), offline: flag("onlyOffline") };
        v.age =
            AgeFilter { max: value.get("ageUpTo").and_then(Value::as_u64).and_then(|a| u8::try_from(a).ok()), unrated: flag("ageUnrated") };
        let names = |key: &str| -> BTreeSet<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)).collect())
                .unwrap_or_default()
        };
        v.categories = names("categories");
        v.statuses = names("statuses");
        v.smart_groups = names("smartGroups");
        // `groupByStatus` is how the grouping was stored when it was a toggle.
        v.group_by = value
            .get("groupBy")
            .and_then(Value::as_str)
            .and_then(GroupBy::parse)
            .or_else(|| value.get("groupByStatus").and_then(Value::as_bool).map(|on| if on { GroupBy::Status } else { GroupBy::None }));
        v.show_hidden = value.get("showHidden").and_then(Value::as_bool);
        v
    }

    pub fn to_value(&self) -> Value {
        // Every kind, as `from_value` reads them, so a new one cannot be forgotten here.
        let mut facets = self.unknown_facets.clone();
        facets.extend(FACET_KINDS.iter().map(|kind| (kind.as_str().to_owned(), json!(self.facet_filters.get(*kind)))));
        let mut doc = json!({
            "installFilter": self.install_filter,
            "facetFilters": facets,
            "sort": self.sort,
            "sortDescending": self.sort_descending,
            "onlyFavourites": self.only.favourites,
            "onlyUpdates": self.only.updates,
            "onlyOffline": self.only.offline,
            "ageUpTo": self.age.max,
            "ageUnrated": self.age.unrated,
            "categories": self.categories,
            "statuses": self.statuses,
            "groupBy": self.group_by.map(GroupBy::as_str),
            "showHidden": self.show_hidden,
        });
        // Only once a group is picked: files of users without groups stay as they were.
        if !self.smart_groups.is_empty() {
            doc["smartGroups"] = json!(self.smart_groups);
        }
        doc
    }
}

/// Key of [`UiState::category_looks`] in the file.
const CATEGORY_LOOKS: &str = "categoryLooks";

/// How a manual category shows: its colour (`None`: the one its place in
/// the list picks) and whether it is pinned to the Collection header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CategoryLook {
    /// `#rrggbb`.
    pub colour: Option<String>,
    pub pinned: bool,
    /// Keys a newer version wrote, kept as read.
    pub extra: Map<String, Value>,
}

impl CategoryLook {
    fn from_value(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let colour = obj.get("colour").and_then(Value::as_str).filter(|c| crate::smart_groups::is_hex_colour(c)).map(str::to_owned);
        let pinned = obj.get("pinned").and_then(Value::as_bool).unwrap_or(false);
        let extra = obj.iter().filter(|(k, _)| !matches!(k.as_str(), "colour" | "pinned")).map(|(k, v)| (k.clone(), v.clone())).collect();
        Some(Self { colour, pinned, extra })
    }

    fn to_value(&self) -> Value {
        let mut obj = self.extra.clone();
        if let Some(c) = &self.colour {
            obj.insert("colour".into(), json!(c));
        }
        if self.pinned {
            obj.insert("pinned".into(), json!(true));
        }
        Value::Object(obj)
    }

    /// Nothing to keep: no colour, not pinned, no unknown keys.
    pub fn is_plain(&self) -> bool {
        self.colour.is_none() && !self.pinned && self.extra.is_empty()
    }
}

/// A view the user saved from the View menu.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedView {
    pub name: String,
    pub filters: ViewFilters,
    /// Keys of the stored view this version does not read (saved by a newer
    /// one), written back unchanged.
    pub extra: Map<String, Value>,
}

/// Keys of a stored view that [`ViewFilters::from_value`] and the name cover;
/// `groupByStatus` is replaced by `groupBy` when written.
const VIEW_KEYS: [&str; 16] = [
    "name",
    "installFilter",
    "facetFilters",
    "sort",
    "sortDescending",
    "onlyFavourites",
    "onlyUpdates",
    "onlyOffline",
    "ageUpTo",
    "ageUnrated",
    "categories",
    "statuses",
    "smartGroups",
    "groupBy",
    "groupByStatus",
    "showHidden",
];

impl SavedView {
    pub fn new(name: impl Into<String>, filters: ViewFilters) -> Self {
        Self { name: name.into(), filters, extra: Map::new() }
    }

    fn extra_of(value: &Value) -> Map<String, Value> {
        let Some(obj) = value.as_object() else { return Map::new() };
        obj.iter().filter(|(k, _)| !VIEW_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn to_value(&self) -> Value {
        let mut obj = self.extra.clone();
        if let Value::Object(known) = self.filters.to_value() {
            obj.extend(known);
        }
        obj.insert("name".into(), Value::String(self.name.clone()));
        Value::Object(obj)
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

/// Keys of the mirrors of `console.json` (see [`UiState::console_open_in`]).
const CONSOLE_OPEN_IN: &str = "consoleOpenIn";
const CONSOLE_AUTO: &str = "consoleAuto";
const CONSOLE_FULLSCREEN: &str = "consoleFullscreen";
const CONSOLE_BUTTON: &str = "consoleButton";

/// Stores `value` under `key`, or removes the key for the default (`None`).
fn set_mirror(paths: &AppPaths, key: &str, value: Option<Value>) -> Result<()> {
    json_store::update_value(&paths.ui_state(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        let obj = doc.as_object_mut().expect("object");
        match value {
            Some(v) => obj.insert(key.to_owned(), v),
            None => obj.remove(key),
        };
        Ok(())
    })
}

/// Mirrors console.json's `openIn` into the UI state (the file is only
/// touched when the value differs from the default or is being reset).
pub fn set_console_open_in(paths: &AppPaths, value: OpenIn) -> Result<()> {
    set_mirror(paths, CONSOLE_OPEN_IN, (value != OpenIn::default()).then(|| value.as_str().into()))
}

/// Mirrors console.json's `autoConsole` into the UI state.
pub fn set_console_auto(paths: &AppPaths, value: AutoConsole) -> Result<()> {
    set_mirror(paths, CONSOLE_AUTO, (value != AutoConsole::default()).then(|| value.as_str().into()))
}

/// Mirrors console.json's `consoleButton` into the UI state.
pub fn set_console_button(paths: &AppPaths, value: ConsoleButton) -> Result<()> {
    set_mirror(paths, CONSOLE_BUTTON, (value != ConsoleButton::default()).then(|| value.as_str().into()))
}

/// Mirrors console.json's `fullscreen` into the UI state (stored only when off).
pub fn set_console_fullscreen(paths: &AppPaths, value: bool) -> Result<()> {
    set_mirror(paths, CONSOLE_FULLSCREEN, (!value).then_some(Value::Bool(false)))
}

/// Key of a notice the user dismissed with "Got it" ("Add game"'s store note).
pub const DISMISSED_NOTES: &str = "dismissedNotes";

/// The notice `id` was dismissed. Kept outside [`UiState`] so a copy of the
/// state held elsewhere does not write it back ([`UiState::save`] keeps keys
/// it does not model).
pub fn note_dismissed(paths: &AppPaths, id: &str) -> bool {
    json_store::read_value(&paths.ui_state())
        .ok()
        .and_then(|doc| doc.get(DISMISSED_NOTES)?.as_array().map(|list| list.iter().any(|v| v.as_str() == Some(id))))
        .unwrap_or(false)
}

/// Remembers that the notice `id` was dismissed.
pub fn dismiss_note(paths: &AppPaths, id: &str) -> Result<()> {
    json_store::update_value(&paths.ui_state(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        let obj = doc.as_object_mut().expect("object");
        let list = obj.entry(DISMISSED_NOTES).or_insert_with(|| json!([]));
        if !list.is_array() {
            *list = json!([]);
        }
        let list = list.as_array_mut().expect("array");
        if !list.iter().any(|v| v.as_str() == Some(id)) {
            list.push(Value::String(id.to_owned()));
        }
        Ok(())
    })
}

/// Key of the main window's last placement ([`WindowPlacement`]).
pub const WINDOW: &str = "window";

/// Where the main window was and whether it was maximized, so it reopens the
/// same way. In the system's px (the interface zoom left out), so changing
/// the zoom does not resize the window it reopens. The position and size are
/// the window's own while maximized (the ones restoring it goes back to).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl WindowPlacement {
    fn from_value(value: &Value) -> Option<Self> {
        let num = |k: &str| value.get(k)?.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
        let p = Self {
            x: num("x")?,
            y: num("y")?,
            width: num("width")?,
            height: num("height")?,
            maximized: value.get("maximized").and_then(Value::as_bool).unwrap_or(false),
        };
        (p.width >= 1. && p.height >= 1.).then_some(p)
    }

    fn to_value(self) -> Value {
        json!({
            "x": self.x.round(),
            "y": self.y.round(),
            "width": self.width.round(),
            "height": self.height.round(),
            "maximized": self.maximized,
        })
    }
}

/// The main window's last placement, if one was saved and is readable. Kept
/// outside [`UiState`] like [`DISMISSED_NOTES`], so a copy of the state held
/// elsewhere does not write an old one back.
pub fn window_placement(paths: &AppPaths) -> Option<WindowPlacement> {
    WindowPlacement::from_value(json_store::read_value(&paths.ui_state()).ok()?.get(WINDOW)?)
}

/// Remembers the main window's placement.
pub fn set_window_placement(paths: &AppPaths, placement: WindowPlacement) -> Result<()> {
    json_store::update_value(&paths.ui_state(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        doc.as_object_mut().expect("object").insert(WINDOW.to_owned(), placement.to_value());
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_with(name: &str, categories: &[&str], statuses: &[&str]) -> SavedView {
        let filters = ViewFilters {
            categories: categories.iter().map(|s| s.to_string()).collect(),
            statuses: statuses.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        SavedView::new(name, filters)
    }

    #[test]
    fn pruning_a_removed_status_reaches_the_saved_views() {
        let mut ui = UiState::default();
        ui.statuses.insert("gone".into());
        ui.statuses.insert("beaten".into());
        ui.saved_views = vec![view_with("A", &[], &["gone", "beaten"]), view_with("B", &[], &["beaten"])];
        assert!(ui.prune_statuses(|id| id != "gone"));
        assert_eq!(ui.statuses.iter().collect::<Vec<_>>(), ["beaten"]);
        assert_eq!(ui.saved_views[0].filters.statuses.iter().collect::<Vec<_>>(), ["beaten"]);
        assert!(!ui.prune_statuses(|id| id != "gone"));
    }

    #[test]
    fn pruning_a_removed_category_reaches_the_saved_views() {
        let mut ui = UiState { saved_views: vec![view_with("A", &["RPG", "Gone"], &[])], ..Default::default() };
        assert!(ui.prune_categories(|name| name != "Gone"));
        assert_eq!(ui.saved_views[0].filters.categories.iter().collect::<Vec<_>>(), ["RPG"]);
    }

    #[test]
    fn renaming_a_category_follows_into_saved_views_and_merges() {
        let mut ui = UiState::default();
        ui.categories.insert("Old".into());
        ui.saved_views = vec![view_with("A", &["Old", "New"], &[]), view_with("B", &["Other"], &[])];
        assert!(ui.rename_category("Old", "New"));
        assert_eq!(ui.categories.iter().collect::<Vec<_>>(), ["New"]);
        assert_eq!(ui.saved_views[0].filters.categories.iter().collect::<Vec<_>>(), ["New"]);
        assert_eq!(ui.saved_views[1].filters.categories.iter().collect::<Vec<_>>(), ["Other"]);
        assert!(!ui.rename_category("Old", "New"));
    }

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
    fn the_grid_is_the_view_until_another_is_picked() {
        assert!(UiState::default().grid_view);
        assert!(UiState::from_value(&json!({})).grid_view);
        assert!(!UiState::from_value(&json!({"gridView": false})).grid_view);
    }

    #[test]
    fn the_grid_label_follows_the_old_titles_switch() {
        assert_eq!(UiState::from_value(&json!({})).grid_label, GridLabel::Info);
        assert_eq!(UiState::from_value(&json!({"gridTitles": true})).grid_label, GridLabel::TitleInfo);
        assert_eq!(UiState::from_value(&json!({"gridTitles": true, "gridLabel": "none"})).grid_label, GridLabel::Nothing);
        assert!(UiState::from_value(&json!({})).grid_open_on_select);
        // The age ratings start off; a stored list (even empty) is kept.
        assert!(UiState::from_value(&json!({})).detail_rows_hidden.contains("ageRating"));
        assert!(UiState::from_value(&json!({ "detailRowsHidden": [] })).detail_rows_hidden.is_empty());
        assert_eq!(UiState::from_value(&json!({ "detailRowsHidden": ["notes"] })).detail_rows_hidden.iter().collect::<Vec<_>>(), ["notes"]);
        assert_eq!(UiState::from_value(&json!({"focusCoverSize": "huge"})).focus_cover_size, FocusCoverSize::Medium);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState {
            install_filter: InstallFilter::Installed,
            sort: SortMode::ReleaseDate,
            sidebar_hidden: true,
            focused_game: Some("sideload_steam_400".into()),
            group_by: GroupBy::Year,
            show_hidden: true,
            console_mode: true,
            palette_recent: vec!["go:downloads".into(), "game:sideload_steam_400:primary".into()],
            sort_descending: true,
            only: OnlyFilters { favourites: true, updates: false, offline: true },
            age: AgeFilter { max: Some(12), unrated: true },
            grid_view: false,
            grid_cover_w: 180.,
            grid_label: GridLabel::Title,
            grid_open_on_select: false,
            focus_list: true,
            focus_cover_size: FocusCoverSize::Large,
            list_column_w: Some(560.),
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
        assert_eq!(loaded.facet_filters.genre, ["RPG"]);
        assert_eq!(loaded.collapsed_groups.iter().collect::<Vec<_>>(), ["ungrouped"]);
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["sort"], "releaseDate");
        assert_eq!((on_disk["ageUpTo"].as_u64(), on_disk["ageUnrated"].as_bool()), (Some(12), Some(true)));
        assert_eq!(on_disk["groupBy"], "year");
        assert_eq!(on_disk["installFilter"], "installed");
        assert_eq!(on_disk["showHidden"], true);
        assert_eq!(on_disk["consoleMode"], true);
        assert_eq!(on_disk["onlyOffline"], true);
        assert_eq!(on_disk["gridView"], false);
        assert_eq!(on_disk["focusList"], true);
        assert_eq!(on_disk["listColumnWidth"], 560.);
        assert_eq!(on_disk["gridDrawerWidth"], Value::Null);
        assert_eq!(on_disk["categories"], json!([crate::categories::UNCATEGORIZED, "RPG"]));
        state.clear_facets();
        assert!(!state.facet_filters.is_active());
    }

    #[test]
    fn console_start_options_are_mirrored_and_never_written_back_by_save() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert_eq!(UiState::load(&paths).console_open_in, OpenIn::Last);
        assert_eq!(UiState::load(&paths).console_auto, AutoConsole::Off);
        assert!(UiState::load(&paths).console_fullscreen);
        set_console_open_in(&paths, OpenIn::Console).unwrap();
        set_console_auto(&paths, AutoConsole::Ask).unwrap();
        set_console_fullscreen(&paths, false).unwrap();
        assert_eq!(UiState::load(&paths).console_button, ConsoleButton::Auto);
        set_console_button(&paths, ConsoleButton::Bumpers).unwrap();
        let stale = UiState::default();
        let loaded = UiState::load(&paths);
        assert_eq!((loaded.console_open_in, loaded.console_auto), (OpenIn::Console, AutoConsole::Ask));
        assert!(!loaded.console_fullscreen);
        assert_eq!(loaded.console_button, ConsoleButton::Bumpers);
        // A copy loaded before the change saves without undoing it.
        stale.save(&paths).unwrap();
        assert_eq!(UiState::load(&paths).console_open_in, OpenIn::Console);
        set_console_open_in(&paths, OpenIn::Last).unwrap();
        set_console_auto(&paths, AutoConsole::Off).unwrap();
        set_console_fullscreen(&paths, true).unwrap();
        set_console_button(&paths, ConsoleButton::Auto).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert!(on_disk.get(CONSOLE_OPEN_IN).is_none() && on_disk.get(CONSOLE_AUTO).is_none());
        assert!(on_disk.get(CONSOLE_FULLSCREEN).is_none() && on_disk.get(CONSOLE_BUTTON).is_none());
    }

    #[test]
    fn facet_filters_read_one_value_or_a_list() {
        let state = UiState::from_value(&json!({ "facetFilters": { "genre": "RPG", "developer": ["Valve", "valve ", "", "Remedy"] } }));
        assert_eq!(state.facet_filters.genre, ["RPG"]);
        assert_eq!(state.facet_filters.developer, ["Valve", "Remedy"]);
        assert_eq!(state.to_value()["facetFilters"]["genre"], json!(["RPG"]));
    }

    #[test]
    fn every_facet_kind_survives_a_restart_and_a_saved_view() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState::default();
        for kind in FACET_KINDS {
            state.toggle_facet(kind, &format!("{} one", kind.as_str()));
            state.toggle_facet(kind, &format!("{} two", kind.as_str()));
        }
        state.toggle_facet(FacetKind::Theme, "Horror");
        state.toggle_facet(FacetKind::Feature, "Co-op");
        state.saved_views.push(SavedView::new("All", state.view_filters()));
        state.save(&paths).unwrap();

        let loaded = UiState::load(&paths);
        assert_eq!(loaded.facet_filters, state.facet_filters);
        assert_eq!(loaded.facet_filters.theme, ["theme one", "theme two", "Horror"]);
        assert_eq!(loaded.facet_filters.feature, ["feature one", "feature two", "Co-op"]);
        assert_eq!(loaded.saved_views[0].filters.facet_filters, state.facet_filters);
        for kind in FACET_KINDS {
            assert_eq!(loaded.facet_filters.get(kind).len(), state.facet_filters.get(kind).len(), "{}", kind.as_str());
        }
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["facetFilters"].as_object().unwrap().len(), FACET_KINDS.len());
    }

    #[test]
    fn views_save_apply_and_match() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState { group_by: GroupBy::None, show_hidden: true, ..Default::default() };
        state.toggle_status("beaten");
        state.toggle_facet(FacetKind::Genre, "RPG");
        state.toggle_facet(FacetKind::Genre, "Indie");
        state.only.favourites = true;
        state.sort = SortMode::Playtime;
        assert_eq!(state.active_filters(), 4);
        state.saved_views.push(SavedView::new("Done RPGs", state.view_filters()));
        state.save(&paths).unwrap();
        let mut loaded = UiState::load(&paths);
        assert_eq!(loaded, state);
        let view = loaded.saved_views[0].filters.clone();
        loaded.clear_filters();
        assert_eq!(loaded.active_filters(), 0);
        assert_eq!(loaded.sort, SortMode::Playtime);
        loaded.group_by = GroupBy::Status;
        loaded.show_hidden = false;
        assert!(!loaded.shows_view(&view));
        loaded.apply_view(&view);
        assert!(loaded.shows_view(&view));
        assert!(loaded.group_by == GroupBy::None && loaded.show_hidden, "a view sets grouping and hidden games");
        // the same values in another order still match
        loaded.facet_filters = loaded.facet_filters.toggled(FacetKind::Genre, "rpg").toggled(FacetKind::Genre, "RPG");
        assert!(loaded.shows_view(&view));
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["savedViews"][0]["name"], "Done RPGs");
        assert_eq!(on_disk["savedViews"][0]["statuses"], json!(["beaten"]));
        assert_eq!(on_disk["statuses"], json!(["beaten"]));
        // a view saved without them leaves both alone
        let old = ViewFilters::from_value(&json!({ "sort": "playtime" }));
        loaded.apply_view(&old);
        assert!(loaded.group_by == GroupBy::None && loaded.show_hidden);
        assert!(loaded.shows_view(&old), "matches whatever the grouping");
        // the grouping stored as the old toggle
        assert_eq!(UiState::from_value(&json!({ "groupByStatus": false })).group_by, GroupBy::None);
        assert_eq!(UiState::from_value(&json!({ "groupByStatus": false, "groupBy": "genre" })).group_by, GroupBy::Genre);
        assert_eq!(UiState::from_value(&json!({})).group_by, GroupBy::Status);
    }

    #[test]
    fn smart_groups_filter_save_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState::default();
        state.save(&paths).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert!(on_disk.get("smartGroups").is_none(), "no key until a group is picked");
        state.toggle_smart_group("g1");
        state.toggle_smart_group("g2");
        assert_eq!(state.active_filters(), 2);
        state.saved_views.push(SavedView::new("Groups", state.view_filters()));
        state.save(&paths).unwrap();
        let mut loaded = UiState::load(&paths);
        assert_eq!(loaded, state);
        assert!(loaded.shows_view(&loaded.saved_views[0].filters.clone()));
        assert!(loaded.prune_smart_groups(|id| id == "g2"));
        assert_eq!(loaded.smart_groups, BTreeSet::from(["g2".to_owned()]));
        assert_eq!(loaded.saved_views[0].filters.smart_groups, BTreeSet::from(["g2".to_owned()]));
        assert!(!loaded.prune_smart_groups(|_| true));
        loaded.clear_filters();
        assert_eq!(loaded.active_filters(), 0);
        // No group picked when the app closed: none when it starts again.
        loaded.save(&paths).unwrap();
        assert!(UiState::load(&paths).smart_groups.is_empty());
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert!(on_disk.get("smartGroups").is_none());
    }

    #[test]
    fn grid_cover_width_is_clamped() {
        let state = UiState::from_value(&json!({ "gridCoverWidth": 9000 }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.1);
        let state = UiState::from_value(&json!({ "gridCoverWidth": "big" }));
        assert_eq!(state.grid_cover_w, GRID_COVER_W.2);
    }

    #[test]
    fn game_mode_filter_and_grouping_survive_a_restart_and_a_saved_view() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let mut state = UiState { group_by: GroupBy::GameMode, ..Default::default() };
        state.toggle_facet(FacetKind::GameMode, "Co-operative");
        state.toggle_facet(FacetKind::GameMode, "Split screen");
        state.saved_views.push(SavedView::new("Couch co-op", state.view_filters()));
        state.save(&paths).unwrap();

        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["facetFilters"]["gameMode"], json!(["Co-operative", "Split screen"]));
        assert_eq!(on_disk["groupBy"], "gameMode");
        assert_eq!(on_disk["savedViews"][0]["facetFilters"]["gameMode"], json!(["Co-operative", "Split screen"]));
        assert_eq!(on_disk["savedViews"][0]["groupBy"], "gameMode");

        let mut loaded = UiState::load(&paths);
        assert_eq!(loaded.facet_filters.game_mode, ["Co-operative", "Split screen"]);
        assert_eq!(loaded.group_by, GroupBy::GameMode);
        loaded.clear_filters();
        loaded.group_by = GroupBy::Status;
        let view = loaded.saved_views[0].filters.clone();
        loaded.apply_view(&view);
        assert_eq!(loaded.facet_filters.game_mode, ["Co-operative", "Split screen"]);
        assert_eq!(loaded.group_by, GroupBy::GameMode);
        assert!(loaded.shows_view(&view));
    }

    /// A file written by v1.2.0, before the Game mode facet: every filter and
    /// view reads as it was, and writing it back changes none of its values.
    #[test]
    fn presets_from_before_game_modes_load_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let facets = json!({
            "series": [], "developer": ["Valve"], "publisher": [], "genre": ["Puzzle", "Shooter"],
            "platform": [], "store": ["Steam"], "theme": ["Sci-fi"], "feature": ["Co-op"]
        });
        let old = json!({
            "installFilter": "installed",
            "facetFilters": facets,
            "sort": "playtime",
            "groupBy": "genre",
            "savedViews": [{ "name": "Valve", "installFilter": "all", "facetFilters": facets, "sort": "title", "groupBy": "developer", "showHidden": false }],
        });
        std::fs::write(paths.ui_state(), serde_json::to_string(&old).unwrap()).unwrap();

        let state = UiState::load(&paths);
        assert_eq!(state.facet_filters.developer, ["Valve"]);
        assert_eq!(state.facet_filters.genre, ["Puzzle", "Shooter"]);
        assert_eq!(state.facet_filters.feature, ["Co-op"]);
        assert!(state.facet_filters.game_mode.is_empty());
        assert!(state.unknown_facets.is_empty(), "no old kind is mistaken for an unknown one");
        assert_eq!(state.group_by, GroupBy::Genre);
        let view = &state.saved_views[0];
        assert_eq!(view.filters.facet_filters, state.facet_filters);
        assert_eq!(view.filters.group_by, Some(GroupBy::Developer));
        assert!(view.extra.is_empty());

        state.save(&paths).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        for (kind, values) in facets.as_object().unwrap() {
            assert_eq!(&on_disk["facetFilters"][kind], values, "{kind}");
            assert_eq!(&on_disk["savedViews"][0]["facetFilters"][kind], values, "{kind}");
        }
        assert_eq!(on_disk["facetFilters"]["gameMode"], json!([]));
        assert_eq!(on_disk["groupBy"], "genre");
        assert_eq!(on_disk["savedViews"][0]["groupBy"], "developer");
        assert_eq!(UiState::load(&paths), state);
    }

    /// Facet kinds and view keys written by a newer version survive a save.
    #[test]
    fn unknown_facet_kinds_and_view_keys_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let newer = json!({
            "facetFilters": { "gameMode": ["Co-operative"], "perspective": ["First person"] },
            "savedViews": [{
                "name": "Future",
                "facetFilters": { "genre": ["RPG"], "perspective": ["Isometric"] },
                "thenSortBy": "rating",
            }],
        });
        std::fs::write(paths.ui_state(), serde_json::to_string(&newer).unwrap()).unwrap();

        let mut state = UiState::load(&paths);
        assert_eq!(state.facet_filters.game_mode, ["Co-operative"]);
        state.toggle_facet(FacetKind::GameMode, "Split screen");
        state.saved_views[0].name = "Renamed".into();
        state.save(&paths).unwrap();

        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["facetFilters"]["perspective"], json!(["First person"]));
        assert_eq!(on_disk["facetFilters"]["gameMode"], json!(["Co-operative", "Split screen"]));
        let view = &on_disk["savedViews"][0];
        assert_eq!(view["name"], "Renamed");
        assert_eq!(view["facetFilters"]["perspective"], json!(["Isometric"]));
        assert_eq!(view["facetFilters"]["genre"], json!(["RPG"]));
        assert_eq!(view["thenSortBy"], "rating");
    }

    #[test]
    fn clearing_resets_game_modes_and_unknown_kinds() {
        let state = UiState::from_value(&json!({
            "facetFilters": { "gameMode": ["Co-operative"], "genre": ["RPG"], "perspective": ["First person"] },
        }));
        assert_eq!(state.active_filters(), 2);

        let mut cleared = state.clone();
        cleared.clear_facets();
        assert!(cleared.facet_filters.game_mode.is_empty() && !cleared.facet_filters.is_active());
        assert!(cleared.unknown_facets.is_empty());
        assert_eq!(cleared.active_filters(), 0);

        let mut cleared = state.clone();
        cleared.clear_filters();
        assert!(cleared.facet_filters.game_mode.is_empty() && cleared.unknown_facets.is_empty());
        assert_eq!(
            cleared.to_value()["facetFilters"],
            json!({
                "store": [], "series": [], "developer": [], "publisher": [], "genre": [], "theme": [], "feature": [], "gameMode": [], "tag": [], "platform": []
            })
        );

        let mut toggled = state;
        toggled.toggle_facet(FacetKind::GameMode, " co-operative ");
        assert!(toggled.facet_filters.game_mode.is_empty(), "toggling the selected value clears it");
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

    #[test]
    fn dismissed_notes_survive_a_save_of_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert!(!note_dismissed(&paths, "add-game-stores"));
        dismiss_note(&paths, "add-game-stores").unwrap();
        dismiss_note(&paths, "add-game-stores").unwrap();
        UiState::default().save(&paths).unwrap();
        assert!(note_dismissed(&paths, "add-game-stores"));
        assert!(!note_dismissed(&paths, "other"));
        assert_eq!(json_store::read_value(&paths.ui_state()).unwrap()[DISMISSED_NOTES], json!(["add-game-stores"]));
    }

    #[test]
    fn the_window_placement_round_trips_and_survives_a_save_of_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert_eq!(window_placement(&paths), None);
        let p = WindowPlacement { x: -1920., y: 40., width: 1600., height: 1000., maximized: true };
        set_window_placement(&paths, p).unwrap();
        UiState::default().save(&paths).unwrap();
        assert_eq!(window_placement(&paths), Some(p));
        // Unusable sizes or missing fields are no placement.
        std::fs::write(paths.ui_state(), r#"{"window":{"x":0,"y":0,"width":0,"height":600}}"#).unwrap();
        assert_eq!(window_placement(&paths), None);
        std::fs::write(paths.ui_state(), r#"{"window":{"x":0,"width":800,"height":600}}"#).unwrap();
        assert_eq!(window_placement(&paths), None);
    }

    /// Colours and pins of manual categories: written under `categoryLooks`,
    /// unknown keys (in a look and beside it) kept, plain looks left out.
    #[test]
    fn category_looks_round_trip_and_keep_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let newer = json!({
            "futureKey": { "a": 1 },
            "categoryLooks": {
                "RPG": { "colour": "#d55181", "pinned": true, "icon": "sword" },
                "Co-op": { "colour": "not a colour" },
                "": { "pinned": true },
            },
        });
        std::fs::write(paths.ui_state(), serde_json::to_string(&newer).unwrap()).unwrap();

        let mut state = UiState::load(&paths);
        let rpg = state.category_look("RPG");
        assert_eq!(rpg.colour.as_deref(), Some("#d55181"));
        assert!(rpg.pinned);
        assert_eq!(rpg.extra["icon"], "sword");
        assert!(state.category_look("Co-op").is_plain(), "a bad colour reads as none");
        assert!(!state.category_looks.contains_key(""), "nameless dropped");

        state.set_category_look("Kids", |l| l.colour = Some("#199e70".into()));
        state.set_category_look("RPG", |l| l.pinned = false);
        state.save(&paths).unwrap();
        let on_disk = json_store::read_value(&paths.ui_state()).unwrap();
        assert_eq!(on_disk["futureKey"], json!({ "a": 1 }));
        assert_eq!(on_disk["categoryLooks"], json!({ "RPG": { "colour": "#d55181", "icon": "sword" }, "Kids": { "colour": "#199e70" } }));
        assert_eq!(UiState::load(&paths).category_looks, state.category_looks);

        // A rename carries the look; a removed category loses it.
        assert!(state.rename_category("Kids", "Family"));
        assert_eq!(state.category_look("Family").colour.as_deref(), Some("#199e70"));
        assert!(!state.prune_categories(|n| n != "Family"), "pruning the filter keeps the looks");
        // A merge keeps the colour merged into, and a pin of either.
        state.set_category_look("Family", |l| l.pinned = true);
        assert!(state.rename_category("Family", "RPG"));
        let merged = state.category_look("RPG");
        assert_eq!(merged.colour.as_deref(), Some("#d55181"));
        assert!(merged.pinned, "the merged category's pin stays");
        assert!(!state.category_looks.contains_key("Family"));
        state.set_category_look("RPG", |l| l.pinned = false);
        state.set_category_look("Family", |l| l.colour = Some("#199e70".into()));
        assert!(state.forget_category("Family"));
        state.set_category_look("RPG", |l| {
            l.colour = None;
            l.extra.clear();
        });
        state.save(&paths).unwrap();
        assert!(json_store::read_value(&paths.ui_state()).unwrap().get("categoryLooks").is_none(), "no looks, no key");
    }

    /// A file from before category looks reads with none, and saving it adds
    /// no key.
    #[test]
    fn old_files_have_no_category_looks() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.ui_state(), r#"{"categories":["RPG"],"sort":"title"}"#).unwrap();
        let state = UiState::load(&paths);
        assert!(state.category_looks.is_empty());
        assert!(state.category_look("RPG").is_plain());
        state.save(&paths).unwrap();
        assert!(json_store::read_value(&paths.ui_state()).unwrap().get("categoryLooks").is_none());
    }
}
