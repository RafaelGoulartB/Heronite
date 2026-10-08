//! `import.ts`: preview, import and merge of a Playnite library.
//!
//! Differences from the TS that do not change the resulting files: the work is
//! synchronous, and the writes are batched (one read-modify-write per store
//! file at the end instead of one per game) which is far faster for a few
//! thousand games and never overwrites what Heronite wrote meanwhile.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use heronite_data::models::*;
use heronite_data::{json_store, settings, status, HeroicPaths};
use heronite_launch::probe::{Probe, SystemProbe};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::{find_steam_binary, installed_app_ids, SteamLocator};
use indexmap::IndexMap;

use crate::covers::{CoverFetcher, SteamCdnCovers};
use crate::game_config::apply_launcher_args;
use crate::mapper::*;
use crate::metadata_seed::seed_playnite_metadata;
use crate::reader::*;
use crate::remap::{collect_windows_drives, DriveRemap};
use crate::store_ops::*;
use crate::types::*;
use crate::user_emulators::ensure_playnite_emulator;

pub const NO_LIBRARY_SAVED: &str = "No Playnite library path is saved. Import a library first, then merge.";

/// Entry point: everything the Playnite menu / import dialog / merge dialog
/// calls. Cheap to construct.
pub struct Playnite {
    pub paths: HeroicPaths,
    probe: Box<dyn Probe>,
    covers: Box<dyn CoverFetcher>,
}

impl Playnite {
    /// Real machine (`SystemProbe`) and offline Steam CDN covers.
    pub fn new(paths: HeroicPaths) -> Self {
        Self { paths, probe: Box::new(SystemProbe), covers: Box::new(SteamCdnCovers) }
    }

    /// Replaces the filesystem/Steam probe (tests).
    pub fn with_probe(mut self, probe: impl Probe + 'static) -> Self {
        self.probe = Box::new(probe);
        self
    }

    /// Replaces the cover fetcher (e.g. one that hits SteamGridDB).
    pub fn with_cover_fetcher(mut self, covers: impl CoverFetcher + 'static) -> Self {
        self.covers = Box::new(covers);
        self
    }

    /// `exportPlayniteLibrary`: writes the `heroic-playnite-export` file at
    /// `target` (a folder, or a `.json` path).
    pub fn export(&self, target: &Path) -> Result<PlayniteExportResult> {
        crate::export::export_playnite_library(&self.paths, target)
    }

    // -- saved context ------------------------------------------------------

    /// `getLastPlayniteLibraryPath`.
    pub fn last_library_path(&self) -> Result<Option<String>> {
        last_playnite_library_path(&self.paths)
    }

    /// `getLastPlayniteDriveMap`.
    pub fn last_drive_map(&self) -> Result<Vec<DriveRemap>> {
        last_playnite_drive_map(&self.paths)
    }

    // -- preview ------------------------------------------------------------

