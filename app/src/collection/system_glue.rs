//! Entry points for the desktop integration (`crate::system`): launch a game
//! asked for from outside (command line, protocol URL, tray, another
//! instance) through the same flow as the play button.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::game_actions::{GameAction, GameActionItem};
use super::CollectionView;
use crate::icons::Icon;
use crate::system::candidate_keys;
use crate::system::cli::LaunchRequest;
use crate::system::shortcuts as sc;
use heronite_i18n::tr;

impl CollectionView {
    /// `--console`: switches to console mode the way the palette's "Console
    /// mode" does (remembered).
    pub fn enter_console_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_console(true, window, cx);
    }

    /// Selects the game and presses its primary button (play, or the install
    /// flow when it is not installed) as soon as the library is loaded.
    pub fn system_launch(&mut self, req: LaunchRequest, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            for _ in 0..200 {
                let ready = this.update(cx, |this, _| this.lib.is_some()).unwrap_or(false);
                if ready {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
            this.update(cx, |this, cx| this.system_launch_now(req, cx)).ok();
        })
        .detach();
    }

    fn system_launch_now(&mut self, req: LaunchRequest, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = candidate_keys(&req).iter().find_map(|k| lib.get(k)).cloned() else {
            self.show_toast(tr!("Could not find the game {name} in the library", name = req.app_name), true, cx);
            return;
        };
        if self.page != crate::pages::Page::Collection {
            self.page = crate::pages::Page::Collection;
        }
        self.select(game.key.clone().into(), true);
        self.reveal_selected();
        cx.notify();
        if self.playing.contains_key(&game.app_name) {
            self.show_toast(tr!("{title} is already running", title = game.title), false, cx);
            return;
        }
        if crate::env::flag("HERONITE_DEBUG_FAKE_LAUNCH") {
            log::info!("[fake-launch] window {} ({}/{}) installed={}", game.title, game.runner, game.app_name, game.installed);
            return;
        }
        let key = game.key.clone();
        self.primary_action(&key, cx);
    }

    /// What the window-independent post-game service reports.
    pub(super) fn on_session_event(&mut self, ev: &crate::services::session::SessionEvent, cx: &mut Context<Self>) {
        use crate::services::session::SessionEvent;
        match ev {
            SessionEvent::Toast { text, error } => self.show_toast(text.clone(), *error, cx),
            SessionEvent::Capture(c) => self.shots_on_msg(super::screenshots::Msg::Capture(c.clone()), cx),
        }
        cx.notify();
    }

    /// The background compression renamed captures (`.png` -> `.webp`).
    pub(super) fn on_compression_event(&mut self, ev: &crate::services::compression::CompressionEvent, cx: &mut Context<Self>) {
        use crate::services::compression::CompressionEvent;
        match ev {
            CompressionEvent::Converted(renamed) => {
                if self.shots.follow_renames(renamed) {
                    cx.notify();
                }
            }
            CompressionEvent::ArtChanged => self.reload(cx),
        }
    }

    /// The question before quitting while games run (window X without a tray
    /// to stay in, tray "Quit").
    pub fn system_quit_confirm(
        &mut self,
        games: Vec<String>,
        post_game: bool,
        install: Option<String>,
        tray: bool,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|_| crate::dialogs::quit_confirm::QuitConfirmDialog::new(games, post_game, install, tray));
        self.open_dialog(view, cx);
    }

    /// A toast for a message from the desktop integration.
    pub fn system_toast(&mut self, text: String, error: bool, cx: &mut Context<Self>) {
        self.show_toast(text, error, cx);
    }
}

/// What the menu shows for a game: filled in the background when the menu is
/// first drawn for it (the checks read files, never on the UI thread).
struct Probe {
    shortcuts: bool,
    steam: bool,
    at: Instant,
    pending: bool,
}

#[derive(Default)]
struct MenuProbes(HashMap<String, Probe>);
impl Global for MenuProbes {}

impl CollectionView {
    /// "Add/Remove shortcuts" and "Add to / Remove from Steam" for installed
    /// games (Heroic's context menu).
    pub(super) fn system_actions(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<GameActionItem> {
        if !game.installed || matches!(game.launch_kind.as_deref(), Some("steam-uri")) || game.game.install.is_dlc == Some(true) {
            return Vec::new();
        }
        let (has_shortcuts, in_steam) = self.probe_shortcuts(game, cx);
        vec![
            GameActionItem::new(
                GameAction::Shortcuts { add: !has_shortcuts },
                "shortcuts",
                if has_shortcuts { tr!("Remove shortcuts") } else { tr!("Add shortcuts") },
                Icon::OpenInNew,
                "desktop application menu launcher atalho atalhos area de trabalho aplicativos menu",
            ),
            GameActionItem::new(
                GameAction::Steam { add: !in_steam },
                "steam",
                if in_steam { tr!("Remove from Steam") } else { tr!("Add to Steam") },
                Icon::SportsEsports,
                "non-steam shortcut big picture atalho adicionar remover",
            ),
        ]
    }

    pub(super) fn run_shortcuts(&mut self, key: &str, add: bool, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let (r, a) = (game.runner.clone(), game.app_name.clone());
        self.system_op(key.to_string(), cx, move |data_paths| {
            if add {
                sc::add_shortcuts(data_paths, &r, &a)
            } else {
                sc::remove_shortcuts(data_paths, &r, &a)
            }
        });
    }

    pub(super) fn run_steam_shortcut(&mut self, key: &str, add: bool, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let (r, a) = (game.runner.clone(), game.app_name.clone());
        self.system_op(key.to_string(), cx, move |data_paths| {
            if add {
                sc::add_to_steam(data_paths, &r, &a)
            } else {
                sc::remove_from_steam(data_paths, &r, &a)
            }
        });
    }

    /// Cached `(shortcuts exist, added to Steam)`; starts a refresh when the
    /// entry is missing or older than a few seconds.
    pub(super) fn probe_shortcuts(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> (bool, bool) {
        let key = game.key.clone();
        let current = cx
            .default_global::<MenuProbes>()
            .0
            .get(&key)
            .map(|p| (p.shortcuts, p.steam, p.pending, p.at.elapsed() > Duration::from_secs(5)));
        let (shortcuts, steam, pending, stale) = current.unwrap_or((false, false, false, true));
        if stale && !pending {
            cx.default_global::<MenuProbes>().0.insert(key.clone(), Probe { shortcuts, steam, at: Instant::now(), pending: true });
            let data_paths = self.data_paths.clone();
            let (runner, app) = (game.runner.clone(), game.app_name.clone());
            cx.spawn(async move |this, cx| {
                let probed = cx
                    .background_spawn(async move {
                        (sc::shortcuts_exist(&data_paths, &runner, &app), sc::is_added_to_steam(&data_paths, &runner, &app))
                    })
                    .await;
                this.update(cx, |_, cx| {
                    cx.default_global::<MenuProbes>()
                        .0
                        .insert(key, Probe { shortcuts: probed.0, steam: probed.1, at: Instant::now(), pending: false });
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        (shortcuts, steam)
    }

    /// Runs a shortcuts operation in the background, shows its result as a
    /// toast and forgets the cached menu state.
    pub(super) fn system_op(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
        op: impl FnOnce(&heronite_data::DataPaths) -> sc::Outcome + Send + 'static,
    ) {
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { op(&data_paths) }).await;
            this.update(cx, |this, cx| {
                cx.default_global::<MenuProbes>().0.remove(&key);
                if let Some(m) = outcome.summary() {
                    this.show_toast(m.text, m.error, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
