//! "ReShade" in the Performance tab: post-processing effects added to a
//! Windows game (`heronite_launch::reshade`). Everything is read in the
//! background when the tab opens; installs and changes run in the
//! background with their progress shown in the section. No I/O in render.
//!
//! `HERONITE_DEBUG_RESHADE=<step>,<step>…` runs steps one after the other
//! once the section has loaded (each waits for the previous one), for
//! captures without clicks: `preset:<file>`, `install`, `adopt`, `replace`,
//! `off`, `on`, `folder:<dir>`, `repo:<url>`, `remove-source:<name>`,
//! `addon` / `standard` (the build), `renodx` / `remove-renodx`, `renodx-details`, `remove`, `tick` / `accept` (the anti-cheat box), and the views
//! `confirm-anticheat`, `confirm-remove`,
//! `confirm-source:<name>`, `add-repo`, `all-packages`, `installing`.

mod actions;
mod render;
mod render_installed;
mod render_mods;
mod render_setup;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::reshade::install::{Ops, Step};
use heronite_launch::reshade::steam::SteamInstall;
use heronite_launch::reshade::{self, Overview};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

use super::logic::RunKind;
use super::{SettingsPanel, Target};

/// A question asked in the section before doing something.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Confirm {
    /// The anti-cheat warning before installing; `accepted`: its box is ticked.
    AntiCheat {
        accepted: bool,
        replace: bool,
    },
    Remove,
    /// Removing a package or extra source; `used`: effects of it the preset uses.
    RemoveSource {
        name: String,
        used: Vec<String>,
    },
}

/// A long operation running in the background.
pub(super) struct Busy {
    pub(super) line: String,
    /// Steps done and expected (`0` expected: unknown, no bar).
    pub(super) done: usize,
    pub(super) total: usize,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct ReshadeUi {
    pub(super) overview: Option<Arc<Overview>>,
    loading: bool,
    pub(super) busy: Option<Busy>,
    /// The outcome of the last operation (`true`: an error).
    pub(super) message: Option<(String, bool)>,
    /// Packages ticked for the install (`None`: the catalog's defaults).
    pub(super) picked: Option<Vec<String>>,
    pub(super) all_packages: bool,
    /// A preset chosen before installing, and its active effects.
    pub(super) preset: Option<(PathBuf, Vec<String>)>,
    pub(super) confirm: Option<Confirm>,
    pub(super) adding_repo: bool,
    /// The install card is open (instead of the short card).
    pub(super) setup: bool,
    /// The executable and DLL name can be picked in it.
    pub(super) advanced: bool,
    /// The full package list is open in it ("Choose packages…").
    pub(super) packages_open: bool,
    /// The installed shaders are listed ("Manage").
    pub(super) manage_shaders: bool,
    /// The HDR (RenoDX) section is open: the mod, its details and the build
    /// with add-on support (closed: one line).
    pub(super) renodx_details: bool,
    /// Scroll the drawer to the install card on its next draw.
    pub(super) reveal: std::rc::Rc<std::cell::Cell<bool>>,
    /// Scroll the drawer to the whole section, its top first.
    pub(super) reveal_section: std::rc::Rc<std::cell::Cell<bool>>,
    /// Steam installed the game's Linux build: no ReShade to install.
    pub(super) linux_build: bool,
    debug: VecDeque<String>,
}

thread_local! {
    /// The next settings opened go to the ReShade section ([`focus_reshade`]).
    static FOCUS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next game settings scroll to the ReShade section and flash its
/// title (the game menu's "ReShade").
pub fn focus_reshade() {
    FOCUS.with(|f| f.set(true));
}

/// Takes the request of [`focus_reshade`].
pub(super) fn take_focus() -> bool {
    FOCUS.with(|f| f.replace(false))
}

/// The game, owned, for the background read.
struct GameRef {
    id: String,
    title: String,
    steam_app_id: Option<String>,
    /// Steam starts the game: its app id there.
    steam_launch: Option<String>,
    install_path: Option<String>,
    executable: Option<String>,
    wine: bool,
}

/// Where ReShade goes for a game.
pub(crate) struct Place {
    pub(crate) install_path: Option<String>,
    pub(crate) executable: Option<String>,
    /// Wine or Proton runs it.
    pub(crate) wine: bool,
}

impl Place {
    /// `steam`: the app id when Steam starts the game, whose folder Steam
    /// knows (the Collection keeps Steam's program as its executable).
    /// `None`: Steam installed the game's Linux build, a Linux game. Blocking.
    pub(crate) fn find(
        paths: &DataPaths,
        steam: Option<&str>,
        install_path: Option<String>,
        executable: Option<String>,
        wine: bool,
    ) -> Option<Self> {
        let Some(app) = steam else { return Some(Self { install_path, executable, wine }) };
        let locator = SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe);
        match reshade::steam::install(&locator, app) {
            SteamInstall::Windows { dir, wine } => {
                Some(Self { install_path: Some(dir.to_string_lossy().into_owned()), executable: None, wine })
            }
            SteamInstall::Linux => None,
            SteamInstall::Missing => Some(Self { install_path: None, executable: None, wine }),
        }
    }
}

impl SettingsPanel {
    /// Whether this game gets the section: a Windows game Heronite starts
    /// (through Wine/Proton, or as it is on Windows) or Steam starts.
    pub(super) fn reshade_applies(&self) -> bool {
        self.reshade_wine().is_some()
    }

