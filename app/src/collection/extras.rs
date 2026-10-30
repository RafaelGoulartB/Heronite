//! Collection leftovers: edit playtime, remove from Collection, manual
//! Ludusavi backup, the focus panel's achievements / session history /
//! "open details" button and the "Steam not found" and "Proton needed"
//! messages.
//!
//! The data work lives in `heronite-data` (playtime, sessions, details URL),
//! `heronite-metadata` (remove), `heronite-achievements` (through
//! `services::achievements`) and `heronite-emulation` (Ludusavi); this file
//! only wires it to the screen.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::details_url::details_url;
use heronite_data::CollectionGame;
use heronite_emulation::ludusavi::{LudusaviGame, SystemRunner};
use heronite_emulation::settings_ext::set_auto_backup;
use heronite_images::{ImageRef, Variant};

use super::game_actions::{GameAction, GameActionItem};
use super::{achievements, focus, CollectionView};
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog};
use crate::dialogs::ludusavi_backup::LudusaviBackupDialog;
use crate::dialogs::remove_game::RemoveGameDialog;
use crate::icons::Icon;
use crate::pages::Page;
use heronite_i18n::tr;

/// Text of the Steam uninstall confirmation (`collection.steam.uninstallHelp`).
fn steam_uninstall_message(title: &str) -> String {
    tr!(
        "This opens Steam to uninstall \"{title}\". The game stays in Collection; Heronite marks it uninstalled after Steam finishes and you refresh.",
        title
    )
}

/// Whether a launch/install error means that there is no Steam client
/// (the message may be translated).
fn is_steam_missing(error: &str) -> bool {
    error.contains("Steam client was not found") || error.contains(heronite_i18n::tr("Steam client was not found"))
}

#[derive(Default)]
pub(super) struct Extras {
    /// Game whose achievements were asked for.
    ach_key: Option<SharedString>,
    steam_missing: bool,
    /// A Windows game could not start: no Wine/Proton on this PC.
    wine_missing: bool,
    /// A page a dialog asked for (dialogs have no `Window` to navigate with).
    go_to: Option<Page>,
    /// `HERONITE_DEBUG_SCROLL=list:<0..1>|focus:<0..1>` already applied.
    debug_scroll_done: bool,
}

impl CollectionView {
    /// Runs on every render, before the layout: opens what needs a `Window`,
    /// starts the achievements of a newly focused game, keeps the facet
    /// search in step with the open flyout.
    pub(super) fn sync_extras(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.extras.steam_missing) {
            // Heronite's `collection.steam.clientMissing*` message.
            let spec = Confirm::new(
                "steam-missing",
                tr!("Steam not found"),
                tr!("Steam was not found. Install or launch the Steam client, then try again."),
            );
            let view = cx.new(|cx| EpicConfirmDialog::new(spec, |_, _| {}, window, cx));
            self.open_dialog(view, cx);
        }
        if std::mem::take(&mut self.extras.wine_missing) {
            let mut spec = Confirm::new(
                "wine-missing",
                tr!("Proton is needed"),
                tr!("Windows games run through Proton, and there is none on this PC yet. Download one in the Wine Manager: it becomes the default, then press Play again."),
            );
            spec.confirm = Some(tr!("Download Proton").into());
            let this = cx.entity().downgrade();
            let view = cx.new(|cx| {
                EpicConfirmDialog::new(
                    spec,
                    move |_, cx| {
                        this.update(cx, |this, cx| {
                            this.extras.go_to = Some(Page::WineManager);
                            cx.notify();
                        })
                        .ok();
                    },
                    window,
                    cx,
                )
            });
            self.open_dialog(view, cx);
        }
        if let Some(page) = self.extras.go_to.take() {
            self.navigate(page, window, cx);
        }

        // A game removed from the Collection leaves the focus panel.
        if let (Some(lib), Some(key)) = (self.lib.clone(), self.selected.clone()) {
            if lib.get(&key).is_none() {
                self.selected = None;
                self.focus = None;
            }
        }

