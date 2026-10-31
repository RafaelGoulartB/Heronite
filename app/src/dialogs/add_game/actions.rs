//! Saving, removing and running an installer.

use std::path::Path;

use futures::channel::oneshot;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config::{self, Settings};
use heronite_data::sideload::{self, PrefixOutcome, SideloadPlatform, SideloadSpec};
use heronite_i18n::tr;
use heronite_launch::sandbox::{self, Sandbox};
use heronite_launch::settings::GameSettings;
use heronite_launch::wine_tools::{build_tool_command, WineTool};
use heronite_launch::{PlanContext, SystemProbe};
use heronite_metadata::edit;
use heronite_metadata::DetailsPatch;
use serde_json::json;

use super::logic::*;
use super::{text_of, AddGameDialog, Mode};
use crate::dialogs::settings_panel::logic::wine_to_value;
use crate::dialogs::DialogEvent;

impl AddGameDialog {
    /// A new Windows or Linux game can start in the sandbox (a game in it
    /// changes it in its settings' Sandbox tab, which only such games have).
    pub(super) fn offers_sandbox(&self) -> bool {
        !self.is_edit()
            && matches!(self.platform, SideloadPlatform::Windows | SideloadPlatform::Linux)
            && heronite_platform::supported(heronite_platform::Feature::Sandbox)
    }

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.confirm_remove || self.confirm_discard {
            return;
        }
        let title = text_of(&self.title, cx).trim().to_string();
        let exe = text_of(&self.exe, cx).trim().to_string();
        let url = normalize_url(&text_of(&self.url, cx));
        let platform = self.platform;
        let folder = text_of(&self.folder, cx).trim().to_string();
        let command = text_of(&self.command, cx).trim().to_string();
        let start = self.start(&exe, &folder, &command);
        let by_command = matches!(start, Start::Command { .. });
        if let Some(msg) = missing_field(&title, platform, start, &url) {
            self.error = msg.to_string();
            cx.notify();
            return;
        }
        // A command that cannot start (a typo in the path) is caught here, not at Play.
        if let Start::Command { folder, command } = start {
            if let Err(why) = heronite_launch::native::check_command(command, folder, &SystemProbe) {
                self.error = why;
                cx.notify();
                return;
            }
        }
        // Edit: set when the title differs from the one the game was showing.
        let title_patch = self
            .is_edit()
            .then(|| self.ctx.lib.games.iter().find(|g| g.app_name == self.app_name && g.runner == "sideload"))
            .flatten()
            .and_then(|g| changed_title(Some(&g.title), &title));
        let (art_cover, art_square) = art_pair(&text_of(&self.square, cx), &text_of(&self.hero, cx));
        let browser = platform == SideloadPlatform::Browser;
        let spec = SideloadSpec {
            app_name: self.app_name.clone(),
            title,
            executable: if browser || by_command { String::new() } else { exe.clone() },
            platform,
            art_cover,
            art_square,
            browser_url: if browser { url } else { String::new() },
            custom_user_agent: if browser { text_of(&self.user_agent, cx).trim().to_string() } else { String::new() },
            launch_full_screen: browser && self.fullscreen,
            run_as: None,
            install: matches!(self.mode, Mode::Install(_)),
            install_folder: if platform == SideloadPlatform::Linux { folder } else { String::new() },
            command: if by_command { command } else { String::new() },
        };
        let new_wine = (!self.is_edit() && platform == SideloadPlatform::Windows && uses_wine())
            .then(|| (self.prefix_path(cx), self.wine.as_ref().map(wine_to_value), self.bottle.clone()));
        let sandbox = (self.offers_sandbox() && self.sandbox).then(|| Sandbox { enabled: true, ..Default::default() }.to_value());
        let (paths, home, existing_run_as, is_edit) =
            (self.ctx.data_paths.clone(), self.home.clone(), self.existing_run_as.clone(), self.is_edit());
        self.saving = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let mut spec = spec;
                    if !browser && !by_command {
                        let detected = detect_platform(&spec.executable, &SystemProbe);
                        spec.run_as = run_as_for(spec.platform, detected, existing_run_as.as_deref());
                    }
                    let out = sideload::save_game(&paths, &spec)?;
                    // The shown title is the Collection's: without this it would hide the new one.
                    if let Some(title) = title_patch {
                        edit::update_collection_game_details(
                            &paths,
                            &DetailsPatch {
                                app_name: spec.app_name.clone(),
                                runner: "sideload".into(),
                                title: Some(title),
                                ..Default::default()
                            },
                        )?;
                    }
                    if let Some((prefix, wine, bottle)) = new_wine {
                        sideload::init_game_config(&paths, &home, &spec.app_name, &prefix, wine, &bottle)?;
                    }
                    if let Some(value) = sandbox {
                        game_config::set_game_settings(&paths, &spec.app_name, &home, &[(sandbox::KEY.to_string(), value)])?;
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
                    Err(err) => this.error = tr!("Could not save the game: {error}", error = format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn remove(&mut self, cx: &mut Context<Self>) {
        let Mode::Edit(app) = self.mode.clone() else { return };
        if self.saving {
            return;
        }
        self.saving = true;
        self.error.clear();
        let (paths, home, with_prefix) = (
            self.ctx.data_paths.clone(),
            self.home.clone(),
            self.remove_prefix_too && self.removable_prefix.is_some() && self.prefix_shared.is_none(),
        );
        let launcher = crate::dialogs::remove_game::launcher_of(&self.ctx.lib, &app);
        let removed = app.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    crate::dialogs::remove_game::remove_from_collection(&paths, "sideload", &app)?;
                    // Only once the game is gone: a failed removal keeps its prefix. Its
                    // path is in the game's settings, which the removal leaves.
                    anyhow::Ok(with_prefix.then(|| sideload::remove_prefix(&paths, &app, &home)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(prefix) => {
                        if let Some(PrefixOutcome::Skipped(why)) = prefix {
                            log::warn!("Wine prefix kept: {why}");
                        }
                        crate::dialogs::remove_game::note_removed(launcher, &removed, cx);
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(err) => this.error = tr!("Could not remove the game: {error}", error = format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Run installer first": runs a setup program inside the chosen prefix
    /// with the chosen Wine, before the game exists.
    pub(super) fn run_installer(&mut self, exe: String, cx: &mut Context<Self>) {
        if self.installer_running {
            return;
        }
        self.installer_running = true;
        self.notice.clear();
        self.error.clear();
        let mut map = game_config::load_game(&self.ctx.data_paths, &self.app_name, &self.home);
        map.insert("winePrefix".into(), json!(self.prefix_path(cx)));
        if let Some(w) = &self.wine {
            map.insert("wineVersion".into(), wine_to_value(w));
        }
        if self.sandbox {
            map.insert(sandbox::KEY.into(), Sandbox { enabled: true, ..Default::default() }.to_value());
        }
        let (paths, home) = (self.ctx.data_paths.clone(), self.home.clone());
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_exe_blocking(&paths, &home, &map, &exe));
        });
        cx.spawn(async move |this, cx| {
            let res = rx.await.unwrap_or_else(|_| Err(tr!("The installer was interrupted").into()));
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
}

/// Adding a game also creates its shortcuts when Heroic's settings ask for
/// them (`addDesktopShortcuts`, `addStartMenuShortcuts`, `addSteamShortcuts`).
fn after_added(paths: &heronite_data::DataPaths, spec: &SideloadSpec) {
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
fn run_exe_blocking(paths: &heronite_data::DataPaths, home: &str, map: &Settings, exe: &str) -> Result<String, String> {
    let settings = GameSettings::from_map(map, home);
    let ctx = PlanContext::load(paths);
    // A new prefix is created first so shared folders get their drive letters.
    heronite_launch::shared_folders::create_prefix_first(&ctx, &SystemProbe, &settings)?;
    let built = build_tool_command(&ctx, &SystemProbe, &settings, &WineTool::RunExe(exe.to_string()))?;
    let c = &built.command;
    let mut cmd = std::process::Command::new(&c.program);
    cmd.args(&c.args)
        .envs(&c.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    heronite_launch::wine::drop_host_certs(c, &mut cmd);
    if let Some(dir) = c.cwd.as_deref().filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| tr!("Could not start {program}: {error}", program = c.program, error = e))?;
    let status = child.wait().map_err(|e| format!("{}: {e}", c.program))?;
    let mut text =
        if status.success() { tr!("The program finished.").to_string() } else { tr!("The program exited with {status}.", status) };
    for n in built.notes {
        text.push_str(&format!(" {n}."));
    }
    Ok(text)
}
