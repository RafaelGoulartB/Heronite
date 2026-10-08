//! Collection leftovers: edit playtime, remove from Collection, manual
//! Ludusavi backup, the focus panel's achievements / session history /
//! "open details" button, the search box inside the facet filter flyout and
//! the "Steam not found" message.
//!
//! The data work lives in `heronite-data` (playtime, sessions, details URL),
//! `heronite-metadata` (remove, achievements) and `heronite-emulation`
//! (Ludusavi); this file only wires it to the screen.

use std::sync::Arc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::details_url::details_url;
use heronite_data::{CollectionGame, FacetKind};
use heronite_emulation::ludusavi::{LudusaviGame, SystemRunner};
use heronite_images::{ImageRef, Variant};
use heronite_metadata::achievements::{load_achievements, Achievement};
use heronite_metadata::{Endpoints, Http};

use super::context_menu::item;
use super::{achievements, focus, CollectionView};
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog};
use crate::dialogs::ludusavi_backup::LudusaviBackupDialog;
use crate::dialogs::playtime::PlaytimeDialog;
use crate::dialogs::remove_game::RemoveGameDialog;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Message of the error dialog Heronite shows when Steam is missing
/// (`collection.steam.clientMissing*`).
const STEAM_MISSING_TITLE: &str = "Steam not found";
const STEAM_MISSING_TEXT: &str = "Steam was not found. Install or launch the Steam client, then try again.";

/// Text of the Steam uninstall confirmation (`collection.steam.uninstallHelp`).
fn steam_uninstall_message(title: &str) -> String {
    format!(
        "This opens Steam to uninstall \"{title}\". The game stays in Collection; Heronite marks it uninstalled after Steam finishes and you refresh."
    )
}

/// Whether a facet option shows for the flyout search `query` (already
/// trimmed and lowercased): `value.toLowerCase().includes(needle)`.
pub(super) fn facet_option_matches(value: &str, query: &str) -> bool {
    query.is_empty() || value.to_lowercase().contains(query)
}

/// Whether a launch/install error means that there is no Steam client.
fn is_steam_missing(error: &str) -> bool {
    error.contains("Steam client was not found")
}

struct FacetSearch {
    kind: FacetKind,
    input: Entity<InputState>,
    _sub: Subscription,
}

#[derive(Default)]
pub(super) struct Extras {
    facet_search: Option<FacetSearch>,
    /// Game whose achievements are loaded (or loading).
    ach_key: Option<SharedString>,
    ach: Vec<Achievement>,
    steam_missing: bool,
    /// `HERONITE_DEBUG_SCROLL=list:<0..1>|focus:<0..1>` already applied.
    debug_scroll_done: bool,
}

