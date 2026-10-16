//! Heroic's game settings (Wine, Other, Advanced, Gamescope, Cloud saves) and
//! its global settings (General) as one reusable view, bound either to one
//! game (`GamesConfig/<app>.json`) or to the global defaults (`config.json`
//! -> `defaultSettings`). Hosted by the "Game settings" dialog and the
//! "Collection settings" dialog; settings are saved as soon as they change,
//! like Heroic does.
//!
//! Layout: `logic` (pure rules, tested), `parts` (row widgets), one file per
//! tab.

mod advanced;
mod changes;
mod companions;
mod console;
mod fixes;
mod fps_limit;
mod gamescope;
mod general;
mod launch_preview;
mod gpu;
pub mod logic;
mod lsfg;
mod other;
mod parts;
mod prefix_menu;
mod presets;
mod protondb;
mod saves;
mod shared_folders;
mod steam;
mod wine;
mod winetricks;

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config::{self, Settings};
use heronite_data::DataPaths;
use heronite_i18n::{n_, tr};
use heronite_wine::WineInstallation;
use serde_json::{json, Value};

use crate::theme;
use crate::ui::fields::new_input;

pub use general::GlobalView;
pub use logic::{GameFacts, Section};

/// What the settings are bound to.
#[derive(Clone, Debug)]
pub enum Target {
    Game(GameFacts),
    /// Global defaults new games inherit.
    Defaults,
}

