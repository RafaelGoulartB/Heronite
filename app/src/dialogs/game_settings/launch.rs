//! Play of local (sideload) games, in the Actions tab: an executable and
//! "run as", or a Linux game's folder and the command run in it. One line
//! that opens into its editor (`HERONITE_DEBUG_PLAY_EDIT=1` opens it).

use std::path::Path;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::{LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use heronite_launch::native::{detect_executable_kind, effective_platform, ExecutableKind, Platform};
use heronite_launch::SystemProbe;

use super::{Busy, GameSettingsDialog};
use crate::dialogs::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, help_text, ButtonKind};
use crate::ui::fields::{input_box, labeled, new_input, select_field};

pub(super) struct LaunchState {
    pub(super) executable: Entity<InputState>,
    pub(super) run_as: String,
    pub(super) run_as_open: bool,
    pub(super) initial_exe: String,
    pub(super) initial_run_as: String,
    pub(super) exists: bool,
    pub(super) detected: ExecutableKind,
    pub(super) effective: Platform,
    pub(super) saved: bool,
    /// Started by a command in `folder` instead of the executable.
    pub(super) by_command: bool,
    pub(super) folder: Entity<InputState>,
    pub(super) command: Entity<InputState>,
    pub(super) initial_by_command: bool,
    pub(super) initial_folder: String,
    pub(super) initial_command: String,
    /// Its editor is open in the Actions tab.
    pub(super) open: bool,
}

impl LaunchState {
    pub(super) fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    /// Something differs from what is saved.
    pub(super) fn dirty(&self, cx: &App) -> bool {
        self.by_command != self.initial_by_command
            || if self.by_command {
                Self::text(&self.folder, cx) != self.initial_folder || Self::text(&self.command, cx) != self.initial_command
            } else {
                self.executable.read(cx).value().as_ref() != self.initial_exe || self.run_as != self.initial_run_as
            }
    }

