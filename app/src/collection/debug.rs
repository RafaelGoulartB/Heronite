//! `HERONITE_DEBUG_*` hooks of the Collection: open a menu or dialog on start
//! so screenshots and tests can reach it without clicking.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::context_menu::{self, Flyout};
use super::header::Menu;
use super::CollectionView;

/// The flyout `HERONITE_DEBUG_MENU` opens: the status one unless another is asked for.
fn debug_flyout() -> Flyout {
    let on = |name| crate::env::value(name).is_some();
    if on("HERONITE_DEBUG_MENU_INSTALL") {
        Flyout::Install
    } else if on("HERONITE_DEBUG_MENU_MANAGE") {
        Flyout::Manage
    } else if on("HERONITE_DEBUG_MENU_CATEGORIES") {
        Flyout::Categories
    } else if on("HERONITE_DEBUG_MENU_SAVES") {
        Flyout::Saves
    } else if on("HERONITE_DEBUG_MENU_SHORTCUTS") {
        Flyout::Shortcuts
    } else if on("HERONITE_DEBUG_MENU_HELP") {
        Flyout::Help
    } else {
        Flyout::Status
    }
}

impl CollectionView {
    /// `HERONITE_DEBUG_MENU=<card key>` opens its context menu (with the
    /// status flyout; `HERONITE_DEBUG_MENU_CATEGORIES=1` for the categories
    /// one, `HERONITE_DEBUG_MENU_MANAGE=1` for Manage installation's,
    /// `HERONITE_DEBUG_MENU_SAVES=1` for the saves'), `HERONITE_DEBUG_DROPDOWN=view|nav` opens a header menu
    /// (`facets`, `categories`, `stores` or `view:<text>`: the View menu with
    /// that search typed; `views`: its views in edit mode; `views-select`:
    /// their select open; `more-group` / `more-sort`: the "More" list of
    /// that row), `HERONITE_DEBUG_ORGANIZE=<group>,<sort>` groups and sorts
    /// the list (`year,releaseDate`), `HERONITE_DEBUG_DIALOG=<name>` opens a dialog on
    /// the first render that has a window (`open_pending_debug_dialog`).
    ///
    /// `HERONITE_DEBUG_MULTI=<n>` selects the first n games shown (a
    /// selection: the dock, the tick circles); `HERONITE_DEBUG_DOCK=status|
    /// categories` opens that dock popover. With a selection,
    /// `HERONITE_DEBUG_MENU` on a selected game opens the batch menu and
    /// `HERONITE_DEBUG_DIALOG=batchedit` the fields editor and
    /// `HERONITE_DEBUG_BATCH` runs batch changes (see `open_pending_debug_dialog`).
    pub(super) fn apply_debug_env(&mut self, cx: &mut Context<Self>) {
        if let Some(n) = crate::env::parse::<usize>("HERONITE_DEBUG_MULTI") {
            self.set_selection(self.shown_keys().into_iter().take(n).collect());
            self.sel.pop = match crate::env::value("HERONITE_DEBUG_DOCK").as_deref() {
                Some("status") => Some(super::selection::DockPop::Status),
                Some("categories") => Some(super::selection::DockPop::Categories),
                _ => None,
            };
        }
        if let Some(key) = crate::env::value("HERONITE_DEBUG_MENU") {
            self.context_menu = Some(context_menu::ContextMenuState {
                batch: self.sel.contains(&key),
                key: key.into(),
                position: point(px(200.), px(160.)),
                open: Some(debug_flyout()),
            });
        }
        if let Some(which) = crate::env::value("HERONITE_DEBUG_DIALOG") {
            self.pending_debug_dialog = Some(which);
        }
        if let Some(steps) = crate::env::value("HERONITE_DEBUG_BATCH") {
            self.debug_batch(steps, cx);
        }
        if let Some(spec) = crate::env::value("HERONITE_DEBUG_ORGANIZE") {
            let (group, sort) = spec.split_once(',').unwrap_or((spec.as_str(), ""));
            if let Some(by) = heronite_data::GroupBy::parse(group) {
                self.ui.group_by = by;
            }
            if let Some(mode) = heronite_data::SortMode::parse(sort) {
                self.ui.sort = mode;
            }
            self.refresh_groups();
        }
        if let Some(menu) = crate::env::value("HERONITE_DEBUG_DROPDOWN") {
            let query = match menu.as_str() {
                "facets" => Some("developer:"),
                "categories" => Some("category:"),
                "stores" => Some("store:"),
                m => m.strip_prefix("view:"),
            };
            self.open_menu = match menu.as_str() {
                "view" | "install" | "sort" | "status" => Some(Menu::View),
                "views" => {
                    self.view_menu.set_editing(true);
                    Some(Menu::View)
                }
                "views-select" => {
                    self.view_menu.set_select_open(true);
                    Some(Menu::View)
                }
                "more-group" | "more-sort" => {
                    self.view_menu.set_more(menu == "more-group");
                    Some(Menu::View)
                }
                "nav" => Some(Menu::Nav),
                _ if query.is_some() => Some(Menu::View),
                _ => None,
            };
            self.view_menu.debug_query = query.map(str::to_owned);
        }
    }

