//! "Add game" / "Edit game": Heroic's sideload dialog (`SideloadDialog` +
//! the platform and Wine selectors of `InstallModal`) for the Collection. Adds
//! a native Linux program, a Windows program (Wine/Proton) or a browser game
//! to `sideload_apps/library.json` the way Heroic does, and edits or removes
//! such games later. The file formats live in `heronite_data::sideload`; the
//! pure rules of the form in [`logic`].

mod logic;
pub mod picker;

use std::path::Path;

use futures::channel::oneshot;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::game_config::{self, Settings};
use heronite_data::models::GameInfo;
use heronite_data::sideload::{self, PrefixOutcome, SideloadPlatform, SideloadSpec};
use heronite_images::ImageRef;
use heronite_launch::settings::GameSettings;
use heronite_launch::wine_tools::{build_tool_command, WineTool};
use heronite_launch::{PlanContext, SystemProbe};
use heronite_wine::WineInstallation;
use serde_json::{json, Value};

use super::art_images::{chain_for_url, picture, DialogImages};
use super::heroic_settings::logic::{default_wine, display_wine_name, wine_of, wine_options, wine_to_value};
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{choose_path, field_label, input_box, labeled, meta_text, new_input, panel, path_field, select_field};
use crate::ui::icon;
use logic::*;
use picker::{ImagePicker, PickerEvent, Slot};

