//! "Wine" tab: Wine version, prefix, DXVK/VKD3D, sync options and tools.

use std::path::Path;

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::dxvk::{self, Action};
use heronite_launch::settings::GameSettings;
use heronite_launch::tools::Tool;
use heronite_launch::wine_tools::{build_tool_command, WineTool};
use heronite_launch::{PlanContext, SystemProbe};
use serde_json::{json, Value};

use super::logic::{self, RunKind};
use super::parts::{column, danger_text, heading, help};
use super::{HeroicSettingsPanel, Notice, Target};
use crate::ui::fields::choose_path;

impl HeroicSettingsPanel {
    pub(super) fn native(&self) -> bool {
        matches!(&self.target, Target::Game(f) if f.run_kind == RunKind::Native)
    }

    pub(super) fn wine_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let wine = self.wine_version();
        let crossover = wine.kind == "crossover";
        let mut col = column().child(self.wine_version_field(cx));
        if crossover {
            col = col.child(self.text_row("crossoverBottle", Some("CrossOver Bottle"), None));
        } else {
            col = col
                .child(self.path_row("winePrefix", Some("WinePrefix folder"), true, "Select WinePrefix Folder", false, cx))
                .child(help(
                    "Wine uses what is called a WINEPREFIX to encapsulate Windows applications. This prefix contains the Wine configuration files and a reproduction of the file hierarchy of C: (the main disk on a Windows OS). In this reproduction of the C: drive, your game save files and dependencies installed via winetricks are stored.",
                ));
            let dxvk = logic::offers_dxvk(&wine);
            let dxvk_on = self.flag("autoInstallDxvk", false);
            // Heroic dims the switches ("Please wait...") while a tool is being installed.
            let tools_busy = self.tool_toggle.is_some();
            if dxvk {
                col = col.child(self.toggle_row(
                    "auto-dxvk",
                    "autoInstallDxvk",
                    false,
                    "Auto Install/Update DXVK on Prefix",
                    Some("DXVK is a Vulkan-based translational layer for DirectX 9, 10 and 11 games. Enabling may improve compatibility. Might cause issues especially for older DirectX games."),
                    self.flag("autoInstallDxvkNvapi", false) || self.flag("autoInstallVkd3d", false) || tools_busy,
                    cx,
                ));
            }
            col = col.child(self.toggle_row(
                "auto-dxvk-nvapi",
                "autoInstallDxvkNvapi",
                false,
                "Auto Install/Update DXVK-NVAPI on Prefix",
                Some("DXVK-NVAPI is an implementation of NVAPI built on top of DXVK and the linux native NVAPI, it allows for the usage of DLSS on Nvidia GPUs."),
                !dxvk_on || tools_busy,
                cx,
            ));
            if wine.kind != "proton" {
                col = col.child(self.toggle_row(
                    "auto-vkd3d",
                    "autoInstallVkd3d",
                    false,
                    "Auto Install/Update VKD3D on Prefix",
                    Some("VKD3D is a Vulkan-based translational layer for DirectX 12 games. Enabling may improve compatibility significantly. Has no effect on older DirectX games, it requires DXVK."),
                    !dxvk_on || tools_busy,
                    cx,
                ));
            }
            col = col
                .child(self.toggle_row(
                    "esync",
                    "enableEsync",
                    false,
                    "Enable Esync",
                    Some("Esync aims to reduce wineserver overhead in CPU-intensive games. Enabling may improve performance."),
                    false,
                    cx,
                ))
                .child(self.toggle_row(
                    "fsync",
                    "enableFsync",
                    false,
                    "Enable Fsync",
                    Some("Fsync aims to reduce wineserver overhead in CPU-intensive games. Enabling may improve performance on supported Linux kernels."),
                    false,
                    cx,
                ))
                .child(self.toggle_row(
                    "wayland",
                    "enableWineWayland",
                    false,
                    "Enable Wine-Wayland (Experimental) (Wine version needs to support it)",
                    Some("The Wine-Wayland experimental driver allows games to run natively under the Wayland display protocol. Enabling might break some games."),
                    false,
                    cx,
                ));
            if self.flag("enableWineWayland", false) {
                col = col.child(self.toggle_row(
                    "hdr",
                    "enableHDR",
                    false,
                    "Enable HDR",
                    Some("HDR enhances colors by providing brighter highlights and deeper contrasts. For HDR to work, both the display and the game must support it."),
                    false,
                    cx,
                ));
            }
            col = col
                .child(self.toggle_row(
                    "wow64",
                    "enableWoW64",
                    false,
                    "Enable WoW64 (Experimental)",
                    Some("The Wine WoW64 mode allows 32-bit Windows games to be run on 64-bit Wine. Enabling may improve performance, but could also break older 32-bit games."),
                    false,
                    cx,
                ))
                .child(self.toggle_row(
                    "fsr",
                    "enableFSR",
                    false,
                    "Enable FSR Hack (Wine version needs to support it)",
                    Some("AMD's FSR helps boost framerate by upscaling lower resolutions in Fullscreen Mode. Image quality increases from 5 to 1 at the cost of a slight performance hit. Enabling may improve performance."),
                    false,
                    cx,
                ));
            if self.flag("enableFSR", false) {
                let sharp = self.number("maxSharpness").unwrap_or(5).clamp(1, 5);
                col = col.child(
                    div().w(px(240.)).child(self.select_row(
                        "fsr-sharpness",
                        Some("FSR Sharpness Strength"),
                        sharp.to_string(),
                        (1..=5).map(|n| (SharedString::from(n.to_string()), n == sharp)).collect(),
                        false,
                        cx,
                        |this, i, _, cx| this.set("maxSharpness", json!(i as i64 + 1), cx),
                    )),
                );
            }
            if self.is_game() {
                col = col.child(self.tools_row(cx));
            }
        }
        col
    }

    fn wine_version_field(&self, cx: &mut Context<Self>) -> Div {
        let current = self.wine_version();
        let loading = self.wine_list.is_none();
        let list = self.wine_list.clone().unwrap_or_default();
        let options = logic::wine_options(&list, &current);
        let selected = options.iter().position(|w| w.name == current.name).unwrap_or(0);
        let shown = if loading && list.is_empty() {
            "Loading\u{2026}".to_string()
        } else {
            logic::display_wine_name(&current.name)
        };
        let items: Vec<(SharedString, bool)> =
            options.iter().enumerate().map(|(i, w)| (SharedString::from(logic::display_wine_name(&w.name)), i == selected)).collect();
        let field = self.select_row("wine-version", Some("Wine Version"), shown, items, loading && list.is_empty(), cx, move |this, i, _, cx| {
            let list = this.wine_list.clone().unwrap_or_default();
            let options = logic::wine_options(&list, &this.wine_version());
            if let Some(w) = options.get(i) {
                this.set("wineVersion", logic::wine_to_value(w), cx);
            }
        });
        let mut col = column().gap(px(6.)).child(field);
        if !loading && list.is_empty() {
            col = col.child(danger_text("No Wine version was found, download one from the Wine Manager"));
        }
        if !list.is_empty() && !Path::new(&current.bin).exists() {
            col = col.child(danger_text("Wine Path is invalid, please select another one."));
        }
        if !self.flag("showValveProton", false) {
            col = col.child(help(
                "Valve Proton versions ignored: Valve versions of Proton are ignored by default, enable them in the General settings",
            ));
        }
        col.child(help(&format!("Wine Path: {}", current.bin))).child(help(
            "Heroic searches for versions of Wine, Proton, and CrossOver in: ~/.config/heroic/tools/wine, ~/.config/heroic/tools/proton, ~/.steam/root/compatibilitytools.d, ~/.steam/steamapps/common, ~/.local/share/lutris/runners/wine, ~/.var/app/com.valvesoftware.Steam (Steam Flatpak) and /usr/share/steam. For other places, use a symbolic link to one of these folders.",
        ))
    }

    fn tools_row(&mut self, cx: &mut Context<Self>) -> Div {
        let busy = self.tool_busy;
        let idle = busy.is_none();
        let label = |name: &'static str, text: &'static str| if busy == Some(name) { format!("{text} (running)") } else { text.to_string() };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(heading("Tools"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(self.outline_button("tool-winecfg", &label("winecfg", "Winecfg"), idle, cx, |this, _, cx| {
                        this.run_tool("winecfg", WineTool::Winecfg, cx)
                    }))
                    .child(self.outline_button("tool-winetricks", &label("winetricks", "Winetricks"), idle, cx, |this, _, cx| {
                        this.run_tool("winetricks", WineTool::Winetricks, cx)
                    }))
                    .child(self.outline_button("tool-runexe", &label("runexe", "Run EXE on Prefix"), idle, cx, |_, window, cx| {
                        choose_path(window, cx, false, "Select EXE to Run", |this: &mut Self, path, _, cx| {
                            this.run_tool("runexe", WineTool::RunExe(path), cx)
                        });
                    })),
            )
            .child(help("Tools run with this game's Wine version and prefix."))
    }

    /// Starts a Wine tool for this game: builds the command off the UI thread
    /// and waits for the program on its own thread.
    pub(super) fn run_tool(&mut self, name: &'static str, tool: WineTool, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        if self.tool_busy.is_some() {
            return;
        }
        self.tool_busy = Some(name);
        self.notice = None;
        let (paths, home) = (self.paths.clone(), self.home.clone());
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_tool_blocking(&paths, &home, &app, &tool));
        });
        cx.spawn(async move |this, cx| {
            let res = rx.await.unwrap_or_else(|_| Err("The tool was interrupted".into()));
            this.update(cx, |this, cx| {
                this.tool_busy = None;
                this.notice = Some(match res {
                    Ok(text) => Notice { text: text.into(), error: false },
                    Err(text) => Notice { text: text.into(), error: true },
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// The prefix tool behind an "Auto Install/Update ..." setting.
pub(super) fn prefix_tool(key: &str) -> Option<Tool> {
    match key {
        "autoInstallDxvk" => Some(Tool::Dxvk),
        "autoInstallDxvkNvapi" => Some(Tool::DxvkNvapi),
        "autoInstallVkd3d" => Some(Tool::Vkd3d),
        _ => None,
    }
}

/// What the background install/remove reports.
enum ToolMsg {
    Progress(String),
    Done(Result<dxvk::Outcome, String>),
}

/// Text for a finished install/remove (no error).
fn outcome_text(tool: Tool, outcome: &dxvk::Outcome) -> String {
    let name = tool.label();
    match outcome {
        dxvk::Outcome::Installed(v) => format!("{name} {v} installed in the prefix."),
        dxvk::Outcome::AlreadyInstalled(v) => format!("{name} {v} is already installed in the prefix."),
        dxvk::Outcome::Removed => format!("{name} removed from the prefix."),
        dxvk::Outcome::Skipped(why) => format!("{name} not changed in the prefix: {why}."),
    }
}

impl HeroicSettingsPanel {
    /// After the switch `key` was saved as `on`: installs (or removes) the tool
    /// in the prefix on a background thread, like Heroic's `toggleDXVK` & co.
    /// Nothing happens when Heroic would not do it either (Proton, GPTK...).
    /// When the install fails the switch goes back, as in Heroic.
    pub(super) fn apply_tool_toggle(&mut self, key: &'static str, tool: Tool, on: bool, cx: &mut Context<Self>) {
        let settings = GameSettings::from_map(&self.settings, &self.home);
        let Some(action) = dxvk::toggle_action(tool, &settings.wine_version, on) else { return };
        self.tool_toggle = Some(key);
        let name = tool.label();
        let verb = if action == Action::Install { "Installing" } else { "Removing" };
        self.notice = Some(Notice { text: format!("Please wait... {verb} {name}").into(), error: false });
        let paths = self.paths.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let res = crate::runtime::toggle_tool_blocking(&paths, &settings, tool, action, &move |m| {
                let _ = progress_tx.unbounded_send(ToolMsg::Progress(m));
            });
            let _ = tx.unbounded_send(ToolMsg::Done(res));
        });
        cx.spawn(async move |this, cx| {
            let mut finished = false;
            while let Some(msg) = rx.next().await {
                let done = matches!(msg, ToolMsg::Done(_));
                this.update(cx, |this, cx| {
                    match msg {
                        ToolMsg::Progress(text) => this.notice = Some(Notice { text: text.into(), error: false }),
                        ToolMsg::Done(Ok(outcome)) => {
                            this.tool_toggle = None;
                            this.notice = Some(Notice { text: outcome_text(tool, &outcome).into(), error: false });
                        }
                        ToolMsg::Done(Err(e)) => {
                            this.tool_toggle = None;
                            this.set(key, Value::Bool(!on), cx);
                            let what = if action == Action::Install { "install" } else { "remove" };
                            this.notice = Some(Notice {
                                text: format!("Could not {what} {name}: {e}. The setting was left unchanged.").into(),
                                error: true,
                            });
                        }
                    }
                    cx.notify();
                })
                .ok();
                if done {
                    finished = true;
                    break;
                }
            }
            if !finished {
                // The worker died without reporting.
                this.update(cx, |this, cx| {
                    this.tool_toggle = None;
                    this.notice = Some(Notice { text: format!("{name} stopped unexpectedly.").into(), error: true });
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

/// Builds and runs a tool, waiting for it to exit. Never called by tests.
fn run_tool_blocking(paths: &heronite_data::HeroicPaths, home: &str, app: &str, tool: &WineTool) -> Result<String, String> {
    let map = heronite_data::game_config::load_game(paths, app, home);
    let settings = GameSettings::from_map(&map, home);
    let ctx = PlanContext::load(paths);
    let built = build_tool_command(&ctx, &SystemProbe, &settings, tool)?;
    let c = &built.command;
    let mut cmd = std::process::Command::new(&c.program);
    cmd.args(&c.args).envs(&c.env).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(dir) = c.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", c.program))?;
    let status = child.wait().map_err(|e| format!("{}: {e}", c.program))?;
    let what = match tool {
        WineTool::Winecfg => "Winecfg",
        WineTool::Winetricks => "Winetricks",
        WineTool::RunExe(_) => "The program",
    };
    let mut text = if status.success() { format!("{what} finished.") } else { format!("{what} exited with {status}.") };
    for n in built.notes {
        text.push_str(&format!(" {n}."));
    }
    Ok(text)
}
