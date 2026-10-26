//! State changes of the Collection view: dialogs, selection, grouping, launch
//! events and toasts.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::FacetKind;
use heronite_launch::LaunchEvent;

use super::vm::{self, GroupVm, ListItem};
use super::{data, CollectionView, ToastAction};
use crate::services::launch_blocker::{launch_blocker, Blocker};
use heronite_i18n::tr;

impl CollectionView {
    pub(super) fn dialog_ctx(&self) -> Option<crate::dialogs::DialogCtx> {
        Some(crate::dialogs::DialogCtx { data_paths: self.data_paths.clone(), app: self.app_paths.clone(), lib: self.lib.clone()? })
    }

    /// Hosts `view` as the open dialog and reacts to its events.
    pub(super) fn open_dialog<V>(&mut self, view: Entity<V>, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<crate::dialogs::DialogEvent>,
    {
        use crate::dialogs::DialogEvent;
        self.open_menu = None;
        self.context_menu = None;
        self.dialog_sub = Some(cx.subscribe(&view, |this, _, ev: &DialogEvent, cx| {
            match ev {
                DialogEvent::Close => {
                    this.dialog = None;
                    this.dialog_sub = None;
                }
                DialogEvent::Changed => this.reload(cx),
                DialogEvent::ChangedAndClose => {
                    this.dialog = None;
                    this.dialog_sub = None;
                    this.reload(cx);
                }
            }
            cx.notify();
        }));
        let view: AnyView = view.into();
        self.dialog_hook = crate::console::registry::hook_for(&view);
        self.dialog = Some(view);
        cx.notify();
    }

    pub(super) fn open_manage_statuses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| crate::dialogs::manage_statuses::ManageStatusesDialog::new(ctx, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn refresh_groups(&mut self) {
        let Some(lib) = self.lib.clone() else { return };
        let playing = &self.playing;
        let keep = |g: &heronite_data::CollectionGame| self.keeps_game(g, &lib.statuses);
        let organize = heronite_data::organize::Organize {
            extras: Some(&self.game_extras),
            ratings: self.facts.ratings(),
            categories: Some(&self.categories),
            statuses: &lib.statuses,
        };
        let is_playing = |key: &str| lib.get(key).is_some_and(|g| playing.contains_key(&g.app_name));
        let step = |key: &str| lib.get(key).and_then(|g| self.preparing.get(&g.app_name)).map(|s| s.label());
        let groups = data::build_groups(&lib, &self.ui, &self.search, &keep, &is_playing, &step, &organize);
        self.set_groups(groups);
        self.refresh_focus();
        self.prune_selection_to_list();
    }

    /// The list's conditions that live in the app: the selected categories
    /// and statuses, and "Only with an update".
    pub(super) fn keeps_game(&self, g: &heronite_data::CollectionGame, statuses: &[heronite_data::models::CompletionStatus]) -> bool {
        let ui = &self.ui;
        self.categories.matches(&ui.categories, &g.runner, &g.app_name)
            && (!ui.only.updates || self.update_apps.contains(&g.app_name))
            && (ui.statuses.is_empty()
                || ui.statuses.contains(&heronite_data::status::resolve_status_id(g.status_raw.as_deref(), statuses)))
    }

    fn refresh_focus(&mut self) {
        let (Some(lib), Some(key)) = (self.lib.clone(), self.selected.clone()) else { return };
        if let Some(game) = lib.get(&key) {
            let playing = self.playing.contains_key(&game.app_name);
            self.focus = Some(data::focus_vm(&lib, &self.wiki, game, playing));
        }
    }

    pub(super) fn show_toast(&mut self, text: impl Into<SharedString>, error: bool, cx: &mut Context<Self>) {
        self.show_toast_for(text, error, std::time::Duration::from_secs(5), cx);
    }

    /// A toast that stays `shown`.
    pub(super) fn show_toast_for(
        &mut self,
        text: impl Into<SharedString>,
        error: bool,
        shown: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        self.toast_action = None;
        self.toast = Some((text.into(), error));
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// A toast for a batch change, with Undo (`selection.rs`).
    pub(super) fn show_undo_toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_toast(text, false, cx);
        self.toast_action = Some(ToastAction::Undo);
    }

    /// For launches that skip the Play button's dialogs: refuses (with a
    /// toast) while a download or an operation owns the game's files.
    pub(super) fn refuse_busy_launch(&mut self, game: &heronite_data::CollectionGame, cx: &mut Context<Self>) -> bool {
        let blocker = launch_blocker(cx, game).filter(Blocker::is_busy);
        let Some(text) = blocker.and_then(|b| b.refusal(&game.title)) else { return false };
        self.show_toast(text, true, cx);
        true
    }

    pub(super) fn on_launch_event(&mut self, ev: LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Launching { app_name } => {
                self.preparing.insert(app_name.clone(), super::launch_progress::LaunchStep::starting());
                self.playing.insert(app_name, false);
            }
            LaunchEvent::Preparing { app_name, message, cancellable } => {
                if self.playing.contains_key(&app_name) {
                    self.set_launch_step(app_name, message, cancellable);
                }
            }
            LaunchEvent::Running { app_name, pid } => {
                log::info!("{app_name} running as pid {pid}");
                self.preparing.remove(&app_name);
                self.playing.insert(app_name, true);
            }
            LaunchEvent::Exited { app_name, elapsed, exit, stopped, .. } => {
                log::info!("{app_name} exited after {elapsed:?}");
                self.preparing.remove(&app_name);
                self.playing.remove(&app_name);
                // Ended at once on its own, badly: maybe it could not start.
                if heronite_launch::failed_launch(exit, stopped, elapsed).is_some() {
                    self.diagnose_exit(&app_name, crate::diagnosis::Ended { elapsed, exit }, None, cx);
                }
                // Post-game work is `services::session`'s; the library only
                // reloads what changed on disk (playtime, last played, recent).
                self.reload(cx);
            }
            LaunchEvent::Error { app_name, message } => {
                self.preparing.remove(&app_name);
                self.playing.remove(&app_name);
                self.show_toast(message, true, cx);
            }
            // Problems while preparing, and what happens after the game (cloud saves).
            LaunchEvent::Status { message, warning, .. } => self.show_toast(message, warning, cx),
        }
        self.refresh_groups();
        cx.notify();
    }

    /// Card play button / focus primary button.
    pub(super) fn primary_action(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key).cloned() else { return };
        if self.preparing.contains_key(&game.app_name) {
            return self.cancel_launch(&game.app_name, cx);
        }
        if self.playing.contains_key(&game.app_name) {
            if !self.runtime.stop(&game.app_name) {
                self.runtime.force_clear(&game.app_name);
            }
            return;
        }
        if self.epic_primary(&game, cx) {
            return;
        }
        if game.installed && self.start_emulated(&game, cx) {
            return;
        }
        let result = if game.installed { self.runtime.play(&game) } else { self.runtime.install(&game) };
        if let Err(err) = result {
            self.report_launch_error(err, cx);
        }
    }