#[derive(Clone, Debug)]
pub enum Mode {
    Add,
    Edit(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Select {
    Platform,
    Wine,
}

/// Cover/background preview box sizes (600x900 and 1600x650 ratios).
const SQUARE_W: f32 = 168.;
const SQUARE_H: f32 = 252.;
const HERO_W: f32 = 168.;
const HERO_H: f32 = 68.;

pub struct AddGameDialog {
    ctx: DialogCtx,
    mode: Mode,
    app_name: String,
    home: String,
    global: Settings,
    title: Entity<InputState>,
    square: Entity<InputState>,
    hero: Entity<InputState>,
    exe: Entity<InputState>,
    url: Entity<InputState>,
    user_agent: Entity<InputState>,
    prefix: Entity<InputState>,
    bottle: String,
    fullscreen: bool,
    platform: SideloadPlatform,
    /// The user picked the platform; executable detection stops changing it.
    platform_manual: bool,
    open_select: Option<Select>,
    shared_prefix: bool,
    show_wine: bool,
    /// Last prefix this dialog suggested (typing over it stops suggestions).
    last_suggested: String,
    wine_list: Option<Vec<WineInstallation>>,
    wine: Option<WineInstallation>,
    images: Entity<DialogImages>,
    picker: Option<(Entity<ImagePicker>, Subscription)>,
    /// `runAs` the game had when it was opened for editing.
    existing_run_as: Option<String>,
    /// Looking for art on the network.
    finding: bool,
    auto_art_tried: Option<String>,
    saving: bool,
    installer_running: bool,
    /// Edit mode: asking to remove the game.
    confirm_remove: bool,
    remove_prefix_too: bool,
    /// Wine prefix of the edited game when it may be deleted with it.
    removable_prefix: Option<String>,
    error: String,
    notice: String,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for AddGameDialog {}

fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn extra_str(g: &GameInfo, key: &str) -> String {
    g.rest.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl AddGameDialog {
    pub fn new(ctx: DialogCtx, mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let global = game_config::load_global(&ctx.heroic, &home);
        let edited: Option<Game> = match &mode {
            Mode::Edit(app) => ctx.lib.games.iter().find(|g| &g.app_name == app && g.runner == "sideload").cloned(),
            Mode::Add => None,
        };
        let app_name = match (&mode, &edited) {
            (Mode::Edit(app), _) => app.clone(),
            _ => sideload::generate_app_name(),
        };

        // Initial values: empty for a new game, the stored ones when editing.
        let (mut title, mut square, mut hero, mut exe, mut url, mut ua, mut fullscreen) =
            (String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), false);
        let mut platform = SideloadPlatform::Windows;
        let mut existing_run_as = None;
        if let Some(game) = &edited {
            let g = &game.game;
            title = g.title.clone();
            square = g.art_square.clone().unwrap_or_default();
            let cover = g.art_cover.clone().unwrap_or_default();
            hero = if cover != square { cover } else { String::new() };
            exe = g.install.executable.clone().unwrap_or_default();
            url = extra_str(g, "browserUrl");
            ua = extra_str(g, "customUserAgent");
            fullscreen = g.rest.get("launchFullScreen").and_then(Value::as_bool).unwrap_or(false);
            existing_run_as = game.meta.as_ref().and_then(|m| m.run_as.clone());
            platform = if !url.is_empty() || g.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser")) {
                SideloadPlatform::Browser
            } else {
                match heronite_launch::native::effective_platform(g, game.meta.as_deref(), &exe, &SystemProbe) {
                    heronite_launch::native::Platform::Linux => SideloadPlatform::Linux,
                    heronite_launch::native::Platform::Windows => SideloadPlatform::Windows,
                }
            };
        }
        let removable_prefix = edited.as_ref().and_then(|_| {
            if platform != SideloadPlatform::Windows {
                return None;
            }
            let prefix = game_config::load_game(&ctx.heroic, &app_name, &home).get("winePrefix")?.as_str()?.to_string();
            (!prefix.is_empty() && Path::new(&prefix).exists() && prefix != game_config::shared_wine_prefix(&home)).then_some(prefix)
        });

        let prefix_dir = global.get("defaultWinePrefixDir").and_then(Value::as_str).unwrap_or("").to_string();
        let suggested = suggested_prefix(&prefix_dir, &title, &app_name);

        let mut subs = Vec::new();
        let title_in = new_input(window, cx, &title, "Add a title to your Game/App");
        let square_in = new_input(window, cx, &square, "Paste an URL of an Image or select one from your computer");
        let hero_in = new_input(window, cx, &hero, "Paste an URL of an Image or select one from your computer");
        let exe_in = new_input(window, cx, &exe, "Select Executable");
        let url_in = new_input(window, cx, &url, "Paste the Game URL here");
        let ua_in = new_input(window, cx, &ua, "Write a custom user agent here to be used on this browser app/game");
        let prefix_in = new_input(window, cx, &suggested, "");

        subs.push(cx.subscribe_in(&title_in, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                let raw = input.read(cx).value().to_string();
                let clean = clean_title(&raw);
                if clean != raw {
                    input.update(cx, |s, cx| s.set_value(clean, window, cx));
                    return;
                }
                this.title_changed(window, cx);
                cx.notify();
            }
            // Heroic searches an image when the title field loses focus.
            InputEvent::Blur => this.auto_find_art(window, cx),
            _ => {}
        }));
        for input in [&square_in, &hero_in, &url_in, &ua_in, &prefix_in] {
            subs.push(cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        subs.push(cx.subscribe_in(&exe_in, window, |this, input, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                let path = input.read(cx).value().to_string();
                this.executable_changed(&path);
                cx.notify();
            }
        }));

        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        let wine_global = {
            let w = wine_of(&global);
            (global.get("wineVersion").is_some()).then_some(w)
        };
        let mut this = Self {
            ctx,
            mode,
            app_name,
            home,
            global,
            title: title_in,
            square: square_in,
            hero: hero_in,
            exe: exe_in,
            url: url_in,
            user_agent: ua_in,
            prefix: prefix_in,
            bottle: String::new(),
            fullscreen,
            platform,
            platform_manual: edited.is_some(),
            open_select: None,
            shared_prefix: false,
            show_wine: true,
            last_suggested: suggested,
            wine_list: None,
            wine: wine_global,
            images,
            picker: None,
            existing_run_as,
            finding: false,
            auto_art_tried: None,
            saving: false,
            installer_running: false,
            confirm_remove: false,
            remove_prefix_too: false,
            removable_prefix,
            error: String::new(),
            notice: String::new(),
            _subs: subs,
        };
        if matches!(this.mode, Mode::Add) {
            this.load_wine_list(cx);
        }
        this
    }

    fn is_edit(&self) -> bool {
        matches!(self.mode, Mode::Edit(_))
    }

    // -- field reactions ---------------------------------------------------------

