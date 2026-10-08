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
mod gamescope;
mod general;
pub mod logic;
mod other;
mod parts;
mod saves;
mod wine;

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config::{self, Settings};
use heronite_data::HeroicPaths;
use heronite_wine::WineInstallation;
use serde_json::{json, Value};

use crate::theme;
use crate::ui::fields::new_input;

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
}

const fn s(key: &'static str) -> Bind {
    Bind::Str { key, trim: true }
}

/// `(input id, binding, placeholder)`.
const INPUTS: &[(&str, Bind, &str)] = &[
    ("launcherArgs", Bind::Str { key: "launcherArgs", trim: false }, "Put here the Launcher Arguments"),
    ("winePrefix", s("winePrefix"), ""),
    ("crossoverBottle", s("wineCrossoverBottle"), ""),
    ("targetExe", s("targetExe"), "Select EXE..."),
    ("beforeScript", s("beforeLaunchScriptPath"), "Select script ..."),
    ("afterScript", s("afterLaunchScriptPath"), "Select script ..."),
    ("language", Bind::Str { key: "language", trim: true }, "2-char or 4-char code (i.e.: \"fr\" or \"pt-BR\")"),
    ("dxvkFps", s("DXVKFpsCap"), "Positive integer value (e.g. 30, 60, ...)"),
    ("savesPath", s("savesPath"), "Select the exact save games folder"),
    ("gs.gameWidth", Bind::Gamescope { field: "gameWidth", digits: true }, ""),
    ("gs.gameHeight", Bind::Gamescope { field: "gameHeight", digits: true }, ""),
    ("gs.upscaleWidth", Bind::Gamescope { field: "upscaleWidth", digits: true }, ""),
    ("gs.upscaleHeight", Bind::Gamescope { field: "upscaleHeight", digits: true }, ""),
    ("gs.fpsLimiter", Bind::Gamescope { field: "fpsLimiter", digits: true }, ""),
    ("gs.fpsLimiterNoFocus", Bind::Gamescope { field: "fpsLimiterNoFocus", digits: true }, ""),
    ("gs.additionalOptions", Bind::Gamescope { field: "additionalOptions", digits: false }, ""),
    ("defaultInstallPath", s("defaultInstallPath"), ""),
    ("defaultWinePrefixDir", s("defaultWinePrefixDir"), ""),
    ("defaultSteamPath", s("defaultSteamPath"), ""),
    ("altLegendaryBin", s("altLegendaryBin"), "Using built-in Legendary binary..."),
    ("env.key", Bind::Scratch, "NAME"),
    ("env.value", Bind::Scratch, "VALUE"),
    ("wrap.exe", Bind::Scratch, "New Wrapper"),
    ("wrap.args", Bind::Scratch, "Wrapper Arguments"),
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
}

pub struct HeroicSettingsPanel {
    pub(super) paths: HeroicPaths,
    pub(super) home: String,
    pub(super) target: Target,
    pub(super) section: Section,
    /// Effective values (defaults overlaid with the game's own).
    pub(super) settings: Settings,
    /// `None` while the Wine list loads.
    pub(super) wine_list: Option<Vec<WineInstallation>>,
    /// Id of the select whose menu is open.
    pub(super) open_select: Option<&'static str>,
    pub(super) inputs: HashMap<&'static str, Entity<InputState>>,
    pub(super) notice: Option<Notice>,
    /// Name of the Wine tool running.
    pub(super) tool_busy: Option<&'static str>,
    pub(super) gpu_count: usize,
    pub(super) gamescope_installed: bool,
    pub(super) runtimes: (bool, bool),
    pub(super) env_error: String,
    pub(super) saves: SavesState,
    debounce: HashMap<&'static str, Task<()>>,
    queue: Vec<(String, Value)>,
    writing: bool,
    _subs: Vec<Subscription>,
}