#[derive(Clone, Copy)]
enum Bind {
    /// A string setting.
    Str { key: &'static str, trim: bool },
    /// A field of the `gamescope` object; `digits`: numbers only.
    Gamescope { field: &'static str, digits: bool },
    /// Scratch input (a "new row" form), never saved by itself.
    Scratch,
    /// A Steam game's launch options as text (saved on Enter or blur).
    SteamLine,
    /// A number of the FPS limit (`heronite_launch::fps_limit`), digits only.
    Fps { key: &'static str },
}

const fn s(key: &'static str) -> Bind {
    Bind::Str { key, trim: true }
}

/// `(input id, binding, placeholder)`.
const INPUTS: &[(&str, Bind, &str)] = &[
    ("launcherArgs", Bind::Str { key: "launcherArgs", trim: false }, n_!("Like -nolauncher")),
    ("winePrefix", s("winePrefix"), ""),
    ("crossoverBottle", s("wineCrossoverBottle"), ""),
    ("targetExe", s("targetExe"), n_!("Select EXE...")),
    ("beforeScript", s("beforeLaunchScriptPath"), n_!("Select script ...")),
    ("afterScript", s("afterLaunchScriptPath"), n_!("Select script ...")),
    ("language", Bind::Str { key: "language", trim: true }, n_!("Like pt-BR")),
    ("fps.limit", Bind::Fps { key: heronite_launch::fps_limit::KEY_LIMIT }, n_!("No limit")),
    ("fps.noFocus", Bind::Fps { key: heronite_launch::fps_limit::KEY_NO_FOCUS }, n_!("No limit")),
    ("savesPath", s("savesPath"), n_!("Select the exact save games folder")),
    ("umuId", s("heroniteUmuId"), n_!("Automatic")),
    ("preset.name", Bind::Scratch, n_!("Preset name")),
    ("gs.gameWidth", Bind::Gamescope { field: "gameWidth", digits: true }, ""),
    ("gs.gameHeight", Bind::Gamescope { field: "gameHeight", digits: true }, ""),
    ("gs.upscaleWidth", Bind::Gamescope { field: "upscaleWidth", digits: true }, ""),
    ("gs.upscaleHeight", Bind::Gamescope { field: "upscaleHeight", digits: true }, ""),
    ("gs.additionalOptions", Bind::Gamescope { field: "additionalOptions", digits: false }, ""),
    ("defaultInstallPath", s("defaultInstallPath"), ""),
    ("defaultWinePrefixDir", s("defaultWinePrefixDir"), ""),
    ("defaultSteamPath", s("defaultSteamPath"), ""),
    ("altLegendaryBin", s("altLegendaryBin"), n_!("Using built-in Legendary binary...")),
    ("altGogdlBin", s("altGogdlBin"), n_!("Using built-in gogdl binary...")),
    ("altNileBin", s("altNileBin"), n_!("Using built-in nile binary...")),
    ("env.key", Bind::Scratch, n_!("NAME")),
    ("env.value", Bind::Scratch, n_!("VALUE")),
    ("wrap.exe", Bind::Scratch, n_!("New Wrapper")),
    ("wrap.args", Bind::Scratch, n_!("Wrapper Arguments")),
    ("steamLine", Bind::SteamLine, n_!("Empty: Steam's defaults")),
    ("select.filter", Bind::Scratch, n_!("Search\u{2026}")),
    ("wt.search", Bind::Scratch, n_!("Search components\u{2026}")),
];

/// Outcome line shown under a tab (tool started, sync result, errors).
#[derive(Clone)]
pub struct Notice {
    pub text: SharedString,
    pub error: bool,
}

#[derive(Default)]
pub struct SavesState {
    pub detecting: bool,
    pub detect_tried: bool,
    pub syncing: bool,
    /// Index into `saves::MODES`.
    pub mode: usize,
    pub output: Option<Vec<String>>,
    pub gog: GogSaves,
}

/// The save folders of a GOG game (GOG's cloud saves tab).
#[derive(Default)]
pub struct GogSaves {
    /// `None` until the first load ends.
    pub locations: Option<Result<Vec<heronite_gog::saves::SaveLocation>, String>>,
    pub loading: bool,
}

pub struct SettingsPanel {
    pub(super) paths: DataPaths,
    pub(super) home: String,
    pub(super) target: Target,
    pub(super) section: Section,
    /// Which global view the General section shows (Settings page).
    pub(super) global_view: GlobalView,
    /// Effective values (defaults overlaid with the game's own).
    pub(super) settings: Settings,
    /// `None` while the Wine list loads.
    pub(super) wine_list: Option<Vec<WineInstallation>>,
    /// The prefixes on this machine, for the prefix menu (`None` while read).
    pub(super) prefix_list: Option<Vec<heronite_wine::prefixes::Prefix>>,
    /// Id of the select whose menu is open.
    pub(super) open_select: Option<&'static str>,
    pub(super) inputs: HashMap<&'static str, Entity<InputState>>,
    pub(super) notice: Option<Notice>,
    /// Name of the Wine tool running.
    pub(super) tool_busy: Option<&'static str>,
    /// The Winetricks page, while open (it takes the tab's place).
    winetricks: Option<winetricks::WinetricksPage>,
    /// An Epic game's other ways to start (Advanced tab).
    pub(super) launch_options: Vec<heronite_launch::launch_option::LaunchOption>,
    /// Console mode: focused row, open keyboard / picker, rows focused.
    console_list: crate::console::list::ListNav,
    console_layer: crate::console::Layer,
    console_active: bool,
    /// Setting whose DXVK/VKD3D/NVAPI install or removal is running.
    pub(super) tool_toggle: Option<&'static str>,
    /// The graphics cards and driver layers (`None` while looked at).
    pub(super) machine: Option<std::sync::Arc<heronite_launch::gpu::detect::Machine>>,
    /// What the game's Proton offers the GPU tab: `(its bin, features)`.
    pub(super) proton_features: Option<(String, heronite_launch::gpu::detect::ProtonFeatures)>,
    pub(super) gamescope_installed: bool,
    pub(super) mangohud_installed: bool,
    pub(super) runtimes: (bool, bool),
    /// Frame generation on this machine (`None` while looked for).
    pub(super) lsfg: Option<heronite_launch::lsfg::Install>,
    /// `lsfg-vk-ui`, when installed.
    pub(super) lsfg_ui: Option<String>,
    /// The umu database, for the Game fixes section (`None` while read).
    pub(super) umu: Option<fixes::UmuData>,
    /// The launch presets of every game.
    pub(super) presets: Vec<heronite_data::launch_presets::Preset>,
    pub(super) env_error: String,
    /// The environment variables edited as text (`NAME=value` lines), while open.
    pub(super) env_text: Option<Entity<gpui_kit::component::input::TextareaState>>,
    pub(super) saves: SavesState,
    /// Steam games: what Steam has (the settings map mirrors its launch options).
    steam: Option<steam::SteamState>,
    /// The game's ProtonDB tier, for the Proton picker.
    protondb: protondb::ProtonDb,
    /// Name of the Wine version new games get (marked in a game's menu).
    pub(super) global_wine: Option<String>,
    /// What the game would have without its own values (`None` for the
    /// defaults themselves): the base of the changed marks.
    inherited: Option<Settings>,
    /// "How the game starts" (Launch tab).
    preview: launch_preview::PreviewState,
    debounce: HashMap<&'static str, Task<()>>,
    queue: Vec<(String, Value)>,
    writing: bool,
    _subs: Vec<Subscription>,
}

impl SettingsPanel {
    pub fn new(paths: DataPaths, target: Target, section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let steam_app = match &target {
            Target::Game(f) => f.steam_app_id.clone(),
            Target::Defaults => None,
        };
        let settings = match &target {
            // Filled from Steam once it is read.
            _ if steam_app.is_some() => heronite_steam::options::defaults(),
            Target::Game(f) => game_config::load_game(&paths, &f.app_name, &home),
            Target::Defaults => game_config::load_global(&paths, &home),
        };
        let mut inputs = HashMap::new();
        let mut subs = Vec::new();
        for (id, bind, placeholder) in INPUTS {
            let initial = bound_text(&settings, bind);
            let input = new_input(window, cx, &initial, tr(placeholder));
            if !matches!(bind, Bind::Scratch) {
                let id: &'static str = id;
                let digits = matches!(bind, Bind::Gamescope { digits: true, .. } | Bind::Fps { .. });
                subs.push(cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        if matches!(bind, Bind::SteamLine) {
                            return;
                        }
                        if digits {
                            let text = state.read(cx).value().to_string();
                            let clean = logic::digits_only(&text);
                            if clean != text {
                                state.update(cx, |s, cx| s.set_value(clean, window, cx));
                            }
                        }
                        this.schedule_commit(id, cx);
                        cx.notify();
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        this.debounce.remove(id);
                        this.commit(id, cx);
                    }
                    _ => {}
                }));
            }
            if *id == "select.filter" || *id == "wt.search" {
                // The open menu narrows as the search changes.
                subs.push(cx.subscribe_in(&input, window, |_, _, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        cx.notify();
                    }
                }));
            }
            inputs.insert(*id, input);
        }
        let inherited = match &target {
            _ if steam_app.is_some() => Some(heronite_steam::options::defaults()),
            Target::Game(_) => Some(game_config::default_game_settings(&game_config::load_global(&paths, &home), false, &home)),
            Target::Defaults => None,
        };
        let tools = paths.tools_dir();
        let runtimes = (tools.join("runtimes/battleye_runtime").exists(), tools.join("runtimes/eac_runtime").exists());
        let mut this = Self {
            paths,
            home,
            target,
            section,
            global_view: GlobalView::default(),
            settings,
            wine_list: None,
            prefix_list: None,
            // `HERONITE_DEBUG_OPEN_SELECT=steam-tool|wine-version|fps-method|gpu-device`: that menu starts open.
            open_select: match crate::env::value("HERONITE_DEBUG_OPEN_SELECT").as_deref() {
                Some("steam-tool") => Some("steam-tool"),
                Some("wine-version") => Some("wine-version"),
                Some("fps-method") => Some("fps-method"),
                Some("gpu-device") => Some("gpu-device"),
                _ => None,
            },
            inputs,
            notice: None,
            tool_busy: None,
            winetricks: None,
            launch_options: Vec::new(),
            tool_toggle: None,
            machine: None,
            proton_features: None,
            gamescope_installed: crate::env::flag("HERONITE_DEBUG_GAMESCOPE")
                || heronite_launch::Probe::which(&heronite_launch::SystemProbe, "gamescope").is_some(),
            // `HERONITE_DEBUG_NO_MANGOHUD=1`: as if MangoHud were not installed.
            mangohud_installed: !crate::env::flag("HERONITE_DEBUG_NO_MANGOHUD")
                && heronite_launch::Probe::which(&heronite_launch::SystemProbe, "mangohud").is_some(),
            runtimes,
            lsfg: None,
            lsfg_ui: None,
            umu: None,
            presets: Vec::new(),
            env_error: String::new(),
            env_text: None,
            saves: SavesState::default(),
            steam: steam_app.map(steam::SteamState::new),
            protondb: protondb::ProtonDb::Off,
            global_wine: None,
            inherited,
            preview: Default::default(),
            debounce: HashMap::new(),
            queue: Vec::new(),
            writing: false,
            console_list: Default::default(),
            console_layer: Default::default(),
            console_active: true,
            _subs: subs,
        };
        let wine = this.facts().is_some_and(|f| matches!(f.run_kind, logic::RunKind::Steam | logic::RunKind::Wine));
        if wine {
            this.load_protondb(cx);
        }
        if this.facts().is_some_and(|f| f.run_kind == logic::RunKind::Wine) {
            this.global_wine = Some(logic::wine_of(&game_config::load_global(&this.paths, &this.home)).name);
        }
        this.load_lsfg(cx);
        this.load_gpu(cx);
        this.load_presets(cx);
        if this.facts().is_some_and(|f| f.run_kind == logic::RunKind::Wine) {
            this.load_umu(cx);
        }
        if this.is_steam() {
            // Debug settings are applied once Steam was read.
            this.load_steam(cx);
            this.watch_steam(cx);
            return this;
        }
        this.load_wine_list(cx);
        if let Some(app) = this.facts().filter(|f| f.runner == "legendary").map(|f| f.app_name.clone()) {
            this.launch_options = heronite_epic::launch_options::list(&this.paths, &app);
        }
        if this.facts().is_some_and(|f| f.is_gog()) {
            // A manual sync runs in the service: redraw when it starts and ends.
            let service = crate::services::downloads(cx);
            this._subs.push(cx.observe(&service, |_, _, cx| cx.notify()));
        }
        if this.is_game() && !this.native() {
            this.load_prefix_list(cx);
        }
        this.maybe_detect_saves(window, cx);
        this.apply_debug_settings(cx);
        // `HERONITE_DEBUG_ENV_TEXT=1`: the variables start open as text.
        if crate::env::flag("HERONITE_DEBUG_ENV_TEXT") {
            this.open_env_text(window, cx);
        }
        // `HERONITE_DEBUG_PREVIEW=1`: "How the game starts" starts open.
        if crate::env::flag("HERONITE_DEBUG_PREVIEW") && this.has_preview() {
            this.open_preview(cx);
        }
        // `HERONITE_DEBUG_WINETRICKS=1`: the Winetricks page starts open.
        if crate::env::flag("HERONITE_DEBUG_WINETRICKS") && this.is_game() {
            this.open_winetricks(window, cx);
        }
        this
    }

    /// Debug: HERONITE_DEBUG_SETTINGS_SET='key=<json or text>,gamescope.field=<value>' saves values as if
    /// the user had changed them (exercises the write path without clicks).
    fn apply_debug_settings(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_SETTINGS_SET") else { return };
        for pair in spec.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                // JSON when it parses, else a plain string (no quotes through the shell).
                let value = serde_json::from_str::<Value>(v).unwrap_or_else(|_| Value::String(v.to_string()));
                match k.trim().strip_prefix("gamescope.") {
                    Some(field) => self.set_gamescope(field, value, cx),
                    None => self.set(k.trim(), value, cx),
                }
            }
        }
    }

    /// Switches tab (closing open menus).
    pub fn set_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            self.open_select = None;
            self.notice = None;
            self.env_error.clear();
            self.maybe_detect_saves(window, cx);
            cx.notify();
        }
    }

    pub(super) fn is_game(&self) -> bool {
        matches!(self.target, Target::Game(_))
    }

    pub(super) fn facts(&self) -> Option<&GameFacts> {
        match &self.target {
            Target::Game(f) => Some(f),
            Target::Defaults => None,
        }
    }

    pub(super) fn flag(&self, key: &str, default: bool) -> bool {
        match self.settings.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Null) | None => default,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        }
    }

    pub(super) fn text(&self, key: &str) -> String {
        value_text(self.settings.get(key))
    }

    pub(super) fn number(&self, key: &str) -> Option<i64> {
        self.settings.get(key).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    }

    pub(super) fn wine_version(&self) -> WineInstallation {
        logic::wine_of(&self.settings)
    }

    /// The `gamescope` object over Heroic's defaults.
    pub(super) fn gamescope(&self) -> serde_json::Map<String, Value> {
        let mut g = match json!({
            "enableUpscaling": false,
            "enableLimiter": false,
            "enableForceGrabCursor": false,
            "windowType": "fullscreen",
            "gameWidth": "",
            "gameHeight": "",
            "upscaleHeight": "",
            "upscaleWidth": "",
            "upscaleMethod": "fsr",
            "fpsLimiter": "",
            "fpsLimiterNoFocus": "",
            "additionalOptions": ""
        }) {
            Value::Object(o) => o,
            _ => unreachable!(),
        };
        if let Some(Value::Object(saved)) = self.settings.get("gamescope") {
            for (k, v) in saved {
                g.insert(k.clone(), v.clone());
            }
        }
        g
    }

    pub(super) fn input(&self, id: &str) -> Entity<InputState> {
        self.inputs.get(id).cloned().unwrap_or_else(|| panic!("unknown input {id}"))
    }

    pub(super) fn input_text(&self, id: &str, cx: &App) -> String {
        self.input(id).read(cx).value().to_string()
    }

    pub(super) fn set_input(&mut self, id: &str, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input(id);
        input.update(cx, |s, cx| s.set_value(text.to_string(), window, cx));
    }

    pub(super) fn set(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        if self.settings.get(key) == Some(&value) {
            return;
        }
        if self.is_steam() {
            return self.steam_set(key, value, cx);
        }
        self.settings.insert(key.to_string(), value.clone());
        self.queue.push((key.to_string(), value));
        self.flush(cx);
        if key == "wineVersion" {
            self.load_proton_features(cx);
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    /// [`set`](Self::set) for several keys, saved together (a Steam game's
    /// launch options are rebuilt once).
    pub(super) fn set_all(&mut self, values: Vec<(&str, Value)>, cx: &mut Context<Self>) {
        let mut changed: Vec<(&str, Value)> = values.into_iter().filter(|(k, v)| self.settings.get(*k) != Some(v)).collect();
        let Some((last_key, last)) = changed.pop() else { return };
        if self.is_steam() {
            if self.steam_fields_editable() {
                for (k, v) in changed {
                    self.settings.insert(k.to_string(), v);
                }
            }
            return self.steam_set(last_key, last, cx);
        }
        for (k, v) in changed.into_iter().chain([(last_key, last)]) {
            self.settings.insert(k.to_string(), v.clone());
            self.queue.push((k.to_string(), v));
        }
        self.flush(cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn toggle(&mut self, key: &'static str, default: bool, cx: &mut Context<Self>) {
        let tool = wine::prefix_tool(key);
        if tool.is_some() && self.tool_toggle.is_some() {
            return;
        }
        let now = self.flag(key, default);
        self.set(key, Value::Bool(!now), cx);
        if let Some(tool) = tool {
            self.apply_tool_toggle(key, tool, !now, cx);
        }
    }

    pub(super) fn set_gamescope(&mut self, field: &str, value: Value, cx: &mut Context<Self>) {
        let mut g = self.gamescope();
        g.insert(field.to_string(), value);
        self.set("gamescope", Value::Object(g), cx);
    }

    /// Writes queued changes off the UI thread, one batch at a time, in order.
    fn flush(&mut self, cx: &mut Context<Self>) {
        if self.writing || self.queue.is_empty() {
            return;
        }
        self.writing = true;
        let batch = std::mem::take(&mut self.queue);
        let (paths, home) = (self.paths.clone(), self.home.clone());
        let app = self.facts().map(|f| f.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match app {
                        Some(app) => game_config::set_game_settings(&paths, &app, &home, &batch),
                        None => game_config::set_global_settings(&paths, &home, &batch),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.writing = false;
                if let Err(err) = res {
                    this.notice =
                        Some(Notice { text: tr!("Could not save settings: {err}", err = format!("{err:#}")).into(), error: true });
                }
                this.flush(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn schedule_commit(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            this.update(cx, |this, cx| {
                this.debounce.remove(id);
                this.commit(id, cx);
            })
            .ok();
        });
        self.debounce.insert(id, task);
    }

    /// Saves the text of input `id` into its setting.
    pub(super) fn commit(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some((_, bind, _)) = INPUTS.iter().find(|(i, _, _)| *i == id) else { return };
        let raw = self.input_text(id, cx);
        match *bind {
            Bind::Str { key, trim } => {
                let v = if trim { raw.trim().to_string() } else { raw };
                if v != self.text(key) {
                    self.set(key, Value::String(v), cx);
                }
            }
            Bind::Gamescope { field, digits } => {
                let v = if digits { logic::digits_only(&raw) } else { raw };
                let current = value_text(self.gamescope().get(field));
                if v != current {
                    self.set_gamescope(field, Value::String(v), cx);
                }
            }
            Bind::Fps { key } => {
                let n: u32 = logic::digits_only(&raw).parse().unwrap_or(0);
                self.update_fps(cx, |f| match key {
                    heronite_launch::fps_limit::KEY_NO_FOCUS => f.no_focus = n,
                    _ => f.limit = n,
                });
            }
            Bind::SteamLine => self.steam_commit_line(raw, cx),
            Bind::Scratch => {}
        }
    }

    /// Shows the current settings in every bound input (after the map was
    /// replaced, like a Steam read).
    fn reload_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (id, bind, _) in INPUTS {
            if !matches!(bind, Bind::Scratch | Bind::SteamLine) {
                let text = bound_text(&self.settings, bind);
                if self.input_text(id, cx) != text {
                    self.set_input(id, &text, window, cx);
                }
            }
        }
    }

    fn load_wine_list(&mut self, cx: &mut Context<Self>) {
        // Debug captures only keep the first frames: load in place there.
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            self.wine_list = Some(heronite_wine::list_installations(&self.paths));
            return;
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_wine::list_installations(&paths) }).await;
            this.update(cx, |this, cx| {
                this.wine_list = Some(list);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// What input `bind` shows for `settings`.
fn bound_text(settings: &Settings, bind: &Bind) -> String {
    match bind {
        Bind::Str { key, .. } => value_text(settings.get(*key)),
        Bind::Gamescope { field, .. } => value_text(settings.get("gamescope").and_then(|g| g.get(*field))),
        Bind::Fps { key } => {
            let f = heronite_launch::fps_limit::FpsLimit::from_map(settings);
            let n = if *key == heronite_launch::fps_limit::KEY_NO_FOCUS { f.no_focus } else { f.limit };
            if n == 0 {
                String::new()
            } else {
                n.to_string()
            }
        }
        Bind::Scratch | Bind::SteamLine => String::new(),
    }
}

pub(super) fn value_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_steam_inputs(window, cx);
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let content = match self.section {
            Section::Compatibility if self.winetricks_open() => self.winetricks_tab(cx),
            Section::Compatibility if self.is_steam() => self.proton_tab(cx),
            Section::Performance if self.is_steam() => self.steam_perf_tab(cx),
            Section::Launch if self.is_steam() => self.steam_launch_tab(cx),
            Section::Gpu if self.is_steam() => self.steam_gpu_tab(cx),
            Section::General => self.general_tab(window, cx),
            Section::Compatibility => self.compat_tab(window, cx),
            Section::Performance => self.perf_tab(cx),
            Section::Gpu => self.gpu_tab(false, cx),
            Section::Launch => self.launch_opts_tab(window, cx),
            Section::Saves => self.saves_tab(window, cx),
        };
        let mut footer = div()
            .flex()
            .flex_wrap()
            .gap_x(px(24.))
            .px(px(6.))
            .pt(px(10.))
            .text_size(px(12.))
            .text_color(theme::text_dim())
            .child(tr!("Settings are saved automatically"));
        if let Some(f) = self.facts() {
            footer = footer.child(format!("AppName: {}", f.app_name));
        }
        let notice = self.notice.clone().map(|n| {
            div()
                .px(px(14.))
                .py(px(10.))
                .rounded(px(theme::R_INPUT))
                .bg(if n.error { theme::danger_fill() } else { theme::card_fill() })
                .border_1()
                .border_color(theme::card_border())
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(if n.error { theme::danger_text() } else { theme::success() })
                .child(n.text)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(!self.is_game() && self.section != Section::General, |d| {
                d.child(parts::hint(tr!("Defaults for games installed from now on. To change a game you already have, open its settings.")))
            })
            .child(content)
            .children(notice)
            .child(footer)
            .into_any_element()
    }
}

/// `HERONITE_DEBUG_DIALOG_H=<px>`: taller settings dialogs for captures.
pub fn debug_dialog_height() -> Option<f32> {
    crate::env::parse("HERONITE_DEBUG_DIALOG_H")
}
