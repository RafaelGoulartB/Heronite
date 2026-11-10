//! Smart groups in the Collection (`heronite_data::smart_groups`): a picked
//! group filters the list like a category (the games of any picked group),
//! the View menu lists them, the editor makes and changes them, and the
//! pinned ones sit in the header. Nothing of it shows until the user makes a
//! group.

use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;

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

/// The console filter's `group` (0 = any, else `groups[group - 1]`) of
/// `old` in `new`: the same group at its new place, or 0 when it is gone.
fn follow_group(group: usize, old: &[SmartGroup], new: &[SmartGroup]) -> usize {
    let id = group.checked_sub(1).and_then(|i| old.get(i)).map(|g| &g.id);
    id.and_then(|id| new.iter().position(|g| &g.id == id)).map_or(0, |i| i + 1)
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
            today: self.group_today.get().unwrap_or_else(|| chrono::Local::now().date_naive()),
        }
    }

    /// The game is in one of the picked categories, manual or smart (or
    /// none is picked): they share one section, so they add up as the
    /// manual ones do among themselves.
    pub(super) fn in_picked_categories(&self, g: &CollectionGame) -> bool {
        let (manual, smart) = (&self.ui.categories, &self.ui.smart_groups);
        if manual.is_empty() && smart.is_empty() {
            return true;
        }
        (!manual.is_empty() && self.categories.matches(manual, &g.runner, &g.app_name))
            || smart.iter().filter_map(|id| self.smart_groups.get(id)).any(|group| self.group_members(group).contains(&g.key))
    }

    /// Keys of the library's games in `group`, worked out once until the
    /// list is built again (for its count, the covers and Group by).
    pub(super) fn group_members(&self, group: &SmartGroup) -> Rc<HashSet<String>> {
        if let Some(keys) = self.group_members.borrow().get(&group.id) {
            return keys.clone();
        }
        let Some(lib) = self.lib.as_deref() else { return Rc::default() };
        let facts = self.group_facts(&lib.statuses);
        let keys: Rc<HashSet<String>> = Rc::new(lib.games.iter().filter(|g| group.matches(g, &facts)).map(|g| g.key.clone()).collect());
        self.group_members.borrow_mut().insert(group.id.clone(), keys.clone());
        keys
    }

    /// Games of the library in `group`.
    pub(super) fn group_count(&self, group: &SmartGroup) -> usize {
        self.group_members(group).len()
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
        // Console mode's Filters panel keeps a group by its place in the
        // list: follow the picked group to its new place (gone: any group).
        for nav in self.console.tab_navs.iter_mut() {
            nav.grid.filters.group = follow_group(nav.grid.filters.group, &self.smart_groups.groups, &groups.groups);
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

    fn write_groups(&mut self, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        if let Err(err) = result {
            self.show_toast(tr!("Could not save the smart categories: {err}", err = format!("{err:#}")), true, cx);
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

#[cfg(test)]
mod tests {
    use super::{follow_group, Rule, SmartGroup};

    fn groups(ids: &[&str]) -> Vec<SmartGroup> {
        ids.iter().map(|id| SmartGroup::new(*id, *id, Rule::default())).collect()
    }

    #[test]
    fn the_console_filter_follows_its_group_through_edits() {
        let old = groups(&["a", "b", "c"]);
        assert_eq!(follow_group(2, &old, &groups(&["new", "a", "b", "c"])), 3, "a group added before it");
        assert_eq!(follow_group(2, &old, &groups(&["b", "c"])), 1, "the one before it deleted");
        assert_eq!(follow_group(2, &old, &groups(&["a", "c"])), 0, "it was deleted");
        assert_eq!(follow_group(0, &old, &groups(&["a"])), 0, "any group stays");
    }
}
