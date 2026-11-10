//! `HERONITE_DEBUG_*` hooks of the Collection: open a menu or dialog on start
//! so screenshots and tests can reach it without clicking.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::context_menu::{self, Flyout};
use super::header::Menu;
use super::CollectionView;

fn quick_menu(value: &str) -> Option<super::toolbar::QuickMenu> {
    use super::toolbar::QuickMenu;
    match value {
        "sort" => Some(QuickMenu::Sort),
        "install" => Some(QuickMenu::Install),
        "group" => Some(QuickMenu::Group),
        "presets" => Some(QuickMenu::Presets),
        _ => None,
    }
}

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
    } else if on("HERONITE_DEBUG_MENU_METADATA") {
        Flyout::Metadata
    } else {
        Flyout::Status
    }
}

impl CollectionView {
    /// `HERONITE_DEBUG_MENU=<card key>` opens its context menu (with the
    /// status flyout; `HERONITE_DEBUG_MENU_CATEGORIES=1` for the categories
    /// one, `HERONITE_DEBUG_MENU_MANAGE=1` for Manage installation's,
    /// `HERONITE_DEBUG_MENU_SAVES=1` for the saves', `HERONITE_DEBUG_MENU_METADATA=1`
    /// for a selection's Metadata one), `HERONITE_DEBUG_DROPDOWN=view|nav|downloads|update|display` opens a header menu
    /// (`facets`, `categories`, `stores` or `view:<text>`: the View menu with
    /// that search typed; `views`: its views in edit mode; `views-select`:
    /// their select open; `more-group` / `more-sort` / `more-filters`: the "More" list of
    /// that row; `manage`: the footer's list; `quick-sort|quick-install|quick-group|quick-presets`: the
    /// corresponding quick toolbar menu, only if its trigger fits),
    /// `HERONITE_DEBUG_ORGANIZE=<group>,<sort>` groups and sorts
    /// the list (`year,releaseDate`), `HERONITE_DEBUG_DIALOG=<name>` opens a dialog on
    /// the first render that has a window (`open_pending_debug_dialog`).
    ///
    /// `HERONITE_DEBUG_MULTI=<n>` selects the first n games shown (a
    /// selection: the dock, the tick circles); `HERONITE_DEBUG_DOCK=status|
    /// categories|metadata` opens that dock popover.
    /// `HERONITE_DEBUG_METADATA_JOB` makes up a bulk metadata job (see
    /// `dialogs/metadata_update/fake.rs`); `HERONITE_DEBUG_DROPDOWN=metadata`
    /// opens the header pill's popover. With a selection,
    /// `HERONITE_DEBUG_MENU` on a selected game opens the batch menu and
    /// `HERONITE_DEBUG_DIALOG=batchedit` the fields editor, `batchremove` the
    /// removal confirmation, and
    /// `HERONITE_DEBUG_BATCH` runs batch changes (see `open_pending_debug_dialog`).
    pub(super) fn apply_debug_env(&mut self, cx: &mut Context<Self>) {
        if let Some(n) = crate::env::parse::<usize>("HERONITE_DEBUG_MULTI") {
            self.set_selection(self.shown_keys().into_iter().take(n).collect());
            self.sel.pop = match crate::env::value("HERONITE_DEBUG_DOCK").as_deref() {
                Some("status") => Some(super::selection::DockPop::Status),
                Some("categories") => Some(super::selection::DockPop::Categories),
                Some("metadata") => {
                    self.ensure_meta_counts(cx);
                    Some(super::selection::DockPop::Metadata)
                }
                _ => None,
            };
        }
        self.debug_metadata_job(cx);
        if let Some(key) = crate::env::value("HERONITE_DEBUG_MENU") {
            if debug_flyout() == Flyout::Metadata {
                self.ensure_meta_counts(cx);
            }
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
        if let Some(steps) = crate::env::value("HERONITE_DEBUG_TOOLBAR") {
            self.debug_toolbar(steps, cx);
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
                "more-group" | "more-sort" | "more-filters" | "manage" => {
                    use super::view_menu::organize::More;
                    self.view_menu.set_more(match menu.as_str() {
                        "more-group" => More::Group,
                        "more-sort" => More::Sort,
                        "more-filters" => More::Filters,
                        _ => More::Manage,
                    });
                    Some(Menu::View)
                }
                "nav" => Some(Menu::Nav),
                "downloads" => Some(Menu::Downloads),
                "update" => Some(Menu::Update),
                "steam" => Some(Menu::Steam),
                "metadata" => Some(Menu::Metadata),
                "display" => Some(Menu::Display),
                m if m.starts_with("quick-") => quick_menu(&m[6..]).map(Menu::Quick),
                _ if query.is_some() => Some(Menu::View),
                _ => None,
            };
            self.view_menu.debug_query = query.map(str::to_owned);
        }
    }

