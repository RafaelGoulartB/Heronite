//! Categories of both kinds as one list: the manual ones
//! (`heronite_data::categories`, whose colour and pin live in
//! `UiState::category_looks`) and the smart ones (`heronite_data::smart_groups`,
//! which keep theirs in `groups.json`). The View menu, the header's pins,
//! the covers, Group by and the console read them from here.

use std::collections::BTreeSet;

use gpui_kit::*;
use heronite_data::organize::SmartSection;
use heronite_data::smart_groups::SmartGroup;
use heronite_data::CollectionGame;

use super::category_badges::{colour_of, CategoryBadge};
use super::smart_groups::group_colour;
use super::CollectionView;
use crate::theme;

/// One category: a manual one by name, a smart one by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::collection) enum CategoryRef {
    Manual(String),
    Smart(String),
}

/// A category as the lists show it.
#[derive(Clone, Debug)]
pub(in crate::collection) struct CategoryEntry {
    pub r: CategoryRef,
    pub name: String,
    pub colour: Rgba,
    pub pinned: bool,
}

impl CategoryEntry {
    pub fn smart(&self) -> bool {
        matches!(self.r, CategoryRef::Smart(_))
    }
}

impl CollectionView {
    /// The colour of the manual category `name`: its own, else the one its
    /// place among the categories picks (as the covers always did).
    pub(in crate::collection) fn manual_colour(&self, name: &str) -> Rgba {
        let own = self.ui.category_looks.get(name).and_then(|l| l.colour.as_deref()).and_then(theme::parse_hex);
        own.unwrap_or_else(|| colour_of(self.categories.names().position(|n| n == name).unwrap_or(0)))
    }

    /// Every category: the manual ones A-Z, then the smart ones in their order.
    pub(in crate::collection) fn category_entries(&self) -> Vec<CategoryEntry> {
        let manual = self.categories.names().map(|name| CategoryEntry {
            r: CategoryRef::Manual(name.to_owned()),
            name: name.to_owned(),
            colour: self.manual_colour(name),
            pinned: self.ui.category_looks.get(name).is_some_and(|l| l.pinned),
        });
        let smart = self.smart_groups.groups.iter().map(|g| CategoryEntry {
            r: CategoryRef::Smart(g.id.clone()),
            name: g.name.clone(),
            colour: group_colour(g),
            pinned: g.pinned,
        });
        manual.chain(smart).collect()
    }

    /// The categories pinned to the header, in the same order.
    pub(in crate::collection) fn pinned_categories(&self) -> Vec<CategoryEntry> {
        self.category_entries().into_iter().filter(|c| c.pinned).collect()
    }

    /// Games of the library in the category.
    pub(in crate::collection) fn category_count(&self, r: &CategoryRef) -> usize {
        match r {
            CategoryRef::Manual(name) => {
                let Some(lib) = self.lib.as_deref() else { return 0 };
                lib.games.iter().filter(|g| self.categories.contains(name, &g.runner, &g.app_name)).count()
            }
            CategoryRef::Smart(id) => self.smart_groups.get(id).map_or(0, |g| self.group_count(g)),
        }
    }

    /// The category filters the list now.
    pub(in crate::collection) fn category_on(&self, r: &CategoryRef) -> bool {
        match r {
            CategoryRef::Manual(name) => self.ui.categories.contains(name),
            CategoryRef::Smart(id) => self.ui.smart_groups.contains(id),
        }
    }

    /// Picks or drops one category of the filter (several add up).
    pub(in crate::collection) fn toggle_category_ref(&mut self, r: &CategoryRef, cx: &mut Context<Self>) {
        match r {
            CategoryRef::Manual(name) => self.toggle_category_filter(name, cx),
            CategoryRef::Smart(id) => self.toggle_smart_group_filter(id, cx),
        }
    }

    /// The category is the only one the list is filtered by.
    pub(in crate::collection) fn category_alone(&self, r: &CategoryRef) -> bool {
        let (manual, smart) = (&self.ui.categories, &self.ui.smart_groups);
        match r {
            CategoryRef::Manual(name) => smart.is_empty() && manual.len() == 1 && manual.contains(name),
            CategoryRef::Smart(id) => manual.is_empty() && smart.len() == 1 && smart.contains(id),
        }
    }

    /// Shows only the games of the category (a click on a pin): picking the
    /// one already shown alone shows every game again.
    pub(in crate::collection) fn show_only_category(&mut self, r: &CategoryRef, cx: &mut Context<Self>) {
        let alone = self.category_alone(r);
        self.ui.categories.clear();
        self.ui.smart_groups.clear();
        if !alone {
            match r {
                CategoryRef::Manual(name) => self.ui.categories = BTreeSet::from([name.clone()]),
                CategoryRef::Smart(id) => self.ui.smart_groups = BTreeSet::from([id.clone()]),
            }
        }
        self.save_ui();
        self.refresh_groups();
        self.scroll_list_to_top();
        cx.notify();
    }

    /// Pins the category to the header or takes it off.
    pub(in crate::collection) fn set_category_pinned(&mut self, r: &CategoryRef, pinned: bool, cx: &mut Context<Self>) {
        match r {
            CategoryRef::Manual(name) => {
                self.ui.set_category_look(name, |l| l.pinned = pinned);
                self.save_ui();
                cx.notify();
            }
            CategoryRef::Smart(id) => self.set_group_pinned(id, pinned, cx),
        }
    }

    /// Gives the manual category `name` its own colour (`None`: the default).
    pub(in crate::collection) fn set_manual_colour(&mut self, name: &str, colour: Option<String>, cx: &mut Context<Self>) {
        self.ui.set_category_look(name, |l| l.colour = colour);
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// The smart categories `g` is in, in their order.
    pub(in crate::collection) fn smart_categories_of(&self, g: &CollectionGame) -> Vec<&SmartGroup> {
        self.smart_groups.groups.iter().filter(|s| self.group_members(s).contains(&g.key)).collect()
    }

    /// The smart categories with their games, for Group by Category.
    pub(in crate::collection) fn smart_sections(&self) -> Vec<SmartSection> {
        self.smart_groups
            .groups
            .iter()
            .map(|g| SmartSection { id: g.id.clone(), name: g.name.clone(), games: self.group_members(g) })
            .collect()
    }

    /// The badges of the game under `key` (and, while copies are grouped,
    /// of its other copies), each once: its manual categories A-Z, then its
    /// smart ones in their order.
    pub(in crate::collection) fn card_categories(&self, key: &str) -> Vec<CategoryBadge> {
        if self.categories.is_empty() && self.smart_groups.is_empty() {
            return Vec::new();
        }
        let copies = self.with_copies(key);
        let mut manual = BTreeSet::new();
        for g in &copies {
            manual.extend(self.categories.of_game(&g.runner, &g.app_name));
        }
        let manual = manual.into_iter().map(|n| CategoryBadge { name: n.to_string().into(), color: self.manual_colour(n) });
        let smart = self.smart_groups.groups.iter().filter(|s| {
            let keys = self.group_members(s);
            copies.iter().any(|g| keys.contains(&g.key))
        });
        manual.chain(smart.map(|s| CategoryBadge { name: s.name.clone().into(), color: group_colour(s) })).collect()
    }
}