    /// Steam starts the game (its folder and programs come from Steam).
    pub(super) fn reshade_steam(&self) -> bool {
        self.facts().is_some_and(|f| matches!(f.run_kind, RunKind::Steam | RunKind::SteamDirect))
    }

    /// [`RunKind::reshade`] of the game.
    pub(super) fn reshade_wine(&self) -> Option<bool> {
        match &self.target {
            Target::Game(f) => f.run_kind.reshade(),
            Target::Defaults => None,
        }
    }

    /// A Steam game under Proton: Steam starts it, so ReShade's DLL
    /// overrides live in its launch options' `WINEDLLOVERRIDES` while it is
    /// on (put back if the line lost them), and leave once it is off.
    pub(super) fn sync_steam_reshade(&mut self, cx: &mut Context<Self>) {
        if !self.is_steam() || self.reshade_wine() != Some(true) || !self.steam_fields_editable() {
            return;
        }
        let Some(m) = self.reshade.overview.as_ref().and_then(|o| o.manifest.clone()) else { return };
        let dlls = reshade_dlls(&m);
        self.steam_reshade_overrides(&dlls, m.enabled, cx);
    }

    /// Adds (`on`) or takes out `dlls` from the Steam line's `WINEDLLOVERRIDES`.
    pub(super) fn steam_reshade_overrides(&mut self, dlls: &[String], on: bool, cx: &mut Context<Self>) {
        if !self.is_steam() || !self.steam_fields_editable() || dlls.is_empty() {
            return;
        }
        let dlls: Vec<&str> = dlls.iter().map(String::as_str).collect();
        let list = self.settings.get("enviromentOptions").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
        let mut found = false;
        let mut out = Vec::with_capacity(list.len() + 1);
        for mut e in list.iter().cloned() {
            if !e.get("key").and_then(serde_json::Value::as_str).is_some_and(|k| k.eq_ignore_ascii_case("WINEDLLOVERRIDES")) {
                out.push(e);
                continue;
            }
            found = true;
            let current = e.get("value").and_then(serde_json::Value::as_str).unwrap_or("");
            let value = match on {
                true => reshade::merge_overrides(current, &dlls).0,
                false => reshade::strip_overrides(current, &dlls),
            };
            if !value.is_empty() {
                e["value"] = serde_json::Value::String(value);
                out.push(e);
            }
        }
        if on && !found {
            out.push(serde_json::json!({"key": "WINEDLLOVERRIDES", "value": reshade::merge_overrides("", &dlls).0}));
        }
        if out != list {
            self.set("enviromentOptions", serde_json::Value::Array(out), cx);
        }
    }

    fn reshade_game(&self) -> Option<GameRef> {
        let f = self.facts()?;
        let g = f.game.as_ref();
        Some(GameRef {
            id: f.app_name.clone(),
            title: f.title.clone(),
            steam_app_id: f.protondb_id.clone(),
            steam_launch: g.and_then(|g| g.steam_launch_id.clone()).filter(|_| matches!(f.run_kind, RunKind::Steam | RunKind::SteamDirect)),
            install_path: g.and_then(|g| g.game.install.install_path.clone()),
            executable: g.and_then(|g| g.game.install.executable.clone()),
            wine: f.run_kind.reshade() == Some(true),
        })
    }

