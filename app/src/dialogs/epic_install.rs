//! Install dialog of an Epic game (port of Heroic's `InstallModal` ->
//! `DownloadDialog` / `ImportDialog`): install path with free space, sizes
//! from `legendary info`, DLC and selective-download (SDL) choices, Wine
//! settings on Linux, and "Import Game" for an existing install.

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
use crate::icons::Icon;
use crate::services::{self, downloads::InstallRequest};
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, meta_text, new_input, on_commit, path_field, select_field};
use crate::ui::icon;

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

fn text_of(input: &Entity<InputState>, cx: &App) -> String {
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

    fn size_row(&self) -> Div {
        let have = self.info.is_some();
        let cell = |ic: Icon, label: &str, value: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(ic, px(16.), crate::theme::text_primary()))
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(format!("{label}:")))
                .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value))
        };
        let mut row = div().flex().flex_wrap().gap_x(px(28.)).gap_y(px(6.));
        if have {
            row = row
                .child(cell(Icon::Download, "Download Size", file_size(self.download_bytes())))
                .child(cell(Icon::FaHardDrive, "Install Size", file_size(self.disk_bytes())));
        } else if let Some(err) = &self.info_error {
            row = row.child(div().w_full().text_size(px(14.)).text_color(theme::danger_text()).child(err.lines().next().unwrap_or("").to_string()));
        } else {
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(Icon::FaSpinner, px(16.), crate::theme::text_primary()))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child("Getting download and install size...")),
            );
        }
        row
    }

    fn space_line(&self, cx: &App) -> Option<Div> {
        let space = self.space.as_ref()?;
        let after = space.free as f64 - self.disk_bytes();
        let enough = self.info.is_none() || after >= 0.;
        let mut line = div().text_size(px(13.)).text_color(theme::text_muted()).flex().flex_wrap().gap(px(4.)).child(format!("Space Available: {}", space.message));
        if self.info.is_some() && enough {
            line = line.child(format!(" - After Install: {}", file_size(after)));
        }
        if !space.writable {
            line = line.child(div().text_color(rgb(0xebcb8b)).child("Warning: path might not be writable."));
        }
        if !enough {
            line = line.child(div().text_color(rgb(0xebcb8b)).child("(Not enough disk space)"));
        }
        let _ = cx;
        Some(line)
    }

    fn wine_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.linux_windows_game() {
            return None;
        }
        let toggle = div()
            .id("epic-wine-toggle")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .py(px(8.))
            .text_size(px(14.))
            .text_color(crate::theme::text_primary())
            .on_click(cx.listener(|this, _, _, cx| {
                this.wine_details = !this.wine_details;
                cx.notify();
            }))
            .child(icon(if self.wine_details { Icon::ExpandMore } else { Icon::ChevronRight }, px(18.), crate::theme::text_primary()))
            .child("Show Wine settings");
        let mut col = crate::ui::controls::card_box().gap(px(10.)).pb(px(14.)).child(toggle);
        if !self.wine_details {
            return Some(col);
        }
        let shared = self.shared_prefix;
        col = col.child(
            checkbox("epic-shared-prefix", shared, "Use shared Wine prefix").on_click(cx.listener(|this, _, window, cx| {
                this.shared_prefix = !this.shared_prefix;
                let value = if this.shared_prefix { this.shared_prefix_path.clone() } else { this.suggested_prefix.clone() };
                this.prefix.update(cx, |s, cx| s.set_value(value, window, cx));
                cx.notify();
            })),
        );
        if shared {
            col = col.child(meta_text("Only use this option if you know what you are doing. Sharing the same prefix for multiple games can create problems if their dependencies are incompatible."));
        }
        let has_prefix = !text_of(&self.prefix, cx).is_empty();
        col = col.child(path_field(
            "epic-prefix-pick",
            Some("WinePrefix"),
            &self.prefix,
            has_prefix,
            shared,
            cx.listener(move |this, _, window, cx| {
                choose_path(window, cx, true, "Select WinePrefix Folder", |this: &mut Self, p, window, cx| {
                    this.prefix.update(cx, |s, cx| s.set_value(p, window, cx));
                    cx.notify();
                });
                let _ = this;
            }),
        ));
        let current = if shared {
            self.default_wine.as_ref().and_then(|w| w.get("name")).and_then(Value::as_str).unwrap_or("").to_string()
        } else {
            self.wine_list.get(self.wine_pick).map(|w| w.name.clone()).unwrap_or_else(|| {
                if self.wine_loaded { "No Wine or Proton found".into() } else { "Looking for Wine and Proton...".into() }
            })
        };
        let options: Vec<(SharedString, bool)> =
            self.wine_list.iter().enumerate().map(|(i, w)| (w.name.clone().into(), i == self.wine_pick)).collect();
        col = col.child(select_field(
            "epic-wine",
            Some("Wine Version:"),
            current,
            shared || self.wine_list.is_empty(),
            self.wine_open,
            options,
            cx,
            |this: &mut Self, _, cx| {
                this.wine_open = !this.wine_open;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.wine_pick = i;
                this.wine_open = false;
                cx.notify();
            },
        ));
        Some(col)
    }
}

