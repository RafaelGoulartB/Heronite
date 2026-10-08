//! Reactions to field changes and the Wine list loading.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::sideload::SideloadPlatform;
use heronite_launch::SystemProbe;
use heronite_wine::WineInstallation;
use serde_json::Value;

use super::logic::*;
use super::{text_of, AddGameDialog};
use crate::dialogs::heroic_settings::logic::wine_of;

impl AddGameDialog {
    pub(super) fn title_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shared_prefix {
            return;
        }
        let current = text_of(&self.prefix, cx);
        // The suggestion follows the title until the user types their own.
        if current.is_empty() || current == self.last_suggested {
            let next = self.suggestion(cx);
            self.last_suggested = next.clone();
            self.prefix.update(cx, |s, cx| s.set_value(next, window, cx));
        }
    }

    fn suggestion(&self, cx: &App) -> String {
        let dir = self.global.get("defaultWinePrefixDir").and_then(Value::as_str).unwrap_or("");
        suggested_prefix(dir, &text_of(&self.title, cx), &self.app_name)
    }

    /// The executable decides the platform until the user chooses one.
    pub(super) fn executable_changed(&mut self, path: &str) {
        if self.platform_manual || self.platform == SideloadPlatform::Browser {
            return;
        }
        if let Some(p) = detect_platform(path, &SystemProbe) {
            self.platform = p;
        }
    }

    pub(super) fn set_platform(&mut self, platform: SideloadPlatform, cx: &mut Context<Self>) {
        self.platform = platform;
        self.platform_manual = true;
        self.open_select = None;
        self.error.clear();
        if platform == SideloadPlatform::Windows && self.wine_list.is_none() && !self.is_edit() {
            self.load_wine_list(cx);
        }
        cx.notify();
    }

    pub(super) fn set_shared_prefix(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.shared_prefix = on;
        if on {
            let shared = game_config::shared_wine_prefix(&self.home);
            let prefix = self.global.get("winePrefix").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string).unwrap_or(shared);
            self.prefix.update(cx, |s, cx| s.set_value(prefix, window, cx));
            if self.global.get("wineVersion").is_some() {
                self.wine = Some(wine_of(&self.global));
            }
            self.bottle = self.global.get("wineCrossoverBottle").and_then(Value::as_str).unwrap_or("").to_string();
        } else {
            let next = self.suggestion(cx);
            self.last_suggested = next.clone();
            self.prefix.update(cx, |s, cx| s.set_value(next, window, cx));
        }
        cx.notify();
    }

    pub(super) fn load_wine_list(&mut self, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        // Debug captures only keep the first frames: load in place there.
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            let list = heronite_wine::list_installations(&paths);
            self.wine_loaded(list);
            return;
        }
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_wine::list_installations(&paths) }).await;
            this.update(cx, |this, cx| {
                this.wine_loaded(list);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn wine_loaded(&mut self, list: Vec<WineInstallation>) {
        // `WineSelector`: the first available version unless one is chosen.
        if self.wine.is_none() {
            self.wine = list.first().cloned();
        }
        self.wine_list = Some(list);
    }
}
