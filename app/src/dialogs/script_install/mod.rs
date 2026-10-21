//! "Install from script": installs a game from a YAML (or JSON) install
//! script file. Three faces:
//!
//! * nothing chosen yet: a button that picks the file (the file chooser
//!   opens by itself when the dialog does);
//! * review: what the script is, what it needs (files to pick, the
//!   install folder, Wine), what it runs, and what stops it ([`check`]);
//! * the install itself, owned by `services::script_install` so it keeps
//!   going when the dialog closes: stages, progress, output, the script's
//!   questions, and the end (notes, or the error and what to do with the
//!   files).
//!
//! The rules live in `heronite_installer`; this is only the face.

mod render;
mod run;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_config::{self, Settings};
use heronite_i18n::tr;
use heronite_installer::{Check, Job, Script};
use heronite_wine::WineInstallation;
use serde_json::Value;

use super::settings_panel::logic::{wine_of, wine_options, wine_to_value};
use super::{text_of, DialogCtx, DialogEvent};
use crate::services::script_install::{self, ScriptInstall};
use crate::ui::fields::{choose_path, new_input};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Select {
    Script,
    Wine,
    Start,
}

pub struct ScriptInstallDialog {
    ctx: DialogCtx,
    home: String,
    global: Settings,
    service: Entity<ScriptInstall>,
    file: Option<PathBuf>,
    loading: bool,
    load_error: Option<String>,
    /// The scripts of the file, each with its check.
    scripts: Vec<(Script, Check)>,
    pick: usize,
    title: Entity<InputState>,
    folder: Entity<InputState>,
    /// The folder this dialog filled in (typing over it keeps the typed one
    /// when another script is picked).
    folder_suggested: String,
    /// The chosen folder already has files.
    folder_used: bool,
    picked: HashMap<String, PathBuf>,
    /// The step the install starts at (index into the script's steps): a
    /// script run again over a folder that has the first part of it.
    start_at: usize,
    wine_list: Option<Vec<WineInstallation>>,
    wine: Option<WineInstallation>,
    open_select: Option<Select>,
    show_runs: bool,
    show_log: bool,
    /// The option ticked in the script's question.
    menu_choice: Option<String>,
    error: String,
    scroll: ScrollHandle,
    log_scroll: ScrollHandle,
    /// `log_count` of the run when the output last scrolled to its end.
    log_seen: usize,
    log_was_open: bool,
    /// Debug hook: install as soon as the script is read.
    auto_start: bool,
    /// Debug hook: takes the preselected answer of the script's questions.
    auto_answer: bool,
    /// The game's sandbox: the install runs in it too.
    sandbox: heronite_launch::sandbox::Sandbox,
    /// Why the sandbox cannot run here (looked at when it is turned on).
    sandbox_problem: Option<String>,
    /// Shown as a step of "Add game": its title, a back arrow to the form.
    in_add_game: bool,
    _subs: Vec<Subscription>,
}

/// The back arrow of a script shown inside "Add game" was pressed.
pub struct BackToForm;

impl EventEmitter<DialogEvent> for ScriptInstallDialog {}
impl EventEmitter<BackToForm> for ScriptInstallDialog {}

impl ScriptInstallDialog {
    /// `file`: a script to open right away; `None` opens the file chooser
    /// (or shows the install already running).
    pub fn new(ctx: DialogCtx, file: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(ctx, file, false, window, cx)
    }

    /// "Use an install script" of "Add game": the same faces inside that
    /// dialog, with a back arrow to its form.
    pub fn inside_add_game(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(ctx, None, false, window, cx);
        this.in_add_game = true;
        this
    }

    /// The last install, also when it has ended (the toast's "Details").
    pub fn showing_last(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(ctx, None, true, window, cx)
    }

