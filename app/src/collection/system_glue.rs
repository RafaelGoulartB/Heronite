//! Entry points for the desktop integration (`crate::system`): launch a game
//! asked for from outside (command line, protocol URL, tray, another
//! instance) through the same flow as the play button.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::CollectionView;
use crate::system::cli::LaunchRequest;
use crate::system::candidate_keys;

impl CollectionView {
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
            self.show_toast(format!("Could not find the game {} in the library", req.app_name), true, cx);
            return;
        };
        if self.page != crate::pages::Page::Collection {
            self.page = crate::pages::Page::Collection;
        }
        self.select(game.key.clone().into(), true);
        self.reveal_selected();
        cx.notify();
        if self.playing.contains_key(&game.app_name) {
            self.show_toast(format!("{} is already running", game.title), false, cx);
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

    /// The question before quitting while games run (window X without a tray
    /// to stay in, tray "Quit").
    pub fn system_quit_confirm(&mut self, games: Vec<String>, post_game: bool, tray: bool, cx: &mut Context<Self>) {
        let view = cx.new(|_| crate::dialogs::quit_confirm::QuitConfirmDialog::new(games, post_game, tray));
        self.open_dialog(view, cx);
    }

    /// A toast for a message from the desktop integration.
    pub fn system_toast(&mut self, text: String, error: bool, cx: &mut Context<Self>) {
        self.show_toast(text, error, cx);
    }
}

// ---------------------------------------------------------------------------
// Context menu: shortcuts / Steam
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::time::Instant;

use heronite_data::CollectionGame;

use super::context_menu::item;
use crate::icons::Icon;
use crate::system::shortcuts as sc;
use crate::ui::icon;

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
    /// games (Heroic's context menu). One block, called from `context_menu.rs`.
    pub(super) fn system_context_items(&mut self, mut menu: Div, game: &CollectionGame, cx: &mut Context<Self>) -> Div {
        if !game.installed || matches!(game.launch_kind.as_deref(), Some("steam-uri")) || game.game.install.is_dlc == Some(true) {
            return menu;
        }
        let (has_shortcuts, in_steam) = self.probe_shortcuts(game, cx);
        let (runner, app) = (game.runner.clone(), game.app_name.clone());
        let key = game.key.clone();

        let (r, a, k) = (runner.clone(), app.clone(), key.clone());
        menu = menu.child(
            item("cm-shortcuts")
                .child(icon(Icon::OpenInNew, px(18.), crate::theme::text_muted()))
                .child(if has_shortcuts { "Remove shortcuts" } else { "Add shortcuts" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.context_menu = None;
                    let (r, a, k) = (r.clone(), a.clone(), k.clone());
                    this.system_op(k, cx, move |heroic| {
                        if has_shortcuts {
                            sc::remove_shortcuts(heroic, &r, &a)
                        } else {
                            sc::add_shortcuts(heroic, &r, &a)
                        }
                    });
                })),
        );

        let (r, a, k) = (runner, app, key);
        menu.child(
            item("cm-steam")
                .child(icon(Icon::SportsEsports, px(18.), crate::theme::text_muted()))
                .child(if in_steam { "Remove from Steam" } else { "Add to Steam" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.context_menu = None;
                    let (r, a, k) = (r.clone(), a.clone(), k.clone());
                    this.system_op(k, cx, move |heroic| {
                        if in_steam {
                            sc::remove_from_steam(heroic, &r, &a)
                        } else {
                            sc::add_to_steam(heroic, &r, &a)
                        }
                    });
                })),
        )
    }

    /// Cached `(shortcuts exist, added to Steam)`; starts a refresh when the
    /// entry is missing or older than a few seconds.
    fn probe_shortcuts(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> (bool, bool) {
        let key = game.key.clone();
        let current = cx
            .default_global::<MenuProbes>()
            .0
            .get(&key)
            .map(|p| (p.shortcuts, p.steam, p.pending, p.at.elapsed() > Duration::from_secs(5)));
        let (shortcuts, steam, pending, stale) = current.unwrap_or((false, false, false, true));
        if stale && !pending {
            cx.default_global::<MenuProbes>().0.insert(key.clone(), Probe { shortcuts, steam, at: Instant::now(), pending: true });
            let heroic = self.heroic.clone();
            let (runner, app) = (game.runner.clone(), game.app_name.clone());
            cx.spawn(async move |this, cx| {
                let probed = cx
                    .background_spawn(async move {
                        (sc::shortcuts_exist(&heroic, &runner, &app), sc::is_added_to_steam(&heroic, &runner, &app))
                    })
                    .await;
                this.update(cx, |_, cx| {
                    cx.default_global::<MenuProbes>().0.insert(
                        key,
                        Probe { shortcuts: probed.0, steam: probed.1, at: Instant::now(), pending: false },
                    );
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
    fn system_op(&mut self, key: String, cx: &mut Context<Self>, op: impl FnOnce(&heronite_data::HeroicPaths) -> sc::Outcome + Send + 'static) {
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { op(&heroic) }).await;
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
