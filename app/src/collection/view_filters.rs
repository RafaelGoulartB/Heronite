//! The View menu's sort direction, "Only" switches and custom categories,
//! the category edits of a game and the library fields manager. The menu itself is drawn in `header.rs`; the categories are
//! `heronite_data::categories`.

use gpui_kit::*;
use heronite_data::categories::{self, Categories, UNCATEGORIZED};
use heronite_data::field_names::Field;
use heronite_i18n::tr;

use super::category_kinds::CategoryRef;
use super::CollectionView;
use crate::dialogs::manage_fields::{ManageFieldsDialog, NamesChanged};
use crate::services::downloads::Downloads;

/// One of the View menu's "Only" switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Only {
    Favourites,
    Updates,
    Offline,
}

impl Only {
    pub const ALL: [Only; 3] = [Only::Favourites, Only::Updates, Only::Offline];

    pub fn label(self) -> &'static str {
        match self {
            Only::Favourites => tr!("Favourites"),
            Only::Updates => tr!("With an update"),
            Only::Offline => tr!("Run offline"),
        }
    }
}

/// How a selected category reads in the menu and its chip.
pub(super) fn category_label(name: &str) -> String {
    if name == UNCATEGORIZED {
        tr!("Uncategorized").to_string()
    } else {
        name.to_string()
    }
}

impl CollectionView {
    pub(super) fn only_on(&self, only: Only) -> bool {
        match only {
            Only::Favourites => self.ui.only.favourites,
            Only::Updates => self.ui.only.updates,
            Only::Offline => self.ui.only.offline,
        }
    }

    pub(super) fn set_only(&mut self, only: Only, on: bool, cx: &mut Context<Self>) {
        match only {
            Only::Favourites => self.ui.only.favourites = on,
            Only::Updates => self.ui.only.updates = on,
            Only::Offline => self.ui.only.offline = on,
        }
        if only == Only::Updates && on {
            self.update_apps = crate::services::downloads(cx).read(cx).update_apps();
        }
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    pub(super) fn toggle_sort_direction(&mut self, cx: &mut Context<Self>) {
        self.ui.sort_descending = !self.ui.sort_descending;
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// The store service changed: "Only with updates" follows its list.
    pub(super) fn sync_update_apps(&mut self, svc: &Entity<Downloads>, cx: &mut Context<Self>) {
        if !self.ui.only.updates && !self.groups_need_updates() {
            return;
        }
        let apps = svc.read(cx).update_apps();
        if apps != self.update_apps {
            self.update_apps = apps;
            self.refresh_groups();
        }
    }

    /// Selects or clears one category of the filter (several add up).
    pub(super) fn toggle_category_filter(&mut self, name: &str, cx: &mut Context<Self>) {
        self.ui.toggle_category(name);
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// Re-reads the categories after an edit (the library itself is unchanged).
    pub(super) fn reload_categories(&mut self, cx: &mut Context<Self>) {
        self.categories = Categories::load(&self.data_paths);
        self.prune_category_filter();
        self.refresh_groups();
        cx.notify();
    }

    /// A removed category no longer filters, nor does it in the saved views.
    pub(super) fn prune_category_filter(&mut self) {
        let cats = &self.categories;
        if self.ui.prune_categories(|n| n == UNCATEGORIZED || cats.exists(n)) {
            self.save_ui();
        }
    }

    /// Puts the game under `key` in `name`, or takes it out (every copy
    /// while they are grouped).
    pub(super) fn set_game_category(&mut self, key: &str, name: &str, on: bool, cx: &mut Context<Self>) {
        let games = self.with_copies(key);
        if games.is_empty() {
            return;
        }
        let writes: Vec<_> = games.iter().map(|g| (g.runner.clone(), g.app_name.clone(), on)).collect();
        if let Err(err) = categories::set_many_in_category(&self.data_paths, name, &writes) {
            self.show_toast(tr!("Could not change the categories: {err}", err = format!("{err:#}")), true, cx);
        }
        self.reload_categories(cx);
    }

    /// The categories manager: both kinds, with the manual ones' colours and
    /// pins and the smart ones' counts.
    pub(super) fn open_manage_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::dialogs::manage_categories::{CategoryAction, Looks, ManageCategoriesDialog, SmartRow};
        let Some(ctx) = self.dialog_ctx() else { return };
        self.open_menu = None;
        self.context_menu = None;
        let smart = self
            .smart_groups
            .groups
            .iter()
            .map(|g| SmartRow {
                id: g.id.clone(),
                name: g.name.clone(),
                colour: super::smart_groups::group_colour(g),
                pinned: g.pinned,
                games: self.group_count(g),
            })
            .collect();
        let looks = Looks { manual: self.ui.category_looks.clone(), smart };
        let view = cx.new(|cx| ManageCategoriesDialog::new(ctx, looks, window, cx));
        cx.subscribe_in(&view, window, |this, _, ev: &CategoryAction, window, cx| match ev {
            CategoryAction::Colour { name, colour } => this.set_manual_colour(name, colour.clone(), cx),
            CategoryAction::Pin { smart, key, pinned } => {
                let r = if *smart { CategoryRef::Smart(key.clone()) } else { CategoryRef::Manual(key.clone()) };
                this.set_category_pinned(&r, *pinned, cx);
            }
            CategoryAction::EditSmart(id) => this.open_group_editor(id.as_deref(), None, window, cx),
            CategoryAction::Removed(name) => {
                if this.ui.forget_category(name) {
                    this.save_ui();
                }
            }
        })
        .detach();
        // Before the reload that follows the rename prunes the old name.
        cx.subscribe(&view, |this, _, ev: &crate::dialogs::manage_categories::CategoryRenamed, cx| {
            if this.ui.rename_category(&ev.from, &ev.to) {
                this.save_ui();
            }
            this.rename_category_in_groups(&ev.from, &ev.to, cx);
        })
        .detach();
        self.open_dialog(view, cx);
    }

    /// The library fields manager, on `field` (companies by default). Its
    /// renames carry over to the View menu's filters and presets.
    pub(super) fn open_manage_fields(&mut self, field: Option<Field>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.open_menu = None;
        self.context_menu = None;
        let view = cx.new(|cx| ManageFieldsDialog::new(ctx, field, window, cx));
        cx.subscribe(&view, |this, _, ev: &NamesChanged, _| {
            if this.ui.rename_facet_values(ev.field.facet_kinds(), &ev.from, ev.to.as_deref()) {
                this.save_ui();
            }
        })
        .detach();
        self.open_dialog(view, cx);
    }
}
