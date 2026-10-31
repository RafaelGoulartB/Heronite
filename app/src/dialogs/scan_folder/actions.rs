//! Scanning in the background, editing the list and adding the chosen games.

use std::path::Path;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::{self, SideloadPlatform, SideloadSpec};
use heronite_data::{game_config, DataPaths, Library};
use heronite_i18n::tr;
use serde_json::Value;

use super::scan::{self, FoundGame, Kind, KnownGames};
use super::{text_of, ScanFolderDialog};
use crate::dialogs::add_game::{clean_title, default_wine, install_folder, suggested_prefix};
use crate::dialogs::settings_panel::logic::{wine_of, wine_to_value};
use crate::dialogs::DialogEvent;

impl ScanFolderDialog {
    pub(super) fn clear_results(&mut self) {
        self.scan = None;
        self.scanned.clear();
        self.renaming = None;
        self.programs_open = None;
        self.error.clear();
    }

    /// Searches the folder in the field. A scan started meanwhile wins.
    pub(super) fn scan(&mut self, cx: &mut Context<Self>) {
        let dir = text_of(&self.directory, cx);
        self.clear_results();
        if dir.is_empty() || self.importing {
            cx.notify();
            return;
        }
        if !Path::new(&dir).is_dir() {
            self.error = tr!("This folder does not exist.").into();
            cx.notify();
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        self.scanning = true;
        let lib = self.ctx.lib.clone();
        let root = dir.clone();
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { find_games(Path::new(&root), &lib) }).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.scanning = false;
                this.scanned = dir;
                this.scan = Some(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Screenshots: the hidden window does not repaint late results.
    pub(super) fn scan_now(&mut self, dir: &str) {
        self.pending_dir = Some(dir.to_string());
        self.scanned = dir.to_string();
        self.scan = Some(find_games(Path::new(dir), &self.ctx.lib));
    }

    pub(super) fn select_all(&mut self, on: bool) {
        if let Some(scan) = self.scan.as_mut() {
            for g in scan.games.iter_mut().filter(|g| g.known != scan::Known::Path) {
                g.import = on;
            }
        }
    }

    pub(super) fn start_rename(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.scan.as_ref().and_then(|s| s.games.get(i)).map(|g| g.title.clone()) else { return };
        self.renaming = Some(i);
        self.programs_open = None;
        self.rename.update(cx, |s, cx| {
            s.set_value(title, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Keeps the title typed (an empty one keeps the old title).
    pub(super) fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.renaming.take() else { return };
        let title = text_of(&self.rename, cx).trim().to_string();
        if let Some(g) = self.scan.as_mut().and_then(|s| s.games.get_mut(i)).filter(|_| !title.is_empty()) {
            g.title = title;
        }
        cx.notify();
    }

    pub(super) fn import(&mut self, cx: &mut Context<Self>) {
        if self.importing || self.scanning {
            return;
        }
        self.finish_rename(cx);
        let games: Vec<FoundGame> = self.scan.as_ref().map(|s| s.games.iter().filter(|g| g.import).cloned().collect()).unwrap_or_default();
        if games.is_empty() {
            return;
        }
        let (paths, own_prefixes) = (self.ctx.data_paths.clone(), self.own_prefixes);
        self.importing = true;
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { add_games(&paths, &games, own_prefixes) }).await;
            this.update(cx, |this, cx| {
                this.importing = false;
                match res {
                    Ok(()) => {
                        // Their covers come from the network afterwards.
                        crate::dialogs::metadata_wizard::hub(cx).update(cx, |h, cx| h.fill_local_covers(cx));
                        cx.emit(DialogEvent::ChangedAndClose);
                    }
                    Err(err) => this.error = tr!("Could not add the games: {error}", error = format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// [`scan::scan`] with the games the Collection has marked.
fn find_games(root: &Path, lib: &Library) -> scan::Scan {
    let mut found = scan::scan(root, !cfg!(windows));
    let mut known = KnownGames::default();
    for g in &lib.games {
        let install = &g.game.install;
        let remapped = g.meta.as_ref().and_then(|m| m.remapped_executable.as_deref());
        let executables: Vec<&str> = [install.executable.as_deref(), remapped].into_iter().flatten().collect();
        let folders: Vec<&str> = [install.install_path.as_deref(), g.game.folder_name.as_deref()].into_iter().flatten().collect();
        known.add(&executables, &folders, &g.title);
    }
    scan::mark_known(&mut found.games, &known);
    found
}

/// What "Add game" writes for a game found in a folder.
fn spec_for(g: &FoundGame) -> SideloadSpec {
    let program = g.program();
    let executable = program.path.to_string_lossy().into_owned();
    let platform = match program.kind {
        Kind::Windows => SideloadPlatform::Windows,
        Kind::Linux => SideloadPlatform::Linux,
    };
    SideloadSpec {
        app_name: sideload::generate_app_name(),
        title: clean_title(g.title.trim()),
        install_folder: if platform == SideloadPlatform::Linux { install_folder(&executable).unwrap_or_default() } else { String::new() },
        executable,
        platform,
        art_cover: String::new(),
        art_square: String::new(),
        browser_url: String::new(),
        custom_user_agent: String::new(),
        launch_full_screen: false,
        run_as: None,
        install: false,
        command: String::new(),
    }
}

/// Adds the games: one write of the library, then the Wine settings of the
/// Windows ones (the Wine version "Add game" would pick, and a prefix of
/// their own when `own_prefixes`) and their shortcuts.
fn add_games(paths: &DataPaths, games: &[FoundGame], own_prefixes: bool) -> anyhow::Result<()> {
    let specs: Vec<SideloadSpec> = games.iter().map(spec_for).collect();
    sideload::save_games(paths, &specs)?;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let windows: Vec<&SideloadSpec> = specs.iter().filter(|s| s.platform == SideloadPlatform::Windows).collect();
    if !windows.is_empty() && heronite_platform::supported(heronite_platform::Feature::Wine) {
        let global = game_config::load_global(paths, &home);
        let current = global.get("wineVersion").is_some().then(|| wine_of(&global));
        let wine = default_wine(current.as_ref(), &heronite_wine::list_installations(paths)).map(|w| wine_to_value(&w));
        let prefix_dir = global.get("defaultWinePrefixDir").and_then(Value::as_str).unwrap_or("");
        for s in windows {
            // No prefix of its own: the default (shared) one.
            let prefix = if own_prefixes { suggested_prefix(prefix_dir, &s.title, &s.app_name) } else { String::new() };
            sideload::init_game_config(paths, &home, &s.app_name, &prefix, wine.clone(), "")?;
        }
    }
    for s in &specs {
        let outcome = crate::system::shortcuts::on_game_added(paths, "sideload", &s.app_name);
        for m in outcome.messages.iter().filter(|m| m.error) {
            log::warn!("shortcuts for {}: {}", s.title, m.text);
        }
    }
    log::info!("added {} game(s) found in a folder", specs.len());
    Ok(())
}
