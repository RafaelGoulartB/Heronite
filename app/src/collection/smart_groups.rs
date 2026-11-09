//! Smart groups in the Collection (`heronite_data::smart_groups`): a picked
//! group filters the list like a category (the games of any picked group),
//! the View menu lists them, the editor makes and changes them, and the
//! pinned ones sit in the header. Nothing of it shows until the user makes a
//! group.

use std::collections::BTreeSet;

use gpui_kit::*;
use heronite_data::models::CompletionStatus;
use heronite_data::smart_groups::{self, Facts, Field, Rule, SmartGroup, SmartGroups};
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::CollectionView;
use crate::dialogs::smart_group::{GroupCtx, GroupSaved, SmartGroupDialog};
use crate::theme;

/// The colour of a group's dot: its own, else the accent.
pub(super) fn group_colour(group: &SmartGroup) -> Rgba {
    group.colour.as_deref().and_then(theme::parse_hex).unwrap_or_else(theme::accent)
}

impl CollectionView {
    /// What the rules read, from what the Collection has loaded.
    pub(super) fn group_facts<'a>(&'a self, statuses: &'a [CompletionStatus]) -> Facts<'a> {
        Facts {
            organize: heronite_data::organize::Organize {
                extras: Some(&self.game_extras),
                ratings: self.facts.ratings(),
                categories: Some(&self.categories),
                statuses,
            },
            wiki: Some(&self.wiki),
            updates: Some(&self.update_apps),
            today: chrono::Local::now().date_naive(),
        }
    }

    /// The game is in one of the picked groups (or none is picked).
    pub(super) fn in_picked_groups(&self, g: &CollectionGame, statuses: &[CompletionStatus]) -> bool {
        if self.ui.smart_groups.is_empty() {
            return true;
        }
        let facts = self.group_facts(statuses);
        self.ui.smart_groups.iter().filter_map(|id| self.smart_groups.get(id)).any(|group| group.matches(g, &facts))
    }

    /// Games of the library in `group`.
    pub(super) fn group_count(&self, group: &SmartGroup) -> usize {
        let Some(lib) = self.lib.as_deref() else { return 0 };
        let facts = self.group_facts(&lib.statuses);
        lib.games.iter().filter(|g| group.matches(g, &facts)).count()
    }

    /// Some group asks whether a game has an update: keep the list current
    /// as "Only with an update" does.
    pub(super) fn groups_need_updates(&self) -> bool {
        self.smart_groups.groups.iter().any(|g| g.rule.uses(Field::HasUpdate))
    }

    /// Reads `groups.json` again (after an edit) and drops picked groups that
    /// are gone.
    pub(super) fn reload_smart_groups(&mut self, cx: &mut Context<Self>) {
        let groups = SmartGroups::load(&self.data_paths);
        // Console mode's Filters panel keeps a group by its place in the list.
        if groups.groups.iter().map(|g| &g.id).ne(self.smart_groups.groups.iter().map(|g| &g.id)) {
            self.console.tab_navs.iter_mut().for_each(|nav| nav.grid.filters.group = 0);
        }
        self.smart_groups = groups;
        self.prune_smart_group_filter();
        if self.groups_need_updates() {
            self.update_apps = crate::services::downloads(cx).read(cx).update_apps();
        }
        self.refresh_groups();
        cx.notify();
    }

    pub(super) fn prune_smart_group_filter(&mut self) {
        let groups = &self.smart_groups;
        if self.ui.prune_smart_groups(|id| groups.get(id).is_some()) {
            self.save_ui();
        }
    }

    /// Picks or drops one group of the filter (several add up).
    pub(super) fn toggle_smart_group_filter(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ui.toggle_smart_group(id);
        self.save_ui();
        self.refresh_groups();
        self.scroll_list_to_top();
        cx.notify();
    }

    /// Shows only the games of `id` (a click on a pinned group): picking the
    /// one already shown alone drops it.
    pub(super) fn show_only_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let alone = self.ui.smart_groups.len() == 1 && self.ui.smart_groups.contains(id);
        self.ui.smart_groups = if alone { BTreeSet::new() } else { BTreeSet::from([id.to_owned()]) };
        self.save_ui();
        self.refresh_groups();
        self.scroll_list_to_top();
        cx.notify();
    }

    fn write_groups(&mut self, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        if let Err(err) = result {
            self.show_toast(tr!("Could not save the groups: {err}", err = format!("{err:#}")), true, cx);
        }
        self.reload_smart_groups(cx);
    }

    pub(super) fn set_group_pinned(&mut self, id: &str, pinned: bool, cx: &mut Context<Self>) {
        let result = smart_groups::set_pinned(&self.data_paths, id, pinned);
        self.write_groups(result, cx);
    }

    /// Follows a category rename into the rules that name it.
    pub(super) fn rename_category_in_groups(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let mut groups = SmartGroups::load(&self.data_paths).groups;
        if smart_groups::rename_category(&mut groups, from, Some(to)) {
            let result = smart_groups::save(&self.data_paths, &groups);
            self.write_groups(result, cx);
        }
    }

    /// The editor on the group `id`, or on a new group of `rule` (the
    /// current filters, or none).
    pub(super) fn open_group_editor(&mut self, id: Option<&str>, rule: Option<Rule>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.open_menu = None;
        self.context_menu = None;
        let group = match id.and_then(|id| self.smart_groups.get(id)) {
            Some(g) => g.clone(),
            None => SmartGroup::new(self.smart_groups.new_id(), String::new(), rule.unwrap_or_default()),
        };
        let facts = GroupCtx {
            categories: self.categories.clone(),
            extras: self.game_extras.clone(),
            ratings: self.facts.ratings().cloned().unwrap_or_default(),
            wiki: self.wiki.clone(),
            updates: self.update_apps.clone(),
        };
        let is_new = self.smart_groups.get(&group.id).is_none();
        let view = cx.new(|cx| SmartGroupDialog::new(ctx, facts, group, is_new, window, cx));
        cx.subscribe(&view, |this, _, ev: &GroupSaved, cx| {
            this.reload_smart_groups(cx);
            if let Some(id) = &ev.show {
                this.ui.smart_groups = BTreeSet::from([id.clone()]);
                this.save_ui();
                this.refresh_groups();
                this.scroll_list_to_top();
            }
        })
        .detach();
        self.open_dialog(view, cx);
    }

    /// "Save as smart group…": the editor on a new group of the current filters.
    pub(super) fn save_filters_as_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rule = Rule::from_view(&self.ui.view_filters());
        self.open_group_editor(None, Some(rule), window, cx);
    }
}
