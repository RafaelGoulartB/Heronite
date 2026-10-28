//! Several games at once: picking them (Ctrl / Shift click, Ctrl A, the
//! context menu's "Select multiple") and the batch changes the dock
//! (`dock.rs`) and the context menu make to all of them (status, categories,
//! favourite, hide, metadata, fields), each with an Undo.
//!
//! Selecting is a mode: while games are selected the cards show a tick
//! circle, a plain click ticks or unticks one, the cards' play bar and the
//! grid's drawer step aside and the dock floats at the bottom. Esc or the
//! dock's × ends it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::status::SetStatusExtras;
use heronite_data::{CollectionGame, DataPaths};
use heronite_i18n::{tr, trn};
use heronite_metadata::edit::{BatchDetails, DetailsBefore};

use super::CollectionView;
use crate::dialogs::batch_edit::{BatchEditApply, BatchEditDialog};

/// The dock's open popover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DockPop {
    Status,
    Categories,
}

/// How many of the selected games are in a category.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tri {
    None,
    Some,
    All,
}

impl Tri {
    fn of(n: usize, total: usize) -> Self {
        if n == 0 {
            Self::None
        } else if n >= total {
            Self::All
        } else {
            Self::Some
        }
    }
}

/// What a click on a category does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    /// Every game back in or out as it was when the selection was made.
    AsWas,
    All,
    None,
}

/// The clicks of a category, from how it was when the selection was made:
/// some of them → all → none → as it was; all → none → as it was; none →
/// all → as it was. Playnite's "assign / remove / leave alone".
fn cycle(orig: Tri) -> &'static [Step] {
    match orig {
        Tri::Some => &[Step::AsWas, Step::All, Step::None],
        Tri::All => &[Step::AsWas, Step::None],
        Tri::None => &[Step::AsWas, Step::All],
    }
}

#[derive(Default)]
pub(super) struct Selection {
    /// Selected game keys, in the order they were picked.
    pub keys: Vec<SharedString>,
    /// Last card clicked: where a Shift click range starts.
    pub anchor: Option<SharedString>,
    /// Games in each category when the selection was made (a category's
    /// last click puts them back).
    orig: HashMap<String, HashSet<SharedString>>,
    /// Clicks on each category since then (position in its [`cycle`]).
    steps: HashMap<String, usize>,
    /// Take `orig` again once the library reloads (after an undo).
    reseed: bool,
    pub pop: Option<DockPop>,
    /// Newest last.
    undo: Vec<Undo>,
    /// A batch change or an undo was written and the library has not been
    /// reloaded yet (since then): an undo now would compare stale games.
    reloading: Option<std::time::Instant>,
}