impl Render for EpicInstallDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let importing = self.mode == Mode::Import;
        let mut body = div().flex().flex_col().gap(px(14.));

        if importing {
            body = body.child(meta_text("Choose the folder where the game is already installed (the folder that holds the game files)."));
            let has = !text_of(&self.import_path, cx).is_empty();
            body = body.child(path_field(
                "epic-import-pick",
                Some("Select Game Folder"),
                &self.import_path,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.import_path.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    } else {
                        choose_path(window, cx, true, "Choose Game Folder to import", |this: &mut Self, p, window, cx| {
                            this.import_path.update(cx, |s, cx| s.set_value(p, window, cx));
                            cx.notify();
                        });
                    }
                }),
            ));
        } else {
            body = body.child(self.size_row());
            let has = !text_of(&self.path, cx).is_empty();
            body = body.child(path_field(
                "epic-path-pick",
                Some("Select Install Path"),
                &self.path,
                has,
                false,
                cx.listener(move |_, _, window, cx| {
                    choose_path(window, cx, true, "Select Install Path", |this: &mut Self, p, window, cx| {
                        this.path.update(cx, |s, cx| s.set_value(p, window, cx));
                        this.check_space(cx);
                        cx.notify();
                    });
                }),
            ));
            body = body.children(self.space_line(cx));

            if !self.sdl.is_empty() || !self.dlcs.is_empty() {
                body = body.child(crate::ui::controls::section_title("Components to install"));
            }
            if !self.sdl.is_empty() {
                let mut rows: Vec<AnyElement> = Vec::new();
                for (i, (sdl, on)) in self.sdl.iter().enumerate() {
                    let checked = sdl.required || *on;
                    let row = checkbox(SharedString::from(format!("epic-sdl-{i}")), checked, sdl.name.clone());
                    rows.push(
                        if sdl.required {
                            row.opacity(0.7).cursor_not_allowed()
                        } else {
                            row.on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(e) = this.sdl.get_mut(i) {
                                    e.1 = !e.1;
                                }
                                cx.notify();
                            }))
                        }
                        .into_any_element(),
                    );
                }
                body = body.child(crate::ui::controls::card(rows));
            }
            if !self.dlcs.is_empty() {
                let all = self.dlcs.iter().all(|(_, on)| *on);
                let mut rows: Vec<AnyElement> = vec![checkbox("epic-dlc-all", all, "Install all DLCs")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let set = !this.dlcs.iter().all(|(_, on)| *on);
                        for d in this.dlcs.iter_mut() {
                            d.1 = set;
                        }
                        cx.notify();
                    }))
                    .into_any_element()];
                for (i, (dlc, on)) in self.dlcs.iter().enumerate() {
                    rows.push(
                        checkbox(SharedString::from(format!("epic-dlc-{i}")), *on, dlc.title.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(e) = this.dlcs.get_mut(i) {
                                    e.1 = !e.1;
                                }
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                }
                body = body.child(div().id("epic-dlc-list").max_h(px(240.)).overflow_y_scroll().child(crate::ui::controls::card(rows)));
            }
        }
        if let Some(wine) = self.wine_section(cx) {
            body = body.child(wine);
        }

        let not_enough = self.space.as_ref().is_some_and(|s| self.info.is_some() && (s.free as f64) < self.disk_bytes());
        let ready = self.info.is_some() && !text_of(&self.path, cx).is_empty() && !self.busy;
        let primary = if importing {
            action_button("epic-do-import", ButtonKind::Primary, "Import", !text_of(&self.import_path, cx).is_empty(), cx.listener(|this, _, _, cx| this.import(cx)))
        } else {
            action_button(
                "epic-do-install",
                ButtonKind::Primary,
                if not_enough { "Force Install" } else { "Install" },
                ready,
                cx.listener(|this, _, _, cx| this.install(cx)),
            )
        };
        let switch = button("epic-switch-mode", ButtonKind::Glass, if importing { "Install" } else { "Import Game" }).on_click(cx.listener(|this, _, _, cx| {
            this.mode = if this.mode == Mode::Install { Mode::Import } else { Mode::Install };
            cx.notify();
        }));
        let footer = div().flex().gap(px(10.)).child(switch).child(primary).into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "epic-install".into(), title: self.title.clone().into(), width: 600., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
