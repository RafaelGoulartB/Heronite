//! Games brought over from Playnite as this app's own: they start here,
//! without Playnite, and come with what Playnite knew of them. A game is
//! copied once (when it first comes in, or when an entry that still asked
//! Playnite to start it is brought over); from then on it is the user's, and
//! later imports only raise its play time.
//!
//! The entry ([`entry`], [`set_meta`]) is written with the other games of
//! the import; the rest of what the game carries goes to its own files
//! afterwards ([`settle`]): its arguments (`GamesConfig`), favourite and
//! hidden flags, categories, rating, extra actions, the day it was added and
//! its details (`heronite_metadata`'s Playnite candidate).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_data::game_actions::Action;
use heronite_data::launchers::{PREFIX_KEY, RUNS_FROM_KEY};
use heronite_data::models::{CompletionStatus, GameInfo, InstallInfo, LocalGameMeta, LocalGameRom};
use heronite_data::sideload::WORKING_DIR_KEY;
use heronite_data::DataPaths;
use heronite_metadata::providers::PlayniteGame;
use serde_json::Value;

use crate::Game;

/// How a game brought over starts here.
#[derive(Debug, Clone, PartialEq)]
pub enum Start {
    /// A program and its arguments (Windows' syntax), from `working_dir`
    /// when that is not the program's folder.
    Program { path: String, args: String, working_dir: Option<String> },
    /// An emulator's program with its argument template (`{ImagePath}`,
    /// `{ImageName}`, `{ImageNameNoExt}`, `{ImageDir}` left for the launch,
    /// shlex syntax) and the game's ROMs, the first one played.
    Emulator { name: String, program: String, args: String, roms: Vec<Rom> },
}

