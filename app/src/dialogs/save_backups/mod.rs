//! The "Backups" tab of a game's settings: its Ludusavi backups in each
//! backup folder, and restoring one into the game's prefix as it is now
//! (`heronite_emulation::ludusavi::restore`), with an undo.
//!
//! Layout: this file holds the state and the background work, `render` the
//! desktop view and `console` the console mode rows.

mod console;
mod render;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::DataPaths;
use heronite_emulation::ludusavi::restore::{
    apply_restore, delete_backup, list_backups, plan_restore, set_locked, set_name, steam_game_running, undo, wine_user, FileKind,
    GameBackups, RestoreOutcome, RestorePlan, RestoreRequest, RestoreTarget, Tool,
};
use heronite_emulation::ludusavi::{backup_for_game, game_wine_prefix, BackupReason, LudusaviEnv, LudusaviGame, SystemRunner};
use heronite_i18n::tr;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

use super::ludusavi_backup::{describe, Outcome};
use super::settings_panel::logic::{GameFacts, RunKind};

/// A restore copies the whole backup; give big ones time.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(600);

/// What the tab knows once loaded.
pub(crate) struct Loaded {
    tool: Tool,
    target: RestoreTarget,
    /// One entry per backup folder, in the order of Settings → Backups.
    sources: Vec<GameBackups>,
    /// The copy of what the last restore replaced.
    undo: Option<undo::UndoRecord>,
}

enum Load {
    Idle,
    Loading,
    Failed(String),
    Ready(Box<Loaded>),
}

/// Where the restore is.
enum Stage {
    List,
    Planning,
    /// Preview: the user picks the files.
    Plan(Box<RestorePlan>),
    Restoring,
    Done(RestoreOutcome),
}

pub struct SaveBackupsView {
    paths: DataPaths,
    game: LudusaviGame,
    run_kind: RunKind,
    /// Not installed: backups can be listed, locked and taken, and restored
    /// only while a prefix of the game is still on this PC.
    installed: bool,
    steam_app_id: Option<String>,
    install_dir: Option<PathBuf>,
    load: Load,
    /// A folder picked for this game only (desktop "Other folder…").
    picked: Option<PathBuf>,
    /// Index into `Loaded::sources`.
    source: usize,
    source_open: bool,
    /// Index into the backups of the chosen source.
    backup: usize,
    stage: Stage,
    /// Background job besides the restore (backup, lock, undo).
    working: bool,
    notice: Option<(String, bool)>,
    /// Desktop: the backup being named and its text field.
    renaming: Option<(usize, Entity<InputState>, Subscription)>,
    /// The backup asking "delete?".
    confirm_delete: Option<usize>,
    /// Console mode: the backup whose actions are listed.
    console_backup: Option<usize>,
    console_list: crate::console::list::ListNav,
    /// Console mode: the on-screen keyboard for the name.
    layer: crate::console::Layer,
}

impl EventEmitter<()> for SaveBackupsView {}

/// How the game runs, for its saves: installed games as their settings
/// say, others from their store data. `None` (no tab) for emulated ROMs,
/// browser games and macOS builds.
fn save_kind(game: &Game) -> Option<RunKind> {
    let g = &game.game;
    let launch_kind = game.launch_kind.as_deref().unwrap_or("");
    let platform = g.install.platform.clone().unwrap_or_default().to_ascii_lowercase();
    if launch_kind == "emulator" || platform == "browser" || g.rest.get("browserUrl").is_some_and(|v| !v.is_null() && v != "") {
        return None;
    }
    if game.installed {
        return Some(GameFacts::of(game).run_kind).filter(|k| *k != RunKind::None);
    }
    Some(if game.steam_launch_id.as_deref().is_some_and(|s| !s.is_empty()) {
        RunKind::Steam
    } else if platform.contains("linux") || (platform.is_empty() && g.is_linux_native == Some(true)) {
        RunKind::Native
    } else if platform.contains("mac") || platform == "osx" {
        return None;
    } else {
        RunKind::Wine
    })
}

/// Whether a game gets the tab (installed or not).
pub fn has_tab(game: &Game) -> bool {
    save_kind(game).is_some()
}