        self.sync_achievements(cx);
        self.debug_scroll(cx);
    }

    /// Screenshot helper: scrolls the list or the focus panel the way a
    /// scrollbar drag does (`HERONITE_DEBUG_SCROLL=list:0.5`), once the
    /// content has been measured.
    fn debug_scroll(&mut self, cx: &mut Context<Self>) {
        if self.extras.debug_scroll_done {
            return;
        }
        let Some(spec) = crate::env::value("HERONITE_DEBUG_SCROLL") else {
            self.extras.debug_scroll_done = true;
            return;
        };
        let Some((which, fraction)) = spec.split_once(':').and_then(|(w, f)| Some((w, f.parse::<f32>().ok()?))) else {
            self.extras.debug_scroll_done = true;
            return;
        };
        let done = match which {
            "list" => {
                let max = f32::from(self.list_state.max_offset_for_scrollbar().y);
                if max > 0. {
                    self.list_state.set_offset_from_scrollbar(point(px(0.), px(-fraction * max)));
                }
                max > 0.
            }
            _ => {
                let max = f32::from(self.focus_scroll.max_offset().y);
                if max > 0. {
                    self.focus_scroll.set_offset(point(px(0.), px(-fraction * max)));
                }
                max > 0.
            }
        };
        if done {
            self.extras.debug_scroll_done = true;
        }
        cx.notify();
    }

    /// `HERONITE_DEBUG_DIALOG=playtime|remove|ludusavi|steam-missing` for the
    /// selected game (`HERONITE_DEBUG_SELECT`).
    pub(super) fn open_extras_debug(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        if which == "steam-missing" {
            self.extras.steam_missing = true;
            return;
        }
        let Some(key) = self.selected.as_ref().map(|k| k.to_string()) else { return };
        if let Some(kind) = which.strip_prefix("ludusavi:") {
            use crate::dialogs::ludusavi_backup::{describe, Outcome};
            use heronite_emulation::ludusavi::LudusaviBackupResult;
            let result = match kind {
                "busy" => LudusaviBackupResult { skipped_reason: Some("already-running".into()), ..Default::default() },
                "error" => LudusaviBackupResult { error: Some("Ludusavi was not found".into()), ..Default::default() },
                _ => LudusaviBackupResult {
                    ran: true,
                    game_name: Some("Portal 2".into()),
                    backup_path: Some("/tmp/heronite-fixture-collection/ludusavi-out/Portal 2".into()),
                    total_games: Some(1),
                    ..Default::default()
                },
            };
            let outcome: Outcome = describe(&result, "Portal 2");
            let view = cx.new(|_| LudusaviBackupDialog::showing(outcome));
            self.open_dialog(view, cx);
            return;
        }
        match which {
            "playtime" => self.open_playtime_settings(&key, window, cx),
            "remove" => self.open_remove_game(&key, cx),
            _ => self.open_ludusavi_backup(&key, cx),
        }
    }

    /// Asks the achievements service for the newly focused game.
    fn sync_achievements(&mut self, cx: &mut Context<Self>) {
        // Off: ask again for the focused game once they are turned on.
        let target = self.selected.clone().filter(|_| crate::services::achievements::enabled());
        if self.extras.ach_key == target {
            return;
        }
        self.extras.ach_key = target.clone();
        let (Some(lib), Some(key)) = (self.lib.clone(), target) else { return };
        let Some(game) = lib.get(&key).cloned() else { return };
        crate::services::achievements::get(cx).update(cx, |s, cx| s.want(&game, cx));
    }

    /// A launch/install failed: a missing Steam client gets Heronite's
    /// "Steam not found" message, everything else a toast.
    pub(super) fn report_launch_error(&mut self, error: String, cx: &mut Context<Self>) {
        if is_steam_missing(&error) {
            self.extras.steam_missing = true;
            cx.notify();
        } else if heronite_launch::wine::is_no_wine_error(&error) {
            self.extras.wine_missing = true;
            cx.notify();
        } else {
            self.show_toast(error, true, cx);
        }
    }

    /// Backup save and its after-play switch (Ludusavi), Uninstall and Remove from Collection, and for
    /// the palette only Details (the store page) and Edit playtime (the time
    /// played is edited in the game settings).
    pub(super) fn extras_actions(&self, game: &CollectionGame) -> Vec<GameActionItem> {
        use GameAction as A;
        let item = GameActionItem::new;
        let tracking = self.playing.contains_key(&game.app_name);
        let mut v = Vec::new();
        if details_url(game).is_some() {
            // Palette only: the store page is a rare stop.
            v.push(
                item(
                    A::Details,
                    "details",
                    tr!("Details"),
                    Icon::OpenInNew,
                    "store page website browser loja pagina site navegador detalhes",
                )
                .palette_only(),
            );
        }
        if !tracking {
            v.push(
                item(A::EditPlaytime, "playtime", tr!("Edit playtime"), Icon::AccessTime, "hours time played editar tempo horas jogado")
                    .palette_only(),
            );
        }
        if heronite_platform::supported(heronite_platform::Feature::SaveBackups) {
            v.push(item(A::Ludusavi, "ludusavi", tr!("Backup save"), Icon::Save, "saves backup ludusavi salvamentos jogos salvos"));
            let auto = self.auto_backup(game);
            v.push(item(
                A::AutoBackup { on: !auto },
                "auto-backup",
                if auto { tr!("Turn off automatic save backup") } else { tr!("Turn on automatic save backup") },
                Icon::CloudSync,
                "saves backup ludusavi after play automatic salvamentos jogos salvos automatico depois de jogar",
            ));
        }
        if game.steam_launch_id.is_some() && game.installed && !tracking {
            v.push(item(
                A::SteamUninstall,
                "steam-uninstall",
                tr!("Uninstall"),
                Icon::DeleteForever,
                "remove delete steam desinstalar remover excluir",
            ));
        }
        if !tracking && super::sideload_uninstall::can_uninstall_local(game) {
            v.push(item(
                A::LocalUninstall,
                "local-uninstall",
                tr!("Uninstall"),
                Icon::DeleteForever,
                "remove delete files prefix desinstalar remover excluir arquivos prefixo",
            ));
        }
        if game.runner == "sideload" && !tracking {
            v.push(item(
                A::RemoveFromCollection,
                "remove-collection",
                tr!("Remove from Collection"),
                Icon::DeleteForever,
                "delete entry remover excluir entrada coleção colecao",
            ));
        }
        v
    }

    /// The game settings on the Game tab, where the time played is edited.
    pub(super) fn open_playtime_settings(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        use crate::dialogs::game_settings::{GameSettingsDialog, Tab};
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, Tab::Game, window, cx));
        self.open_dialog(view, cx);
    }

    /// Uninstall of a Steam game (or of a game's Steam copy): asks, then opens Steam's uninstall page
    /// (`steam://uninstall/<id>`). The game stays in the Collection.
    pub(super) fn open_steam_uninstall(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let Some(app_id) = game.steam_installed_id().map(str::to_string) else { return };
        let mut spec = Confirm::new("steam-uninstall", tr!("Uninstall"), steam_uninstall_message(&game.title));
        spec.confirm = Some(tr!("Yes").into());
        let weak = cx.entity().downgrade();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |_, cx| {
                    let app_id = app_id.clone();
                    weak.update(cx, |this, cx| this.steam_uninstall(&app_id, cx)).ok();
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    fn steam_uninstall(&mut self, app_id: &str, cx: &mut Context<Self>) {
        use heronite_launch::steam::{open_steam_client_uri, SteamAction};
        use heronite_launch::{SystemProbe, SystemSpawner};
        if let Err(err) = open_steam_client_uri(&SystemSpawner::default(), &SystemProbe, SteamAction::Uninstall, app_id) {
            self.report_launch_error(err, cx);
        }
    }

    pub(super) fn open_remove_game(&mut self, key: &str, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let view = cx.new(|_| RemoveGameDialog::new(ctx, game.app_name.clone(), game.runner.clone(), game.title.clone()));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_ludusavi_backup(&mut self, key: &str, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let target = LudusaviGame {
            app_name: game.app_name.clone(),
            title: game.title.clone(),
            runner: game.runner.clone(),
            // `steamAppIdFromMeta`: only Steam-launched games.
            steam_app_id: game.steam_launch_id.clone(),
            store_game_id: game.meta.as_ref().and_then(|m| m.store_game_id.clone()),
        };
        let view = cx.new(|cx| LudusaviBackupDialog::new(ctx, target, Arc::new(SystemRunner::default()), cx));
        self.open_dialog(view, cx);
    }

    /// Whether a session of `game` ends with a save backup.
    pub(super) fn auto_backup(&self, game: &CollectionGame) -> bool {
        self.lib.as_ref().is_some_and(|l| l.settings.ludusavi.auto_backup_for(&game.runner, &game.app_name))
    }

    /// Leaves the game out of the after-play backup or puts it back in. While
    /// that backup is off for every game, opens Settings → Saves instead.
    pub(super) fn set_auto_backup(&mut self, key: &str, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key) else { return };
        if !lib.settings.ludusavi.enabled {
            self.open_settings_section(crate::pages::settings::Section::Saves, window, cx);
            return;
        }
        let (paths, runner, app_name) = (self.data_paths.clone(), game.runner.clone(), game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { set_auto_backup(&paths, &runner, &app_name, on) }).await;
            this.update(cx, |this, cx| {
                if let Err(err) = res {
                    this.show_toast(tr!("Could not save the setting: {error}", error = format!("{err:#}")), true, cx);
                }
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The focus panel's "open details" button / the "Details" menu item:
    /// the game's store page in the browser.
    pub(super) fn open_details(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        match details_url(&game) {
            Some(url) => {
                if let Err(err) = heronite_platform::shell::open(&url) {
                    self.show_toast(tr!("Could not open {url}: {err}", url, err = err.to_string()), true, cx);
                }
            }
            None => self.show_toast(tr!("No store page is known for this game."), false, cx),
        }
    }

    /// The Achievements section.
    pub(super) fn focus_extras(&mut self, key: &str, scale: f32, cx: &mut Context<Self>) -> focus::FocusExtras {
        let mut extras = focus::FocusExtras::default();
        if !crate::services::achievements::enabled() {
            return extras;
        }
        let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else { return extras };
        let (hide_secrets, entry) = {
            let service = crate::services::achievements::get(cx);
            let service = service.read(cx);
            (service.prefs().hide_secrets, service.entry(&crate::services::achievements::key_of(&game.runner, &game.app_name)).cloned())
        };
        let Some(entry) = entry else { return extras };
        let body = achievements::body_of(&entry, |set| {
            let (recent, next) = achievements::picks(set);
            let mut row = |a: &heronite_achievements::Achievement| {
                let image = self.achievement_icon(a.icon_now(), achievements::ROW_ICON, scale);
                achievements::Row::of(a, hide_secrets, image)
            };
            (recent.into_iter().map(&mut row).collect(), next.into_iter().map(&mut row).collect())
        });
        let Some(body) = body else { return extras };
        let weak = cx.entity().downgrade();
        let k = SharedString::from(key.to_string());
        let actions = achievements::Actions {
            view_all: Rc::new({
                let (weak, k) = (weak.clone(), k.clone());
                move |window, cx| {
                    weak.update(cx, |this, cx| this.open_achievements(&k, window, cx)).ok();
                }
            }),
            refresh: Rc::new({
                let key = crate::services::achievements::key_of(&game.runner, &game.app_name);
                move |_, cx| {
                    crate::services::achievements::get(cx).update(cx, |s, cx| s.refresh(&key, cx));
                }
            }),
            settings: Rc::new(move |window, cx| {
                weak.update(cx, |this, cx| this.open_settings_section(crate::pages::settings::Section::Achievements, window, cx)).ok();
            }),
        };
        extras.achievements = Some(achievements::view(body, actions));
        extras
    }

    /// "All achievements" of a game (`key` = its Collection key).
    pub(super) fn open_achievements(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else { return };
        let Some(ctx) = self.dialog_ctx() else { return };
        let service_key = crate::services::achievements::key_of(&game.runner, &game.app_name);
        let title = game.title.clone();
        let view = cx.new(|cx| crate::dialogs::achievements::AchievementsDialog::new(ctx, service_key, title, window, cx));
        self.open_dialog(view, cx);
    }

    /// An achievement icon (URL or file) at `size` logical px, once loaded.
    pub(super) fn achievement_icon(&mut self, url: &str, size: f32, scale: f32) -> Option<ImageSource> {
        if url.is_empty() {
            return None;
        }
        let px_size = (size * scale).round() as u32;
        let id: SharedString = format!("ach:{url}:{px_size}").into();
        let chain = if url.starts_with("http") { ImageRef::Url(url.to_string()) } else { ImageRef::File(std::path::PathBuf::from(url)) };
        self.images.get_or_request(&id, || vec![chain], px_size, px_size, Variant::Cover, 30).map(ImageSource::Render)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_steam_missing, steam_uninstall_message};

    #[test]
    fn steam_uninstall_text_names_the_game() {
        assert!(steam_uninstall_message("Portal 2").starts_with("This opens Steam to uninstall \"Portal 2\"."));
    }

    #[test]
    fn steam_errors_are_recognised() {
        assert!(is_steam_missing("Steam client was not found"));
        assert!(is_steam_missing("Steam client was not found on PATH"));
        assert!(!is_steam_missing("Epic games are installed from the install dialog."));
    }
}
