//! `HERONITE_DEBUG_*` hooks of the Collection: open a menu or dialog on start
//! so screenshots and tests can reach it without clicking.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::FacetKind;

use super::header::Menu;
use super::{context_menu, CollectionView};

impl CollectionView {
    /// `HERONITE_DEBUG_MENU=<card key>` opens its context menu (with the
    /// status flyout), `HERONITE_DEBUG_DROPDOWN=view|nav|facets`
    /// opens a header menu, `HERONITE_DEBUG_DIALOG=<name>` opens a dialog on
    /// the first render that has a window (`open_pending_debug_dialog`).
    pub(super) fn apply_debug_env(&mut self) {
        if let Some(key) = crate::env::value("HERONITE_DEBUG_MENU") {
            self.context_menu = Some(context_menu::ContextMenuState {
                key: key.into(),
                position: point(px(200.), px(160.)),
                status_open: true,
            });
        }
        if let Some(which) = crate::env::value("HERONITE_DEBUG_DIALOG") {
            self.pending_debug_dialog = Some(which);
        }
        if let Some(menu) = crate::env::value("HERONITE_DEBUG_DROPDOWN") {
            self.open_menu = match menu.as_str() {
                "view" | "install" | "sort" | "status" => Some(Menu::View),
                "nav" => Some(Menu::Nav),
                "facets" => {
                    self.facet_flyout = Some(FacetKind::Developer);
                    Some(Menu::View)
                }
                _ => None,
            };
        }
    }

    pub(super) fn open_pending_debug_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.pending_debug_dialog.take() else { return };
        match which.as_str() {
            w if w == "settings" || w.starts_with("settings:") => {
                let section = w.strip_prefix("settings:").and_then(crate::pages::settings::Section::parse);
                self.open_settings_section(section.unwrap_or(crate::pages::settings::Section::Appearance), window, cx);
            }
            "statuses" => self.open_manage_statuses(window, cx),
            "gallery" | "viewer" | "viewer-zoom" | "viewer-fit" | "confirm" | "delete" => self.shots.debug = Some(which),
            // Heroic game settings tabs: gamesettings:wine|other|advanced|gamescope|saves
            w @ ("gamesettings:wine" | "gamesettings:other" | "gamesettings:advanced" | "gamesettings:gamescope" | "gamesettings:saves") => {
                use crate::dialogs::game_settings::Tab;
                let tab = match w {
                    "gamesettings:wine" => Tab::Wine,
                    "gamesettings:other" => Tab::Other,
                    "gamesettings:advanced" => Tab::Advanced,
                    "gamesettings:saves" => Tab::Saves,
                    _ => Tab::Gamescope,
                };
                self.open_game_settings_tab(tab, window, cx);
            }
            "metadata" | "gamesettings" | "gamesettings:details" | "gamesettings:launch" | "webimage" | "wizard" => {
                self.open_metadata_debug(which.as_str(), window, cx)
            }
            "viewlog" => {
                if let Some(key) = self.selected.as_ref().map(|k| k.to_string()) {
                    self.open_game_log(&key, cx);
                }
            }
            w if matches!(w, "playtime" | "remove" | "ludusavi" | "steam-missing") || w.starts_with("ludusavi:") => self.open_extras_debug(which.as_str(), window, cx),
            w if w.starts_with("add-game") || w.starts_with("edit-game") => {
                self.add_game_debug_dialog(w, window, cx);
            }
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
