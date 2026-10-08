//! Install dialog of an Epic game (port of Heroic's `InstallModal` ->
//! `DownloadDialog` / `ImportDialog`): install path with free space, sizes
//! from `legendary info`, DLC and selective-download (SDL) choices, Wine
//! settings on Linux, and "Import Game" for an existing install.

mod render;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::{AppPaths, HeroicPaths};
use heronite_epic::cmd::Legendary;
use heronite_epic::disk::{self, DiskSpace};
use heronite_epic::info::{self, Dlc, InstallInfo, Sdl};
use heronite_epic::util::{file_size, remove_special_characters};
use heronite_wine::WineInstallation;
use serde_json::Value;

use super::DialogEvent;
use crate::services::{self, downloads::InstallRequest};
use crate::ui::fields::{new_input, on_commit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Install,
    Import,
}

pub struct EpicInstallDialog {
    heroic: HeroicPaths,
    game: Value,
    app_name: String,
    title: String,
    mode: Mode,
    platform: String,
    /// `legendary info` result (None while loading).
    info: Option<InstallInfo>,
    info_error: Option<String>,
    path: Entity<InputState>,
    space: Option<DiskSpace>,
    dlcs: Vec<(Dlc, bool)>,
    sdl: Vec<(Sdl, bool)>,
    // Wine (Linux, Windows builds).
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
    busy: bool,
}

impl EventEmitter<DialogEvent> for EpicInstallDialog {}

pub(super) fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}

impl EpicInstallDialog {
    pub fn new(
        heroic: HeroicPaths,
        _app: AppPaths,
        game: Value,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_name = game.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let title = game.get("title").and_then(Value::as_str).unwrap_or(&app_name).to_string();
        let home = std::env::var("HOME").unwrap_or_default();
        let global = game_config::load_global(&heroic, &home);
        let default_install = global
            .get("defaultInstallPath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('\'', ""))
            .unwrap_or_else(|| format!("{home}/Games/Heroic"));
        let prefix_dir = global
            .get("defaultWinePrefixDir")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| game_config::default_wine_prefix_dir(&home));
        let suggested_prefix = format!("{prefix_dir}/{}", remove_special_characters(&title));
        let shared_prefix_path = global
            .get("winePrefix")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| game_config::shared_wine_prefix(&home));
        let default_wine = global.get("wineVersion").filter(|v| v.get("bin").is_some()).cloned();

        let path = new_input(window, cx, &default_install, &default_install);
        on_commit(&path, window, cx, |this: &mut Self, _, _, cx| this.check_space(cx));
        let prefix = new_input(window, cx, &suggested_prefix, "");
        let import_path = new_input(window, cx, "", "");
        let mut this = Self {
            heroic,
            game,
            app_name,
            title,
            mode,
            platform: "Windows".into(),
            info: None,
            info_error: None,
            path,
            space: None,
            dlcs: Vec::new(),
            sdl: Vec::new(),
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
            busy: false,
        };
        // A warm cache (6 hours) shows the sizes at once; a statvfs is cheap.
        let cached = heronite_epic::cache_store::CacheStore::install_info(&this.heroic)
            .get(&this.app_name)
            .filter(|c| c.get("manifest").is_some_and(|m| !m.is_null()));
        match cached {
            Some(c) => this.apply_info(InstallInfo(c)),
            None => this.load_info(cx),
        }
        this.load_sdl(cx);
        this.load_wine(cx);
        this.space = disk::check(&text_of(&this.path, cx));
        this
    }

    fn apply_info(&mut self, i: InstallInfo) {
        log::info!("install info of {}: download {} disk {}", self.app_name, i.download_size(), i.disk_size());
        // Like Heroic: every owned DLC starts selected.
        self.dlcs = i.owned_dlc().into_iter().map(|d| (d, true)).collect();
        self.info = Some(i);
        self.info_error = None;
    }

    fn linux_windows_game(&self) -> bool {
        cfg!(target_os = "linux") && self.platform == "Windows"
    }

    fn load_info(&mut self, cx: &mut Context<Self>) {
        let (heroic, app, platform) = (self.heroic.clone(), self.app_name.clone(), self.platform.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&heroic)?;
                    info::install_info(&heroic, &lg, &app, &platform)
                })
                .await;
            this.update(cx, |this, cx| {
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

    /// Selective downloads come from a web API: they must not hold the sizes back.
    fn load_sdl(&mut self, cx: &mut Context<Self>) {
        let (heroic, app) = (self.heroic.clone(), self.app_name.clone());
        cx.spawn(async move |this, cx| {
            let sdl = cx.background_spawn(async move { info::game_sdl(&heroic, &app) }).await;
            this.update(cx, |this, cx| {
                this.sdl = sdl.into_iter().map(|s| (s, false)).collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_wine(&mut self, cx: &mut Context<Self>) {
        if !cfg!(target_os = "linux") {
            return;
        }
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_wine::list_installations(&heroic) }).await;
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

    fn download_bytes(&self) -> f64 {
        let Some(i) = &self.info else { return 0. };
        let tags = self.selected_tags();
        let extra: f64 = tags.iter().map(|t| i.tag_sizes(t).0).sum();
        i.download_size() + if self.sdl.is_empty() { 0. } else { extra }
    }

    fn disk_bytes(&self) -> f64 {
        let Some(i) = &self.info else { return 0. };
        let extra: f64 = if self.sdl.is_empty() { 0. } else { self.selected_tags().iter().map(|t| i.tag_sizes(t).1).sum() };
        i.disk_size() + extra
    }

    /// Tags of the required and chosen selective downloads.
    fn selected_tags(&self) -> Vec<String> {
        self.sdl
            .iter()
            .filter(|(s, on)| s.required || *on)
            .flat_map(|(s, _)| s.tags.iter().cloned())
            .filter(|t| !t.is_empty())
            .collect()
    }

    fn wine_selection(&self, cx: &App) -> (Option<String>, Option<Value>) {
        if !self.linux_windows_game() {
            return (None, None);
        }
        if self.shared_prefix {
            return (Some(self.shared_prefix_path.clone()), self.default_wine.clone());
        }
        let version = self.wine_list.get(self.wine_pick).and_then(|w| serde_json::to_value(w).ok());
        let prefix = text_of(&self.prefix, cx);
        ((!prefix.is_empty()).then_some(prefix), version.or_else(|| self.default_wine.clone()))
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        let path = text_of(&self.path, cx);
        if path.is_empty() || self.info.is_none() {
            return;
        }
        let (wine_prefix, wine_version) = self.wine_selection(cx);
        let req = InstallRequest {
            game_info: self.game.clone(),
            path,
            platform: self.platform.clone(),
            sdl_tags: self.selected_tags(),
            dlcs: self.dlcs.iter().filter(|(_, on)| *on).map(|(d, _)| d.app_name.clone()).collect(),
            language: String::new(),
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
        let (app, platform) = (self.app_name.clone(), self.platform.clone());
        services::downloads(cx).update(cx, |s, cx| s.import_game(&app, &folder, &platform, wine_prefix, wine_version, cx));
        cx.emit(DialogEvent::Close);
    }
}
