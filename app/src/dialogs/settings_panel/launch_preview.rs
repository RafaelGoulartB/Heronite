//! "How the game starts" in the Launch tab: the command and the environment
//! variables a launch would use right now, each variable with the setting it
//! comes from. Built off the UI thread from the settings on screen (not the
//! saved file), and built again while open when a setting changes.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_launch::settings::GameSettings;
use heronite_launch::{gpu, LaunchOptions, LaunchPlan, PlanContext, SystemProbe};

use super::logic::Section;
use super::parts::{column, heading, help, rows};
use super::SettingsPanel;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};

/// Where a variable comes from.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Source {
    /// A setting of a tab.
    Setting(Section, &'static str),
    /// The game's own variables (Launch › Environment variables).
    Yours,
    /// What every launch gets (Steam ids, logs folder, the store...).
    Always,
}

#[derive(Clone)]
pub(super) struct Var {
    pub name: String,
    pub value: String,
    pub source: Source,
}

/// A launch, as shown.
#[derive(Clone, Default)]
pub(super) struct Preview {
    /// Shell-quoted command line.
    pub command: String,
    pub vars: Vec<Var>,
    /// Scripts, programs alongside, decisions of the plan.
    pub notes: Vec<String>,
    /// Why the game cannot start now.
    pub error: Option<String>,
}

impl Preview {
    /// The variables and the command as one line to paste in a terminal.
    fn shell_line(&self) -> String {
        let vars = self.vars.iter().map(|v| format!("{}={}", v.name, shlex::try_quote(&v.value).unwrap_or_default()));
        vars.chain(std::iter::once(self.command.clone())).collect::<Vec<_>>().join(" ")
    }
}

/// The preview's state in the panel.
#[derive(Default)]
pub(super) struct PreviewState {
    pub open: bool,
    pub busy: bool,
    /// Shows the variables every launch gets too.
    pub show_all: bool,
    pub preview: Option<Preview>,
}

impl SettingsPanel {
    /// Whether this game has a launch to show (Steam starts its own games).
    pub(super) fn has_preview(&self) -> bool {
        !self.is_steam() && self.facts().is_some_and(|f| f.game.is_some())
    }

    pub(super) fn open_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.open = true;
        self.build_preview(cx);
    }

    /// Builds the preview again when it is open (after a setting changed).
    pub(super) fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview.open {
            self.build_preview(cx);
        }
    }

    fn build_preview(&mut self, cx: &mut Context<Self>) {
        let Some(game) = self.facts().and_then(|f| f.game.clone()) else { return };
        let (paths, home, settings) = (self.paths.clone(), self.home.clone(), self.settings.clone());
        // Debug captures only keep the first frames: build in place there.
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            self.preview.preview = Some(compute(&paths, &home, settings, &game));
            return;
        }
        self.preview.busy = true;
        cx.spawn(async move |this, cx| {
            let preview = cx.background_spawn(async move { compute(&paths, &home, settings, &game) }).await;
            this.update(cx, |this, cx| {
                this.preview.busy = false;
                this.preview.preview = Some(preview);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The caption and card at the end of the Launch tab.
    pub(super) fn preview_section(&mut self, cx: &mut Context<Self>) -> Div {
        let col = column().child(heading(tr!("How the game starts")));
        let state = &self.preview;
        let Some(p) = state.preview.as_ref().filter(|_| state.open) else {
            let button = action_button(
                "preview-show",
                ButtonKind::Glass,
                tr!("Show").to_string(),
                !state.busy,
                cx.listener(|this, _, _, cx| this.open_preview(cx)),
            );
            let r = crate::ui::controls::row(
                tr!("Command and variables"),
                Some(tr!("What a launch runs now, and the setting each variable comes from")),
                button,
            );
            return col.child(rows().child(r).card());
        };
        let mut card = rows();
        if let Some(err) = &p.error {
            card = card.child(div().py(px(12.)).text_size(px(13.)).text_color(theme::danger_text()).child(err.clone()));
        } else {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .py(px(12.))
                    .child(crate::ui::fields::field_label(tr!("Command").to_string()))
                    .child(mono(&p.command, theme::text_primary())),
            );
            let (mine, always): (Vec<&Var>, Vec<&Var>) = p.vars.iter().partition(|v| v.source != Source::Always);
            if !mine.is_empty() {
                let list = div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .py(px(12.))
                    .child(crate::ui::fields::field_label(tr!("From your settings").to_string()));
                card = card.child(grouped(list, &mine));
            }
            if !always.is_empty() {
                let shown = self.preview.show_all;
                let label = if shown {
                    tr!("Hide what every launch gets").to_string()
                } else {
                    trn!(always.len(), "And {n} variable every launch gets", "And {n} variables every launch gets")
                };
                let mut list = div().flex().flex_col().gap(px(4.)).py(px(12.)).child(
                    div().id("preview-all").cursor_pointer().text_size(px(12.)).text_color(theme::accent()).child(label).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.preview.show_all = !this.preview.show_all;
                            cx.notify();
                        }),
                    ),
                );
                if shown {
                    list = list.children(always.iter().map(|v| var_line(v)));
                }
                card = card.child(list);
            }
            if !p.notes.is_empty() {
                let mut notes = div().flex().flex_col().gap(px(3.)).py(px(12.));
                for n in &p.notes {
                    notes = notes.child(help(n));
                }
                card = card.child(notes);
            }
        }
        let line = p.shell_line();
        let failed = p.error.is_some();
        let buttons = div()
            .flex()
            .gap(px(8.))
            .py(px(12.))
            .when(!failed, |d| {
                d.child(action_button(
                    "preview-copy",
                    ButtonKind::Glass,
                    tr!("Copy as a command").to_string(),
                    true,
                    cx.listener(move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(line.clone()))),
                ))
            })
            .child(action_button(
                "preview-hide",
                ButtonKind::Ghost,
                tr!("Hide").to_string(),
                true,
                cx.listener(|this, _, _, cx| {
                    this.preview.open = false;
                    cx.notify();
                }),
            ));
        col.child(card.child(buttons).card())
    }
}