impl Selection {
    pub fn active(&self) -> bool {
        !self.keys.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.iter().any(|k| k.as_ref() == key)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
}

/// What a batch change touched and what it left each game with. An undo
/// only puts back the games that still have it: a later change to one of
/// them stays.
enum After {
    Status(String),
    /// The category and whether each game (in `Undo::games` order) is in it.
    Categories {
        name: String,
        on: Vec<bool>,
    },
    Favourite(bool),
    Hidden(bool),
    Details(BatchDetails),
}

impl After {
    /// Whether `g` (game `i` of the change) still is as the change left it.
    fn holds(&self, i: usize, g: &CollectionGame, categories: &heronite_data::categories::Categories) -> bool {
        match self {
            Self::Status(id) => g.status_id == *id,
            Self::Categories { name, on } => on.get(i).is_none_or(|on| categories.contains(name, &g.runner, &g.app_name) == *on),
            Self::Favourite(on) => g.favourite == *on,
            Self::Hidden(on) => g.hidden == *on,
            Self::Details(edit) => {
                let Some(md) = g.metadata.as_deref() else { return true };
                let has = |list: &[String], name: &str| list.iter().any(|n| n.eq_ignore_ascii_case(name));
                let series = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
                edit.series.as_deref().is_none_or(|v| series(md.series.as_deref()) == series(Some(v)))
                    && edit.lists.iter().all(|(field, delta)| {
                        let list = field.get(md);
                        delta.add.iter().all(|a| has(list, a)) && !delta.remove.iter().any(|r| has(list, r))
                    })
            }
        }
    }
}

/// One game as it was before a batch change.
struct Snap {
    key: SharedString,
    runner: String,
    app_name: String,
    title: String,
    status: String,
    extras: SetStatusExtras,
    favourite: bool,
    hidden: bool,
    categories: Vec<String>,
}

struct Undo {
    after: After,
    keys: Vec<SharedString>,
    games: Vec<Snap>,
    /// Metadata entries before a fields edit, in `games` order
    /// (`update_many_details` keeps the order it was given).
    details: Option<DetailsBefore>,
    /// What the change was, for the undo's toast.
    message: String,
}

const UNDO_DEPTH: usize = 20;
/// How long an undo waits for the reload after a change before it goes ahead
/// anyway (a reload that failed must not block it for good).
const RELOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn extras_of(g: &CollectionGame) -> SetStatusExtras {
    SetStatusExtras {
        runner: Some(g.runner.clone()),
        title: Some(g.title.clone()),
        playnite_id: g.meta.as_ref().map(|m| m.playnite_id.clone()),
    }
}

impl CollectionView {
    /// The selected games, in the order they were picked.
    pub(super) fn selected_games(&self) -> Vec<Arc<CollectionGame>> {
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        self.sel.keys.iter().filter_map(|k| lib.get(k).cloned()).collect()
    }

    /// Replaces the selection (unknown and repeated keys dropped).
    pub(super) fn set_selection(&mut self, keys: Vec<SharedString>) {
        let Some(lib) = self.lib.clone() else { return };
        let mut seen = HashSet::new();
        let keys: Vec<SharedString> = keys.into_iter().filter(|k| lib.get(k).is_some() && seen.insert(k.clone())).collect();
        let cats = &self.categories;
        let orig = cats
            .names()
            .map(|name| {
                let inside = keys.iter().filter(|k| lib.get(k).is_some_and(|g| cats.contains(name, &g.runner, &g.app_name))).cloned();
                (name.to_string(), inside.collect())
            })
            .collect();
        let s = &mut self.sel;
        s.keys = keys;
        s.orig = orig;
        s.steps.clear();
        s.reseed = false;
        if s.keys.is_empty() {
            s.pop = None;
        }
    }

    pub(super) fn clear_selection(&mut self) {
        self.set_selection(Vec::new());
    }

    fn toggle_selected(&mut self, key: SharedString) {
        let mut keys = self.sel.keys.clone();
        match keys.iter().position(|k| *k == key) {
            Some(i) => {
                keys.remove(i);
            }
            None => keys.push(key.clone()),
        }
        self.set_selection(keys);
        self.sel.anchor = Some(key);
    }

    /// The cards on screen, in order (folded groups left out).
    pub(super) fn shown_keys(&self) -> Vec<SharedString> {
        self.groups.iter().filter(|g| !(g.collapsed && g.show_header)).flat_map(|g| g.cards.iter().map(|c| c.key.clone())).collect()
    }

    /// A card was clicked. Shift adds the cards from the last one clicked;
    /// Ctrl, or any click while selecting, ticks or unticks it (Ctrl on a
    /// second game also keeps the focused one); a plain click focuses it.
    pub(super) fn card_clicked(&mut self, key: SharedString, ev: &ClickEvent) {
        let m = ev.modifiers();
        let ctrl = m.control || m.platform;
        if m.shift {
            let from = self.sel.anchor.clone().or_else(|| self.selected.clone());
            let order = self.shown_keys();
            let at = |k: &SharedString| order.iter().position(|o| o == k);
            if let Some((a, b)) = from.as_ref().and_then(at).zip(at(&key)) {
                let mut keys = self.sel.keys.clone();
                keys.extend(order[a.min(b)..=a.max(b)].iter().cloned());
                self.set_selection(keys);
                self.sel.anchor = Some(key);
                return;
            }
        }
        if ctrl && !self.sel.active() {
            if let Some(focused) = self.selected.clone().filter(|f| *f != key) {
                self.set_selection(vec![focused, key.clone()]);
                self.sel.anchor = Some(key);
                return;
            }
        }
        if ctrl || self.sel.active() {
            self.toggle_selected(key);
            return;
        }
        self.select(key.clone(), true);
        self.sel.anchor = Some(key);
    }

