//! "Uninstall" of a game added by hand (sideload with an executable): asks
//! which of its install folder and its own Wine/Proton prefix to delete, then
//! does it off the UI thread. The game stays in the Collection, not
//! installed. The checks live in `heronite_data::sideload_uninstall`.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload_uninstall::{guess_game_folder, plan, uninstall, UninstallInput, UninstallPlan};
use heronite_data::{CollectionGame, HeroicPaths, Library};
use heronite_launch::native::{effective_platform, Platform};
use heronite_launch::SystemProbe;

use super::CollectionView;
use crate::dialogs::epic_confirm::{Confirm, EpicConfirmDialog, Toggle};

/// What the uninstall needs, read from the library on the UI thread.
#[derive(Clone)]
struct Job {
    app_name: String,
    runner: String,
    title: String,
    home: String,
    folder: Option<PathBuf>,
    windows: bool,
    others: Vec<PathBuf>,
}

impl Job {
    fn input(&self) -> UninstallInput<'_> {
        UninstallInput {
            app_name: &self.app_name,
            home: &self.home,
            folder: self.folder.as_deref(),
            windows: self.windows,
            others: &self.others,
        }
    }
}

/// A sideloaded program installed from files on this computer (not a Steam,
/// emulated or browser entry).
pub(super) fn can_uninstall_local(game: &CollectionGame) -> bool {
    game.runner == "sideload"
        && game.installed
        && game.steam_launch_id.is_none()
        && !matches!(game.launch_kind.as_deref(), Some("steam-uri" | "emulator"))
        && !game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser"))
        && executable(game).is_some()
}

fn executable(game: &CollectionGame) -> Option<String> {
    game.meta
        .as_ref()
        .and_then(|m| m.remapped_executable.clone())
        .or_else(|| game.game.install.executable.clone())
        .filter(|e| Path::new(e.trim()).is_absolute())
}

/// The folders and executables of every other game (a folder holding one of
/// them is never deleted).
fn other_paths(lib: &Library, app_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for g in lib.games.iter().filter(|g| g.app_name != app_name) {
        let raw = [
            g.details.install_path.as_deref(),
            g.game.install.install_path.as_deref(),
            g.game.folder_name.as_deref(),
            g.game.install.executable.as_deref(),
            g.meta.as_ref().and_then(|m| m.remapped_executable.as_deref()),
        ];
        out.extend(raw.into_iter().flatten().map(str::trim).filter(|p| Path::new(p).is_absolute()).map(PathBuf::from));
    }
    out
}

fn job_for(lib: &Library, game: &CollectionGame) -> Job {
    let exe = executable(game).unwrap_or_default();
    let folder = guess_game_folder(Path::new(exe.trim()));
    let windows = effective_platform(&game.game, game.meta.as_deref(), &exe, &SystemProbe) == Platform::Windows;
    Job {
        app_name: game.app_name.clone(),
        runner: game.runner.clone(),
        title: game.title.clone(),
        home: std::env::var("HOME").unwrap_or_default(),
        folder,
        windows,
        others: other_paths(lib, &game.app_name),
    }
}

fn confirm_spec(job: &Job, plan: &UninstallPlan) -> Confirm {
    let mut spec = Confirm::new(
        "local-uninstall",
        "Uninstall",
        format!(
            "Do you want to uninstall \"{}\"?\nIt stays in Collection with its playtime and status, marked as not installed.",
            job.title
        ),
    );
    match &plan.files {
        Some(t) => {
            spec.toggles.push(Toggle {
                label: "Delete the game files".into(),
                value: t.removable(),
                disabled: false,
                note: Some(match &t.blocked {
                    Some(why) => why.clone(),
                    None => "Everything in the folder below is deleted. This can't be undone.".into(),
                }),
            });
            spec.path = Some(("Game folder".into(), t.path.display().to_string(), "Choose the game folder".into()));
        }
        None => spec.message.push_str("\nThe game folder was not found on this computer."),
    }
    if let Some(t) = &plan.prefix {
        spec.toggles.push(Toggle {
            label: format!("Remove prefix: {}", t.path.display()),
            value: t.removable(),
            disabled: !t.removable(),
            note: Some(match &t.blocked {
                Some(why) => why.clone(),
                None => "Only this game uses it. Save files that are not backed up are lost.".into(),
            }),
        });
    }
    spec.confirm = Some("Uninstall".into());
    spec.danger = true;
    spec
}

