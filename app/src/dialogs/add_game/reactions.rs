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
use crate::dialogs::settings_panel::logic::wine_of;

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

    /// The prefix named after the title; none while there is no title.
    fn suggestion(&self, cx: &App) -> String {
        let title = text_of(&self.title, cx);
        if title.trim().is_empty() {
            return String::new();
        }
        let dir = self.global.get("defaultWinePrefixDir").and_then(Value::as_str).unwrap_or("");
        suggested_prefix(dir, &title, &self.app_name)
    }

    /// The prefix the game gets: the one in the field, else the suggestion.
    pub(super) fn prefix_path(&self, cx: &App) -> String {
        let typed = text_of(&self.prefix, cx).trim().to_string();
        if typed.is_empty() {
            self.suggestion(cx)
        } else {
            typed
        }
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

    /// An executable named before the title names the game (its file, or its
    /// folder when the file's name is generic). The title follows the
    /// executable until the user types their own.
    pub(super) fn fill_title_from(&mut self, executable: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_edit() || executable.trim().is_empty() {
            return;
        }
        let current = text_of(&self.title, cx);
        if !current.trim().is_empty() && current != self.last_auto_title {
            return;
        }
        let title = super::dropped::title_from_executable(std::path::Path::new(executable.trim()));
        if title.is_empty() || title == current {
            return;
        }
        self.last_auto_title = title.clone();
        // `set_value` sends no change event: the prefix follows by hand.
        self.title.update(cx, |s, cx| s.set_value(title, window, cx));
        self.title_changed(window, cx);
    }

    /// An executable chosen in the file dialog: the platform, the title, the
    /// Linux game folder and the art follow from it.
    pub(super) fn executable_picked(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.exe.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
        self.executable_changed(&path);
        self.fill_folder_from(&path, window, cx);
        self.fill_title_from(&path, window, cx);
        self.auto_find_art(window, cx);
        cx.notify();
    }

    /// A picked executable gives the game folder when none was chosen yet.
    pub(super) fn fill_folder_from(&mut self, executable: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.platform != SideloadPlatform::Linux || !text_of(&self.folder, cx).trim().is_empty() {
            return;
        }
        if let Some(folder) = install_folder(executable) {
            self.folder.update(cx, |s, cx| s.set_value(folder, window, cx));
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
            let prefix =
                self.global.get("winePrefix").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string).unwrap_or(shared);
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
        let paths = self.ctx.data_paths.clone();
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
        // A default that is not installed anymore (its folder is gone) is
        // replaced too: one with the same name, else the first.
        let missing = self.wine.as_ref().is_some_and(|w| !list.iter().any(|l| l.bin == w.bin));
        if self.wine.is_none() || missing {
            let same_name = self
                .wine
                .as_ref()
                .and_then(|w| list.iter().find(|l| l.name.replace(' ', "-").eq_ignore_ascii_case(&w.name.replace(' ', "-"))));
            self.wine = same_name.or(list.first()).cloned();
        }
        self.wine_list = Some(list);
    }
}