    /// `previewPlayniteImport`.
    pub fn preview_import(&self, library_path: &str) -> Result<PlayniteImportPreview> {
        if !Path::new(library_path).join("games.db").exists() {
            return Ok(PlayniteImportPreview {
                library_path: library_path.to_owned(),
                playnite_root: library_path.to_owned(),
                errors: vec!["games.db was not found in the selected folder".into()],
                ..Default::default()
            });
        }
        let dump = load_playnite_library(library_path)?;
        let stores = StoreIndexes::load(&self.paths);
        let local: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        let by_playnite = playnite_index(&local);

        let mut games = Vec::with_capacity(dump.games.len());
        let mut counts = PlayniteImportCounts::default();
        for game in &dump.games {
            let sessions = playnite_sessions_to_local(&game.id, &dump.sessions_by_game_id);
            let session_count = sessions.len();
            let mapped = map_playnite_game(game, &dump.emulators, sessions, &[], self.probe.as_ref());
            let (destination, matched) = destination_for(&mapped, &stores);
            match mapped.meta.source.as_str() {
                "steam" => counts.steam += 1,
                "emulator" => counts.emulator += 1,
                "manual" => counts.manual += 1,
                "xbox" => counts.xbox += 1,
                "amazon" => counts.amazon += 1,
                "epic" if destination == Destination::Legendary => counts.epic_match += 1,
                "gog" if destination == Destination::Gog => counts.gog_match += 1,
                _ if destination == Destination::Skip => counts.skipped_store += 1,
                _ => counts.other += 1,
            }
            games.push(PlaynitePreviewGame {
                playnite_id: game.id.clone(),
                title: game.name.clone(),
                source: mapped.meta.source.clone(),
                destination,
                playtime_minutes: mapped.playtime_minutes.into(),
                session_count,
                already_imported: by_playnite.contains_key(&game.id),
                matched_app_name: match destination {
                    Destination::Legendary | Destination::Gog => matched,
                    _ => Some(mapped.meta.app_name.clone()),
                },
            });
        }

        let paths_to_scan: Vec<Option<&str>> = dump
            .games
            .iter()
            .flat_map(|g| {
                std::iter::once(g.install_directory.as_deref())
                    .chain(g.game_actions.iter().map(|a| a.path.as_deref()))
                    .chain(g.roms.iter().map(|r| Some(r.path.as_str())))
            })
            .filter(|p| p.is_some_and(|p| !p.is_empty()))
            .collect();

        Ok(PlayniteImportPreview {
            library_path: dump.library_path.clone(),
            playnite_root: dump.playnite_root.clone(),
            game_count: dump.games.len(),
            session_file_count: dump.sessions_by_game_id.len(),
            detected_drives: collect_windows_drives(paths_to_scan),
            games,
            counts,
            errors: Vec::new(),
        })
    }

    /// `previewPlayniteMerge` (uses the saved library path and drive map).
    pub fn preview_merge(&self) -> Result<PlayniteMergePreview> {
        let Some(library_path) = self.saved_library_path()? else {
            return Ok(PlayniteMergePreview { errors: vec![NO_LIBRARY_SAVED.into()], ..Default::default() });
        };
        let dump = load_playnite_library(&library_path)?;
        let drive_map = self.last_drive_map()?;
        let statuses = status::load_statuses(&self.paths)?;
        let stores = StoreIndexes::load(&self.paths);
        let local: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        let by_playnite = playnite_index(&local);
        let sessions_file: SessionsFile = json_store::read(&self.paths.sessions())?;
        let timestamps: TimestampFile = json_store::read(&self.paths.timestamps())?;
        let incoming_statuses: Vec<(String, String)> =
            dump.completion_statuses.iter().map(|s| (s.id.clone(), s.name.clone())).collect();

        let name_of = |id: Option<&str>| -> String {
            statuses
                .iter()
                .find(|s| Some(s.id.as_str()) == id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| id.unwrap_or_default().to_owned())
        };
        let playnite_name = |id: Option<&str>| -> String {
            dump.completion_statuses
                .iter()
                .find(|s| Some(s.id.as_str()) == id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| name_of(id))
        };

        let mut preview = PlayniteMergePreview { library_path: library_path.clone(), ..Default::default() };
        for game in &dump.games {
            let sessions = playnite_sessions_to_local(&game.id, &dump.sessions_by_game_id);
            let mapped = map_playnite_game(game, &dump.emulators, sessions, &drive_map, self.probe.as_ref());
            let (destination, _) = destination_for(&mapped, &stores);
            let existed = by_playnite.get(&game.id).and_then(|app| local.games.get(app));
            let pid = mapped.meta.playnite_completion_status_id.as_deref();

            if destination == Destination::Skip && existed.is_none() {
                preview.skipped += 1;
                continue;
            }
            let Some(existed) = existed else {
                preview.new_games.push(PlayniteMergePreviewItem {
                    title: game.name.clone(),
                    playnite_id: game.id.clone(),
                    playnite_minutes: Some(mapped.playtime_minutes.into()),
                    playnite_status: Some(playnite_name(pid)),
                    ..Default::default()
                });
                preview.new_sessions += mapped.sessions.len();
                continue;
            };

            let current_minutes = timestamps.get(&existed.app_name).map(|t| t.total_played.get()).unwrap_or(0.0);
            let existing_sessions = sessions_file.sessions.get(&existed.app_name).map(Vec::as_slice).unwrap_or(&[]);
            let new_sessions = count_new_sessions(existing_sessions, &mapped.sessions);
            preview.new_sessions += new_sessions;

            let from_playnite = status::resolve_playnite_status_id(pid, &incoming_statuses, &statuses);
            let (_, decision) = status::decide_status_merge(
                existed.completion_status_id.as_deref(),
                existed.playnite_completion_status_id.as_deref(),
                pid,
                &from_playnite,
                &statuses,
            );

            let mut changed = false;
            if mapped.playtime_minutes > current_minutes {
                preview.playtime_updates.push(PlayniteMergePreviewItem {
                    title: game.name.clone(),
                    playnite_id: game.id.clone(),
                    heroic_minutes: Some(current_minutes.into()),
                    playnite_minutes: Some(mapped.playtime_minutes.into()),
                    ..Default::default()
                });
                changed = true;
            }
            let status_item = || PlayniteMergePreviewItem {
                title: game.name.clone(),
                playnite_id: game.id.clone(),
                heroic_status: Some(name_of(existed.completion_status_id.as_deref())),
                playnite_status: Some(playnite_name(pid)),
                ..Default::default()
            };
            match decision {
                status::StatusMergeDecision::TakePlaynite => {
                    preview.status_from_playnite.push(status_item());
                    changed = true;
                }
                status::StatusMergeDecision::Conflict => {
                    preview.status_conflicts.push(status_item());
                    changed = true;
                }
                status::StatusMergeDecision::Unchanged => {}
            }
            if new_sessions > 0 {
                changed = true;
            }
            if !changed {
                preview.unchanged += 1;
            }
        }
        Ok(preview)
    }

