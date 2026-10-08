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
        if std::env::var_os("HERONITE_DEBUG_FAKE_LAUNCH").is_some_and(|v| !v.is_empty() && v != "0") {
            log::info!("[fake-launch] window {} ({}/{}) installed={}", game.title, game.runner, game.app_name, game.installed);
            return;
        }
        let key = game.key.clone();
        self.primary_action(&key, cx);
    }

    /// A toast for a message from the desktop integration.
    pub fn system_toast(&mut self, text: String, error: bool, cx: &mut Context<Self>) {
        self.show_toast(text, error, cx);
    }
}