    fn build(ctx: DialogCtx, file: Option<PathBuf>, show_last: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let global = game_config::load_global(&ctx.data_paths, &home);
        let service = script_install::get(cx);
        // A finished install from before is not shown again, unless asked.
        let keep = service.read(cx).run().is_some_and(|r| r.running() || show_last);
        if !keep {
            service.update(cx, |s, cx| s.clear(cx));
        }
        let title = new_input(window, cx, "", tr!("Add a title to your Game/App"));
        let folder = new_input(window, cx, "", tr!("The folder the game is installed in"));
        let mut subs = vec![cx.observe(&service, |this, service, cx| {
            if this.auto_answer {
                let q = service.read(cx).run().and_then(|r| r.question.clone());
                if let Some(heronite_installer::Question::Menu { preselect, options, .. }) = q {
                    let a = preselect.or_else(|| options.first().map(|o| o.0.clone())).map(heronite_installer::Answer::Choice);
                    let service = service.clone();
                    cx.defer(move |cx| service.update(cx, |s, cx| s.answer(a, cx)));
                }
            }
            cx.notify();
        })];
        subs.push(cx.subscribe_in(&folder, window, |this, input, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                let path = input.read(cx).value().trim().to_string();
                this.folder_used = folder_has_files(&path);
                cx.notify();
            }
        }));
        subs.push(cx.subscribe_in(&title, window, |_, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        }));
        let mut this = Self {
            ctx,
            home,
            global,
            service,
            file: None,
            loading: false,
            load_error: None,
            scripts: Vec::new(),
            pick: 0,
            title,
            folder,
            folder_suggested: String::new(),
            folder_used: false,
            picked: HashMap::new(),
            start_at: 0,
            wine_list: None,
            wine: None,
            open_select: None,
            show_runs: false,
            show_log: false,
            menu_choice: None,
            error: String::new(),
            scroll: ScrollHandle::new(),
            log_scroll: ScrollHandle::new(),
            log_seen: 0,
            log_was_open: false,
            auto_start: false,
            auto_answer: false,
            sandbox: Default::default(),
            sandbox_problem: None,
            in_add_game: false,
            _subs: subs,
        };
        match file {
            Some(f) => this.load(f, window, cx),
            None if !keep && !crate::env::flag("HERONITE_DEBUG_DIALOG") => this.choose_file(window, cx),
            None => {}
        }
        this
    }

    fn running(&self, cx: &App) -> bool {
        self.service.read(cx).run().is_some()
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, false, tr!("Select an install script"), |this: &mut Self, path, window, cx| {
            this.load(PathBuf::from(path), window, cx);
        });
    }

    /// Reads and checks the scripts of `file` off the UI thread.
    fn load(&mut self, file: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.file = Some(file.clone());
        self.loading = true;
        self.load_error = None;
        self.scripts.clear();
        self.error.clear();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let scripts = heronite_installer::read_file(&file)?;
                    Ok::<_, String>(
                        scripts
                            .into_iter()
                            .map(|s| {
                                let c = heronite_installer::check(&s);
                                (s, c)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match res {
                    Ok(scripts) => {
                        this.scripts = scripts;
                        this.set_pick(0, window, cx);
                        if std::mem::take(&mut this.auto_start) {
                            this.start(window, cx);
                        }
                    }
                    Err(e) => this.load_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn script(&self) -> Option<&Script> {
        self.scripts.get(self.pick).map(|(s, _)| s)
    }

    fn check(&self) -> Option<&Check> {
        self.scripts.get(self.pick).map(|(_, c)| c)
    }

    /// Shows the `i`-th script of the file: its title and folder, and the
    /// Wine list when it needs one.
    fn set_pick(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((script, check)) = self.scripts.get(i) else { return };
        self.pick = i;
        self.start_at = 0;
        self.picked.retain(|id, _| check.asks.iter().any(|(a, _)| a == id));
        let title = script.title().to_string();
        let base = self.global.get("defaultInstallPath").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).map(str::to_string);
        let base = game_config::expand_tilde(&base.unwrap_or_else(|| game_config::default_install_dir(&self.home)), &self.home);
        let suggested = format!("{}/{}", base.trim_end_matches('/'), script.folder_name());
        let typed = text_of(&self.folder, cx);
        if typed.is_empty() || typed == self.folder_suggested {
            self.folder.update(cx, |s, cx| s.set_value(suggested.clone(), window, cx));
            self.folder_used = folder_has_files(&suggested);
        }
        self.folder_suggested = suggested;
        self.title.update(cx, |s, cx| s.set_value(title, window, cx));
        let needs_wine = check.needs_wine();
        if needs_wine && self.wine_list.is_none() {
            self.load_wine_list(cx);
        } else if needs_wine {
            self.pick_wine();
        }
        cx.notify();
    }

    fn load_wine_list(&mut self, cx: &mut Context<Self>) {
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { heronite_wine::list_installations(&paths) }).await;
            this.update(cx, |this, cx| {
                this.wine_list = Some(list);
                this.pick_wine();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The Wine the script asks for when it is installed, else the default
    /// one, else the first of the list.
    fn pick_wine(&mut self) {
        let Some(list) = self.wine_list.as_deref() else { return };
        let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '_'], "-");
        let wanted =
            self.script().and_then(|s| s.runner_config.get("version")).and_then(heronite_installer::text::scalar).map(|v| norm(&v));
        let by_script =
            wanted.filter(|w| !w.is_empty()).and_then(|w| list.iter().find(|i| norm(&i.name) == w || norm(&i.name).ends_with(&w)));
        let default = self.global.get("wineVersion").map(|_| wine_of(&self.global));
        let by_default = default.and_then(|d| list.iter().find(|i| i.bin == d.bin || i.name == d.name));
        // The list in the order the selects show it (Proton first).
        let first = list.first().and_then(|f| wine_options(list, f).into_iter().next());
        self.wine = by_script.or(by_default).cloned().or(first);
    }

    /// Turns the sandbox (`network: false`) or its Internet exception on or off.
    fn toggle_sandbox(&mut self, network: bool, cx: &mut Context<Self>) {
        let s = self.sandbox;
        self.sandbox = match network {
            false => heronite_launch::sandbox::Sandbox { enabled: !s.enabled, ..s },
            true => heronite_launch::sandbox::Sandbox { network: !s.network, ..s },
        };
        if self.sandbox.enabled {
            cx.spawn(async move |this, cx| {
                let why = cx.background_spawn(async move { heronite_launch::sandbox::unavailable(&heronite_launch::SystemProbe) }).await;
                this.update(cx, |this, cx| {
                    this.sandbox_problem = why;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Why Install cannot be pressed yet (`None`: it can).
    fn not_ready(&self, cx: &App) -> Option<String> {
        if let Some(why) = self.sandbox_problem.as_ref().filter(|_| self.sandbox.enabled) {
            return Some(why.clone());
        }
        let check = self.check()?;
        if check.blocked() {
            return Some(tr!("This script cannot be installed here.").into());
        }
        if text_of(&self.title, cx).is_empty() {
            return Some(tr!("Add a title to continue.").into());
        }
        if text_of(&self.folder, cx).is_empty() || !Path::new(&text_of(&self.folder, cx)).is_absolute() {
            return Some(tr!("Choose the install folder to continue.").into());
        }
        if let Some((_, what)) = check.asks.iter().find(|(id, _)| !self.picked.contains_key(id)) {
            return Some(tr!("Select {file} to continue.", file = what.clone()));
        }
        if check.needs_wine() && self.wine.is_none() {
            return Some(tr!("Windows games need Wine or Proton. Download one from Wine Manager in the menu (\u{2630}).").into());
        }
        None
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.not_ready(cx).is_some() {
            return;
        }
        let Some(script) = self.script().cloned() else { return };
        let resolution = window.display(cx).map(|d| {
            let size = d.bounds().size;
            let scale = window.scale_factor();
            ((f32::from(size.width) * scale).round() as u32, (f32::from(size.height) * scale).round() as u32)
        });
        let job = Job {
            app_name: heronite_data::sideload::generate_app_name(),
            title: text_of(&self.title, cx),
            game_dir: PathBuf::from(text_of(&self.folder, cx)),
            wine_version: self.check().filter(|c| c.needs_wine()).and(self.wine.as_ref()).map(wine_to_value),
            provided: self.picked.clone(),
            resolution,
            cache_root: self.ctx.app.cache.join("install-scripts"),
            paths: self.ctx.data_paths.clone(),
            home: self.home.clone(),
            resume: (self.start_at > 0).then(|| heronite_installer::Resume { from: self.start_at, ..Default::default() }),
            sandbox: self.sandbox,
            script,
        };
        let res = self.service.update(cx, |s, cx| s.start(job, cx));
        match res {
            Ok(()) => {
                self.error.clear();
                self.menu_choice = None;
            }
            Err(e) => self.error = e,
        }
        cx.notify();
    }

    /// `HERONITE_DEBUG_DIALOG=install-script:<file>[:runs|log|install|answer]`
    /// (`install` starts it right away, `answer` takes the preselected
    /// answers: data copies only).
    pub fn debug_state(&mut self, state: &str, cx: &mut Context<Self>) {
        for part in state.split('+') {
            match part {
                "runs" => self.show_runs = true,
                "log" => self.show_log = true,
                "install" => self.auto_start = true,
                "answer" => self.auto_answer = true,
                _ => {}
            }
        }
        cx.notify();
    }
}

/// Whether `path` is a folder with something in it (tiny read).
fn folder_has_files(path: &str) -> bool {
    !path.is_empty() && std::fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false)
}