impl CollectionView {
    pub(super) fn sideload_uninstall_item(&mut self, menu: Div, game: &CollectionGame, tracking: bool, cx: &mut Context<Self>) -> Div {
        if tracking || !can_uninstall_local(game) {
            return menu;
        }
        let k = game.key.clone();
        menu.child(
            super::context_menu::item("cm-local-uninstall")
                .child(crate::ui::icon(crate::icons::Icon::DeleteForever, px(18.), crate::theme::text_muted()))
                .child("Uninstall")
                .on_click(cx.listener(move |this, _, window, cx| this.open_sideload_uninstall(&k, window, cx))),
        )
    }

    /// Checks what can be deleted (off the UI thread), then asks.
    pub(super) fn open_sideload_uninstall(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key).cloned() else { return };
        if self.playing.contains_key(&game.app_name) {
            self.show_toast(format!("{} is running. Close the game to uninstall it.", game.title), true, cx);
            return;
        }
        let job = job_for(&lib, &game);
        let paths = self.heroic.clone();
        cx.spawn_in(window, async move |this, cx| {
            let (job, plan) = cx
                .background_spawn(async move {
                    let p = plan(&paths, &job.input());
                    (job, p)
                })
                .await;
            this.update_in(cx, |this, window, cx| this.ask_sideload_uninstall(job, plan, window, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn ask_sideload_uninstall(&mut self, job: Job, plan: UninstallPlan, window: &mut Window, cx: &mut Context<Self>) {
        let spec = confirm_spec(&job, &plan);
        let has_files = plan.files.is_some();
        let has_prefix = plan.prefix.is_some();
        let weak = cx.entity().downgrade();
        let view = cx.new(|cx| {
            EpicConfirmDialog::new(
                spec,
                move |choice, cx| {
                    let mut toggles = choice.toggles.iter().copied();
                    let delete_files = has_files && toggles.next().unwrap_or(false);
                    let delete_prefix = has_prefix && toggles.next().unwrap_or(false);
                    let mut job = job.clone();
                    if has_files {
                        job.folder = Some(PathBuf::from(choice.path.trim()));
                    }
                    weak.update(cx, |this, cx| this.run_sideload_uninstall(job, delete_files, delete_prefix, cx)).ok();
                },
                window,
                cx,
            )
        });
        self.open_dialog(view, cx);
    }

    fn run_sideload_uninstall(&mut self, job: Job, delete_files: bool, delete_prefix: bool, cx: &mut Context<Self>) {
        let paths: HeroicPaths = self.heroic.clone();
        let title = job.title.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let out = uninstall(&paths, &job.input(), delete_files, delete_prefix).map_err(|e| format!("{e:#}"))?;
                    // Shortcuts to a game that is gone only break; failures only log.
                    let shortcuts = crate::system::shortcuts::remove_shortcuts(&paths, &job.runner, &job.app_name);
                    log::info!("shortcuts of {}: {shortcuts:?}", job.app_name);
                    if crate::system::shortcuts::is_added_to_steam(&paths, &job.runner, &job.app_name) {
                        let steam = crate::system::shortcuts::remove_from_steam(&paths, &job.runner, &job.app_name);
                        log::info!("steam shortcut of {}: {steam:?}", job.app_name);
                    }
                    Ok::<_, String>(out)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(out) => {
                        log::info!("uninstalled {title}: {out:?}");
                        this.show_toast(format!("{title} was uninstalled"), false, cx);
                    }
                    Err(err) => this.show_toast(format!("Could not uninstall {title}: {err}"), true, cx),
                }
                this.reload(cx);
            })
            .ok();
        })
        .detach();
    }
}