    fn saved_library_path(&self) -> Result<Option<String>> {
        Ok(self
            .last_library_path()?
            .filter(|p| Path::new(p).join("games.db").exists()))
    }

    // -- import / merge -----------------------------------------------------

    /// `mergePlayniteLibrary`: re-imports the saved library with
    /// `mergeExisting`.
    pub fn merge(&self) -> Result<PlayniteImportResult> {
        let Some(library_path) = self.saved_library_path()? else {
            return Ok(PlayniteImportResult { errors: vec![NO_LIBRARY_SAVED.into()], ..Default::default() });
        };
        log::info!("Merging Playnite library from {library_path}");
        self.import(&PlayniteImportArgs {
            library_path,
            drive_map: self.last_drive_map()?,
            fetch_covers: Some(true),
            merge_existing: true,
        })
    }

    /// `importPlayniteLibrary`.
    pub fn import(&self, args: &PlayniteImportArgs) -> Result<PlayniteImportResult> {
        let mut result = PlayniteImportResult::default();
        let dump = load_playnite_library(&args.library_path)?;
        let drive_map = args.drive_map.as_slice();
        let probe = self.probe.as_ref();

        let incoming_statuses: Vec<(String, String)> =
            dump.completion_statuses.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        save_import_context(&self.paths, &incoming_statuses, &dump.library_path, drive_map)?;
        let statuses = status::load_statuses(&self.paths)?;
        log::info!("Importing Playnite library from {} ({} games)", args.library_path, dump.games.len());

        let stores = StoreIndexes::load(&self.paths);
        let mut local: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        let mut by_playnite = playnite_index(&local);

        let global = GlobalSettings::load(&self.paths);
        let steam_bin = find_steam_binary(probe);
        let steam_installed: OnceCell<HashSet<String>> = OnceCell::new();
        let steam_is_installed = |app_id: &str| -> bool {
            steam_installed
                .get_or_init(|| installed_app_ids(&SteamLocator::detect(&global, probe)))
                .contains(app_id)
        };

        let mut batch = Batch::default();
        let mut emulator_cache: HashMap<String, Option<heronite_launch::emulators::UserEmulator>> = HashMap::new();

        for (game_index, game) in dump.games.iter().enumerate() {
            let outcome: Result<()> = (|| {
                let sessions = playnite_sessions_to_local(&game.id, &dump.sessions_by_game_id);
                let mut mapped = map_playnite_game(game, &dump.emulators, sessions, drive_map, probe);

                if mapped.meta.launch_kind == "emulator" {
                    if let Some(emulator_id) = mapped.meta.emulator_id.clone() {
                        if let Some(playnite_emu) = dump.emulators.iter().find(|e| e.id == emulator_id) {
                            if !emulator_cache.contains_key(&emulator_id) {
                                let user = ensure_playnite_emulator(&self.paths, playnite_emu, drive_map)?;
                                emulator_cache.insert(emulator_id.clone(), user);
                            }
                            if let Some(user) = emulator_cache[&emulator_id].as_ref() {
                                mapped.meta.emulator_id = Some(user.id.clone());
                                mapped.meta.emulator_profile_id = user
                                    .profiles
                                    .iter()
                                    .find(|p| Some(p.id.as_str()) == mapped.meta.emulator_profile_id.as_deref())
                                    .or_else(|| user.profiles.first())
                                    .map(|p| p.id.clone());
                                mapped.meta.emulator_name = Some(user.name.clone());
                                mapped.meta.remapped_executable = Some(user.executable.clone());
                            }
                        }
                    }
                }

                let (destination, matched) = destination_for(&mapped, &stores);
                let existed: Option<LocalGameMeta> =
                    by_playnite.get(&game.id).and_then(|app| local.games.get(app)).cloned();

                let pid = mapped.meta.playnite_completion_status_id.clone();
                mapped.meta.completion_status_id = Some(match &existed {
                    Some(existed) => {
                        let from_playnite = match pid.as_deref() {
                            Some(p) => status::status_id_for_playnite(Some(p), &statuses),
                            None => status::inferred_status_id(mapped.playtime_minutes, &statuses),
                        };
                        status::decide_status_merge(
                            existed.completion_status_id.as_deref(),
                            existed.playnite_completion_status_id.as_deref(),
                            pid.as_deref(),
                            &from_playnite,
                            &statuses,
                        )
                        .0
                    }
                    None => match pid.as_deref() {
                        Some(p) => status::status_id_for_playnite(Some(p), &statuses),
                        None => status::inferred_status_id(mapped.playtime_minutes, &statuses),
                    },
                });

                if let Some(existed) = &existed {
                    mapped.meta = if args.merge_existing {
                        merge_existing_meta(existed, &mapped.meta)
                    } else {
                        spread_incoming(existed, &mapped.meta)
                    };
                }

                if destination == Destination::Skip && existed.is_none() {
                    result.skipped += 1;
                    return Ok(());
                }

                if args.merge_existing && existed.is_some() {
                    batch.add_game(&mapped, game_index, &mut local, &mut by_playnite);
                    result.sessions_imported += mapped.sessions.len();
                    result.updated += 1;
                    if mapped.meta.runner == "legendary" || mapped.meta.runner == "gog" {
                        result.matched_store += 1;
                    }
                    return Ok(());
                }

                if destination == Destination::Skip {
                    result.skipped += 1;
                    return Ok(());
                }

                if destination == Destination::Legendary || destination == Destination::Gog {
                    let Some(matched) = matched else {
                        result.skipped += 1;
                        return Ok(());
                    };
                    mapped.meta.app_name = matched;
                    mapped.meta.runner = destination.as_str().to_owned();
                    batch.add_game(&mapped, game_index, &mut local, &mut by_playnite);
                    result.sessions_imported += mapped.sessions.len();
                    result.matched_store += 1;
                    return Ok(());
                }

                let Some(mut info) = mapped.game_info.take() else {
                    result.skipped += 1;
                    return Ok(());
                };

                if mapped.meta.launch_kind == "steam-uri" {
                    let installed = mapped.meta.steam_app_id.as_deref().is_some_and(&steam_is_installed);
                    info.is_installed = installed;
                    info.is_linux_native = Some(true);
                    info.install.executable =
                        Some(if installed { steam_bin.clone().unwrap_or_else(|| "steam".into()) } else { String::new() });
                }

                if args.fetch_covers != Some(false) {
                    let covers = self.covers.fetch(&mapped.meta, &info);
                    if covers.has_cover() {
                        info.art_cover = covers.art_cover;
                        info.art_square = covers.art_square;
                        result.covers_fetched += 1;
                    }
                }

                batch.sideload.push(info);
                batch.add_game(&mapped, game_index, &mut local, &mut by_playnite);
                if mapped.meta.launch_kind != "emulator" {
                    batch.launcher_args.push((mapped.meta.app_name.clone(), mapped.meta.launcher_args.clone()));
                }
                result.sessions_imported += mapped.sessions.len();
                if existed.is_some() {
                    result.updated += 1;
                } else {
                    result.imported += 1;
                }
                Ok(())
            })();
            if let Err(error) = outcome {
                let message = format!("Failed to import {}: {error:#}", game.name);
                log::error!("{message}");
                result.errors.push(message);
            }
        }

        if let Err(error) = batch.commit(&self.paths, &dump, probe) {
            let message = format!("Failed to save the imported library: {error:#}");
            log::error!("{message}");
            result.errors.push(message);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// `playniteIndex`: playniteId -> appName, later entries win.
fn playnite_index(local: &LocalMetaFile) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for meta in local.games.values() {
        if !meta.playnite_id.is_empty() {
            map.insert(meta.playnite_id.clone(), meta.app_name.clone());
        }
    }
    map
}

/// `destinationFor` + the matched store app name.
fn destination_for(mapped: &MappedPlayniteGame, stores: &StoreIndexes) -> (Destination, Option<String>) {
    let lookup = |idx: &StoreIndex| idx.find(mapped.meta.store_game_id.as_deref(), &mapped.meta.title);
    match mapped.destination {
        Destination::Legendary => match lookup(&stores.legendary) {
            Some(app) => (Destination::Legendary, Some(app)),
            None => (Destination::Skip, None),
        },
        Destination::Gog => match lookup(&stores.gog) {
            Some(app) => (Destination::Gog, Some(app)),
            None => (Destination::Skip, None),
        },
        other => (other, None),
    }
}

/// `mergeExistingMeta`.
fn merge_existing_meta(existed: &LocalGameMeta, incoming: &LocalGameMeta) -> LocalGameMeta {
    let mut m = existed.clone();
    if !incoming.title.is_empty() {
        m.title = incoming.title.clone();
    }
    m.notes = incoming.notes.clone().or_else(|| existed.notes.clone());
    m.plugin_id = incoming.plugin_id.clone().or_else(|| existed.plugin_id.clone());
    m.source = incoming.source.clone();
    m.steam_app_id = incoming.steam_app_id.clone().or_else(|| existed.steam_app_id.clone());
    m.store_game_id = incoming.store_game_id.clone().or_else(|| existed.store_game_id.clone());
    m.windows_install_directory = incoming.windows_install_directory.clone();
    m.windows_executable = incoming.windows_executable.clone();
    m.remapped_executable = incoming.remapped_executable.clone().or_else(|| existed.remapped_executable.clone());
    m.launcher_args = incoming.launcher_args.clone().or_else(|| existed.launcher_args.clone());
    m.roms = incoming.roms.clone().or_else(|| existed.roms.clone());
    m.emulator_name = incoming.emulator_name.clone().or_else(|| existed.emulator_name.clone());
    m.emulator_id = incoming.emulator_id.clone().or_else(|| existed.emulator_id.clone());
    m.emulator_profile_id = incoming.emulator_profile_id.clone().or_else(|| existed.emulator_profile_id.clone());
    m.platform_id = incoming.platform_id.clone().or_else(|| existed.platform_id.clone());
    m.playnite_completion_status_id = incoming.playnite_completion_status_id.clone();
    m.completion_status_id = incoming.completion_status_id.clone();
    m
}

/// `{...existed, ...incoming}`: every key the mapper sets replaces the stored
/// one (also when unset); the others (`platformId`, `selectedRomPath`,
/// `saveStateKeys`, `runAs`, unknown fields) survive.
fn spread_incoming(existed: &LocalGameMeta, incoming: &LocalGameMeta) -> LocalGameMeta {
    let mut m = existed.clone();
    m.app_name = incoming.app_name.clone();
    m.runner = incoming.runner.clone();
    m.playnite_id = incoming.playnite_id.clone();
    m.plugin_id = incoming.plugin_id.clone();
    m.source = incoming.source.clone();
    m.launch_kind = incoming.launch_kind.clone();
    m.steam_app_id = incoming.steam_app_id.clone();
    m.store_game_id = incoming.store_game_id.clone();
    m.title = incoming.title.clone();
    m.windows_install_directory = incoming.windows_install_directory.clone();
    m.windows_executable = incoming.windows_executable.clone();
    m.remapped_executable = incoming.remapped_executable.clone();
    m.launcher_args = incoming.launcher_args.clone();
    m.roms = incoming.roms.clone();
    m.emulator_name = incoming.emulator_name.clone();
    m.emulator_id = incoming.emulator_id.clone();
    m.emulator_profile_id = incoming.emulator_profile_id.clone();
    m.notes = incoming.notes.clone();
    m.playnite_completion_status_id = incoming.playnite_completion_status_id.clone();
    m.completion_status_id = incoming.completion_status_id.clone();
    m
}

// ---------------------------------------------------------------------------
// Batched writes
// ---------------------------------------------------------------------------

struct PlaytimeOp {
    app_name: String,
    minutes: f64,
    first: Option<String>,
    last: Option<String>,
}

struct SeedOp {
    app_name: String,
    runner: String,
    game_index: usize,
}

#[derive(Default)]
struct Batch {
    metas: Vec<LocalGameMeta>,
    sideload: Vec<GameInfo>,
    playtime: Vec<PlaytimeOp>,
    sessions: Vec<(String, Vec<LocalGameSession>)>,
    seeds: Vec<SeedOp>,
    launcher_args: Vec<(String, Option<String>)>,
}

impl Batch {
    /// meta upsert + writePlaytime + mergeLocalSessions + seedPlayniteMetadata,
    /// keeping the in-memory view (`local`, `by_playnite`) in step so later
    /// games see earlier ones.
    fn add_game(
        &mut self,
        mapped: &MappedPlayniteGame,
        game_index: usize,
        local: &mut LocalMetaFile,
        by_playnite: &mut HashMap<String, String>,
    ) {
        let meta = mapped.meta.clone();
        by_playnite.insert(meta.playnite_id.clone(), meta.app_name.clone());
        local.games.insert(meta.app_name.clone(), meta.clone());
        self.metas.push(meta.clone());
        self.playtime.push(PlaytimeOp {
            app_name: meta.app_name.clone(),
            minutes: mapped.playtime_minutes,
            first: mapped.first_played.clone(),
            last: mapped.last_played.clone(),
        });
        self.sessions.push((meta.app_name.clone(), mapped.sessions.clone()));
        self.seeds.push(SeedOp { app_name: meta.app_name, runner: meta.runner, game_index });
    }

    fn commit(self, paths: &HeroicPaths, dump: &PlayniteLibraryDump, probe: &dyn Probe) -> Result<()> {
        if !self.sideload.is_empty() {
            json_store::update::<SideloadLibraryFile, _>(&paths.sideload_library(), |file| {
                for game in self.sideload {
                    upsert_sideload_game(file, game);
                }
                Ok(())
            })?;
        }
        if !self.metas.is_empty() {
            json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
                for meta in self.metas {
                    file.games.insert(meta.app_name.clone(), meta);
                }
                Ok(())
            })?;
        }
        if !self.playtime.is_empty() {
            json_store::update::<TimestampFile, _>(&paths.timestamps(), |file| {
                for op in &self.playtime {
                    write_playtime_into(file, &op.app_name, op.minutes, op.first.as_deref(), op.last.as_deref());
                }
                Ok(())
            })?;
        }
        if !self.sessions.is_empty() {
            json_store::update::<SessionsFile, _>(&paths.sessions(), |file| {
                for (app_name, incoming) in &self.sessions {
                    let list = file.sessions.entry(app_name.clone()).or_default();
                    merge_sessions_into(list, incoming);
                }
                Ok(())
            })?;
        }
        if !self.seeds.is_empty() {
            let priority: IndexMap<String, Vec<String>> = settings::load(paths)?.metadata.field_priority;
            json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
                for seed in &self.seeds {
                    let game = &dump.games[seed.game_index];
                    seed_playnite_metadata(
                        paths,
                        file,
                        &priority,
                        &seed.app_name,
                        &seed.runner,
                        game,
                        Path::new(&dump.library_path),
                    )?;
                }
                Ok(())
            })?;
        }
        let home = probe.home();
        for (app_name, args) in &self.launcher_args {
            apply_launcher_args(paths, app_name, args.as_deref(), &home)
                .with_context(|| format!("saving launcher args of {app_name}"))?;
        }
        Ok(())
    }
}