    /// `HERONITE_DEBUG_BATCH=status:<id>,cat:<name>,fav,hide,undo`: these
    /// batch changes on the selection, a second apart (tests of the writes
    /// and their undo; use a copy of the data folder).
    fn debug_batch(&mut self, steps: String, cx: &mut Context<Self>) {
        let steps: Vec<String> = steps.split(',').map(str::to_owned).collect();
        cx.spawn(async move |this, cx| {
            for step in steps {
                cx.background_executor().timer(std::time::Duration::from_millis(1200)).await;
                let ok = this.update(cx, |this, cx| {
                    log::info!("debug batch: {step}");
                    match step.split_once(':') {
                        Some(("status", id)) => this.batch_status(id.to_string(), cx),
                        Some(("cat", name)) => this.batch_category(name.to_string(), cx),
                        _ if step == "fav" => this.batch_favourite(cx),
                        _ if step == "hide" => this.batch_hide(cx),
                        _ if step == "undo" => {
                            this.undo_batch(cx);
                        }
                        _ => {}
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// `HERONITE_DEBUG_KEYS=enter,down`: keystrokes on the Collection's list,
    /// a moment apart, once the library is on screen.
    pub(super) fn start_debug_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let Some(keys) = crate::env::value("HERONITE_DEBUG_KEYS") else { return };
        if self.lib.is_none() || STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        window.focus(&self.focus_handle, cx);
        cx.spawn_in(window, async move |_, cx| {
            for k in keys.split(',').filter(|k| !k.is_empty()) {
                cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
                if let Ok(ks) = Keystroke::parse(k) {
                    cx.update(|window, cx| window.dispatch_keystroke(ks, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn open_pending_debug_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.pending_debug_dialog.take() else { return };
        match which.as_str() {
            w if w == "settings" || w.starts_with("settings:") => {
                let section = w.strip_prefix("settings:").and_then(crate::pages::settings::Section::parse);
                self.open_settings_section(section.unwrap_or(crate::pages::settings::Section::Appearance), window, cx);
            }
            "statuses" => self.open_manage_statuses(window, cx),
            "batchedit" => self.open_batch_edit(window, cx),
            "categories" => self.open_manage_categories(window, cx),
            // fields[:genres|companies|series|themes|features|platforms]
            w if w == "fields" || w.starts_with("fields:") => {
                let field = w.strip_prefix("fields:").and_then(heronite_data::field_names::Field::parse);
                self.open_manage_fields(field, window, cx);
            }
            // What Play shows for a store game whose folder is gone.
            w if w.starts_with("folder-missing:") => {
                self.epic.pending = Some(super::epic::Request::FolderMissing(w["folder-missing:".len()..].to_string()));
            }
            w if w == "onboarding" || w.starts_with("onboarding:") => {
                let step = w.strip_prefix("onboarding:").and_then(crate::dialogs::onboarding::Step::parse);
                self.open_onboarding_at(step, cx);
            }
            "wine-missing" => self.report_launch_error(heronite_launch::wine::no_wine_message(""), cx),
            "gallery" | "viewer" | "viewer-zoom" | "viewer-fit" | "confirm" | "delete" => self.shots.debug = Some(which),
            // Game settings tabs: gamesettings:wine|other|advanced|gamescope|gpu|saves|proton|backups|sandbox
            w @ ("gamesettings:wine"
            | "gamesettings:gpu"
            | "gamesettings:sandbox"
            | "gamesettings:other"
            | "gamesettings:advanced"
            | "gamesettings:gamescope"
            | "gamesettings:saves"
            | "gamesettings:proton"
            | "gamesettings:backups") => {
                use crate::dialogs::game_settings::Tab;
                let tab = match w {
                    "gamesettings:wine" | "gamesettings:proton" => Tab::Compatibility,
                    "gamesettings:advanced" => Tab::Launch,
                    "gamesettings:saves" => Tab::CloudSaves,
                    "gamesettings:backups" => Tab::Saves,
                    "gamesettings:gpu" => Tab::Gpu,
                    "gamesettings:sandbox" => Tab::Sandbox,
                    _ => Tab::Performance,
                };
                self.open_game_settings_tab(tab, window, cx);
            }
            "metadata" | "gamesettings" | "gamesettings:details" | "gamesettings:launch" | "webimage" | "wizard" => {
                self.open_metadata_debug(which.as_str(), window, cx)
            }
            w if w.starts_with("launch-problem:") => self.debug_launch_problem(&w["launch-problem:".len()..], false, cx),
            w if w.starts_with("launch-problem-toast:") => self.debug_launch_problem(&w["launch-problem-toast:".len()..], true, cx),
            // troubleshoot[:<log>]: "Troubleshoot", reading that log.
            w if w == "troubleshoot" || w.starts_with("troubleshoot:") => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    let log = w.strip_prefix("troubleshoot:").map(std::path::PathBuf::from);
                    self.troubleshoot_launch(&key, log, cx);
                }
            }
            "viewlog" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_game_log(&key, cx);
                }
            }
            w if w.starts_with("manual-install:") => self.open_manual_install(&w["manual-install:".len()..], window, cx),
            w if w.starts_with("play-as-pc:") => self.open_play_as_pc(&w["play-as-pc:".len()..], window, cx),
            w if w.starts_with("local-uninstall:") => self.open_sideload_uninstall(&w["local-uninstall:".len()..], window, cx),
            w if matches!(w, "playtime" | "remove" | "ludusavi" | "steam-missing") || w.starts_with("ludusavi:") => {
                self.open_extras_debug(which.as_str(), window, cx)
            }
            w if w.starts_with("add-game") || w.starts_with("edit-game") => {
                self.add_game_debug_dialog(w, window, cx);
            }
            w if w.starts_with("install-script") => self.script_install_debug_dialog(w, window, cx),
            other => {
                if !self.epic_debug_dialog(other, window, cx) {
                    self.open_emu_debug_dialog(other, window, cx);
                }
            }
        }
    }

    /// `HERONITE_DEBUG_BROWSE=<count>@<ms>`: selects the next `count` games of
    /// the list, one every `ms`, like a user browsing (memory checks).
    pub(super) fn spawn_debug_browse(&self, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_BROWSE") else { return };
        let (count, ms) = spec.split_once('@').unwrap_or((spec.as_str(), "300"));
        let (Ok(count), Ok(ms)) = (count.parse::<usize>(), ms.parse::<u64>()) else { return };
        cx.spawn(async move |this, cx| {
            for i in 0..count {
                cx.background_executor().timer(std::time::Duration::from_millis(ms)).await;
                let alive = this.update(cx, |this, cx| {
                    let keys: Vec<SharedString> = this.groups.iter().flat_map(|g| g.cards.iter().map(|c| c.key.clone())).collect();
                    if let Some(key) = keys.get(i % keys.len().max(1)).cloned() {
                        this.select(key, false);
                        this.reveal_selected();
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
            log::info!("[debug] browse done");
        })
        .detach();
    }
}