    fn title_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn executable_changed(&mut self, path: &str) {
        if self.platform_manual || self.platform == SideloadPlatform::Browser {
            return;
        }
        if let Some(p) = detect_platform(path, &SystemProbe) {
            self.platform = p;
        }
    }

    fn set_platform(&mut self, platform: SideloadPlatform, cx: &mut Context<Self>) {
        self.platform = platform;
        self.platform_manual = true;
        self.open_select = None;
        self.error.clear();
        if platform == SideloadPlatform::Windows && self.wine_list.is_none() && !self.is_edit() {
            self.load_wine_list(cx);
        }
        cx.notify();
    }

    fn set_shared_prefix(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
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

    fn load_wine_list(&mut self, cx: &mut Context<Self>) {
        let paths = self.ctx.heroic.clone();
        // Debug captures only keep the first frames: load in place there.
        if std::env::var_os("HERONITE_DEBUG_DIALOG").is_some() {
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

    // -- art -----------------------------------------------------------------

    fn open_picker(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx);
        let ctx = self.ctx.clone();
        let view = cx.new(|cx| ImagePicker::new(&ctx, &title, slot, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &PickerEvent, window, cx| match ev {
            PickerEvent::Picked(slot, url) => {
                let input = if *slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
                input.update(cx, |s, cx| s.set_value(url.clone(), window, cx));
            }
            PickerEvent::Close => {
                this.picker = None;
                cx.notify();
            }
        });
        self.picker = Some((view, sub));
        cx.notify();
    }

    fn choose_art_file(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, false, "Select Image", move |this: &mut Self, path, window, cx| {
            let input = if slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
            input.update(cx, |s, cx| s.set_value(file_url(&path), window, cx));
        });
    }

    /// Title field lost focus: like Heroic, look for a cover when there is none.
    fn auto_find_art(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx).trim().to_string();
        let empty = text_of(&self.square, cx).trim().is_empty() && text_of(&self.hero, cx).trim().is_empty();
        if self.is_edit() || title.is_empty() || !empty || self.finding || self.auto_art_tried.as_deref() == Some(title.as_str()) {
            return;
        }
        if std::env::var_os("HERONITE_DEBUG_DIALOG").is_some() {
            return;
        }
        self.find_art(window, cx);
    }

    /// Looks the title up (Steam CDN, SteamGridDB with a key, Bottles' mirror)
    /// and fills the art fields that are empty.
    fn find_art(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx).trim().to_string();
        if title.is_empty() || self.finding {
            return;
        }
        self.auto_art_tried = Some(title.clone());
        self.finding = true;
        self.notice.clear();
        let service = super::metadata_wizard::hub(cx).read(cx).service.clone();
        let app_name = self.app_name.clone();
        cx.spawn_in(window, async move |this, cx| {
            let t = title.clone();
            let found = cx
                .background_spawn(async move {
                    let key = heronite_metadata::covers::steamgrid_api_key(&service.paths);
                    let meta = heronite_data::models::LocalGameMeta {
                        app_name: app_name.clone(),
                        runner: "sideload".into(),
                        title: t.clone(),
                        ..Default::default()
                    };
                    let game = GameInfo { runner: "sideload".into(), app_name, title: t, ..Default::default() };
                    heronite_metadata::covers::fetch_covers_for_game(&service.http, key.as_deref(), &meta, &game)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.finding = false;
                let fill = |input: &Entity<InputState>, value: Option<String>, window: &mut Window, cx: &mut Context<Self>| -> bool {
                    match value.filter(|v| !v.is_empty()) {
                        Some(v) if text_of(input, cx).trim().is_empty() => {
                            input.update(cx, |s, cx| s.set_value(v, window, cx));
                            true
                        }
                        _ => false,
                    }
                };
                let a = fill(&this.square, found.art_square.clone(), window, cx);
                let b = fill(&this.hero, found.art_cover.clone(), window, cx);
                this.notice = if a || b { "Art found. You can change it below.".into() } else { "No art found for this title.".into() };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // -- save / remove ---------------------------------------------------------

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.confirm_remove {
            return;
        }
        let title = text_of(&self.title, cx).trim().to_string();
        let exe = text_of(&self.exe, cx).trim().to_string();
        let url = normalize_url(&text_of(&self.url, cx));
        let platform = self.platform;
        if let Some(msg) = missing_field(&title, platform, &exe, &url) {
            self.error = msg.to_string();
            cx.notify();
            return;
        }
        let (art_cover, art_square) = art_pair(&text_of(&self.square, cx), &text_of(&self.hero, cx));
        let browser = platform == SideloadPlatform::Browser;
        let spec = SideloadSpec {
            app_name: self.app_name.clone(),
            title,
            executable: if browser { String::new() } else { exe.clone() },
            platform,
            art_cover,
            art_square,
            browser_url: if browser { url } else { String::new() },
            custom_user_agent: if browser { text_of(&self.user_agent, cx).trim().to_string() } else { String::new() },
            launch_full_screen: browser && self.fullscreen,
            run_as: None,
        };
        let new_wine = (!self.is_edit() && platform == SideloadPlatform::Windows).then(|| {
            (
                text_of(&self.prefix, cx).trim().to_string(),
                self.wine.as_ref().map(wine_to_value),
                self.bottle.clone(),
            )
        });
        let (paths, home, existing_run_as, is_edit) =
            (self.ctx.heroic.clone(), self.home.clone(), self.existing_run_as.clone(), self.is_edit());
        self.saving = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let mut spec = spec;
                    if !browser {
                        let detected = detect_platform(&spec.executable, &SystemProbe);
                        spec.run_as = run_as_for(spec.platform, detected, existing_run_as.as_deref());
                    }
                    let out = sideload::save_game(&paths, &spec)?;
                    if let Some((prefix, wine, bottle)) = new_wine {
                        sideload::init_game_config(&paths, &home, &spec.app_name, &prefix, wine, &bottle)?;
                    }
                    if out.added && !is_edit {
                        after_added(&paths, &spec);
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(()) => cx.emit(DialogEvent::ChangedAndClose),
                    Err(err) => this.error = format!("Could not save the game: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Mode::Edit(app) = self.mode.clone() else { return };
        if self.saving {
            return;
        }
        self.saving = true;
        self.error.clear();
        let (paths, home, with_prefix) = (self.ctx.heroic.clone(), self.home.clone(), self.remove_prefix_too && self.removable_prefix.is_some());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    // The prefix path comes from the game's config: delete it first.
                    let prefix = with_prefix.then(|| sideload::remove_prefix(&paths, &app, &home));
                    heronite_metadata::remove::remove_collection_game(
                        &paths,
                        &heronite_metadata::remove::ShortcutDirs::detect(),
                        &app,
                        "sideload",
                    )?;
                    anyhow::Ok(prefix)
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(prefix) => {
                        if let Some(PrefixOutcome::Skipped(why)) = prefix {
                            log::warn!("Wine prefix kept: {why}");
                        }
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(err) => this.error = format!("Could not remove the game: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Run Installer First": runs a setup program inside the chosen prefix
    /// with the chosen Wine, before the game exists.
    fn run_installer(&mut self, exe: String, cx: &mut Context<Self>) {
        if self.installer_running {
            return;
        }
        self.installer_running = true;
        self.notice.clear();
        self.error.clear();
        let mut map = game_config::load_game(&self.ctx.heroic, &self.app_name, &self.home);
        map.insert("winePrefix".into(), json!(text_of(&self.prefix, cx).trim()));
        if let Some(w) = &self.wine {
            map.insert("wineVersion".into(), wine_to_value(w));
        }
        let (paths, home) = (self.ctx.heroic.clone(), self.home.clone());
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_exe_blocking(&paths, &home, &map, &exe));
        });
        cx.spawn(async move |this, cx| {
            let res = rx.await.unwrap_or_else(|_| Err("The installer was interrupted".into()));
            this.update(cx, |this, cx| {
                this.installer_running = false;
                match res {
                    Ok(text) => this.notice = text,
                    Err(text) => this.error = text,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // -- debug -------------------------------------------------------------------

    /// `HERONITE_DEBUG_DIALOG=add-game:<state>`: pre-fills the form like a user would.
    pub fn debug_state(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let set = |input: &Entity<InputState>, v: &str, window: &mut Window, cx: &mut Context<Self>| {
            input.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
        };
        for part in state.split('+') {
            match part {
                "windows" => {
                    set(&self.title, "Sample Adventure", window, cx);
                    set(&self.exe, "/data/Games/Sample Adventure/sample.exe", window, cx);
                    self.platform = SideloadPlatform::Windows;
                }
                "linux" => {
                    set(&self.title, "Native Racer", window, cx);
                    set(&self.exe, "/data/Games/native-racer/start.sh", window, cx);
                    self.platform = SideloadPlatform::Linux;
                    self.platform_manual = true;
                }
                "browser" => {
                    set(&self.title, "Browser Quest", window, cx);
                    set(&self.url, "https://browserquest.example.com/play", window, cx);
                    self.platform = SideloadPlatform::Browser;
                    self.platform_manual = true;
                }
                "shared" => self.set_shared_prefix(true, window, cx),
                "platform-open" => self.open_select = Some(Select::Platform),
                "wine-open" => self.open_select = Some(Select::Wine),
                "remove" => self.confirm_remove = true,
                // Exercises the save path (fixture data only).
                "save" => self.save(cx),
                "remove-now" => self.remove(cx),
                "noprefix" => self.removable_prefix = None,
                "picker" => self.open_picker(Slot::Square, window, cx),
                art if art.starts_with("art=") => {
                    // art=<cover url>[,<hero url>]
                    let urls: Vec<&str> = art[4..].split(',').collect();
                    set(&self.square, urls[0], window, cx);
                    if let Some(h) = urls.get(1) {
                        set(&self.hero, h, window, cx);
                    }
                }
                _ => {}
            }
        }
        cx.notify();
    }
}

/// Adding a game also creates its shortcuts when Heroic's settings ask for
/// them (`addDesktopShortcuts`, `addStartMenuShortcuts`, `addSteamShortcuts`).
fn after_added(paths: &heronite_data::HeroicPaths, spec: &SideloadSpec) {
    let outcome = crate::system::shortcuts::on_game_added(paths, "sideload", &spec.app_name);
    for m in outcome.messages {
        if m.error {
            log::warn!("shortcuts for {}: {}", spec.title, m.text);
        } else {
            log::info!("shortcuts for {}: {}", spec.title, m.text);
        }
    }
}

/// Builds and runs "Run EXE on prefix" for a game that has no files yet and
/// waits for it. Never called by tests.
fn run_exe_blocking(paths: &heronite_data::HeroicPaths, home: &str, map: &Settings, exe: &str) -> Result<String, String> {
    let settings = GameSettings::from_map(map, home);
    let ctx = PlanContext::load(paths);
    let built = build_tool_command(&ctx, &SystemProbe, &settings, &WineTool::RunExe(exe.to_string()))?;
    let c = &built.command;
    let mut cmd = std::process::Command::new(&c.program);
    cmd.args(&c.args).envs(&c.env).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(dir) = c.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", c.program))?;
    let status = child.wait().map_err(|e| format!("{}: {e}", c.program))?;
    let mut text = if status.success() { "The program finished.".to_string() } else { format!("The program exited with {status}.") };
    for n in built.notes {
        text.push_str(&format!(" {n}."));
    }
    Ok(text)
}

// -- rendering ---------------------------------------------------------------

impl AddGameDialog {
    fn art_box(&self, id: &'static str, slot: Slot, url: &str, w: f32, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let chain: Vec<ImageRef> = chain_for_url(&self.ctx.heroic, None, url.trim());
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("ag-{id}:{url}"), chain, w, h)) };
        let empty = src.is_none();
        div()
            .id(id)
            .relative()
            .w(px(w))
            .h(px(h))
            .rounded(px(6.))
            .overflow_hidden()
            .cursor_pointer()
            .border_1()
            .border_color(theme::white_a(0.14))
            .hover(|s| s.border_color(theme::accent()))
            .on_click(cx.listener(move |this, _, window, cx| this.open_picker(slot, window, cx)))
            .child(picture(src).size_full())
            .when(empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .text_size(px(13.33))
                        .text_color(theme::white_a(0.6))
                        .child(icon(Icon::FaSearch, px(18.), theme::white_a(0.6)))
                        .child(if slot == Slot::Square { "Cover" } else { "Background" }),
                )
            })
    }

    fn art_field(&self, id: &'static str, label: &str, input: &Entity<InputState>, slot: Slot, cx: &mut Context<Self>) -> Div {
        let has = !text_of(input, cx).is_empty();
        path_field(
            SharedString::from(format!("{id}-pick")),
            Some(label),
            input,
            has,
            false,
            cx.listener(move |this, _, window, cx| {
                if has {
                    let input = if slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    this.choose_art_file(slot, window, cx);
                }
            }),
        )
    }

    fn platform_select(&self, cx: &mut Context<Self>) -> Div {
        let options: Vec<(SharedString, bool)> =
            PLATFORMS.iter().map(|p| (SharedString::from(platform_label(*p)), *p == self.platform)).collect();
        select_field(
            "ag-platform",
            Some("Platform"),
            platform_label(self.platform),
            false,
            self.open_select == Some(Select::Platform),
            options,
            cx,
            |this, _, cx| {
                this.open_select = if this.open_select == Some(Select::Platform) { None } else { Some(Select::Platform) };
                cx.notify();
            },
            |this, i, _, cx| this.set_platform(PLATFORMS[i], cx),
        )
    }

    fn wine_section(&mut self, cx: &mut Context<Self>) -> Div {
        let open = self.show_wine;
        let header = div()
            .id("ag-wine-toggle")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(16.))
            .text_color(crate::theme::text_primary())
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_wine = !this.show_wine;
                cx.notify();
            }))
            .child(icon(if open { Icon::FaChevronUp } else { Icon::FaChevronDown }, px(12.), crate::theme::text_primary()))
            .child("Show Wine settings");
        let mut col = div().flex().flex_col().gap(px(12.)).child(header);
        if !open {
            return col;
        }
        let shared = self.shared_prefix;
        let mut share_label = "Use shared Wine prefix".to_string();
        if let Some(w) = self.global.get("wineVersion").and_then(|_| Some(wine_of(&self.global))) {
            share_label = format!("Use shared Wine prefix ({})", display_wine_name(&w.name));
        }
        col = col.child(
            checkbox("ag-shared", shared, share_label).on_click(cx.listener(|this, _, window, cx| {
                let on = !this.shared_prefix;
                this.set_shared_prefix(on, window, cx);
            })),
        );
        if shared {
            col = col.child(
                panel().child(meta_text(
                    "Only use this option if you know what you are doing. Sharing the same prefix for multiple games can create problems if their dependencies are incompatible.",
                )),
            );
        }
        if self.wine.as_ref().is_some_and(|w| w.kind == "crossover") {
            col = col.child(meta_text("CrossOver bottles are chosen in Game settings after the game is added."));
        } else {
            let has = !text_of(&self.prefix, cx).is_empty();
            col = col.child(path_field(
                "ag-prefix-pick",
                Some("WinePrefix"),
                &self.prefix,
                has,
                shared,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.prefix.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, true, "Select WinePrefix Folder", |this: &mut Self, path, window, cx| {
                            this.prefix.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }
                }),
            ));
        }
        // Wine version.
        let list = self.wine_list.clone();
        let loading = list.is_none();
        let current = self.wine.clone().unwrap_or_else(default_wine);
        let options = wine_options(list.as_deref().unwrap_or(&[]), &current);
        let none_found = list.as_ref().is_some_and(|l| l.is_empty()) && self.wine.is_none();
        let shown = if loading {
            "Loading\u{2026}".to_string()
        } else if self.wine.is_none() {
            "No Wine version found".to_string()
        } else {
            display_wine_name(&current.name)
        };
        let items: Vec<(SharedString, bool)> = options
            .iter()
            .map(|w| (SharedString::from(display_wine_name(&w.name)), self.wine.as_ref().is_some_and(|c| c.name == w.name)))
            .collect();
        col = col.child(select_field(
            "ag-wine",
            Some("Wine Version"),
            shown,
            shared || loading || options.is_empty() || none_found,
            self.open_select == Some(Select::Wine),
            items,
            cx,
            |this, _, cx| {
                this.open_select = if this.open_select == Some(Select::Wine) { None } else { Some(Select::Wine) };
                cx.notify();
            },
            move |this, i, _, cx| {
                let list = this.wine_list.clone().unwrap_or_default();
                let cur = this.wine.clone().unwrap_or_else(default_wine);
                if let Some(w) = wine_options(&list, &cur).get(i) {
                    this.wine = Some(w.clone());
                }
                this.open_select = None;
                cx.notify();
            },
        ));
        if none_found {
            col = col.child(div().text_size(px(13.33)).text_color(theme::danger_text()).child("No Wine version was found, download one from the Wine Manager"));
        }
        col
    }

    fn form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let (square_url, hero_url) = (text_of(&self.square, cx), text_of(&self.hero, cx));
        let title = text_of(&self.title, cx);
        let preview_hero = if hero_url.trim().is_empty() { square_url.clone() } else { hero_url.clone() };

        let left = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .w(px(SQUARE_W))
            .child(self.art_box("ag-art-square", Slot::Square, &square_url, SQUARE_W, SQUARE_H, cx))
            .child(self.art_box("ag-art-hero", Slot::Cover, &preview_hero, HERO_W, HERO_H, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(theme::W_BOLD)
                            .text_color(crate::theme::text_primary())
                            .child(if title.trim().is_empty() { "Title".to_string() } else { title.clone() }),
                    )
                    .child(meta_text(platform_label(self.platform))),
            );

        let mut right = div().flex().flex_col().flex_1().min_w_0().gap(px(16.));
        if !self.is_edit() {
            right = right.child(panel().child(div().text_size(px(14.)).line_height(px(20.)).text_color(white()).child(
                "Important! Are you adding a game from Epic, GOG or Amazon? Do NOT use this feature for that. Log into the store from the menu, look for the game in your library and use Install / Import instead.",
            )));
        }
        right = right.child(
            div()
                .flex()
                .flex_col()
                .child(field_label("Game/App Title"))
                .child(input_box(&self.title, false)),
        );

        // Images.
        let finding = self.finding;
        right = right
            .child(self.art_field("ag-square", "Square Art (paste an URL, pick a file or click the image to search the web)", &self.square, Slot::Square, cx))
            .child(self.art_field("ag-hero", "Cover/Hero Art (optional, wide background)", &self.hero, Slot::Cover, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(action_button(
                        "ag-find-art",
                        ButtonKind::Glass,
                        if finding { "Searching\u{2026}" } else { "Find art for this title" },
                        !finding && !title.trim().is_empty(),
                        cx.listener(|this, _, window, cx| this.find_art(window, cx)),
                    ))
                    .child(action_button(
                        "ag-search-square",
                        ButtonKind::Glass,
                        "Search the web: cover",
                        true,
                        cx.listener(|this, _, window, cx| this.open_picker(Slot::Square, window, cx)),
                    ))
                    .child(action_button(
                        "ag-search-hero",
                        ButtonKind::Glass,
                        "Search the web: background",
                        true,
                        cx.listener(|this, _, window, cx| this.open_picker(Slot::Cover, window, cx)),
                    )),
            );

        right = right.child(self.platform_select(cx));

        if self.platform == SideloadPlatform::Windows && !self.is_edit() {
            right = right.child(self.wine_section(cx));
        }

        if self.platform == SideloadPlatform::Browser {
            right = right
                .child(labeled("BrowserURL", input_box(&self.url, false)))
                .child(labeled("Custom User Agent", input_box(&self.user_agent, false)))
                .child(checkbox("ag-fullscreen", self.fullscreen, "Launch Fullscreen (F11 to exit)").on_click(cx.listener(|this, _, _, cx| {
                    this.fullscreen = !this.fullscreen;
                    cx.notify();
                })));
        } else {
            let exe = text_of(&self.exe, cx);
            let has = !exe.is_empty();
            right = right.child(path_field(
                "ag-exe-pick",
                Some("Select Executable"),
                &self.exe,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.exe.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, false, "Select Executable", |this: &mut Self, path, window, cx| {
                            this.exe.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }
                }),
            ));
            if let Some(folder) = install_folder(&exe) {
                right = right.child(meta_text(format!("Install folder: {folder}")));
            }
        }