/// The variables under their source ("GPU › Smooth Motion"), in order.
fn grouped(mut list: Div, vars: &[&Var]) -> Div {
    let mut last: Option<&Source> = None;
    for v in vars {
        if last != Some(&v.source) {
            list = list.child(div().pt(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(source_label(&v.source)));
            last = Some(&v.source);
        }
        list = list.child(var_line(v));
    }
    list
}

fn var_line(v: &Var) -> Div {
    mono(&format!("{}={}", v.name, v.value), theme::text_primary())
}

fn mono(text: &str, color: impl Into<Hsla>) -> Div {
    div().font_family(theme::FONT_MONO).text_size(px(12.)).line_height(px(17.)).text_color(color.into()).child(text.to_string())
}

/// "GPU › Smooth Motion".
pub(super) fn source_label(s: &Source) -> String {
    match s {
        Source::Setting(section, label) => format!("{} \u{203a} {label}", section.label()),
        Source::Yours => format!("{} \u{203a} {}", Section::Launch.label(), tr!("Environment Variables")),
        Source::Always => tr!("Every launch").to_string(),
    }
}

/// Builds the plan a launch would run with `settings` (blocking).
fn compute(
    paths: &heronite_data::DataPaths,
    home: &str,
    settings: heronite_data::game_config::Settings,
    game: &heronite_data::collection::Game,
) -> Preview {
    let ctx = PlanContext::load(paths);
    // As a launch reads them: every global default under the game's values.
    let mut merged = ctx.global.defaults.clone();
    merged.extend(settings);
    let s = GameSettings::from_map(&merged, home);
    let plan =
        heronite_launch::build_plan_with_settings(&ctx, &SystemProbe, &game.game, game.meta.as_deref(), &s, &LaunchOptions::default());
    describe(&plan, &s)
}

/// The plan as shown: its command, variables and notes.
fn describe(plan: &LaunchPlan, s: &GameSettings) -> Preview {
    let process = match plan {
        LaunchPlan::Unavailable(why) => return Preview { error: Some(why.clone()), ..Default::default() },
        LaunchPlan::SteamUri { uri, .. } => return Preview { command: uri.clone(), ..Default::default() },
        _ => plan.process().expect("every other plan runs a process"),
    };
    let mut vars: Vec<Var> =
        process.command.env.iter().map(|(k, v)| Var { name: k.clone(), value: v.clone(), source: source_of(k, v, s) }).collect();
    // Settings first, by tab; what every launch gets last.
    let rank = |v: &Var| match &v.source {
        Source::Setting(sec, _) => *sec as usize,
        Source::Yours => 50,
        Source::Always => 100,
    };
    vars.sort_by_cached_key(|v| (rank(v), source_label(&v.source)));
    let mut notes = Vec::new();
    if process.sandboxed {
        notes.push(tr!("In the sandbox: what follows heronite-sandbox starts inside it. Programs alongside the game join it.").to_string());
    }
    if let Some(c) = &process.before_script {
        notes.push(tr!("Before it starts: {command}", command = c.display()).to_string());
    }
    if let Some(c) = &process.after_script {
        notes.push(tr!("After it exits: {command}", command = c.display()).to_string());
    }
    for c in &process.companions {
        notes.push(tr!("Alongside: {command}", command = c.command.display()).to_string());
    }
    notes.extend(process.notes.iter().cloned());
    Preview { command: process.command.display(), vars, notes, error: None }
}

/// The setting a variable comes from (`Always` when none does).
fn source_of(name: &str, value: &str, s: &GameSettings) -> Source {
    use Section::{Compatibility as C, Gpu as G, Launch as L, Performance as P};
    // The game's own variables win over everything (`DXVK_CONFIG` is merged).
    if s.env_options.iter().any(|(k, v)| k == name && v == value) {
        return Source::Yours;
    }
    // DXVK's options: vsync, the maker and the memory together.
    if name == gpu::DXVK_CONFIG {
        return Source::Setting(G, "DXVK");
    }
    if let Some(key) = gpu::var_setting(name) {
        return Source::Setting(G, gpu_label(key));
    }
    if name.starts_with("LSFG") {
        return Source::Setting(P, tr!("Lossless Scaling frame generation"));
    }
    let found = match name {
        "WINEPREFIX" | "STEAM_COMPAT_DATA_PATH" => (C, tr!("Prefix")),
        "PROTONPATH" => (C, tr!("Wine / Proton")),
        "WINEESYNC" | "WINEFSYNC" | "PROTON_NO_ESYNC" | "PROTON_NO_FSYNC" => (C, tr!("Sync")),
        "PROTON_ENABLE_WAYLAND" | "DISPLAY" => (C, tr!("Wayland driver")),
        "PROTON_ENABLE_HDR" | "DXVK_HDR" => (C, "HDR"),
        "PROTON_USE_WOW64" | "WINEARCH" => (C, "WoW64"),
        "PROTON_ENABLE_NVAPI" | "PROTON_DISABLE_NVAPI" | "DXVK_ENABLE_NVAPI" | "DXVK_NVAPI_ALLOW_OTHER_DRIVERS" => (C, "DXVK-NVAPI"),
        "PROTON_EAC_RUNTIME" => (C, "Easy Anti-Cheat"),
        "PROTON_BATTLEYE_RUNTIME" => (C, "BattlEye"),
        "PRESSURE_VESSEL_FILESYSTEMS_RW" => (C, tr!("Folders visible to Proton")),
        "GAMEID" => (C, tr!("Game fixes")),
        "DXVK_HUD" => (P, tr!("FPS counter")),
        "DXVK_FRAME_RATE" | "VKD3D_FRAME_RATE" => (P, tr!("FPS limit")),
        "MANGOHUD_CONFIG" | "MANGOHUD" => (P, "MangoHud"),
        "WINE_FULLSCREEN_FSR" | "WINE_FULLSCREEN_FSR_STRENGTH" if s.enable_fsr => (P, tr!("Wine FSR")),
        "PROTON_PREFER_SDL" => (P, tr!("Read controllers through SDL")),
        "PROTON_LOG" | "WINEDEBUG" | "DXVK_LOG_LEVEL" | "VKD3D_DEBUG" if s.verbose_logs => (L, tr!("Detailed logs")),
        _ => return Source::Always,
    };
    Source::Setting(found.0, found.1)
}

/// The GPU tab's label of a setting key.
fn gpu_label(key: &str) -> &'static str {
    match key {
        gpu::KEY_DEVICE => tr!("Draws the game"),
        gpu::KEY_SMOOTH_MOTION => tr!("Smooth Motion"),
        gpu::KEY_REFLEX => tr!("Reflex in Vulkan games"),
        gpu::KEY_SHADER_CACHE => tr!("Bigger shader cache"),
        gpu::KEY_DLSS_UPGRADE => tr!("Newest DLSS"),
        gpu::KEY_DLSS_PRESET => tr!("DLSS model"),
        gpu::KEY_DLSS_INDICATOR => tr!("Show the DLSS indicator"),
        gpu::KEY_FSR4_UPGRADE => tr!("FSR 4"),
        gpu::KEY_FSR4_INDICATOR => tr!("Show the FSR indicator"),
        gpu::KEY_XESS_UPGRADE => tr!("Newest XeSS"),
        gpu::KEY_ANTI_LAG => tr!("Anti-Lag"),
        gpu::KEY_GLTHREAD => tr!("Threaded OpenGL"),
        gpu::KEY_VSYNC => tr!("VSync"),
        _ => tr!("GPU"),
    }
}