impl Start {
    fn program(&self) -> &str {
        match self {
            Start::Program { path, .. } => path,
            Start::Emulator { program, .. } => program,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rom {
    pub name: Option<String>,
    pub path: String,
}

/// What a game brought over from Playnite carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub start: Start,
    /// Its details, for the metadata.
    pub details: PlayniteGame,
    /// Playnite's library folder (its pictures are under `files`).
    pub library: PathBuf,
    pub favourite: bool,
    pub hidden: bool,
    /// `1..=5`.
    pub stars: Option<u8>,
    /// Its completion status: Playnite's id and name.
    pub status: Option<(String, String)>,
    pub notes: Option<String>,
    /// `YYYY-MM-DD`.
    pub added: Option<String>,
    pub categories: Vec<String>,
    /// Its other actions.
    pub actions: Vec<Action>,
}

/// The sideload entry of a game brought over: its program and folder.
pub(crate) fn entry(info: &mut GameInfo, game: &Game, transfer: &Transfer) {
    let program = transfer.start.program();
    let parent = || Path::new(program).parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty());
    // A program's folder stands in for a game with none, so it stays
    // Playnite's (`launchers::managed_paths`); never a drive's root.
    let own_folder = || parent().filter(|p| Path::new(p).components().count() > 2);
    let folder = game.folder.clone().or_else(|| matches!(transfer.start, Start::Program { .. }).then(own_folder).flatten());
    info.install = InstallInfo {
        executable: Some(program.to_string()),
        platform: Some("Windows".into()),
        install_path: folder.clone(),
        is_dlc: Some(false),
        ..Default::default()
    };
    info.is_linux_native = Some(false);
    info.folder_name = folder.or_else(parent);
    if let Start::Program { working_dir: Some(dir), .. } = &transfer.start {
        info.rest.insert(WORKING_DIR_KEY.into(), Value::String(dir.clone()));
    }
}

/// Sets on the Collection entry of `game` how it starts here, its notes and
/// its status. `fresh`: a new entry (else one that asked Playnite to start
/// it, whose status the user may have set).
pub(crate) fn set_meta(meta: &mut LocalGameMeta, game: &Game, transfer: &Transfer, statuses: &[CompletionStatus], fresh: bool) {
    meta.playnite_id = game.id.clone();
    meta.launcher_args = None;
    meta.extra.remove(PREFIX_KEY);
    meta.extra.remove(RUNS_FROM_KEY);
    meta.emulator_id = None;
    meta.emulator_profile_id = None;
    match &transfer.start {
        Start::Program { path, .. } => {
            meta.launch_kind = "executable".into();
            meta.remapped_executable = Some(path.clone());
            meta.roms = None;
            meta.emulator_name = None;
        }
        Start::Emulator { name, program, args, roms } => {
            meta.launch_kind = "emulator".into();
            meta.remapped_executable = Some(program.clone());
            meta.launcher_args = Some(args.clone());
            meta.roms =
                Some(roms.iter().map(|r| LocalGameRom { name: r.name.clone(), path: r.path.clone(), extra: Default::default() }).collect());
            meta.emulator_name = Some(name.clone());
        }
    }
    if meta.notes.as_deref().is_none_or(|n| n.trim().is_empty()) {
        meta.notes = transfer.notes.clone();
    }
    let inferred = heronite_data::status::inferred_status_id(game.minutes as f64, statuses);
    if let Some((id, name)) = &transfer.status {
        meta.playnite_completion_status_id = Some(id.clone());
        // An entry that asked Playnite got a status from its play time then
        // (played or not); one the user picked since stays.
        let guessed = |s: &str| [0.0, 1.0].iter().any(|m| heronite_data::status::inferred_status_id(*m, statuses) == s);
        if fresh || meta.completion_status_id.as_deref().is_none_or(guessed) {
            let incoming = [(id.clone(), name.clone())];
            meta.completion_status_id = Some(heronite_data::status::resolve_playnite_status_id(Some(id), &incoming, statuses));
        }
    } else if meta.completion_status_id.is_none() {
        meta.completion_status_id = Some(inferred);
    }
}

/// The game's key in the files keyed by `CollectionGame.key`.
fn key(app_name: &str) -> String {
    heronite_data::journal::collection_key("sideload", app_name)
}

/// The days the games about to be brought over were added to Playnite, kept
/// before their entries are written (a Collection load in between would
/// give them today).
pub(crate) fn record_added(paths: &DataPaths, games: &[(String, &Game, &Transfer)]) {
    let dates: Vec<(String, String)> = games.iter().filter_map(|(app, _, t)| Some((key(app), t.added.clone()?))).collect();
    if let Err(e) = heronite_data::added::set(paths, &dates) {
        log::warn!("Keeping the days Playnite's games were added: {e:#}");
    }
}

/// Writes what the games just brought over carry beyond their entry. What
/// the user already has for a game (a rating, actions, details) stays.
/// `home` is the user's home folder. A part that fails is logged; the
/// others are still written.
pub(crate) fn settle(paths: &DataPaths, home: &Path, games: &[(String, &Game, &Transfer)]) {
    if games.is_empty() {
        return;
    }
    let mut failed = Vec::new();
    let mut note = |what: &str, r: Result<()>| {
        if let Err(e) = r {
            failed.push(format!("{what}: {e:#}"));
        }
    };

    let home = home.to_string_lossy();
    for (app, _, t) in games {
        if let Start::Program { args, .. } = &t.start {
            if !args.trim().is_empty() {
                let change = [("launcherArgs".to_string(), Value::String(args.trim().to_string()))];
                note("arguments", heronite_data::game_config::set_game_settings(paths, app, &home, &change));
            }
        }
    }

    let flag = |on: fn(&Transfer) -> bool| -> Vec<(String, String, bool)> {
        games.iter().filter(|(_, _, t)| on(t)).map(|(app, g, _)| (app.clone(), g.title.clone(), true)).collect()
    };
    note("favourites", heronite_data::config_games::set_favourites(paths, &flag(|t| t.favourite)));
    note("hidden games", heronite_data::config_games::set_hidden(paths, &flag(|t| t.hidden)));

    let mut categories: BTreeMap<&str, Vec<(String, String, bool)>> = BTreeMap::new();
    for (app, _, t) in games {
        for name in &t.categories {
            categories.entry(name.as_str()).or_default().push(("sideload".into(), app.clone(), true));
        }
    }
    for (name, members) in categories {
        note("categories", heronite_data::categories::set_many_in_category(paths, name, &members));
    }

    let rated = heronite_data::ratings::load(paths);
    for (app, _, t) in games {
        if let Some(stars) = t.stars.filter(|_| !rated.contains_key(&key(app))) {
            note("ratings", heronite_data::ratings::set_rating(paths, &key(app), Some(stars)));
        }
    }

    let actions: HashMap<String, Vec<Action>> = heronite_data::game_actions::load(paths);
    for (app, _, t) in games.iter().filter(|(app, _, t)| !t.actions.is_empty() && !actions.contains_key(&key(app))) {
        note("actions", heronite_data::game_actions::save(paths, &key(app), &t.actions));
        // Playnite asks which play action to start: so does Play here.
        if t.actions.iter().any(Action::plays) {
            note("actions", heronite_data::game_actions::set_ask_on_play(paths, &key(app), true));
        }
    }

    let service = heronite_metadata::MetadataService::new(paths.clone());
    let details: Vec<_> = games.iter().map(|(app, _, t)| (app.as_str(), "sideload", &t.details, t.library.as_path())).collect();
    note("details", service.seed_playnite_games(&details));

    for f in failed {
        log::warn!("Bringing Playnite's games over: {f}");
    }
}

/// Playnite's 0–100 score as stars (`None` below 10).
pub fn stars(score: i64) -> Option<u8> {
    let stars = (score.clamp(0, 100) as f64 / 20.0).round() as u8;
    (stars > 0).then_some(stars)
}

/// The ids of `actions`, each new among them.
pub fn with_ids(mut actions: Vec<Action>) -> Vec<Action> {
    let mut taken: Vec<Action> = Vec::new();
    for a in &mut actions {
        a.id = heronite_data::game_actions::new_id(&taken);
        taken.push(a.clone());
    }
    actions
}