impl CollectionView {
    /// Runs on every render, before the layout: opens what needs a `Window`,
    /// starts the achievements of a newly focused game, keeps the facet
    /// search in step with the open flyout.
    pub(super) fn sync_extras(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.extras.steam_missing) {
            let spec = Confirm::new("steam-missing", STEAM_MISSING_TITLE, STEAM_MISSING_TEXT);
            let view = cx.new(|cx| EpicConfirmDialog::new(spec, |_, _| {}, window, cx));
            self.open_dialog(view, cx);
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

        match (self.facet_flyout, self.extras.facet_search.as_ref().map(|f| f.kind)) {
            (Some(kind), Some(have)) if kind == have => {}
            (Some(kind), _) => {
                let input = cx.new(|cx| {
                    let mut s = InputState::new(window, cx);
                    s.set_placeholder("Filter list", window, cx);
                    s
                });
                let sub = cx.subscribe_in(&input, window, |_, _, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        cx.notify();
                    }
                });
                self.extras.facet_search = Some(FacetSearch { kind, input, _sub: sub });
            }
            (None, Some(_)) => self.extras.facet_search = None,
            (None, None) => {}
        }
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
            "playtime" => self.open_edit_playtime(&key, window, cx),
            "remove" => self.open_remove_game(&key, cx),
            _ => self.open_ludusavi_backup(&key, cx),
        }
    }

    fn sync_achievements(&mut self, cx: &mut Context<Self>) {
        let target = self.selected.clone();
        if self.extras.ach_key == target {
            return;
        }
        self.extras.ach_key = target.clone();
        self.extras.ach.clear();
        let (Some(lib), Some(key)) = (self.lib.clone(), target) else { return };
        let Some(game) = lib.get(&key) else { return };
        // Only GOG has achievements (Heroic's `getAchievements`).
        if game.runner != "gog" {
            return;
        }
        // The cache is a small local file: show it right away, no flicker.
        if let Some(cached) = heronite_metadata::achievements::read_cache(&self.heroic, &game.app_name) {
            self.extras.ach = cached;
            return;
        }
        let (paths, runner, app_name) = (self.heroic.clone(), game.runner.clone(), game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let items = cx
                .background_spawn(async move {
                    load_achievements(&paths, &Http::new(Endpoints::default()), &runner, &app_name)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.extras.ach_key.as_ref() == Some(&key) {
                    this.extras.ach = items;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A launch/install failed: a missing Steam client gets Heronite's
    /// "Steam not found" message, everything else a toast.
    pub(super) fn report_launch_error(&mut self, error: String, cx: &mut Context<Self>) {
        if is_steam_missing(&error) {
            self.extras.steam_missing = true;
            cx.notify();
        } else {
            self.show_toast(error, true, cx);
        }
    }

    /// Details, Edit playtime, Backup save (Ludusavi) and Remove from
    /// Collection (the order of `CollectionCard`'s menu).
    pub(super) fn extras_context_items(&mut self, mut menu: Div, game: &CollectionGame, cx: &mut Context<Self>) -> Div {
        let key = game.key.clone();
        let tracking = self.playing.contains_key(&game.app_name);
        let row = |id: &'static str, ic: Icon, label: &'static str| {
            item(id).child(icon(ic, px(18.), crate::theme::text_muted())).child(label)
        };

        if details_url(game).is_some() {
            let k = key.clone();
            menu = menu.child(row("cm-details", Icon::OpenInNew, "Details").on_click(cx.listener(move |this, _, _, cx| {
                this.context_menu = None;
                this.open_details(&k, cx);
                cx.notify();
            })));
        }
        if !tracking {
            let k = key.clone();
            menu = menu.child(row("cm-playtime", Icon::AccessTime, "Edit playtime").on_click(cx.listener(
                move |this, _, window, cx| this.open_edit_playtime(&k, window, cx),
            )));
        }
        let k = key.clone();
        menu = menu.child(
            row("cm-ludusavi", Icon::Save, "Backup save (Ludusavi)").on_click(cx.listener(move |this, _, _, cx| {
                this.open_ludusavi_backup(&k, cx);
            })),
        );
        if game.steam_launch_id.is_some() && game.installed && !tracking {
            let k = key.clone();
            menu = menu.child(row("cm-steam-uninstall", Icon::DeleteForever, "Uninstall").on_click(cx.listener(
                move |this, _, window, cx| this.open_steam_uninstall(&k, window, cx),
            )));
        }
        if game.runner == "sideload" && !tracking {
            let k = key.clone();
            menu = menu.child(row("cm-remove-collection", Icon::DeleteForever, "Remove from Collection").on_click(
                cx.listener(move |this, _, _, cx| this.open_remove_game(&k, cx)),
            ));
        }
        menu
    }

    pub(super) fn open_edit_playtime(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let view = cx.new(|cx| PlaytimeDialog::new(ctx, game.app_name.clone(), game.title.clone(), game.playtime_minutes, window, cx));
        self.open_dialog(view, cx);
    }

    /// Uninstall of a Steam game: asks, then opens Steam's uninstall page
    /// (`steam://uninstall/<id>`). The game stays in the Collection.
    pub(super) fn open_steam_uninstall(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let Some(app_id) = game.steam_launch_id.clone() else { return };
        let mut spec = Confirm::new("steam-uninstall", "Uninstall", steam_uninstall_message(&game.title));
        spec.confirm = Some("Yes".into());
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

    /// The focus panel's "open details" button / the "Details" menu item:
    /// the game's store page in the browser.
    pub(super) fn open_details(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        match details_url(&game) {
            Some(url) => {
                if let Err(err) = std::process::Command::new("xdg-open")
                    .arg(&url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .and_then(|mut child| std::thread::Builder::new().spawn(move || child.wait()).map(|_| ()))
                {
                    self.show_toast(format!("Could not open {url}: {err}"), true, cx);
                }
            }
            None => self.show_toast("No store page is known for this game.", false, cx),
        }
    }

    /// Achievements (above Details).
    pub(super) fn focus_extras(&mut self, key: &str, scale: f32) -> focus::FocusExtras {
        let mut extras = focus::FocusExtras::default();
        if self.lib.as_ref().and_then(|lib| lib.get(key)).is_none() {
            return extras;
        }

        if self.extras.ach_key.as_deref() == Some(key) && !self.extras.ach.is_empty() {
            let total = self.extras.ach.len();
            let (unlocked, mut rows) = achievements::rows(&self.extras.ach);
            let (pw, ph) = ((28. * scale).round() as u32, (28. * scale).round() as u32);
            for row in rows.iter_mut().filter(|r| !r.image_url.is_empty()) {
                let url = row.image_url.clone();
                let id: SharedString = format!("ach:{url}:{pw}x{ph}").into();
                let chain = if url.starts_with("http") {
                    ImageRef::Url(url)
                } else {
                    ImageRef::File(std::path::PathBuf::from(url))
                };
                row.image = self
                    .images
                    .get_or_request(&id, || vec![chain], pw, ph, Variant::Cover, 30)
                    .map(ImageSource::Render);
            }
            extras.top = Some(achievements::view(unlocked, total, rows));
        }
        extras
    }

    /// What is typed in the open flyout's search (`kind` must be the open one).
    pub(super) fn facet_query(&self, kind: FacetKind, cx: &App) -> String {
        match &self.extras.facet_search {
            Some(f) if f.kind == kind => f.input.read(cx).value().trim().to_lowercase(),
            _ => String::new(),
        }
    }

    /// The "Filter list" box at the top of the flyout.
    pub(super) fn facet_search_box(&self, kind: FacetKind) -> Option<Div> {
        let f = self.extras.facet_search.as_ref().filter(|f| f.kind == kind)?;
        Some(
            div()
                .mx(px(5.6))
                .mb(px(3.))
                .px(px(8.))
                .h(px(30.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .bg(theme::white_a(0.07))
                .border_1()
                .border_color(theme::white_a(0.1))
                .text_size(px(14.))
                .text_color(white())
                .child(div().flex_1().min_w_0().child(Input::new(&f.input).appearance(false))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{facet_option_matches, is_steam_missing, steam_uninstall_message};

    #[test]
    fn flyout_search_is_a_case_insensitive_substring_match() {
        assert!(facet_option_matches("Blizzard Entertainment", ""));
        assert!(facet_option_matches("Blizzard Entertainment", "zard ent"));
        assert!(facet_option_matches("Ōkami Studio", "studio"));
        assert!(!facet_option_matches("Valve", "blizzard"));
    }

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