impl SettingsPanel {
    /// Console rows of the preview: "Show", then the command and the
    /// variables from the settings.
    pub(super) fn c_preview(&self) -> Vec<super::console::Item> {
        use super::console::Item;
        use crate::console::list::Row;
        use crate::icons::Icon;
        if !self.has_preview() {
            return Vec::new();
        }
        let mut v: Vec<Item> = vec![Self::c_header(tr!("How the game starts"))];
        let Some(p) = self.preview.preview.as_ref().filter(|_| self.preview.open) else {
            v.push(Self::c_do(
                "preview-show",
                tr!("Show the command and variables"),
                Icon::PlayArrow,
                !self.preview.busy,
                |this, _, cx| this.open_preview(cx),
            ));
            return v;
        };
        if let Some(err) = &p.error {
            v.push(super::console::item(
                Row::info("i:preview-error", tr!("Cannot start"), "").danger().desc(err.clone()),
                super::console::Act::None,
            ));
            return v;
        }
        v.push(Self::c_info("preview-command", tr!("Command"), p.command.clone()));
        for (i, var) in p.vars.iter().filter(|x| x.source != Source::Always).enumerate() {
            v.push(super::console::item(
                Row::info(SharedString::from(format!("i:preview-var-{i}")), var.name.clone(), var.value.clone())
                    .desc(source_label(&var.source)),
                super::console::Act::None,
            ));
        }
        v.push(Self::c_do("preview-hide", tr!("Hide"), Icon::Close, true, |this, _, cx| {
            this.preview.open = false;
            cx.notify();
        }));
        v
    }
}