impl SaveBackupsView {
    pub fn new(paths: DataPaths, game: &Game) -> Self {
        let run_kind = save_kind(game).unwrap_or(RunKind::None);
        let steam_app_id = if run_kind == RunKind::Steam { game.steam_launch_id.clone().filter(|s| !s.is_empty()) } else { None };
        let install_dir = game.game.install.install_path.as_deref().filter(|p| !p.is_empty()).map(PathBuf::from);
        Self {
            paths,
            game: LudusaviGame {
                app_name: game.app_name.clone(),
                title: game.title.clone(),
                runner: game.runner.clone(),
                steam_app_id: game.steam_launch_id.clone(),
                store_game_id: game.meta.as_ref().and_then(|m| m.store_game_id.clone()),
            },
            run_kind,
            installed: game.installed,
            steam_app_id,
            install_dir,
            load: Load::Idle,
            picked: None,
            source: 0,
            source_open: false,
            backup: 0,
            stage: Stage::List,
            working: false,
            notice: None,
            renaming: None,
            confirm_delete: None,
            console_backup: None,
            console_list: Default::default(),
            layer: Default::default(),
        }
    }

    /// The tab became visible: reads the backups the first time.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        if matches!(self.load, Load::Idle) {
            self.reload(cx);
        }
    }

    fn loaded(&self) -> Option<&Loaded> {
        match &self.load {
            Load::Ready(l) => Some(l),
            _ => None,
        }
    }

    fn busy(&self) -> bool {
        self.working || matches!(self.stage, Stage::Planning | Stage::Restoring)
    }

    /// Runs `job` with a Ludusavi environment off the UI thread.
    fn background<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        timeout: Duration,
        job: impl FnOnce(&LudusaviEnv) -> R + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let runner = SystemRunner { timeout };
                    let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &SystemProbe };
                    job(&env)
                })
                .await;
            this.update(cx, |v, cx| {
                done(v, result, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads Ludusavi, the prefix and the backups again.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        let (game, kind, steam, install, picked, installed) =
            (self.game.clone(), self.run_kind, self.steam_app_id.clone(), self.install_dir.clone(), self.picked.clone(), self.installed);
        cx.notify();
        self.background(
            cx,
            heronite_emulation::ludusavi::LUDUSAVI_TIMEOUT,
            move |env| -> Result<Loaded, String> {
                let tool = Tool::resolve(env)?;
                let target = resolve_target(env, kind, steam.as_deref(), &game.app_name, install, installed);
                let sources = list_backups(env, &tool, &game, picked.as_deref());
                let undo = undo::latest(&env.paths.save_restore_undo(), &game.app_name);
                Ok(Loaded { tool, target, sources, undo })
            },
            |v, result, cx| {
                v.load = match result {
                    Ok(l) => {
                        // Keep the folder chosen; else the first one with backups.
                        let keep = v.source < l.sources.len() && !l.sources[v.source].backups.is_empty();
                        if !keep {
                            v.source = l.sources.iter().position(|s| !s.backups.is_empty()).unwrap_or(0);
                        }
                        if let Some(p) = &v.picked {
                            v.source = l.sources.iter().position(|s| &s.source.root == p).unwrap_or(v.source);
                        }
                        let count = l.sources.get(v.source).map(|s| s.backups.len()).unwrap_or(0);
                        v.backup = v.backup.min(count.saturating_sub(1));
                        Load::Ready(Box::new(l))
                    }
                    Err(e) => Load::Failed(e),
                };
                v.apply_debug_env(cx);
            },
        );
    }

    /// `HERONITE_DEBUG_BACKUPS=plan|folder[@<folder index>]`: once loaded,
    /// previews the selected backup (only reads it) or opens the folder menu
    /// (screenshots).
    fn apply_debug_env(&mut self, cx: &mut Context<Self>) {
        let Some(value) = crate::env::value("HERONITE_DEBUG_BACKUPS") else { return };
        let (which, folder) = value.split_once('@').unwrap_or((value.as_str(), ""));
        if let Ok(i) = folder.parse::<usize>() {
            self.source = i;
            self.backup = 0;
        }
        match which {
            "folder" => self.source_open = true,
            "plan" if matches!(self.stage, Stage::List) => self.start_restore(cx),
            _ => {}
        }
    }

    pub(crate) fn pick_source(&mut self, index: usize, cx: &mut Context<Self>) {
        self.source = index;
        self.backup = 0;
        self.source_open = false;
        self.stage = Stage::List;
        cx.notify();
    }

    /// A folder for this game only, then the list again.
    pub(crate) fn use_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.picked = Some(PathBuf::from(path));
        self.source_open = false;
        self.reload(cx);
    }

    pub(crate) fn pick_backup(&mut self, index: usize, cx: &mut Context<Self>) {
        self.backup = index;
        cx.notify();
    }

    fn running_now(&self, cx: &App) -> bool {
        crate::services::session::is_running(cx, &self.game.app_name)
    }

    /// Previews the restore of the selected backup.
    pub(crate) fn start_restore(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        if self.running_now(cx) {
            self.notice = Some((tr!("Close the game before restoring a save.").into(), true));
            cx.notify();
            return;
        }
        let Some(l) = self.loaded() else { return };
        let Some(src) = l.sources.get(self.source) else { return };
        let (Some(name), Some(backup)) = (src.game_name.clone(), src.backups.get(self.backup)) else { return };
        let request = RestoreRequest {
            source_root: src.source.root.clone(),
            game_name: name,
            backup_id: backup.id.clone(),
            target: l.target.clone(),
        };
        let tool = l.tool.clone();
        self.stage = Stage::Planning;
        self.notice = None;
        cx.notify();
        self.background(
            cx,
            RESTORE_TIMEOUT,
            move |env| plan_restore(env, &tool, request),
            |v, result, _| match result {
                Ok(plan) => v.stage = Stage::Plan(Box::new(plan)),
                Err(e) => {
                    v.stage = Stage::List;
                    v.notice = Some((e, true));
                }
            },
        );
        self.console_list.reset();
    }

    /// Picks or leaves out every file of one kind (the preview's groups).
    pub(crate) fn toggle_kind(&mut self, kind: FileKind, cx: &mut Context<Self>) {
        if kind == FileKind::Unmapped {
            return;
        }
        if let Stage::Plan(plan) = &mut self.stage {
            let on = !plan.files.iter().any(|f| f.kind == kind && f.selected);
            for f in plan.files.iter_mut().filter(|f| f.kind == kind) {
                f.selected = on;
            }
        }
        cx.notify();
    }

    pub(crate) fn back_to_list(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.stage, Stage::Planning | Stage::Restoring) {
            self.stage = Stage::List;
            self.console_list.reset();
            cx.notify();
        }
    }

    /// Restores the files picked in the preview.
    pub(crate) fn confirm_restore(&mut self, cx: &mut Context<Self>) {
        let Stage::Plan(plan) = &self.stage else { return };
        if self.running_now(cx) {
            self.notice = Some((tr!("Close the game before restoring a save.").into(), true));
            cx.notify();
            return;
        }
        let Some(tool) = self.loaded().map(|l| l.tool.clone()) else { return };
        let plan = (**plan).clone();
        let back = plan.clone();
        let app_name = self.game.app_name.clone();
        let steam = self.steam_app_id.clone();
        self.stage = Stage::Restoring;
        self.notice = None;
        cx.notify();
        self.background(
            cx,
            RESTORE_TIMEOUT,
            move |env| {
                if steam.as_deref().is_some_and(steam_game_running) {
                    return Err(tr!("Close the game before restoring a save.").to_string());
                }
                apply_restore(env, &tool, &plan, &env.paths.save_restore_undo(), &app_name)
            },
            move |v, result, _| match result {
                Ok(outcome) => {
                    if let (Some(record), Load::Ready(l)) = (&outcome.undo, &mut v.load) {
                        l.undo = Some(record.clone());
                    }
                    v.stage = Stage::Done(outcome);
                }
                Err(e) => {
                    log::error!("Ludusavi restore failed: {e}");
                    v.notice = Some((e, true));
                    v.stage = Stage::Plan(Box::new(back));
                }
            },
        );
        self.console_list.reset();
    }

    /// Puts back what the last restore replaced.
    pub(crate) fn undo_last(&mut self, cx: &mut Context<Self>) {
        let Some(record) = self.loaded().and_then(|l| l.undo.clone()) else { return };
        if self.busy() {
            return;
        }
        if self.running_now(cx) {
            self.notice = Some((tr!("Close the game before restoring a save.").into(), true));
            cx.notify();
            return;
        }
        self.working = true;
        cx.notify();
        self.background(
            cx,
            RESTORE_TIMEOUT,
            move |_| undo::undo(&record).map_err(|e| format!("{e:#}")),
            |v, result, _| {
                v.working = false;
                v.notice = Some(match result {
                    Ok(n) => {
                        (heronite_i18n::trn!(n, "Undone: {n} file is back as it was.", "Undone: {n} files are back as they were."), false)
                    }
                    Err(e) => (e, true),
                });
                if let Load::Ready(l) = &mut v.load {
                    l.undo = undo::latest(&v.paths.save_restore_undo(), &v.game.app_name);
                }
                if matches!(v.stage, Stage::Done(_)) {
                    v.stage = Stage::List;
                }
            },
        );
    }

    /// Locks or unlocks the selected backup.
    pub(crate) fn toggle_lock(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(l) = self.loaded() else { return };
        let Some(src) = l.sources.get(self.source) else { return };
        let (Some(name), Some(b)) = (src.game_name.clone(), src.backups.get(index)) else { return };
        let (tool, root, id, lock) = (l.tool.clone(), src.source.root.clone(), b.id.clone(), !b.locked);
        self.working = true;
        cx.notify();
        self.background(
            cx,
            heronite_emulation::ludusavi::LUDUSAVI_TIMEOUT,
            move |env| set_locked(env, &tool, &root, &name, &id, lock),
            move |v, result, cx| {
                v.working = false;
                match result {
                    Ok(()) => {
                        if let Load::Ready(l) = &mut v.load {
                            if let Some(b) = l.sources.get_mut(v.source).and_then(|s| s.backups.get_mut(index)) {
                                b.locked = lock;
                            }
                        }
                    }
                    Err(e) => v.notice = Some((e, true)),
                }
                cx.notify();
            },
        );
    }

    /// The selected folder's game folder, game name and the backup `index`.
    fn backup_ref(&self, index: usize) -> Option<(Tool, PathBuf, Option<PathBuf>, String, String)> {
        let l = self.loaded()?;
        let src = l.sources.get(self.source)?;
        let b = src.backups.get(index)?;
        Some((l.tool.clone(), src.source.root.clone(), src.dir.clone(), src.game_name.clone()?, b.id.clone()))
    }

    /// Desktop: shows a text field for the backup's name.
    pub(crate) fn start_rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.loaded().and_then(|l| l.sources.get(self.source)?.backups.get(index)?.comment.clone()).unwrap_or_default();
        let input = crate::ui::fields::new_input(window, cx, &current, tr!("Name this backup (\u{201c}Before the last boss\u{201d})"));
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, |v, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                v.save_rename(cx);
            }
        });
        self.renaming = Some((index, input, sub));
        self.confirm_delete = None;
        cx.notify();
    }

    pub(crate) fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming = None;
        cx.notify();
    }

    pub(crate) fn save_rename(&mut self, cx: &mut Context<Self>) {
        let Some((index, input, _)) = self.renaming.take() else { return };
        let name = input.read(cx).value().trim().to_string();
        self.set_backup_name(index, name, cx);
    }

    /// Saves a backup's name (Ludusavi's comment); empty clears it.
    pub(crate) fn set_backup_name(&mut self, index: usize, name: String, cx: &mut Context<Self>) {
        let Some((tool, root, _, game, id)) = self.backup_ref(index).filter(|_| !self.busy()) else { return };
        self.working = true;
        cx.notify();
        let saved = name.clone();
        self.background(
            cx,
            heronite_emulation::ludusavi::LUDUSAVI_TIMEOUT,
            move |env| set_name(env, &tool, &root, &game, &id, &name),
            move |v, result, _| {
                v.working = false;
                match result {
                    Ok(()) => {
                        if let Load::Ready(l) = &mut v.load {
                            if let Some(b) = l.sources.get_mut(v.source).and_then(|s| s.backups.get_mut(index)) {
                                b.comment = Some(saved).filter(|s| !s.is_empty());
                            }
                        }
                    }
                    Err(e) => v.notice = Some((e, true)),
                }
            },
        );
    }

    pub(crate) fn ask_delete(&mut self, index: usize, cx: &mut Context<Self>) {
        self.confirm_delete = Some(index);
        self.renaming = None;
        cx.notify();
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        cx.notify();
    }

    /// Deletes the backup asked about, then reads the list again.
    pub(crate) fn delete_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.confirm_delete.take() else { return };
        let Some((_, _, Some(dir), _, id)) = self.backup_ref(index).filter(|_| !self.busy()) else { return };
        self.working = true;
        self.console_backup = None;
        cx.notify();
        self.background(
            cx,
            heronite_emulation::ludusavi::LUDUSAVI_TIMEOUT,
            move |_| delete_backup(&dir, &id),
            |v, result, cx| {
                v.working = false;
                match result {
                    Ok(()) => {
                        v.notice = Some((tr!("Backup deleted.").into(), false));
                        v.reload(cx);
                    }
                    Err(e) => v.notice = Some((e, true)),
                }
            },
        );
    }

    /// Backs the saves up now (the same as the card's menu item).
    pub(crate) fn backup_now(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let game = self.game.clone();
        let title = game.title.clone();
        self.working = true;
        self.notice = None;
        cx.notify();
        self.background(
            cx,
            heronite_emulation::ludusavi::LUDUSAVI_TIMEOUT,
            move |env| backup_for_game(env, &game, BackupReason::Manual),
            move |v, result, cx| {
                v.working = false;
                v.notice = Some(match describe(&result, &title) {
                    Outcome::Done { message, .. } | Outcome::Message(message) => (message, false),
                    Outcome::Error(e) => (e, true),
                    Outcome::Running => (String::new(), false),
                });
                if result.ran {
                    v.backup = 0;
                    v.reload(cx);
                }
            },
        );
    }
}