    /// The context menu's "Select multiple" / "Add to selection".
    pub(super) fn start_selection(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.context_menu = None;
        let mut keys = self.sel.keys.clone();
        keys.push(key.clone());
        self.set_selection(keys);
        self.sel.anchor = Some(key);
        cx.notify();
    }

    pub(super) fn select_all_shown(&mut self) {
        self.context_menu = None;
        self.set_selection(self.shown_keys());
    }

    /// The group header's tick: the whole group in, or out when all of it is in.
    pub(super) fn toggle_group_selection(&mut self, group_id: &str) {
        let Some(g) = self.groups.iter().find(|g| g.id.as_ref() == group_id) else { return };
        let group: Vec<SharedString> = g.cards.iter().map(|c| c.key.clone()).collect();
        let all = group.iter().all(|k| self.sel.contains(k));
        let mut keys = self.sel.keys.clone();
        if all {
            keys.retain(|k| !group.contains(k));
        } else {
            keys.extend(group);
        }
        self.set_selection(keys);
    }

    /// How many of the group's games are selected, of how many.
    pub(super) fn group_selection(&self, group_id: &str) -> Option<(usize, usize)> {
        let g = self.groups.iter().find(|g| g.id.as_ref() == group_id)?;
        Some((g.cards.iter().filter(|c| self.sel.contains(&c.key)).count(), g.cards.len()))
    }

