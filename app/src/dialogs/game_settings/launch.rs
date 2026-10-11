//! The Launch tab: executable and "run as" for local (sideload) games.

use std::path::Path;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::{LocalGameMeta, LocalMetaFile};
use heronite_data::{json_store, DataPaths};
use heronite_launch::native::{detect_executable_kind, effective_platform, ExecutableKind, Platform};
use heronite_launch::SystemProbe;

use super::{block, Busy, GameSettingsDialog};
use crate::dialogs::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, help_text, row, ButtonKind};
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
}

pub(super) fn detect(path: &str) -> (bool, ExecutableKind) {
    (!path.is_empty() && Path::new(path).exists(), detect_executable_kind(path, &SystemProbe))
}

impl GameSettingsDialog {
    pub(super) fn build_launch(game: &Game, window: &mut Window, cx: &mut Context<Self>) -> Option<LaunchState> {
        let show_launch = game.runner == "sideload" && !matches!(game.launch_kind.as_deref(), Some("emulator") | Some("steam-uri"));
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
            let mut state = LaunchState {
                executable: input,
                run_as: run_as.clone(),
                run_as_open: crate::env::flag("HERONITE_DEBUG_SELECT_OPEN"),
                initial_exe: exe.clone(),
                initial_run_as: run_as,
                exists: false,
                detected: ExecutableKind::Unknown,
                effective: Platform::Windows,
                saved: false,
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
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Select".into()) });
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

    pub(super) fn save_launch(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.launch.as_ref() else { return };
        let exe = l.executable.read(cx).value().trim().to_string();
        let run_as = l.run_as.clone();
        self.busy = Some(Busy::Launch);
        self.error.clear();
        let (data_paths, game) = (self.ctx.data_paths.clone(), self.game.clone());
        cx.spawn(async move |this, cx| {
            let res =
                cx.background_spawn(async move { write_launch_options(&data_paths, &game, &exe, &run_as).map(|_| (exe, run_as)) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok((exe, run_as)) => {
                        if let Some(l) = this.launch.as_mut() {
                            l.initial_exe = exe;
                            l.initial_run_as = run_as;
                            l.saved = true;
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

    pub(super) fn launch_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(l) = self.launch.as_ref() else { return div() };
        let dirty = l.executable.read(cx).value().as_ref() != l.initial_exe || l.run_as != l.initial_run_as;
        let exe_empty = l.executable.read(cx).value().trim().is_empty();
        let busy = self.busy.is_some();
        let detected = match l.detected {
            ExecutableKind::Linux => "Linux program (ELF, script or AppImage)",
            ExecutableKind::Windows => "Windows program",
            ExecutableKind::Unknown => "Unknown file type",
        };
        let effective = if l.effective == Platform::Linux { "Runs natively on Linux" } else { "Runs through Wine/Proton" };
        let run_as = [("auto", "Automatic (from the file)"), ("linux", "Linux native"), ("windows", "Windows (Wine/Proton)")];
        let current_label = run_as.iter().find(|(v, _)| *v == l.run_as).map(|(_, t)| *t).unwrap_or_default();
        let options: Vec<(SharedString, bool)> = run_as.iter().map(|(v, t)| ((*t).into(), *v == l.run_as)).collect();
        let select = select_field(
            "gs-runas",
            Some("Run as"),
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
        let saved = l.saved && !dirty;
        let detected_value = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .child(detected)
            .when(!l.exists, |d| d.child(div().text_color(theme::danger_text()).child(" (file not found)")));
        let exe_row = block(labeled(
            "Executable",
            div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(input_box(&l.executable, false))).child(
                action_button("gs-browse", ButtonKind::Glass, "Browse", !busy, cx.listener(|this, _, _, cx| this.browse_executable(cx))),
            ),
        ));
        let file = card([
            exe_row,
            row("Detected", None, detected_value).into_any_element(),
            row("Current", None, div().text_size(px(13.)).text_color(theme::text_muted()).child(effective)).into_any_element(),
        ]);
        let how = card([block(select)]);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                help_text(
                    "Linux programs (ELF binaries, shell scripts, AppImages) run directly; Windows programs run through Wine/Proton. Automatic decides from the file.",
                )
                .px(px(6.)),
            )
            .child(caption("Program"))
            .child(file)
            .child(caption("How to run it"))
            .child(how)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pt(px(10.))
                    .child(action_button(
                        "gs-launch-save",
                        ButtonKind::Primary,
                        if self.busy == Some(Busy::Launch) { "Saving\u{2026}" } else { "Save" },
                        !busy && dirty && !exe_empty,
                        cx.listener(|this, _, _, cx| this.save_launch(cx)),
                    ))
                    .when(saved, |d| d.child(div().text_size(px(13.)).text_color(theme::success()).child("Saved"))),
            )
    }
}

/// Port of `setLocalLaunchOptions`: overlay meta (`runAs`, executable) and the
/// sideload entry (platform, executable, folder).
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