    /// `HERONITE_DEBUG_TOOLBAR=sort:playtime,direction,install:installed,
    /// favourites,group:status,preset:0,reset,toolbar:off,toolbar:on,
    /// open:sort,hover:on,age:12,age:unrated`: runs the same actions as the controls, 1.2 seconds apart,
    /// without OS input. Presets are zero-based; use a scratch profile as
    /// these actions persist. `open` accepts sort/install/group/presets;
    /// `hover` shows or hides the View button's arrow as a pointer would;
    /// `age` sets "Ages up to" (`unrated` also shows the unrated games).
    fn debug_toolbar(&mut self, steps: String, cx: &mut Context<Self>) {
        let steps: Vec<String> = steps.split(',').map(str::to_owned).collect();
        cx.spawn(async move |this, cx| {
            for step in steps {
                cx.background_executor().timer(std::time::Duration::from_millis(1200)).await;
                if this
                    .update(cx, |this, cx| {
                        log::info!("debug toolbar: {step}");
                        match step.split_once(':') {
                            Some(("sort", value)) => {
                                if let Some(mode) = heronite_data::SortMode::parse(value) {
                                    this.set_sort(mode, cx);
                                } else {
                                    log::warn!("unknown toolbar sort: {value}");
                                }
                            }
                            Some(("group", value)) => {
                                if let Some(by) = heronite_data::GroupBy::parse(value) {
                                    this.set_group_by(by, cx);
                                } else {
                                    log::warn!("unknown toolbar grouping: {value}");
                                }
                            }
                            Some(("install", value)) => {
                                if let Some(filter) = heronite_data::InstallFilter::parse(value) {
                                    this.set_install_filter(filter, cx);
                                } else {
                                    log::warn!("unknown toolbar install scope: {value}");
                                }
                            }
                            Some(("preset", value)) => {
                                if let Some(view) = value.parse::<usize>().ok().and_then(|i| this.ui.saved_views.get(i)).cloned() {
                                    this.apply_view_filters(&view.filters, cx);
                                } else {
                                    log::warn!("unknown toolbar preset index: {value}");
                                }
                            }
                            Some(("hover", value @ ("on" | "off"))) => this.toolbar.debug_hover(value == "on"),
                            Some(("age", value)) => {
                                let age = match value.parse::<u8>() {
                                    Ok(max) => heronite_data::age_rating::AgeFilter { max: Some(max), ..this.ui.age },
                                    Err(_) => heronite_data::age_rating::AgeFilter { unrated: value == "unrated", ..this.ui.age },
                                };
                                this.apply_pick(&super::view_menu::Pick::Age(age), cx);
                            }
                            Some(("toolbar", value @ ("on" | "off"))) => {
                                this.set_look(|a| a.quick_toolbar_controls = value == "on", cx);
                            }
                            Some(("open", value)) => {
                                if let Some(menu) = quick_menu(value) {
                                    this.open_menu = Some(Menu::Quick(menu));
                                } else {
                                    log::warn!("unknown toolbar menu: {value}");
                                }
                            }
                            _ if step == "direction" => this.toggle_sort_direction(cx),
                            _ if step == "favourites" => {
                                this.apply_pick(&super::view_menu::Pick::Only(super::view_filters::Only::Favourites), cx)
                            }
                            _ if step == "reset" => this.reset_view_filters(cx),
                            _ => log::warn!("unknown HERONITE_DEBUG_TOOLBAR step: {step}"),
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
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
            w if w.starts_with("mod") && self.mods_debug_dialog(w, window, cx) => {}
            "statuses" => self.open_manage_statuses(window, cx),
            "hltb" => self.open_hltb_match(window, cx),
            "batchedit" => self.open_batch_edit(window, cx),
            "batchremove" => self.open_batch_remove(cx),
            "categories" => self.open_manage_categories(window, cx),
            // smartgroup (a new one), smartgroup:<id> (that one), smartgroup-filters (from the filters on screen)
            "smartgroup" => self.open_group_editor(None, None, window, cx),
            "smartgroup-filters" => self.save_filters_as_group(window, cx),
            w if w.starts_with("smartgroup:") => self.open_group_editor(Some(&w["smartgroup:".len()..]), None, window, cx),
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
            "metadata" | "gamesettings" | "gamesettings:details" | "gamesettings:launch" | "webimage" | "wizard" | "review" | "covers" => {
                self.open_metadata_debug(which.as_str(), window, cx)
            }
            w if w.starts_with("wizard:") || w.starts_with("metadata:") || w.starts_with("review:") || w.starts_with("covers:") => {
                self.open_metadata_debug(w, window, cx)
            }
            // "Get more sources" (`sources:<id>` picks that entry) and the
            // installer of a package (`extension-file:<path>`).
            w if w == "sources" || w.starts_with("sources:") => {
                let pick = w.strip_prefix("sources:").map(str::to_owned);
                self.open_sources(crate::dialogs::extensions::SourcesRequest::Gallery { pick }, window, cx);
            }
            w if w.starts_with("extension-file:") => {
                let path = std::path::PathBuf::from(&w["extension-file:".len()..]);
                self.open_sources(crate::dialogs::extensions::SourcesRequest::File(path), window, cx);
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
            // What the game menu's "ReShade" opens, for the selected game.
            "reshade" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_reshade(&key, window, cx);
                }
            }
            // "All achievements" of the selected game (`HERONITE_DEBUG_ACH_SET` picks a set).
            "achievements" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_achievements(&key, window, cx);
                }
            }
            "viewlog" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_game_log(&key, cx);
                }
            }
            w if w.starts_with("manual-install:") => self.open_manual_install(&w["manual-install:".len()..], window, cx),
            // The Copies card's Play / Install button of that copy.
            w if w.starts_with("play-copy:") => self.play_copy(&w["play-copy:".len()..], window, cx),
            w if w.starts_with("play-as-pc:") => self.open_play_as_pc(&w["play-as-pc:".len()..], window, cx),
            w if w.starts_with("local-uninstall:") => self.open_sideload_uninstall(&w["local-uninstall:".len()..], window, cx),
            w if matches!(w, "playtime" | "remove" | "ludusavi" | "steam-missing") || w.starts_with("ludusavi:") => {
                self.open_extras_debug(which.as_str(), window, cx)
            }
            w if w.starts_with("scan-folder") => self.scan_folder_debug_dialog(w, window, cx),
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