    /// After a reload: games gone from the list (hidden, removed) leave the
    /// selection; after an undo the categories' starting point is taken again.
    pub(super) fn prune_selection(&mut self) {
        // A load that read the files before the change is followed by another.
        if !self.reload_again {
            self.sel.reloading = None;
        }
        if !self.sel.active() {
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        let before = self.sel.keys.len();
        if self.sel.reseed || self.sel.keys.iter().any(|k| lib.get(k).is_none()) {
            let (keys, anchor, steps) = (self.sel.keys.clone(), self.sel.anchor.clone(), std::mem::take(&mut self.sel.steps));
            let reseed = self.sel.reseed;
            self.set_selection(keys);
            self.sel.anchor = anchor;
            // A hidden game leaving keeps the categories' clicks going.
            if !reseed && self.sel.keys.len() < before {
                self.sel.steps = steps;
            }
        }
        self.prune_selection_to_list();
    }

    /// Games the list no longer shows (the search or a filter changed) leave
    /// the selection: the dock acts on the games on screen only. Folded
    /// groups still count. Not while an undo waits for its reload (its games
    /// may only come back with it).
    pub(super) fn prune_selection_to_list(&mut self) {
        if !self.sel.active() || self.sel.reseed {
            return;
        }
        let listed: HashSet<&SharedString> = self.groups.iter().flat_map(|g| g.cards.iter().map(|c| &c.key)).collect();
        if self.sel.keys.iter().all(|k| listed.contains(k)) {
            return;
        }
        let keys: Vec<SharedString> = self.sel.keys.iter().filter(|k| listed.contains(k)).cloned().collect();
        let anchor = self.sel.anchor.clone();
        // A new set of games: the categories' clicks start over from it.
        self.set_selection(keys);
        self.sel.anchor = anchor.filter(|a| self.sel.contains(a));
    }

    /// A category's state over the selection, its count and what a click does next.
    pub(super) fn category_tri(&self, name: &str, games: &[Arc<CollectionGame>]) -> (Tri, usize, Step) {
        let n = games.iter().filter(|g| self.categories.contains(name, &g.runner, &g.app_name)).count();
        let orig = Tri::of(self.sel.orig.get(name).map_or(0, HashSet::len), games.len());
        let cyc = cycle(orig);
        let next = cyc[(self.sel.steps.get(name).copied().unwrap_or(0) + 1) % cyc.len()];
        (Tri::of(n, games.len()), n, next)
    }

    fn snapshot(&self, after: After, games: &[Arc<CollectionGame>]) -> Undo {
        let games = games
            .iter()
            .map(|g| Snap {
                key: g.key.clone().into(),
                runner: g.runner.clone(),
                app_name: g.app_name.clone(),
                title: g.title.clone(),
                status: g.status_id.clone(),
                extras: extras_of(g),
                favourite: g.favourite,
                hidden: g.hidden,
                categories: self.categories.of_game(&g.runner, &g.app_name).into_iter().map(str::to_owned).collect(),
            })
            .collect();
        Undo { after, keys: self.sel.keys.clone(), games, details: None, message: String::new() }
    }

    /// Runs a batch write off the UI thread, then keeps its undo, says what
    /// changed (with Undo) and reloads.
    fn run_batch(
        &mut self,
        mut undo: Undo,
        message: String,
        work: impl FnOnce(&DataPaths) -> anyhow::Result<Option<DetailsBefore>> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { work(&paths) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(details) => {
                        undo.details = details;
                        undo.message = message.clone();
                        this.sel.undo.push(undo);
                        let extra = this.sel.undo.len().saturating_sub(UNDO_DEPTH);
                        this.sel.undo.drain(..extra);
                        this.show_undo_toast(message, cx);
                    }
                    Err(err) => this.show_toast(tr!("Could not change the games: {err}", err = format!("{err:#}")), true, cx),
                }
                this.sel.reloading = Some(std::time::Instant::now());
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn batch_status(&mut self, status_id: String, cx: &mut Context<Self>) {
        let games = self.selected_games();
        if games.is_empty() {
            return;
        }
        self.sel.pop = None;
        self.context_menu = None;
        let name = self
            .lib
            .as_ref()
            .and_then(|l| l.statuses.iter().find(|s| s.id == status_id))
            .map(|s| heronite_data::status::status_label(&s.name).to_string())
            .unwrap_or_else(|| status_id.clone());
        let undo = self.snapshot(After::Status(status_id.clone()), &games);
        let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), status_id.clone(), extras_of(g))).collect();
        let message = trn!(games.len(), "Status set to {status} on {n} game", "Status set to {status} on {n} games", status = name);
        self.run_batch(undo, message, move |p| heronite_data::status::set_games_status(p, &writes).map(|_| None), cx);
        self.mark_onboarding(crate::dialogs::onboarding::Step::Statuses);
    }

    /// A click on a category: the next step of its [`cycle`] for every
    /// selected game. The menu stays open for the next click.
    pub(super) fn batch_category(&mut self, name: String, cx: &mut Context<Self>) {
        let games = self.selected_games();
        if games.is_empty() {
            return;
        }
        let orig = self.sel.orig.get(&name).cloned().unwrap_or_default();
        let cyc = cycle(Tri::of(orig.len(), games.len()));
        let step = (self.sel.steps.get(&name).copied().unwrap_or(0) + 1) % cyc.len();
        self.sel.steps.insert(name.clone(), step);
        let to = cyc[step];
        let writes: Vec<(String, String, bool)> = games
            .iter()
            .map(|g| {
                let on = match to {
                    Step::All => true,
                    Step::None => false,
                    Step::AsWas => orig.contains(g.key.as_str()),
                };
                (g.runner.clone(), g.app_name.clone(), on)
            })
            .collect();
        let on = writes.iter().map(|(_, _, on)| *on).collect();
        let undo = self.snapshot(After::Categories { name: name.clone(), on }, &games);
        let n = games.len();
        let message = match to {
            Step::All => trn!(n, "{name} added to {n} game", "{name} added to {n} games", name = name.clone()),
            Step::None => trn!(n, "{name} removed from {n} game", "{name} removed from {n} games", name = name.clone()),
            Step::AsWas => tr!("{name} is back as it was", name = name.clone()),
        };
        self.run_batch(undo, message, move |p| heronite_data::categories::set_many_in_category(p, &name, &writes).map(|_| None), cx);
    }

