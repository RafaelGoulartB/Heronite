//! "Wine" tab: Wine version, prefix, DXVK/VKD3D, sync options and tools.

use std::path::Path;

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::dxvk::{self, Action};
use heronite_launch::settings::GameSettings;
use heronite_launch::tools::Tool;
use heronite_launch::wine_tools::{build_tool_command, WineTool};
use heronite_launch::{PlanContext, SystemProbe};
use serde_json::{json, Value};

use super::logic::{self, RunKind};
use super::parts::{column, danger_text, heading, help, rows};
use super::protondb::{best_ge, ProtonDb, Recommendation};
use super::{Notice, SettingsPanel, Target};
use crate::theme;
use crate::ui::fields::{choose_path, SelectOption};
use heronite_wine::WineInstallation;

impl SettingsPanel {
    pub(super) fn native(&self) -> bool {
        matches!(&self.target, Target::Game(f) if f.run_kind == RunKind::Native)
    }

    pub(super) fn wine_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let wine = self.wine_version();
        let crossover = wine.kind == "crossover";
        let mut col = column();
        if crossover {
            let top =
                rows().field(self.wine_version_field(cx)).field(self.text_row("crossoverBottle", Some(tr!("CrossOver Bottle")), None));
            return col.child(top.card());
        }
        // The same sections as a Steam game's Proton tab: the build, then the prefix.
        col = col.child(heading(tr!("Wine / Proton"))).child(rows().field(self.wine_version_field(cx)).card());
        let mut prefix = rows();
        let folder_label = if self.is_game() {
            prefix = prefix.field(self.prefix_menu(cx));
            tr!("Prefix folder")
        } else {
            tr!("Prefix")
        };
        prefix = prefix.field_help(
            self.path_row("winePrefix", Some(folder_label), true, tr!("Select WinePrefix Folder"), false, cx),
            vec![help(
                tr!("Wine uses what is called a WINEPREFIX to encapsulate Windows applications. This prefix contains the Wine configuration files and a reproduction of the file hierarchy of C: (the main disk on a Windows OS). In this reproduction of the C: drive, your game save files and dependencies installed via winetricks are stored."),
            )],
        );
        if self.is_game() {
            prefix = prefix.child(self.tools_row(cx));
        }
        col = col.child(heading(tr!("Prefix"))).child(prefix.card());

        let dxvk = logic::offers_dxvk(&wine);
        let dxvk_on = self.flag("autoInstallDxvk", false);
        // Heroic dims the switches ("Please wait...") while a tool is being installed.
        let tools_busy = self.tool_toggle.is_some();
        let mut graphics = rows();
        if dxvk {
            graphics = graphics.child(self.toggle_row(
                "auto-dxvk",
                "autoInstallDxvk",
                false,
                tr!("Auto Install/Update DXVK on Prefix"),
                Some(tr!("DXVK is a Vulkan-based translational layer for DirectX 9, 10 and 11 games. Enabling may improve compatibility. Might cause issues especially for older DirectX games.")),
                self.flag("autoInstallDxvkNvapi", false) || self.flag("autoInstallVkd3d", false) || tools_busy,
                cx,
            ));
        }
        if wine.kind != "proton" {
            graphics = graphics.child(self.toggle_row(
                "auto-vkd3d",
                "autoInstallVkd3d",
                false,
                tr!("Auto Install/Update VKD3D on Prefix"),
                Some(tr!("VKD3D is a Vulkan-based translational layer for DirectX 12 games. Enabling may improve compatibility significantly. Has no effect on older DirectX games, it requires DXVK.")),
                !dxvk_on || tools_busy,
                cx,
            ));
        }
        graphics = graphics.child(self.toggle_row(
            "auto-dxvk-nvapi",
            "autoInstallDxvkNvapi",
            false,
            tr!("Auto Install/Update DXVK-NVAPI on Prefix"),
            Some(tr!("DXVK-NVAPI is an implementation of NVAPI built on top of DXVK and the linux native NVAPI, it allows for the usage of DLSS on Nvidia GPUs.")),
            !dxvk_on || tools_busy,
            cx,
        ));
        col = col.child(heading(tr!("Graphics"))).child(graphics.card());
        if self.shows_game_fixes() {
            col = col.child(self.game_fixes_section(cx));
        }

