//! Install / import dialog of a GOG or Amazon Games game: install folder with
//! free space, platform and language (GOG), DLCs, sizes from gogdl / nile, and
//! the Wine settings on Linux. "Import game" adds a copy that is already on disk.

mod console;
mod render;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_amazon::cmd::Nile;
use heronite_data::game_config;
use heronite_data::DataPaths;
use heronite_epic::disk::{self, DiskSpace};
use heronite_epic::util::{file_size, remove_special_characters};
use heronite_gog::cmd::Gogdl;
use heronite_gog::info::{Dlc, InstallInfo as GogInfo};
use heronite_i18n::Lang;
use heronite_wine::WineInstallation;
use serde_json::Value;

use super::{text_of, DialogEvent};
use crate::services;
use crate::services::downloads::{InstallRequest, Store};
use crate::ui::fields::{new_input, on_commit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreInstallMode {
    Install,
    Import,
}

/// What the store's tool answered for the chosen platform.
enum Loaded {
    Gog(GogInfo),
    /// `(download, disk)` bytes.
    Amazon(heronite_amazon::info::InstallInfo),
}

pub struct StoreInstallDialog {
    data_paths: DataPaths,
    store: Store,
    game: Value,
    app_name: String,
    title: String,
    mode: StoreInstallMode,
    /// GOG: `windows` | `linux`; Amazon: `Windows`.
    platform: String,
    platforms: Vec<String>,
    platform_open: bool,
    /// `None` while loading or after an error.
    info: Option<Loaded>,
    info_error: Option<String>,
    /// Bumped by every load: an answer for an older platform is dropped.
    load_token: u64,
    /// GOG language code (empty when the game lists none).
    language: String,
    language_open: bool,
    dlcs: Vec<(Dlc, bool)>,
    path: Entity<InputState>,
    space: Option<DiskSpace>,
    wine_list: Vec<WineInstallation>,
    wine_loaded: bool,
    wine_pick: usize,
    wine_open: bool,
    wine_details: bool,
    shared_prefix: bool,
    prefix: Entity<InputState>,
    /// `defaultWinePrefixDir/<title>` and the shared prefix of the settings.
    suggested_prefix: String,
    shared_prefix_path: String,
    default_wine: Option<Value>,
    import_path: Entity<InputState>,
    /// Console mode: focused row and the folder picker.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<DialogEvent> for StoreInstallDialog {}

/// Language codes of the UI language, as the stores name them.
fn ui_language() -> &'static str {
    match heronite_i18n::current() {
        Lang::En => "en-US",
        Lang::PtBr => "pt-BR",
    }
}

/// Readable name of a store language code; unknown codes are shown as they are.
fn language_name(code: &str) -> String {
    let name = match code {
        "en-US" => "English",
        "en-GB" => "English (UK)",
        "pt-BR" => "Português (Brasil)",
        "pt-PT" => "Português (Portugal)",
        "es-ES" => "Español",
        "es-MX" => "Español (México)",
        "fr-FR" => "Français",
        "de-DE" => "Deutsch",
        "it-IT" => "Italiano",
        "ru-RU" => "Русский",
        "pl-PL" => "Polski",
        "uk-UA" => "Українська",
        "zh-Hans" | "zh-CN" => "中文 (简体)",
        "zh-Hant" | "zh-TW" => "中文 (繁體)",
        "ja-JP" => "日本語",
        "ko-KR" => "한국어",
        "tr-TR" => "Türkçe",
        "cs-CZ" => "Čeština",
        "hu-HU" => "Magyar",
        "nl-NL" => "Nederlands",
        "sv-SE" => "Svenska",
        "da-DK" => "Dansk",
        "fi-FI" => "Suomi",
        "nb-NO" => "Norsk",
        "ro-RO" => "Română",
        "bg-BG" => "Български",
        "el-GR" => "Ελληνικά",
        _ => return code.to_string(),
    };
    name.to_string()
}

/// "Windows" / "Linux" for a platform value.
fn platform_name(platform: &str) -> &'static str {
    if platform.eq_ignore_ascii_case("linux") {
        "Linux"
    } else {
        "Windows"
    }
}