        if !self.error.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::danger_text()).child(self.error.clone()));
        } else if !self.notice.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::success()).child(self.notice.clone()));
        }

        div().flex().gap(px(24.)).child(left).child(right)
    }

    fn remove_confirmation(&self, cx: &mut Context<Self>) -> Div {
        let title = text_of(&self.title, cx);
        let mut col = div().flex().flex_col().gap(px(16.)).child(div().text_size(px(16.)).line_height(px(22.)).text_color(crate::theme::text_primary()).child(format!(
            "Remove {} from your library? The game's files are not deleted. Its playtime, status, notes, images and Collection data are removed, and so are its shortcuts.",
            if title.trim().is_empty() { "this game" } else { title.trim() }
        )));
        if let Some(prefix) = &self.removable_prefix {
            col = col
                .child(
                    checkbox("ag-remove-prefix", self.remove_prefix_too, "Also delete the Wine prefix").on_click(cx.listener(|this, _, _, cx| {
                        this.remove_prefix_too = !this.remove_prefix_too;
                        cx.notify();
                    })),
                )
                .child(meta_text(format!("{prefix}  (saves and settings inside it will be lost)")));
        }
        if !self.error.is_empty() {
            col = col.child(div().text_size(px(14.)).text_color(theme::danger_text()).child(self.error.clone()));
        }
        col
    }
}

