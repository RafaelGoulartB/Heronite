//! Views: a set of filters and a sort applied in one click. A few come
//! built in (by status slug, so they follow renamed statuses); the user
//! saves more from the menu.

use std::collections::BTreeSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use heronite_data::status::status_label;
use heronite_data::ui_state::{SavedView, ViewFilters};
use heronite_data::{InstallFilter, SortMode};
use heronite_i18n::{tr, trn};

use super::super::CollectionView;
use crate::icons::Icon;
use crate::theme;

pub(super) struct ViewCard {
    pub name: String,
    pub sub: String,
    pub icon: Icon,
    pub color: Rgba,
    pub filters: ViewFilters,
    /// Index in `UiState::saved_views` (built-in views have none).
    pub saved: Option<usize>,
}

fn sort_label(sort: SortMode) -> &'static str {
    match sort {
        SortMode::Title => tr!("Title"),
        SortMode::LastPlayed => tr!("Last played"),
        SortMode::Playtime => tr!("Playtime"),
    }
}

impl CollectionView {
    pub(super) fn view_cards(&self) -> Vec<ViewCard> {
        let statuses = self.lib.as_deref().map(|l| l.statuses.as_slice()).unwrap_or(&[]);
        // The statuses of `slugs` that exist: (ids, names joined with " + ").
        let pick = |slugs: &[&str]| -> (BTreeSet<String>, String) {
            let found: Vec<_> = slugs.iter().filter_map(|slug| statuses.iter().find(|s| s.slug == *slug)).collect();
            let names = found.iter().map(|s| status_label(&s.name)).collect::<Vec<_>>().join(" + ");
            (found.into_iter().map(|s| s.id.clone()).collect(), names)
        };
        let mut out = Vec::new();
        let mut by_status = |slugs: &[&str], name: &str, sub: &dyn Fn(&str) -> String, icon: Icon, base: ViewFilters| {
            let (ids, names) = pick(slugs);
            if !ids.is_empty() {
                out.push(ViewCard {
                    name: name.into(),
                    sub: sub(&names),
                    icon,
                    color: theme::status_color(slugs[0]),
                    filters: ViewFilters { statuses: ids, ..base },
                    saved: None,
                });
            }
        };
        by_status(
            &["playing", "endless"],
            tr!("Playing now"),
            &|n| n.to_string(),
            Icon::History,
            ViewFilters { sort: SortMode::LastPlayed, ..Default::default() },
        );
        by_status(
            &["plan-to-play", "not-played"],
            tr!("Backlog"),
            &|n| tr!("{statuses}, installed", statuses = n).to_string(),
            Icon::Flag,
            ViewFilters { install_filter: InstallFilter::Installed, ..Default::default() },
        );
        by_status(
            &["beaten", "completed"],
            tr!("Finished"),
            &|n| n.to_string(),
            Icon::Check,
            ViewFilters { sort: SortMode::Playtime, ..Default::default() },
        );
        let mut favourites = ViewFilters { sort: SortMode::Playtime, ..Default::default() };
        favourites.only.favourites = true;
        out.push(ViewCard {
            name: tr!("Favourites").into(),
            sub: tr!("Most played first").into(),
            icon: Icon::Star,
            color: theme::status_color("completed"),
            filters: favourites,
            saved: None,
        });
        for (i, v) in self.ui.saved_views.iter().enumerate() {
            let f = &v.filters;
            let n = usize::from(f.install_filter != InstallFilter::All)
                + usize::from(f.only.favourites)
                + usize::from(f.only.updates)
                + usize::from(f.only.offline)
                + f.categories.len()
                + f.statuses.len()
                + f.facet_filters.len();
            out.push(ViewCard {
                name: v.name.clone(),
                sub: format!("{} · {}", sort_label(f.sort), trn!(n, "{n} filter", "{n} filters")),
                icon: Icon::Tune,
                color: theme::accent(),
                filters: f.clone(),
                saved: Some(i),
            });
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
        let name = input.read(cx).value().trim().to_string();
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