impl HeroicSettingsPanel {
    pub fn new(paths: HeroicPaths, target: Target, section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let settings = match &target {
            Target::Game(f) => game_config::load_game(&paths, &f.app_name, &home),
            Target::Defaults => game_config::load_global(&paths, &home),
        };
        let mut inputs = HashMap::new();
        let mut subs = Vec::new();
        for (id, bind, placeholder) in INPUTS {
            let initial = match bind {
                Bind::Str { key, .. } => value_text(settings.get(*key)),
                Bind::Gamescope { field, .. } => value_text(settings.get("gamescope").and_then(|g| g.get(*field))),
                Bind::Scratch => String::new(),
            };
            let input = new_input(window, cx, &initial, placeholder);
            if !matches!(bind, Bind::Scratch) {
                let id: &'static str = id;
                let digits = matches!(bind, Bind::Gamescope { digits: true, .. });
                subs.push(cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
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
            inputs.insert(*id, input);
        }
        let tools = paths.tools_dir();
        let runtimes = (tools.join("runtimes/battleye_runtime").exists(), tools.join("runtimes/eac_runtime").exists());
        let mut this = Self {
            paths,
            home,
            target,
            section,
            settings,
            wine_list: None,
            open_select: None,
            inputs,
            notice: None,
            tool_busy: None,
            gpu_count: logic::count_gpus(),
            gamescope_installed: std::env::var_os("HERONITE_DEBUG_GAMESCOPE").is_some()
                || heronite_launch::Probe::which(&heronite_launch::SystemProbe, "gamescope").is_some(),
            runtimes,
            env_error: String::new(),
            saves: SavesState::default(),
            debounce: HashMap::new(),
            queue: Vec::new(),
            writing: false,
            _subs: subs,
        };
        this.load_wine_list(cx);
        this.maybe_detect_saves(window, cx);
        // Debug: HERONITE_DEBUG_SETTINGS_SET='key=<json or text>,gamescope.field=<value>' saves values as if
        // the user had changed them (exercises the write path without clicks).
        if let Ok(spec) = std::env::var("HERONITE_DEBUG_SETTINGS_SET") {
            for pair in spec.split(',') {
                if let Some((k, v)) = pair.split_once('=') {
                    // JSON when it parses, else a plain string (no quotes through the shell).
                    let value = serde_json::from_str::<Value>(v).unwrap_or_else(|_| Value::String(v.to_string()));
                    match k.trim().strip_prefix("gamescope.") {
                        Some(field) => this.set_gamescope(field, value, cx),
                        None => this.set(k.trim(), value, cx),
                    }
                }
            }
        }
        this
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

    // -- values ---------------------------------------------------------------

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

    // -- saving ---------------------------------------------------------------

    pub(super) fn set(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        if self.settings.get(key) == Some(&value) {
            return;
        }
        self.settings.insert(key.to_string(), value.clone());
        self.queue.push((key.to_string(), value));
        self.flush(cx);
        cx.notify();
    }

    pub(super) fn toggle(&mut self, key: &str, default: bool, cx: &mut Context<Self>) {
        let now = self.flag(key, default);
        self.set(key, Value::Bool(!now), cx);
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
                    this.notice = Some(Notice { text: format!("Could not save settings: {err:#}").into(), error: true });
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
            Bind::Scratch => {}
        }
    }

    // -- background work --------------------------------------------------------

    fn load_wine_list(&mut self, cx: &mut Context<Self>) {
        // Debug captures only keep the first frames: load in place there.
        if std::env::var_os("HERONITE_DEBUG_DIALOG").is_some() {
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

pub(super) fn value_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

impl Render for HeroicSettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::General => self.general_tab(window, cx),
            Section::Wine => self.wine_tab(window, cx),
            Section::Other => self.other_tab(cx),
            Section::Advanced => self.advanced_tab(window, cx),
            Section::Gamescope => self.gamescope_tab(cx),
            Section::Saves => self.saves_tab(window, cx),
        };
        let mut footer = div()
            .flex()
            .flex_wrap()
            .gap_x(px(24.))
            .pt(px(8.))
            .text_size(px(13.33))
            .text_color(theme::white_a(0.7))
            .child("Settings are saved automatically");
        if let Some(f) = self.facts() {
            footer = footer.child(format!("AppName: {}", f.app_name));
        }
        let notice = self.notice.clone().map(|n| {
            div()
                .text_size(px(14.))
                .line_height(px(20.))
                .text_color(if n.error { theme::danger() } else { theme::success() })
                .child(n.text)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .when(!self.is_game() && self.section != Section::General, |d| {
                d.child(parts::hint(
                    "Changes in this section only apply as default values when installing games. If you want to change the settings of an already installed game, use the Settings button in the game page.",
                ))
            })
            .child(content)
            .children(notice)
            .child(footer)
    }
}

/// `HERONITE_DEBUG_DIALOG_H=<px>`: taller settings dialogs for captures.
pub fn debug_dialog_height() -> Option<f32> {
    std::env::var("HERONITE_DEBUG_DIALOG_H").ok().and_then(|v| v.parse().ok())
}