impl StoreInstallDialog {
    pub fn new(
        data_paths: DataPaths,
        store: Store,
        game: Value,
        mode: StoreInstallMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_name = game.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let title = game.get("title").and_then(Value::as_str).unwrap_or(&app_name).to_string();
        let home = std::env::var("HOME").unwrap_or_default();
        let global = game_config::load_global(&data_paths, &home);
        let default_install = global
            .get("defaultInstallPath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('\'', ""))
            .unwrap_or_else(|| game_config::default_install_dir(&home));
        let prefix_dir = global
            .get("defaultWinePrefixDir")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| game_config::default_wine_prefix_dir(&home));
        let suggested_prefix = format!("{prefix_dir}/{}", remove_special_characters(&title));
        let shared_prefix_path =
            global.get("winePrefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| game_config::shared_wine_prefix(&home));
        let default_wine = global.get("wineVersion").filter(|v| v.get("bin").is_some()).cloned();

        let path = new_input(window, cx, &default_install, &default_install);
        on_commit(&path, window, cx, |this: &mut Self, _, _, cx| this.check_space(cx));
        let prefix = new_input(window, cx, &suggested_prefix, "");
        let import_path = new_input(window, cx, "", "");
        let platform = if store == Store::Gog { "windows" } else { "Windows" };
        let mut this = Self {
            data_paths,
            store,
            game,
            app_name,
            title,
            mode,
            platform: platform.into(),
            platforms: vec![platform.into()],
            platform_open: false,
            info: None,
            info_error: None,
            load_token: 0,
            language: String::new(),
            language_open: false,
            dlcs: Vec::new(),
            path,
            space: None,
            wine_list: Vec::new(),
            wine_loaded: false,
            wine_pick: 0,
            wine_open: false,
            wine_details: true,
            shared_prefix: false,
            prefix,
            suggested_prefix,
            shared_prefix_path,
            default_wine,
            import_path,
            console_list: Default::default(),
            layer: Default::default(),
        };
        if store == Store::Gog {
            this.load_platforms(cx);
        }
        if mode == StoreInstallMode::Install {
            this.load_info(cx);
        }
        this.load_wine(cx);
        this.space = disk::check(&text_of(&this.path, cx));
        this
    }

    fn importing(&self) -> bool {
        self.mode == StoreInstallMode::Import
    }

    /// Wine settings apply to Windows builds on Linux; an imported copy's
    /// platform is unknown, so they are offered too.
    fn wine_applies(&self) -> bool {
        cfg!(target_os = "linux") && (self.importing() || self.platform.eq_ignore_ascii_case("windows"))
    }

    fn load_platforms(&mut self, cx: &mut Context<Self>) {
        let (data_paths, app) = (self.data_paths.clone(), self.app_name.clone());
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_gog::info::platforms(&data_paths, &app) }).await;
            this.update(cx, |this, cx| {
                if !list.is_empty() {
                    this.platforms = list;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks the store's tool for the sizes (and GOG's languages and DLCs) of
    /// the chosen platform. The first use may download the tool.
    fn load_info(&mut self, cx: &mut Context<Self>) {
        self.info = None;
        self.info_error = None;
        self.dlcs.clear();
        self.load_token += 1;
        let token = self.load_token;
        let (data_paths, app, store, platform) = (self.data_paths.clone(), self.app_name.clone(), self.store, self.platform.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match store {
                        Store::Gog => {
                            let gogdl = Gogdl::locate_or_install(&data_paths)?;
                            heronite_gog::info::install_info(&data_paths, &gogdl, &app, &platform).map(Loaded::Gog)
                        }
                        Store::Amazon => {
                            let nile = Nile::locate_or_install(&data_paths)?;
                            heronite_amazon::info::install_info(&data_paths, &nile, &app).map(Loaded::Amazon)
                        }
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.load_token != token {
                    return;
                }
                match res {
                    Ok(i) => this.apply_info(i),
                    Err(err) => this.info_error = Some(format!("{err:#}")),
                }
                this.check_space(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_info(&mut self, loaded: Loaded) {
        if let Loaded::Gog(i) = &loaded {
            log::info!("install info of {}: {} languages, {} DLCs", self.app_name, i.languages.len(), i.dlcs.len());
            self.dlcs = i.dlcs.iter().cloned().map(|d| (d, true)).collect();
            if !i.languages.contains(&self.language) {
                self.language = if i.languages.is_empty() { String::new() } else { i.default_language(ui_language()) };
            }
        }
        self.info = Some(loaded);
        self.info_error = None;
    }

    fn load_wine(&mut self, cx: &mut Context<Self>) {
        if !cfg!(target_os = "linux") {
            return;
        }
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_wine::list_installations(&data_paths) }).await;
            this.update(cx, |this, cx| {
                let default_name = this.default_wine.as_ref().and_then(|w| w.get("name")).and_then(Value::as_str).map(str::to_string);
                this.wine_pick = default_name.and_then(|n| list.iter().position(|w| w.name == n)).unwrap_or(0);
                this.wine_list = list;
                this.wine_loaded = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn check_space(&mut self, cx: &mut Context<Self>) {
        let path = text_of(&self.path, cx);
        cx.spawn(async move |this, cx| {
            let space = cx.background_spawn(async move { disk::check(&path) }).await;
            this.update(cx, |this, cx| {
                this.space = space;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Picks a platform (GOG) and reloads what depends on it.
    fn set_platform(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(platform) = self.platforms.get(index).cloned() else { return };
        if platform == self.platform {
            return;
        }
        self.platform = platform;
        self.language.clear();
        self.load_info(cx);
    }

    fn selected_dlcs(&self) -> Vec<String> {
        self.dlcs.iter().filter(|(_, on)| *on).map(|(d, _)| d.id.clone()).collect()
    }

    /// `(download, disk)` bytes of the current choices; `None` until the info is in.
    fn sizes(&self) -> Option<(f64, f64)> {
        Some(match self.info.as_ref()? {
            Loaded::Gog(i) => {
                let (download, disk) = i.size_for(&self.language, &self.selected_dlcs());
                (download as f64, disk as f64)
            }
            Loaded::Amazon(i) => (i.download_size as f64, i.disk_size as f64),
        })
    }

    fn download_bytes(&self) -> f64 {
        self.sizes().map_or(0., |s| s.0)
    }

    fn disk_bytes(&self) -> f64 {
        self.sizes().map_or(0., |s| s.1)
    }

    fn not_enough_space(&self) -> bool {
        self.space.as_ref().is_some_and(|s| self.info.is_some() && (s.free as f64) < self.disk_bytes())
    }

    fn languages(&self) -> &[String] {
        match &self.info {
            Some(Loaded::Gog(i)) => &i.languages,
            _ => &[],
        }
    }

    fn wine_selection(&self, cx: &App) -> (Option<String>, Option<Value>) {
        if !self.wine_applies() {
            return (None, None);
        }
        if self.shared_prefix {
            return (Some(self.shared_prefix_path.clone()), self.default_wine.clone());
        }
        let version = self.wine_list.get(self.wine_pick).and_then(|w| serde_json::to_value(w).ok());
        let prefix = text_of(&self.prefix, cx);
        ((!prefix.is_empty()).then_some(prefix), version.or_else(|| self.default_wine.clone()))
    }

    fn can_install(&self, cx: &App) -> bool {
        self.info.is_some() && !text_of(&self.path, cx).is_empty()
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if !self.can_install(cx) {
            return;
        }
        let mut game_info = self.game.clone();
        if let Some(Loaded::Gog(i)) = &self.info {
            if !i.folder_name.is_empty() {
                game_info["folder_name"] = Value::String(i.folder_name.clone());
            }
        }
        let (wine_prefix, wine_version) = self.wine_selection(cx);
        let req = InstallRequest {
            game_info,
            path: text_of(&self.path, cx),
            platform: self.platform.clone(),
            sdl_tags: Vec::new(),
            dlcs: self.selected_dlcs(),
            language: self.language.clone(),
            size: Some(file_size(self.download_bytes())),
            wine_prefix,
            wine_version,
        };
        services::downloads(cx).update(cx, |s, cx| s.queue_install(req, cx));
        cx.emit(DialogEvent::Close);
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let folder = text_of(&self.import_path, cx);
        if folder.is_empty() {
            return;
        }
        let (wine_prefix, wine_version) = self.wine_selection(cx);
        let (store, app) = (self.store, (self.store == Store::Amazon).then(|| self.app_name.clone()));
        services::downloads(cx).update(cx, |s, cx| s.store_import(store, app, &folder, wine_prefix, wine_version, cx));
        cx.emit(DialogEvent::Close);
    }

    /// Switches between installing and importing; the first install view loads its info.
    fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        self.mode = if self.importing() { StoreInstallMode::Install } else { StoreInstallMode::Import };
        if self.mode == StoreInstallMode::Install && self.info.is_none() && self.info_error.is_none() && self.load_token == 0 {
            self.load_info(cx);
        }
        cx.notify();
    }
}