    pub(super) fn toggle_favourite(&mut self, cx: &mut Context<Self>) {
        let (Some(lib), Some(key)) = (self.lib.clone(), self.selected.clone()) else { return };
        let Some(game) = lib.get(&key) else { return };
        if let Err(err) = heronite_data::config_games::toggle_favourite(&self.data_paths, &game.app_name, &game.title) {
            self.show_toast(tr!("Could not update favourites: {err}", err = format!("{err:#}")), true, cx);
        }
        self.reload(cx);
    }

    pub(super) fn save_ui(&self) {
        if let Err(err) = self.ui.save(&self.app_paths) {
            log::warn!("saving ui state: {err:#}");
        }
    }

    /// Writes a changed focused game once the selection rests for a moment.
    pub(super) fn schedule_ui_save(&mut self, cx: &mut Context<Self>) {
        if !self.ui_focus_dirty || self.ui_save.is_some() {
            return;
        }
        self.ui_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            this.update(cx, |this, _| {
                this.ui_save = None;
                if std::mem::take(&mut this.ui_focus_dirty) {
                    this.save_ui();
                }
            })
            .ok();
        }));
    }

    pub(super) fn toggle_facet(&mut self, kind: &str, value: &str) {
        if let Some(kind) = FacetKind::parse(kind) {
            self.ui.toggle_facet(kind, value);
            self.save_ui();
            self.refresh_groups();
        }
    }

    /// "Library" of the facts card: shows only the games of the store this
    /// one comes from (Steam, Epic Games, Emulated...), in place of any other
    /// store filter.
    pub(super) fn filter_by_store_of(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)) else { return };
        let Some(store) = g.facets[FacetKind::Store.index()].first().map(|i| i.value.clone()) else { return };
        self.ui.facet_filters.store = vec![store];
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    fn set_groups(&mut self, groups: Vec<GroupVm>) {
        // The anchor reads the rows of the groups that are about to go.
        let anchor = self.scroll_anchor();
        self.groups = groups;
        self.rebuild_with(anchor);
    }

    pub(super) fn rebuild_items(&mut self) {
        let anchor = self.scroll_anchor();
        self.rebuild_with(anchor);
    }

    fn scroll_anchor(&self) -> Option<vm::ScrollAnchor> {
        let top = self.list_state.logical_scroll_top();
        vm::scroll_anchor(&self.items, &self.groups, top.item_ix, top.offset_in_item)
    }

    /// Rebuilds the rows and keeps the user's place: the list state is reset
    /// (the row heights changed), which would send it to the top, so the
    /// anchored card or header is scrolled back to.
    fn rebuild_with(&mut self, anchor: Option<vm::ScrollAnchor>) {
        self.items = vm::build_items(&self.groups, self.columns);
        self.list_state.reset(self.items.len());
        let place = anchor.and_then(|a| vm::resolve_anchor(&self.items, &self.groups, self.columns, &a));
        if let Some((item_ix, offset_in_item)) = place {
            self.list_state.scroll_to(ListOffset { item_ix, offset_in_item });
        }
    }

    /// Back to the first row: a new search is a new list, not the old one
    /// narrowed down.
    pub(super) fn scroll_list_to_top(&self) {
        self.list_state.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
    }

    /// Scrolls the list so the selected card's row is visible
    /// (`scrollIntoView({ block: 'nearest' })`).
    pub(super) fn reveal_selected(&mut self) {
        let Some(sel) = self.selected.clone() else { return };
        let ix = self.items.iter().position(|it| match *it {
            ListItem::Row { group, start } => {
                let g = &self.groups[group];
                g.cards[start..(start + self.columns).min(g.cards.len())].iter().any(|c| c.key == sel)
            }
            _ => false,
        });
        if let Some(ix) = ix {
            let viewport = self.list_state.viewport_bounds();
            if viewport.size.height > px(0.) && self.list_state.bounds_for_item(ix).is_some() {
                self.list_state.scroll_to_reveal_item(ix);
            } else {
                // Not laid out yet: put the row (and its group header when it
                // is the group's first row) at the top.
                let top = if ix > 0 && matches!(self.items[ix - 1], ListItem::Header { .. }) { ix - 1 } else { ix };
                self.list_state.scroll_to(ListOffset { item_ix: top, offset_in_item: px(0.) });
            }
        }
    }

    pub(super) fn select(&mut self, key: SharedString, persist: bool) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(&key) else { return };
        let playing = self.playing.contains_key(&game.app_name);
        self.focus = Some(data::focus_vm(&lib, &self.wiki, game, playing));
        if self.selected.as_ref() != Some(&key) {
            self.focus_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.selected = Some(key.clone());
        self.grid.drawer_closed = false;
        if persist {
            self.ui.focused_game = Some(key.to_string());
            self.ui_focus_dirty = true;
        }
    }
}
