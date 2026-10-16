//! Views: a set of filters and a sort applied in one click. A few come
//! built in (by status slug, so they follow renamed statuses); the user
//! saves more from the menu.

use std::collections::BTreeSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use heronite_data::ui_state::{SavedView, ViewFilters};
use heronite_data::{InstallFilter, SortMode};
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::theme;

pub(super) struct ViewCard {
    pub name: String,
    /// Dot before the name: the first status of a built-in view.
    pub color: Rgba,
    pub filters: ViewFilters,
    /// Index in `UiState::saved_views` (built-in views have none).
    pub saved: Option<usize>,
}

impl CollectionView {
    pub(super) fn view_cards(&self) -> Vec<ViewCard> {
        let statuses = self.lib.as_deref().map(|l| l.statuses.as_slice()).unwrap_or(&[]);
        let mut out = Vec::new();
        // A built-in view of the statuses of `slugs` that exist (none: no view).
        let mut by_status = |slugs: &[&str], name: &str, base: ViewFilters| {
            let ids: BTreeSet<String> =
                slugs.iter().filter_map(|slug| statuses.iter().find(|s| s.slug == *slug)).map(|s| s.id.clone()).collect();
            if !ids.is_empty() {
                let filters = ViewFilters { statuses: ids, ..base };
                out.push(ViewCard { name: name.into(), color: theme::status_color(slugs[0]), filters, saved: None });
            }
        };
        by_status(&["playing", "endless"], tr!("Playing now"), ViewFilters { sort: SortMode::LastPlayed, ..Default::default() });
        by_status(
            &["plan-to-play", "not-played"],
            tr!("Backlog"),
            ViewFilters { install_filter: InstallFilter::Installed, ..Default::default() },
        );
        by_status(&["beaten", "completed"], tr!("Finished"), ViewFilters { sort: SortMode::Playtime, ..Default::default() });
        let mut favourites = ViewFilters { sort: SortMode::Playtime, ..Default::default() };
        favourites.only.favourites = true;
        out.push(ViewCard { name: tr!("Favourites").into(), color: theme::status_color("beaten"), filters: favourites, saved: None });
        for (i, v) in self.ui.saved_views.iter().enumerate() {
            out.push(ViewCard { name: v.name.clone(), color: theme::accent(), filters: v.filters.clone(), saved: Some(i) });
        }
        out
    }

    pub(super) fn apply_view_filters(&mut self, filters: &ViewFilters, cx: &mut Context<Self>) {
        self.ui.apply_view(filters);
        if self.ui.only.updates {
            self.update_apps = crate::services::downloads(cx).read(cx).update_apps();
        }
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// "Save current as a view": opens the name box.
    pub(super) fn start_naming_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(tr!("Name of the view"), window, cx);
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.save_named_view(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.view_menu.naming = Some((input, sub));
        cx.notify();
    }

    /// Saves the filters and sort under the typed name (a view with the same
    /// name is replaced).
    pub(super) fn save_named_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, _)) = self.view_menu.naming.as_ref() else { return };
        // Short enough for one pill of the menu.
        let name: String = input.read(cx).value().trim().chars().take(32).collect();
        if name.is_empty() {
            return;
        }
        let view = SavedView { name, filters: self.ui.view_filters() };
        let key = view.name.to_lowercase();
        match self.ui.saved_views.iter_mut().find(|v| v.name.to_lowercase() == key) {
            Some(old) => *old = view,
            None => self.ui.saved_views.push(view),
        }
        self.save_ui();
        self.view_menu.naming = None;
        self.focus_view_search(window, cx);
        cx.notify();
    }

    pub(super) fn delete_view(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.ui.saved_views.len() {
            self.ui.saved_views.remove(index);
            self.save_ui();
            cx.notify();
        }
    }
}