/// The prefix and folders the game writes saves to on this PC.
fn resolve_target(
    env: &LudusaviEnv,
    kind: RunKind,
    steam: Option<&str>,
    app_name: &str,
    install: Option<PathBuf>,
    installed: bool,
) -> RestoreTarget {
    let login = std::env::var("USER").ok();
    match (kind, steam) {
        (RunKind::Steam, Some(id)) => {
            let locator = SteamLocator::detect(&GlobalSettings::load(env.paths), &SystemProbe);
            let prefix =
                heronite_steam::files::prefix(&locator, id).map(|p| p.compatdata.join("pfx")).filter(|p| p.join("drive_c").is_dir());
            let user = prefix.as_deref().and_then(|p| wine_user(p, login.as_deref())).or_else(|| Some("steamuser".into()));
            let userdata = locator.steam_path.join("userdata");
            RestoreTarget {
                // An installed Steam game without a compatdata prefix is
                // native (or has never run): its Linux paths stay where they
                // are. Uninstalled, it may be either, so it needs the prefix.
                needs_prefix: prefix.is_some() || !installed,
                user: prefix.as_ref().and(user),
                prefix,
                install_dir: locator.install_dir(id).or(install),
                steam_userdata: userdata.is_dir().then_some(userdata),
            }
        }
        (RunKind::Wine, _) => {
            let prefix = game_wine_prefix(env, app_name).map(PathBuf::from);
            let user = prefix.as_deref().and_then(|p| wine_user(p, login.as_deref()));
            RestoreTarget { prefix, user, install_dir: install, steam_userdata: None, needs_prefix: true }
        }
        _ => RestoreTarget { install_dir: install, ..Default::default() },
    }
}

impl Render for SaveBackupsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.render_desktop(window, cx)
    }
}