    pub(super) fn reshade_settings(&self) -> reshade::ReShade {
        reshade::ReShade::from_map(&self.settings)
    }

    /// Reads everything the section shows. `refresh`: ask the network even
    /// within a day (after an install, the lists are fresh already).
    pub(super) fn load_reshade(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if !self.reshade_applies() || self.reshade.loading {
            return;
        }
        let Some(game) = self.reshade_game() else { return };
        if self.reshade.overview.is_none() {
            self.reshade.debug = crate::env::value("HERONITE_DEBUG_RESHADE")
                .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
        }
        self.reshade.loading = true;
        let paths = self.paths.clone();
        let settings = self.reshade_settings();
        let target_exe = self.text("targetExe");
        // A Steam line's WINEDLLOVERRIDES carries ReShade's own overrides:
        // they are not the user's.
        let env = match self.is_steam() {
            true => Vec::new(),
            false => heronite_launch::settings::GameSettings::from_map(&self.settings, &self.home).env_options,
        };
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let place = Place::find(&paths, game.steam_launch.as_deref(), game.install_path, game.executable, game.wine)?;
                    let facts = reshade::Facts {
                        game_id: &game.id,
                        title: &game.title,
                        steam_app_id: game.steam_app_id.as_deref(),
                        install_path: place.install_path.as_deref(),
                        executable: place.executable.as_deref(),
                        target_exe: &target_exe,
                        settings: &settings,
                        env_options: &env,
                        wine: place.wine,
                    };
                    Some(reshade::overview(&paths, &heronite_launch::net::SystemNet, &facts, refresh))
                })
                .await;
            this.update(cx, |this, cx| {
                this.reshade.loading = false;
                match found {
                    Some(o) => this.reshade_loaded(o, cx),
                    None => this.reshade.linux_build = true,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reshade_loaded(&mut self, o: Overview, cx: &mut Context<Self>) {
        // The settings mirror what is on disk (a DLL renamed by hand, a
        // recovery that switched it off).
        let on = o.manifest.as_ref().is_some_and(|m| m.enabled);
        if self.reshade_settings().enabled != on && (o.manifest.is_some() || self.reshade_settings().enabled) {
            self.set(reshade::KEY_ENABLED, serde_json::Value::Bool(on), cx);
        }
        self.reshade.overview = Some(Arc::new(o));
        self.sync_steam_reshade(cx);
        self.next_reshade_debug(cx);
    }

    /// Runs `op` in the background with its progress in the section, then
    /// `done` on the UI thread and a fresh read of the section.
    pub(super) fn reshade_op<T: Send + 'static>(
        &mut self,
        first: String,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Ops) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        if self.reshade.busy.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.reshade.busy = Some(Busy { line: first, done: 0, total: 0, cancel: cancel.clone() });
        self.reshade.message = None;
        self.reshade.confirm = None;
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Step>();
        let paths = self.paths.clone();
        let task = cx.background_spawn(async move {
            let stop = || cancel.load(Ordering::Relaxed);
            let progress = |s: Step| {
                let _ = tx.unbounded_send(s);
            };
            let ops = Ops { paths: &paths, net: &heronite_launch::net::SystemNet, cancel: &stop, progress: &progress };
            op(&ops)
        });
        cx.spawn(async move |this, cx| {
            while let Some(step) = rx.next().await {
                let ok = this.update(cx, |this, cx| {
                    if let Some(b) = &mut this.reshade.busy {
                        b.line = step_line(&step);
                        b.done += 1;
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.reshade.busy = None;
                match res {
                    Ok(v) => done(this, v, cx),
                    Err(e) if e == heronite_launch::net::CANCELLED => {
                        this.reshade.message = Some((tr!("Cancelled: nothing was changed").into(), false))
                    }
                    Err(e) => {
                        log::warn!("ReShade: {e}");
                        this.reshade.message = Some((e, true));
                    }
                }
                this.load_reshade(false, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn cancel_reshade(&mut self, cx: &mut Context<Self>) {
        if let Some(b) = &mut self.reshade.busy {
            b.cancel.store(true, Ordering::Relaxed);
            b.line = tr!("Cancelling…").to_string();
        }
        cx.notify();
    }

    /// The next `HERONITE_DEBUG_RESHADE` step, once the section is idle.
    fn next_reshade_debug(&mut self, cx: &mut Context<Self>) {
        if self.reshade.busy.is_some() {
            return;
        }
        while let Some(step) = self.reshade.debug.pop_front() {
            let (name, arg) = step.split_once(':').map_or((step.as_str(), ""), |(a, b)| (a, b));
            let arg = arg.to_string();
            match name {
                "confirm-anticheat" => {
                    self.reshade.setup = true;
                    self.reshade.confirm = Some(Confirm::AntiCheat { accepted: false, replace: false });
                }
                "confirm-remove" => self.reshade.confirm = Some(Confirm::Remove),
                // The first source whose name starts with the argument.
                "confirm-source" => {
                    let o = self.reshade.overview.clone();
                    let found = o.iter().flat_map(|o| o.sources.iter()).find(|s| s.name.to_lowercase().starts_with(&arg.to_lowercase()));
                    if let Some(s) = found {
                        self.reshade.confirm = Some(Confirm::RemoveSource { name: s.name.clone(), used: s.used.clone() });
                    }
                }
                "add-repo" => self.reshade.adding_repo = true,
                "setup" => self.open_reshade_setup(cx),
                "advanced" => self.reshade.advanced = true,
                "packages" => {
                    self.reshade.setup = true;
                    self.reshade.packages_open = true;
                }
                "manage" => self.reshade.manage_shaders = true,
                "renodx-details" => self.reshade.renodx_details = true,
                "all-packages" => self.reshade.all_packages = true,
                "installing" => {
                    self.reshade.setup = true;
                    self.reshade.busy = Some(Busy {
                        line: tr!("Installing shaders: {name}…", name = "SweetFX by CeeJay.dk"),
                        done: 2,
                        total: 5,
                        cancel: Arc::default(),
                    })
                }
                "tick" => {
                    if let Some(Confirm::AntiCheat { accepted, .. }) = &mut self.reshade.confirm {
                        *accepted = true;
                    }
                }
                "accept" => {
                    if let Some(Confirm::AntiCheat { accepted, .. }) = &mut self.reshade.confirm {
                        *accepted = true;
                    }
                    return self.accept_anticheat(cx);
                }
                // Reads the file in the background, then goes on.
                "preset" => return self.choose_reshade_preset(PathBuf::from(arg), cx),
                // The steps below run in the background and read the section again.
                "install" => {
                    self.reshade.setup = true;
                    return self.start_reshade_install(false, true, cx);
                }
                "replace" => return self.start_reshade_install(true, true, cx),
                "adopt" => return self.adopt_reshade(cx),
                "off" | "on" => return self.switch_reshade(name == "on", cx),
                "folder" => return self.add_reshade_extra(arg, cx),
                "repo" => return self.add_reshade_extra(arg, cx),
                "remove-source" => return self.remove_reshade_source(arg, cx),
                "remove" => return self.remove_reshade(cx),
                "addon" | "standard" => return self.set_reshade_addon(name == "addon", cx),
                "renodx" => {
                    if let Some(f) = self.reshade.overview.as_ref().and_then(|o| o.renodx.clone()) {
                        return self.install_renodx(f, false, cx);
                    }
                }
                "remove-renodx" => return self.remove_renodx(cx),
                other => log::warn!("HERONITE_DEBUG_RESHADE: unknown step {other}"),
            }
        }
        cx.notify();
    }
}

fn step_line(step: &Step) -> String {
    match step {
        Step::Dll(version) => tr!("Downloading ReShade {version}…", version),
        Step::Compiler => tr!("Downloading d3dcompiler_47…").to_string(),
        Step::Shaders(name) => tr!("Installing shaders: {name}…", name),
        Step::Addon(name) => tr!("Downloading the RenoDX mod: {name}…", name),
        Step::Config => tr!("Writing ReShade.ini…").to_string(),
    }
}

/// The DLLs Wine must take from the game's folder for `m`.
fn reshade_dlls(m: &reshade::manifest::Manifest) -> Vec<String> {
    let compiler = m.compiler.is_some().then(|| reshade::install::COMPILER.to_string());
    std::iter::once(m.proxy.clone()).chain(compiler).collect()
}