    /// Play's line while closed: the file and how it runs, or the command.
    pub(super) fn summary(&self, cx: &App) -> String {
        if self.by_command {
            return format!("{} \u{b7} {}", tr!("Command"), Self::text(&self.command, cx));
        }
        let exe = Self::text(&self.executable, cx);
        let file = Path::new(&exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(exe);
        let how = if !self.exists {
            tr!("File not found")
        } else if self.effective == Platform::Linux {
            tr!("Runs natively on Linux")
        } else {
            tr!("Runs through Wine/Proton")
        };
        format!("{file} \u{b7} {how}")
    }

    /// Why the command cannot start (checked as it is typed).
    pub(super) fn command_problem(&self, cx: &App) -> Option<String> {
        if !self.by_command {
            return None;
        }
        let (folder, command) = (Self::text(&self.folder, cx), Self::text(&self.command, cx));
        if folder.is_empty() || command.is_empty() {
            return None;
        }
        heronite_launch::native::check_command(&command, &folder, &SystemProbe).err()
    }

    /// Ready to be saved.
    pub(super) fn can_save(&self, cx: &App) -> bool {
        if self.by_command {
            !Self::text(&self.folder, cx).is_empty() && !Self::text(&self.command, cx).is_empty() && self.command_problem(cx).is_none()
        } else {
            !self.executable.read(cx).value().trim().is_empty()
        }
    }
}

pub(super) fn detect(path: &str) -> (bool, ExecutableKind) {
    (!path.is_empty() && Path::new(path).exists(), detect_executable_kind(path, &SystemProbe))
}

impl GameSettingsDialog {
    pub(super) fn build_launch(game: &Game, window: &mut Window, cx: &mut Context<Self>) -> Option<LaunchState> {
        // A game started by a command is edited in "Edit game" (folder and
        // command); one another launcher or a store client starts, in that
        // launcher or client.
        let show_launch = game.runner == "sideload"
            && !matches!(
                game.launch_kind.as_deref(),
                Some("emulator" | "steam-uri" | heronite_data::launchers::LAUNCH_KIND | heronite_data::clients::LAUNCH_KIND)
            );
        show_launch.then(|| {
            let exe = game.game.install.executable.clone().unwrap_or_default();
            let run_as = game.meta.as_deref().and_then(|m| m.run_as.clone()).unwrap_or_else(|| "auto".into());
            let input = new_input(window, cx, &exe, "");
            cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.refresh_launch(&text);
                    if let Some(l) = this.launch.as_mut() {
                        l.saved = false;
                    }
                    cx.notify();
                }
            })
            .detach();
            let command = game.game.rest.get(heronite_data::sideload::COMMAND_KEY).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let folder = game
                .game
                .install
                .install_path
                .clone()
                .filter(|f| !f.trim().is_empty())
                .or_else(|| Path::new(&exe).parent().map(|p| p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let folder_in = new_input(window, cx, &folder, tr!("The folder the game is in"));
            let command_in = new_input(window, cx, &command, "./start.sh");
            for input in [&folder_in, &command_in] {
                cx.subscribe_in(input, window, |this, _, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        if let Some(l) = this.launch.as_mut() {
                            l.saved = false;
                        }
                        cx.notify();
                    }
                })
                .detach();
            }
            let by_command = !command.is_empty();
            let mut state = LaunchState {
                by_command,
                folder: folder_in,
                command: command_in,
                initial_by_command: by_command,
                initial_folder: folder.clone(),
                initial_command: command.clone(),
                executable: input,
                run_as: run_as.clone(),
                run_as_open: crate::env::flag("HERONITE_DEBUG_SELECT_OPEN"),
                initial_exe: exe.clone(),
                initial_run_as: run_as,
                exists: false,
                detected: ExecutableKind::Unknown,
                effective: Platform::Windows,
                saved: false,
                open: crate::env::flag("HERONITE_DEBUG_PLAY_EDIT"),
            };
            let (exists, detected) = detect(&exe);
            state.exists = exists;
            state.detected = detected;
            state
        })
    }

    pub(super) fn refresh_launch(&mut self, exe: &str) {
        let (exists, detected) = detect(exe);
        let mut meta = self.game.meta.as_deref().cloned();
        let run_as = self.launch.as_ref().map(|l| l.run_as.clone()).unwrap_or_else(|| "auto".into());
        if let Some(m) = meta.as_mut() {
            m.run_as = Some(run_as.clone());
        } else {
            let m = LocalGameMeta { run_as: Some(run_as.clone()), ..Default::default() };
            meta = Some(m);
        }
        let effective = effective_platform(&self.game.game, meta.as_ref(), exe, &SystemProbe);
        if let Some(l) = self.launch.as_mut() {
            l.exists = exists;
            l.detected = detected;
            l.effective = effective;
        }
    }

    pub(super) fn browse_executable(&mut self, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update_in(cx, |this, window, cx| {
                if let Some(l) = this.launch.as_ref() {
                    let text = path.to_string_lossy().to_string();
                    l.executable.update(cx, |s, cx| s.set_value(text, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    /// Picks the game folder (command mode).
    pub(super) fn browse_folder(&mut self, cx: &mut Context<Self>) {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(tr!("Select").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update_in(cx, |this, window, cx| {
                if let Some(l) = this.launch.as_ref() {
                    let text = path.to_string_lossy().to_string();
                    l.folder.update(cx, |s, cx| s.set_value(text, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn set_launch_by_command(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(l) = self.launch.as_mut() {
            l.by_command = on;
            l.saved = false;
        }
        self.error.clear();
        cx.notify();
    }

    pub(super) fn save_launch(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.launch.as_ref() else { return };
        if !l.can_save(cx) {
            return;
        }
        let exe = l.executable.read(cx).value().trim().to_string();
        let run_as = l.run_as.clone();
        let command = l.by_command.then(|| (LaunchState::text(&l.folder, cx), LaunchState::text(&l.command, cx)));
        self.busy = Some(Busy::Launch);
        self.error.clear();
        let (data_paths, game) = (self.ctx.data_paths.clone(), self.game.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match &command {
                        Some((folder, cmd)) => write_command_options(&data_paths, &game, folder, cmd),
                        None => write_launch_options(&data_paths, &game, &exe, &run_as),
                    }
                    .map(|_| (exe, run_as, command))
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok((exe, run_as, command)) => {
                        if let Some(l) = this.launch.as_mut() {
                            l.initial_by_command = command.is_some();
                            match command {
                                Some((folder, cmd)) => {
                                    l.initial_folder = folder;
                                    l.initial_command = cmd;
                                }
                                None => {
                                    l.initial_exe = exe;
                                    l.initial_run_as = run_as;
                                }
                            }
                            l.saved = true;
                            l.open = false;
                        }
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Opens Play's editor (an action's closes).
    pub(super) fn open_play(&mut self, cx: &mut Context<Self>) {
        if let Some(l) = self.launch.as_mut() {
            l.open = true;
        }
        self.actions.editor = None;
        self.actions.deleting = None;
        cx.notify();
    }

    /// Closes Play's editor, back to what is saved.
    pub(super) fn close_play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(l) = self.launch.as_mut().filter(|l| l.open) else { return };
        l.open = false;
        l.run_as_open = false;
        l.run_as = l.initial_run_as.clone();
        l.by_command = l.initial_by_command;
        let exe = l.initial_exe.clone();
        for (input, value) in [(&l.executable, &l.initial_exe), (&l.folder, &l.initial_folder), (&l.command, &l.initial_command)] {
            let value = value.clone();
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.refresh_launch(&exe);
        self.error.clear();
        cx.notify();
    }

    /// Play's editor, under its row: an executable and "run as", or a command.
    pub(super) fn play_editor(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.launch.as_ref() else { return div() };
        let dirty = l.dirty(cx);
        let can_save = l.can_save(cx);
        let busy = self.busy.is_some();
        let by_command = l.by_command;
        let weak = cx.entity().downgrade();
        let start = labeled(
            tr!("Start with"),
            crate::ui::controls::tabs(
                "gs-start",
                &[(false, tr!("An executable")), (true, tr!("A command"))],
                by_command,
                move |on, _, cx| {
                    weak.update(cx, |this, cx| this.set_launch_by_command(on, cx)).ok();
                },
            ),
        );
        let mut col = div().flex().flex_col().gap(px(14.)).pt(px(2.)).pb(px(14.)).child(start);
        if by_command {
            let problem = l.command_problem(cx);
            col = col
                .child(labeled(
                    tr!("Game folder"),
                    div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(input_box(&l.folder, false))).child(
                        action_button(
                            "gs-browse-folder",
                            ButtonKind::Glass,
                            tr!("Browse"),
                            !busy,
                            cx.listener(|this, _, _, cx| this.browse_folder(cx)),
                        ),
                    ),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(labeled(tr!("Command"), input_box(&l.command, false)))
                        .child(help_text(tr!(
                            "Runs inside the game folder, like a terminal opened there. Example: ./start.sh --fullscreen"
                        )))
                        .children(problem.map(|p| div().text_size(px(13.)).text_color(theme::danger_text()).child(p))),
                );
        } else {
            let detected = match l.detected {
                ExecutableKind::Linux => tr!("Linux program (ELF, script or AppImage)"),
                ExecutableKind::Windows => tr!("Windows program"),
                ExecutableKind::Unknown => tr!("Unknown file type"),
            };
            let effective = if l.effective == Platform::Linux { tr!("Runs natively on Linux") } else { tr!("Runs through Wine/Proton") };
            let run_as =
                [("auto", tr!("Automatic (from the file)")), ("linux", tr!("Linux native")), ("windows", tr!("Windows (Wine/Proton)"))];
            let current_label = run_as.iter().find(|(v, _)| *v == l.run_as).map(|(_, t)| *t).unwrap_or_default();
            let options: Vec<(SharedString, bool)> = run_as.iter().map(|(v, t)| ((*t).into(), *v == l.run_as)).collect();
            let select = select_field(
                "gs-runas",
                Some(tr!("Run as")),
                current_label,
                false,
                l.run_as_open,
                options,
                cx,
                |this, _, cx| {
                    if let Some(l) = this.launch.as_mut() {
                        l.run_as_open = !l.run_as_open;
                    }
                    cx.notify();
                },
                move |this, i, _, cx| {
                    let exe = this.launch.as_ref().map(|l| l.executable.read(cx).value().to_string()).unwrap_or_default();
                    if let Some(l) = this.launch.as_mut() {
                        l.run_as = run_as[i].0.to_string();
                        l.run_as_open = false;
                        l.saved = false;
                    }
                    this.refresh_launch(&exe);
                    cx.notify();
                },
            );
            // What the file is and how it will run, under "Run as".
            let found = div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(theme::text_dim())
                .child(format!("{detected} \u{b7} {effective}"))
                .when(!l.exists, |d| d.child(div().text_color(theme::danger_text()).child(tr!(" (file not found)"))));
            col = col
                .child(labeled(
                    tr!("Executable"),
                    div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(input_box(&l.executable, false))).child(
                        action_button(
                            "gs-browse",
                            ButtonKind::Glass,
                            tr!("Browse"),
                            !busy,
                            cx.listener(|this, _, _, cx| this.browse_executable(cx)),
                        ),
                    ),
                ))
                .child(div().flex().flex_col().gap(px(6.)).child(select).child(found));
        }
        col.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .pt(px(2.))
                .child(action_button(
                    "gs-launch-cancel",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    !busy,
                    cx.listener(|this, _, window, cx| this.close_play(window, cx)),
                ))
                .child(action_button(
                    "gs-launch-save",
                    ButtonKind::Primary,
                    if self.busy == Some(Busy::Launch) { tr!("Saving…") } else { tr!("Save") },
                    !busy && dirty && can_save,
                    cx.listener(|this, _, _, cx| this.save_launch(cx)),
                )),
        )
    }
}

/// A Linux game started by a command: the folder and command in the sideload
/// entry (it becomes Linux native), the overlay meta an executable game.
fn write_command_options(paths: &DataPaths, game: &heronite_data::CollectionGame, folder: &str, command: &str) -> anyhow::Result<()> {
    if let Err(why) = heronite_launch::native::check_command(command, folder, &SystemProbe) {
        anyhow::bail!(why);
    }
    let folder = folder.trim().trim_end_matches('/').to_string();
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
        if let Some(e) = f.games.get_mut(&game.app_name) {
            e.launch_kind = "executable".into();
            e.run_as = Some("linux".into());
        }
        Ok(())
    })?;
    json_store::update_value(&paths.sideload_library(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(|g| g.as_array_mut()) else { return Ok(()) };
        for g in games {
            if g.get("app_name").and_then(|v| v.as_str()) != Some(game.app_name.as_str()) {
                continue;
            }
            g[heronite_data::sideload::COMMAND_KEY] = serde_json::Value::String(command.trim().to_string());
            g["folder_name"] = serde_json::Value::String(folder.clone());
            g["is_linux_native"] = serde_json::Value::Bool(true);
            if !g["install"].is_object() {
                g["install"] = serde_json::json!({});
            }
            g["install"]["install_path"] = serde_json::Value::String(folder.clone());
            g["install"]["platform"] = serde_json::Value::String("linux".into());
        }
        Ok(())
    })?;
    Ok(())
}

/// Port of `setLocalLaunchOptions`: overlay meta (`runAs`, executable) and the
/// sideload entry (platform, executable, folder). A command the game was
/// started by is dropped.
fn write_launch_options(paths: &DataPaths, game: &heronite_data::CollectionGame, executable: &str, run_as: &str) -> anyhow::Result<()> {
    let exe = {
        let t = executable.trim();
        if t.is_empty() {
            game.game.install.executable.clone().unwrap_or_default()
        } else {
            t.to_string()
        }
    };
    let mut new_meta = game.meta.as_deref().cloned();
    if let Some(m) = new_meta.as_mut() {
        m.run_as = Some(run_as.to_string());
        // An emulated game that now runs a program: drop the emulator's arguments.
        let emulator_args = (!exe.is_empty() && m.launch_kind == "emulator").then(|| m.launcher_args.take().unwrap_or_default());
        if !exe.is_empty() {
            m.remapped_executable = Some(exe.clone());
            m.launch_kind = "executable".into();
        }
        let (ra, re, lk, la) = (m.run_as.clone(), m.remapped_executable.clone(), m.launch_kind.clone(), m.launcher_args.clone());
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            if let Some(e) = f.games.get_mut(&game.app_name) {
                e.run_as = ra;
                e.remapped_executable = re;
                e.launch_kind = lk;
                e.launcher_args = la;
            }
            Ok(())
        })?;
        if let Some(args) = emulator_args {
            heronite_data::game_config::forget_emulator_args(paths, &game.app_name, &args)?;
        }
    }
    let mut info = (*game.game).clone();
    info.install.executable = Some(exe.clone());
    let mut meta_for_platform = new_meta.unwrap_or_default();
    meta_for_platform.run_as = Some(run_as.to_string());
    let platform = effective_platform(&info, Some(&meta_for_platform), &exe, &SystemProbe);
    let linux = platform == Platform::Linux;
    json_store::update_value(&paths.sideload_library(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(|g| g.as_array_mut()) else { return Ok(()) };
        for g in games {
            if g.get("app_name").and_then(|v| v.as_str()) != Some(game.app_name.as_str()) {
                continue;
            }
            g["is_linux_native"] = serde_json::Value::Bool(linux);
            if let Some(obj) = g.as_object_mut() {
                obj.shift_remove(heronite_data::sideload::COMMAND_KEY);
            }
            if !exe.is_empty() {
                if let Some(dir) = Path::new(&exe).parent() {
                    g["folder_name"] = serde_json::Value::String(dir.to_string_lossy().into_owned());
                }
                if !g["install"].is_object() {
                    g["install"] = serde_json::json!({});
                }
                g["install"]["executable"] = serde_json::Value::String(exe.clone());
            }
            if !g["install"].is_object() {
                g["install"] = serde_json::json!({});
            }
            g["install"]["platform"] = serde_json::Value::String(if linux { "linux" } else { "Windows" }.into());
        }
        Ok(())
    })?;
    Ok(())
}