    /// Adds every selected game to the favourites, or takes them all out when
    /// they all are.
    pub(super) fn batch_favourite(&mut self, cx: &mut Context<Self>) {
        let games = self.selected_games();
        if games.is_empty() {
            return;
        }
        self.context_menu = None;
        let all = games.iter().all(|g| g.favourite);
        let undo = self.snapshot(After::Favourite(!all), &games);
        let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), g.title.clone(), !all)).collect();
        let n = games.len();
        let message = if all {
            trn!(n, "{n} game removed from favourites", "{n} games removed from favourites")
        } else {
            trn!(n, "{n} game added to favourites", "{n} games added to favourites")
        };
        self.run_batch(undo, message, move |p| heronite_data::config_games::set_favourites(p, &writes).map(|_| None), cx);
    }

    /// Hides every selected game (they leave the list unless hidden games
    /// are shown), or shows them again when they all are hidden.
    pub(super) fn batch_hide(&mut self, cx: &mut Context<Self>) {
        let games = self.selected_games();
        if games.is_empty() {
            return;
        }
        self.context_menu = None;
        let all = games.iter().all(|g| g.hidden);
        let undo = self.snapshot(After::Hidden(!all), &games);
        let writes: Vec<_> = games.iter().map(|g| (g.app_name.clone(), g.title.clone(), !all)).collect();
        let n = games.len();
        let message =
            if all { trn!(n, "{n} game shown again", "{n} games shown again") } else { trn!(n, "{n} game hidden", "{n} games hidden") };
        self.run_batch(undo, message, move |p| heronite_data::config_games::set_hidden(p, &writes).map(|_| None), cx);
    }

    /// The metadata wizard on the selected games only.
    pub(super) fn batch_metadata(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.sel.pop = None;
        let games = self.selected_games();
        if !games.is_empty() {
            self.open_wizard_for(games.iter().map(|g| super::metadata_ui::bulk_item(g)).collect(), cx);
        }
    }

    /// F3: the fields editor of the selected games.
    pub(super) fn open_batch_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.sel.pop = None;
        let games = self.selected_games();
        if games.is_empty() {
            return;
        }
        let metadata = games.iter().map(|g| g.metadata.clone()).collect();
        let view = cx.new(|cx| BatchEditDialog::new(metadata, window, cx));
        cx.subscribe(&view, |this, _, ev: &BatchEditApply, cx| this.apply_batch_edit(ev.0.clone(), cx)).detach();
        self.open_dialog(view, cx);
    }

    fn apply_batch_edit(&mut self, edit: BatchDetails, cx: &mut Context<Self>) {
        let games = self.selected_games();
        if games.is_empty() || edit.is_empty() {
            return;
        }
        let undo = self.snapshot(After::Details(edit.clone()), &games);
        let pairs: Vec<_> = games.iter().map(|g| (g.runner.clone(), g.app_name.clone())).collect();
        let message = trn!(games.len(), "{n} game edited", "{n} games edited");
        self.run_batch(undo, message, move |p| heronite_metadata::edit::update_many_details(p, &pairs, &edit).map(Some), cx);
    }

    /// Puts back what the last batch change changed (Ctrl Z, the toast's
    /// Undo) and selects its games again. A game changed since keeps its
    /// newer state. False when there is nothing to undo.
    pub(super) fn undo_batch(&mut self, cx: &mut Context<Self>) -> bool {
        if self.sel.undo.is_empty() {
            return false;
        }
        // The last change is not on screen yet: its games cannot be compared.
        if self.sel.reloading.is_some_and(|at| at.elapsed() < RELOAD_WAIT) {
            return true;
        }
        let Some(u) = self.sel.undo.pop() else { return false };
        self.toast = None;
        let Undo { after, keys, games, details, message } = u;
        // A game gone from the list (hidden, removed) cannot be compared: it is put back.
        let lib = self.lib.clone();
        let holds: Vec<bool> = games
            .iter()
            .enumerate()
            .map(|(i, s)| lib.as_ref().and_then(|l| l.get(&s.key)).is_none_or(|g| after.holds(i, g, &self.categories)))
            .collect();
        let skipped = holds.iter().filter(|h| !**h).count();
        let games: Vec<Snap> = games.into_iter().zip(&holds).filter(|(_, h)| **h).map(|(s, _)| s).collect();
        let details: DetailsBefore = details.unwrap_or_default().into_iter().zip(&holds).filter(|(_, h)| **h).map(|(d, _)| d).collect();
        if games.is_empty() {
            self.show_toast(tr!("Nothing undone: these games were changed again since “{what}”", what = message), false, cx);
            return true;
        }
        let done = if skipped == 0 {
            tr!("Undone: {what}", what = message.as_str())
        } else {
            trn!(
                skipped,
                "Undone: {what} ({n} game changed since kept its change)",
                "Undone: {what} ({n} games changed since kept their changes)",
                what = message.as_str()
            )
        };
        let paths = self.data_paths.clone();
        self.sel.reloading = Some(std::time::Instant::now());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match after {
                        After::Status(_) => {
                            let w: Vec<_> = games.into_iter().map(|s| (s.app_name, s.status, s.extras)).collect();
                            heronite_data::status::set_games_status(&paths, &w)
                        }
                        After::Categories { .. } => {
                            let w: Vec<_> = games.into_iter().map(|s| (s.runner, s.app_name, s.categories)).collect();
                            heronite_data::categories::restore_categories(&paths, &w)
                        }
                        After::Favourite(_) => {
                            let w: Vec<_> = games.into_iter().map(|s| (s.app_name, s.title, s.favourite)).collect();
                            heronite_data::config_games::set_favourites(&paths, &w)
                        }
                        After::Hidden(_) => {
                            let w: Vec<_> = games.into_iter().map(|s| (s.app_name, s.title, s.hidden)).collect();
                            heronite_data::config_games::set_hidden(&paths, &w)
                        }
                        After::Details(_) => heronite_metadata::edit::restore_details(&paths, &details),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(()) => {
                        // Hidden games come back with the reload: keep their keys until then.
                        this.sel.keys = keys;
                        this.sel.reseed = true;
                        this.sel.pop = None;
                        this.show_toast(done, false, cx);
                    }
                    Err(err) => this.show_toast(tr!("Could not undo: {err}", err = format!("{err:#}")), true, cx),
                }
                this.sel.reloading = Some(std::time::Instant::now());
                this.reload(cx);
            })
            .ok();
        })
        .detach();
        true
    }

    /// Keys of the selection mode: Esc ends it, Ctrl A selects every game on
    /// screen, Ctrl Z undoes the last batch change, F3 edits the fields of the
    /// selected games (the focused game's details when none is). True when
    /// the key was used.
    pub(super) fn selection_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let free = self.page == crate::pages::Page::Collection
            && !crate::console::active()
            && self.dialog.is_none()
            && self.context_menu.is_none()
            && self.open_menu.is_none()
            && !self.palette_open()
            && self.focus_handle.is_focused(window);
        if !free {
            return false;
        }
        let m = &ev.keystroke.modifiers;
        let ctrl = (m.control || m.platform) && !m.alt && !m.shift;
        match ev.keystroke.key.as_str() {
            "escape" if self.sel.pop.is_some() => self.sel.pop = None,
            "escape" if self.sel.active() => self.clear_selection(),
            "a" if ctrl && self.lib.is_some() => self.select_all_shown(),
            "z" if ctrl => return self.undo_batch(cx),
            "f3" if !m.modified() => {
                if self.sel.active() {
                    self.open_batch_edit(window, cx);
                } else if self.selected.is_some() {
                    self.open_game_settings(window, cx);
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }
}