impl Render for AddGameDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let edit = self.is_edit();
        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);

        let (title, body, footer) = if self.confirm_remove {
            let footer = div()
                .flex()
                .gap(px(10.))
                .child(action_button(
                    "ag-remove-back",
                    ButtonKind::Ghost,
                    "Back",
                    !self.saving,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = false;
                        this.error.clear();
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "ag-remove-yes",
                    ButtonKind::Danger,
                    if self.saving { "Removing\u{2026}" } else { "Remove game" },
                    !self.saving,
                    cx.listener(|this, _, _, cx| this.remove(cx)),
                ));
            ("Remove game", self.remove_confirmation(cx), footer)
        } else {
            let can_save = !self.saving && !self.installer_running;
            let finish_label = match (edit, self.saving) {
                (false, false) => "Finish",
                (true, false) => "Save",
                (_, true) => "Saving\u{2026}",
            };
            let mut footer = div().flex().items_center().w_full().gap(px(16.));
            if edit {
                footer = footer.child(action_button(
                    "ag-remove",
                    ButtonKind::Glass,
                    "Remove game",
                    can_save,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = true;
                        this.error.clear();
                        this.notice.clear();
                        cx.notify();
                    }),
                ));
            }
            footer = footer.child(div().flex_1());
            footer = footer.child(action_button(
                "ag-cancel",
                ButtonKind::Ghost,
                "Cancel",
                !self.saving,
                cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)),
            ));
            if !edit && self.platform == SideloadPlatform::Windows {
                let can_run = can_save && !text_of(&self.title, cx).trim().is_empty();
                footer = footer.child(action_button(
                    "ag-run-installer",
                    ButtonKind::Glass,
                    if self.installer_running { "Running Setup" } else { "Run Installer First" },
                    can_run,
                    cx.listener(|_, _, window, cx| {
                        choose_path(window, cx, false, "Select EXE to Run", |this: &mut Self, path, _, cx| this.run_installer(path, cx));
                    }),
                ));
            }
            footer = footer.child(button("ag-finish", ButtonKind::Primary, finish_label).when(!can_save, |b| b.opacity(0.5)).on_click(cx.listener(
                |this, _, _, cx| this.save(cx),
            )));
            (if edit { "Edit game" } else { "Add game" }, self.form(window, cx), footer)
        };

        let main = dialog(
            DialogProps {
                id: "add-game".into(),
                title: title.into(),
                width: 860f32.min(vw * 0.96),
                height: super::heroic_settings::debug_dialog_height(),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        );
        let nested = self.picker.as_ref().map(|(p, _)| AnyView::from(p.clone()));
        div().absolute().inset_0().child(main).children(nested)
    }
}