        let mut sync = rows()
            .child(self.toggle_row(
                "esync",
                "enableEsync",
                false,
                tr!("Enable Esync"),
                Some(tr!("Esync aims to reduce wineserver overhead in CPU-intensive games. Enabling may improve performance.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "fsync",
                "enableFsync",
                false,
                tr!("Enable Fsync"),
                Some(tr!("Fsync aims to reduce wineserver overhead in CPU-intensive games. Enabling may improve performance on supported Linux kernels.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "wayland",
                "enableWineWayland",
                false,
                tr!("Enable Wine-Wayland (Experimental) (Wine version needs to support it)"),
                Some(tr!("The Wine-Wayland experimental driver allows games to run natively under the Wayland display protocol. Enabling might break some games.")),
                false,
                cx,
            ));
        if self.flag("enableWineWayland", false) {
            sync = sync.child(self.toggle_row(
                "hdr",
                "enableHDR",
                false,
                tr!("Enable HDR"),
                Some(tr!("HDR enhances colors by providing brighter highlights and deeper contrasts. For HDR to work, both the display and the game must support it.")),
                false,
                cx,
            ));
        }
        sync = sync
            .child(self.toggle_row(
                "wow64",
                "enableWoW64",
                false,
                tr!("Enable WoW64 (Experimental)"),
                Some(tr!("The Wine WoW64 mode allows 32-bit Windows games to be run on 64-bit Wine. Enabling may improve performance, but could also break older 32-bit games.")),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "fsr",
                "enableFSR",
                false,
                tr!("Enable FSR Hack (Wine version needs to support it)"),
                Some(tr!("AMD's FSR helps boost framerate by upscaling lower resolutions in Fullscreen Mode. Image quality increases from 5 to 1 at the cost of a slight performance hit. Enabling may improve performance.")),
                false,
                cx,
            ));
        if self.flag("enableFSR", false) {
            let sharp = self.number("maxSharpness").unwrap_or(5).clamp(1, 5);
            sync = sync.field(self.select_row(
                "fsr-sharpness",
                Some(tr!("FSR Sharpness Strength")),
                sharp.to_string(),
                (1..=5).map(|n| (SharedString::from(n.to_string()), n == sharp)).collect(),
                false,
                cx,
                |this, i, _, cx| this.set("maxSharpness", json!(i as i64 + 1), cx),
            ));
        }
        col = col.child(heading(tr!("Sync and compatibility"))).child(sync.card());
        if !self.is_game() {
            col = col.child(self.shared_folders_section(cx));
        }
        col
    }

    fn wine_version_field(&self, cx: &mut Context<Self>) -> Div {
        let current = self.wine_version();
        let loading = self.wine_list.is_none();
        let list = self.wine_list.clone().unwrap_or_default();
        let options = logic::wine_options(&list, &current);
        let selected = options.iter().position(|w| w.name == current.name).unwrap_or(0);
        let shown = if loading && list.is_empty() { tr!("Loading\u{2026}").to_string() } else { logic::display_wine_name(&current.name) };
        let items: Vec<SelectOption> = options
            .iter()
            .enumerate()
            .map(|(i, w)| SelectOption {
                label: logic::display_wine_name(&w.name).into(),
                selected: i == selected,
                group: Some(logic::wine_group(w).into()),
                note: (self.global_wine.as_deref() == Some(w.name.as_str())).then(|| tr!("default").into()),
            })
            .collect();
        let field = self.select_menu_row(
            "wine-version",
            Some(tr!("Wine / Proton version")),
            shown,
            items,
            loading && list.is_empty(),
            cx,
            |this, i, _, cx| this.pick_wine(i, cx),
        );
        let mut col = column().gap(px(6.)).child(field);
        if !loading && list.is_empty() {
            col = col.child(danger_text(tr!("No Wine version was found, download one from the Wine Manager")));
        }
        if !list.is_empty() && !Path::new(&current.bin).exists() {
            col = col.child(danger_text(tr!("Wine Path is invalid, please select another one.")));
        }
        if !self.flag("showValveProton", false) {
            col = col.child(help(tr!(
                "Valve Proton versions ignored: Valve versions of Proton are ignored by default, enable them in the General settings"
            )));
        }
        col = col.child(help(&tr!("Wine Path: {path}", path = current.bin))).child(help(
            tr!("Heronite searches for versions of Wine, Proton, and CrossOver in: ~/.local/share/heronite/tools/wine, ~/.local/share/heronite/tools/proton, ~/.steam/root/compatibilitytools.d, ~/.steam/steamapps/common, ~/.local/share/lutris/runners/wine, ~/.var/app/com.valvesoftware.Steam (Steam Flatpak) and /usr/share/steam. For other places, use a symbolic link to one of these folders."),
        ));
        let rec = self.wine_recommendation(&options, &current);
        if let Some(row) = self.protondb_row(rec, cx, Self::pick_wine) {
            col = col.child(div().border_t_1().border_color(theme::white_a(0.06)).mt(px(4.)).child(row));
        }
        col
    }

    /// `drive_c` of the game's prefix (`pfx/drive_c` in a Proton one).
    pub(super) fn open_wine_prefix(&mut self, cx: &mut Context<Self>) {
        let dir = std::path::PathBuf::from(heronite_data::game_config::expand_tilde(&self.text("winePrefix"), &self.home));
        let target = [dir.join("pfx/drive_c"), dir.join("drive_c"), dir.clone()].into_iter().find(|d| d.is_dir());
        let res = match target {
            Some(d) => crate::pages::settings::open_folder(&d).map_err(|e| format!("{}: {e}", d.display())),
            None => Err(tr!("The prefix does not exist yet: it is created the first time the game runs.").to_string()),
        };
        if let Err(e) = res {
            self.notice = Some(Notice { text: e.into(), error: true });
            cx.notify();
        }
    }

    /// For games run through Wine: the newest GE-Proton, whatever the
    /// tier (umu runs them like Steam would; GE carries the extra fixes).
    pub(super) fn wine_recommendation(&self, options: &[WineInstallation], current: &WineInstallation) -> Option<Recommendation> {
        let ProtonDb::Tier(tier) = &self.protondb else { return None };
        if tier == "pending" || !self.is_game() {
            return None;
        }
        let names: Vec<String> = options.iter().map(|w| logic::display_wine_name(&w.name)).collect();
        Some(match best_ge(names.iter().map(String::as_str)) {
            Some(i) => Recommendation { label: names[i].clone(), index: Some(i), in_use: options[i].name == current.name },
            None => Recommendation { label: "GE-Proton".into(), index: None, in_use: false },
        })
    }

    /// Picks entry `index` of the Wine menu.
    pub(super) fn pick_wine(&mut self, index: usize, cx: &mut Context<Self>) {
        let list = self.wine_list.clone().unwrap_or_default();
        if let Some(w) = logic::wine_options(&list, &self.wine_version()).get(index) {
            self.set("wineVersion", logic::wine_to_value(w), cx);
        }
    }

    fn tools_row(&mut self, cx: &mut Context<Self>) -> Div {
        let busy = self.tool_busy;
        let idle = busy.is_none();
        let label =
            |name: &'static str, text: &'static str| if busy == Some(name) { tr!("{text} (running)", text) } else { text.to_string() };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(self.outline_button("tool-winecfg", &label("winecfg", "Winecfg"), idle, cx, |this, _, cx| {
                        this.run_tool("winecfg", WineTool::Winecfg, cx)
                    }))
                    .child(self.outline_button("tool-winetricks", &label("winetricks", "Winetricks"), idle, cx, |this, window, cx| {
                        this.open_winetricks(window, cx)
                    }))
                    .child(self.outline_button("tool-runexe", &label("runexe", tr!("Run .exe in prefix")), idle, cx, |_, window, cx| {
                        choose_path(window, cx, false, tr!("Select EXE to Run"), |this: &mut Self, path, _, cx| {
                            this.run_tool("runexe", WineTool::RunExe(path), cx)
                        });
                    }))
                    .child(
                        self.outline_button("tool-openpfx", tr!("Open prefix folder"), true, cx, |this, _, cx| this.open_wine_prefix(cx)),
                    ),
            )
            .child(help(tr!("Tools run with this game's Wine version and prefix.")))
    }

    /// Starts a Wine tool for this game: builds the command off the UI thread
    /// and waits for the program on its own thread.
    pub(super) fn run_tool(&mut self, name: &'static str, tool: WineTool, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        if self.tool_busy.is_some() {
            return;
        }
        let steam = self.steam_tool_settings();
        if self.is_steam() && steam.is_none() {
            return;
        }
        self.tool_busy = Some(name);
        self.notice = None;
        let (paths, home) = (self.paths.clone(), self.home.clone());
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let res = match steam {
                Some(settings) => run_settings_tool(&paths, &settings, &tool),
                None => run_tool_blocking(&paths, &home, &app, &tool),
            };
            let _ = tx.send(res);
        });
        cx.spawn(async move |this, cx| {
            let res = rx.await.unwrap_or_else(|_| Err(tr!("The tool was interrupted").into()));
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
        dxvk::Outcome::Installed(version) => tr!("{name} {version} installed in the prefix.", name, version),
        dxvk::Outcome::AlreadyInstalled(version) => tr!("{name} {version} is already installed in the prefix.", name, version),
        dxvk::Outcome::Removed => tr!("{name} removed from the prefix.", name),
        dxvk::Outcome::Skipped(why) => tr!("{name} not changed in the prefix: {why}.", name, why),
    }
}

impl SettingsPanel {
    /// After the switch `key` was saved as `on`: installs (or removes) the tool
    /// in the prefix on a background thread, like Heroic's `toggleDXVK` & co.
    /// Nothing happens when Heroic would not do it either (Proton, GPTK...).
    /// When the install fails the switch goes back, as in Heroic.
    pub(super) fn apply_tool_toggle(&mut self, key: &'static str, tool: Tool, on: bool, cx: &mut Context<Self>) {
        let settings = GameSettings::from_map(&self.settings, &self.home);
        let Some(action) = dxvk::toggle_action(tool, &settings.wine_version, on) else { return };
        self.tool_toggle = Some(key);
        let name = tool.label();
        let wait = if action == Action::Install {
            tr!("Please wait... Installing {name}", name)
        } else {
            tr!("Please wait... Removing {name}", name)
        };
        self.notice = Some(Notice { text: wait.into(), error: false });
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
                            let text = if action == Action::Install {
                                tr!("Could not install {name}: {error}. The setting was left unchanged.", name, error = e)
                            } else {
                                tr!("Could not remove {name}: {error}. The setting was left unchanged.", name, error = e)
                            };
                            this.notice = Some(Notice { text: text.into(), error: true });
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
                    this.notice = Some(Notice { text: tr!("{name} stopped unexpectedly.", name).into(), error: true });
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
fn run_tool_blocking(paths: &heronite_data::DataPaths, home: &str, app: &str, tool: &WineTool) -> Result<String, String> {
    let map = heronite_data::game_config::load_game(paths, app, home);
    let settings = GameSettings::from_map(&map, home);
    // A new prefix is created first so shared folders get their drive letters.
    heronite_launch::shared_folders::create_prefix_first(&PlanContext::load(paths), &SystemProbe, &settings)?;
    run_settings_tool(paths, &settings, tool)
}

/// Runs a tool with `settings`' Wine and prefix, waiting for it to exit.
fn run_settings_tool(paths: &heronite_data::DataPaths, settings: &GameSettings, tool: &WineTool) -> Result<String, String> {
    let ctx = PlanContext::load(paths);
    let built = build_tool_command(&ctx, &SystemProbe, settings, tool)?;
    let c = &built.command;
    let mut cmd = std::process::Command::new(&c.program);
    cmd.args(&c.args)
        .envs(&c.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = c.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| tr!("Could not start {program}: {error}", program = c.program, error = e))?;
    let status = child.wait().map_err(|e| format!("{}: {e}", c.program))?;
    let name = match tool {
        WineTool::Winecfg => Some("Winecfg"),
        WineTool::Winetricks => Some("Winetricks"),
        WineTool::RunExe(_) => None,
    };
    let mut text = match (name, status.success()) {
        (Some(name), true) => tr!("{name} finished.", name),
        (Some(name), false) => tr!("{name} exited with {status}.", name, status),
        (None, true) => tr!("The program finished.").to_string(),
        (None, false) => tr!("The program exited with {status}.", status),
    };
    for n in built.notes {
        text.push_str(&format!(" {n}."));
    }
    Ok(text)
}
