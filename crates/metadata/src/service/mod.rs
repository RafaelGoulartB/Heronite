//! The metadata service: provider orchestration, preview/apply, bulk jobs and
//! idle auto-fill. Port of `metadata/fetch.ts` plus thin entry points for
//! everything else the Collection UI calls (art, Steam details/heroes,
//! covers, web image search, HowLongToBeat).
//!
//! Everything is blocking. Run calls on worker threads; the service is
//! `Send + Sync`, keep it in an `Arc`.

mod bulk;

pub use bulk::AUTO_FILL_BATCH;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use heronite_data::field_names::NameRules;
use heronite_data::json_store;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::paths::DataPaths;
use heronite_data::settings;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::art;
use crate::edit;
use crate::hltb::{self, HltbClient, HltbLookup};
use crate::http::{Endpoints, Http};
use crate::igdb::{Igdb, IgdbCredentials};
use crate::images::materialize_metadata_image;
use crate::lutris::fetch_lutris_metadata;
use crate::merge::{merge_metadata, with_preferred_source, MergeArgs, MergeCandidate};
use crate::providers::{
    fetch_playnite_metadata, metadata_from_playnite_game, metadata_from_store_game, PlayniteGame, PlayniteSource, StoreLibraries,
};
use crate::steam::{self, SteamAppDetails, SteamState};
use crate::store::{as_runner, current_or_blank, get_game_metadata, list_game_metadata, upsert_game_metadata};
use crate::types::*;
use crate::util::now_iso;
use crate::{covers, web_images};

pub struct MetadataService {
    pub paths: DataPaths,
    pub http: Http,
    igdb: Igdb,
    steam: SteamState,
    hltb: HltbClient,
    stores: StoreLibraries,
    playnite: RwLock<Option<Arc<dyn PlayniteSource>>>,
    bulk: Mutex<BulkProgress>,
    bulk_cancel: AtomicBool,
    bulk_running: AtomicBool,
    filling: AtomicBool,
}

impl MetadataService {
    pub fn new(paths: DataPaths) -> Self {
        Self::with_endpoints(paths, Endpoints::default())
    }

    /// Custom base URLs (tests point them at a local server).
    pub fn with_endpoints(paths: DataPaths, endpoints: Endpoints) -> Self {
        Self {
            paths,
            http: Http::new(endpoints),
            igdb: Igdb::new(),
            steam: SteamState::default(),
            hltb: HltbClient::new(),
            stores: StoreLibraries::default(),
            playnite: RwLock::new(None),
            bulk: Mutex::new(BulkProgress::default()),
            bulk_cancel: AtomicBool::new(false),
            bulk_running: AtomicBool::new(false),
            filling: AtomicBool::new(false),
        }
    }

    /// Where the Playnite candidate reads its data (`getCachedPlayniteDump`).
    pub fn set_playnite_source(&self, source: Option<Arc<dyn PlayniteSource>>) {
        *self.playnite.write() = source;
    }

    /// Effective (normalized) `settings.metadata`.
    pub fn metadata_settings(&self) -> MetadataSettings {
        settings::load(&self.paths).map(|s| s.metadata).unwrap_or_else(|_| settings::normalize_metadata(None))
    }

    fn credentials(&self) -> IgdbCredentials {
        let m = self.metadata_settings();
        IgdbCredentials::new(&m.igdb_client_id, &m.igdb_client_secret)
    }

    /// `igdbConfigured`.
    pub fn igdb_configured(&self) -> bool {
        self.credentials().configured()
    }

    /// `testIgdbCredentials` with the saved credentials.
    pub fn test_igdb_credentials(&self) -> IgdbCredentialTest {
        self.igdb.test_credentials(&self.http, &self.credentials())
    }

    /// Same with credentials typed in the settings dialog (not saved).
    pub fn test_igdb_credentials_with(&self, client_id: &str, client_secret: &str) -> IgdbCredentialTest {
        self.igdb.test_credentials(&self.http, &IgdbCredentials::new(client_id, client_secret))
    }

    /// `searchGameMetadata`: IGDB title search for the manual picker.
    pub fn search_game_metadata(&self, title: &str) -> Vec<MetadataSearchHit> {
        self.igdb.search(&self.http, &self.credentials(), title)
    }

    /// `getCollectionGameMetadata`.
    pub fn get_game_metadata(&self, runner: &str, app_name: &str) -> Result<Option<CollectionGameMetadata>> {
        get_game_metadata(&self.paths, as_runner(runner), app_name)
    }

    /// `getAllCollectionMetadata`: every record without description, notes and
    /// websites.
    pub fn all_metadata(&self) -> Result<IndexMap<String, CollectionGameMetadata>> {
        list_game_metadata(&self.paths)
    }

    fn steam_id_for(&self, args: &MetadataApplyArgs) -> Option<String> {
        if let Some(id) = args.steam_app_id.clone().filter(|s| !s.is_empty()) {
            return Some(id);
        }
        let meta: LocalMetaFile = json_store::read(&self.paths.local_meta()).ok()?;
        meta.games.get(&args.app_name)?.steam_app_id.clone().filter(|s| !s.is_empty())
    }

    fn candidate_for(&self, source: &str, game: &MetadataApplyArgs) -> Option<MetadataCandidate> {
        let (app, runner) = (game.app_name.as_str(), game.runner.as_str());
        let metadata = match source {
            "igdb" => {
                self.igdb.fetch(&self.http, &self.credentials(), app, runner, &game.title, self.steam_id_for(game).as_deref(), game.igdb_id)
            }
            "steam" => steam::fetch_steam_metadata(
                &self.paths,
                &self.http,
                &self.steam,
                app,
                runner,
                self.steam_id_for(game).as_deref(),
                game.language.as_deref(),
            ),
            "store" => self.stores.find(&self.paths, runner, app).map(|g| metadata_from_store_game(app, runner, &g)),
            "lutris" => fetch_lutris_metadata(&self.http, app, runner, &game.title, game.lutris_slug.as_deref()),
            "playnite" => {
                let src = self.playnite.read().clone();
                fetch_playnite_metadata(&self.paths, src.as_deref(), app, runner)
            }
            _ => None,
        }?;
        let match_year = metadata.release_date.as_deref().and_then(|d| {
            let head: String = d.chars().take(4).collect();
            head.trim().parse::<i32>().ok().filter(|y| *y != 0)
        });
        Some(MetadataCandidate { source: source.to_owned(), match_name: metadata.title.clone(), match_year, metadata })
    }

    /// `gatherCandidates`: asks every provider at once (IGDB calls queue
    /// among themselves) and keeps those that answered, in provider order.
    pub fn gather_candidates(&self, game: &MetadataApplyArgs) -> Vec<MetadataCandidate> {
        let current = get_game_metadata(&self.paths, &game.runner, &game.app_name).ok().flatten();
        let mut with_ids = game.clone();
        with_ids.igdb_id = game.igdb_id.or_else(|| current.as_ref().and_then(|c| c.igdb_id).map(|n| n.get() as i64));
        with_ids.lutris_slug = game.lutris_slug.clone().or_else(|| current.as_ref().and_then(|c| c.lutris_slug.clone()));
        std::thread::scope(|scope| {
            let handles: Vec<_> = PROVIDER_ORDER
                .iter()
                .map(|source| {
                    let with_ids = &with_ids;
                    scope.spawn(move || self.candidate_for(source, with_ids))
                })
                .collect();
            handles
                .into_iter()
                .zip(PROVIDER_ORDER)
                .filter_map(|(h, source)| match h.join() {
                    Ok(c) => c,
                    Err(_) => {
                        log::warn!("Metadata source {source} failed for {}: panicked", game.title);
                        None
                    }
                })
                .collect()
        })
    }

    /// `previewGameMetadata`: what each provider returned and what the merge
    /// would store. Nothing is written.
    pub fn preview_game_metadata(&self, args: &MetadataApplyArgs) -> Result<MetadataPreview> {
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner.clone();
        let candidates = self.gather_candidates(&args);

        let has_picks = args.field_picks.as_ref().is_some_and(|p| !p.is_empty());
        let overwrite = args.overwrite_existing != Some(false);
        let preserve_manual = if has_picks { false } else { args.preserve_manual.unwrap_or(args.overwrite_existing == Some(false)) };
        let priority = with_preferred_source(&self.metadata_settings().field_priority, args.source.as_deref());
        let merge_candidates: Vec<MergeCandidate<'_>> =
            candidates.iter().map(|c| MergeCandidate { source: &c.source, metadata: &c.metadata }).collect();
        let mut proposed = merge_metadata(MergeArgs {
            app_name: &args.app_name,
            runner: &runner,
            current: current.as_ref(),
            candidates: &merge_candidates,
            priority: &priority,
            overwrite_existing: overwrite,
            preserve_manual,
            field_picks: args.field_picks.as_ref(),
        });
        // Names the user renamed, merged or removed in the fields manager.
        NameRules::load(&self.paths).apply_metadata(&mut proposed);
        Ok(MetadataPreview { app_name: args.app_name.clone(), current, proposed, candidates, igdb_configured: self.igdb_configured() })
    }

    /// `persistImages`.
    fn persist_images(&self, mut md: CollectionGameMetadata, download: bool) -> CollectionGameMetadata {
        md.cover_url = materialize_metadata_image(
            &self.paths,
            &self.http,
            &md.runner,
            &md.app_name,
            ArtKind::Cover,
            md.cover_url.as_deref(),
            download,
        );
        md.hero_url =
            materialize_metadata_image(&self.paths, &self.http, &md.runner, &md.app_name, ArtKind::Hero, md.hero_url.as_deref(), download);
        md
    }

    /// `applyGameMetadata`: fetch, merge and store. When no provider answers
    /// the record is stored with `fetchError` so the idle fill retries after
    /// a week.
    pub fn apply_game_metadata(&self, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        let preview = self.preview_game_metadata(args)?;
        if preview.candidates.is_empty() {
            let mut failed = current_or_blank(&self.paths, as_runner(&args.runner), &args.app_name)?;
            failed.fetch_error = Some("No metadata sources returned data".into());
            failed.fetched_at = Some(now_iso());
            return upsert_game_metadata(&self.paths, failed);
        }
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let saved = self.persist_images(preview.proposed, download);
        upsert_game_metadata(&self.paths, saved)
    }

    /// `seedPlayniteMetadata`: first import of a Playnite game's own metadata
    /// (skipped when the game already has field sources).
    pub fn seed_playnite_metadata(
        &self,
        app_name: &str,
        runner: &str,
        game: &PlayniteGame,
        library_path: &std::path::Path,
    ) -> Result<CollectionGameMetadata> {
        let runner = as_runner(runner);
        let existing = get_game_metadata(&self.paths, runner, app_name)?;
        if let Some(e) = existing.as_ref().filter(|e| !e.field_sources.is_empty()) {
            return Ok(e.clone());
        }
        let from_playnite = metadata_from_playnite_game(app_name, runner, game, library_path);
        let mut merged = merge_metadata(MergeArgs {
            app_name,
            runner,
            current: existing.as_ref(),
            candidates: &[MergeCandidate { source: "playnite", metadata: &from_playnite }],
            priority: &self.metadata_settings().field_priority,
            overwrite_existing: false,
            preserve_manual: true,
            field_picks: None,
        });
        NameRules::load(&self.paths).apply_metadata(&mut merged);
        upsert_game_metadata(&self.paths, self.persist_images(merged, false))
    }

    /// `updateCollectionGameDetails`.
    pub fn update_game_details(&self, patch: &DetailsPatch) -> Result<DetailsResult> {
        edit::update_collection_game_details(&self.paths, patch)
    }

    /// Notes only (`updateCollectionGameDetails({ notes })`).
    pub fn set_notes(&self, runner: &str, app_name: &str, notes: &str) -> Result<DetailsResult> {
        self.update_game_details(&DetailsPatch {
            app_name: app_name.into(),
            runner: runner.into(),
            notes: Some(notes.into()),
            ..Default::default()
        })
    }

    pub fn get_collection_art(&self, runner: &str, app_name: &str) -> Result<CollectionGameArt> {
        art::get_collection_game_art(&self.paths, runner, app_name)
    }

    pub fn all_collection_art(&self) -> Result<IndexMap<String, CollectionGameArt>> {
        art::get_all_collection_art(&self.paths)
    }

    /// Custom cover/hero from a local file.
    pub fn set_art_from_file(&self, runner: &str, app_name: &str, kind: ArtKind, source_path: &str) -> Result<CollectionGameArt> {
        art::set_collection_game_art(&self.paths, runner, app_name, kind, source_path)
    }

    /// Custom cover/hero downloaded from a URL (`thumb_url` is the fallback).
    pub fn set_art_from_url(
        &self,
        runner: &str,
        app_name: &str,
        kind: ArtKind,
        image_url: &str,
        thumb_url: Option<&str>,
    ) -> Result<CollectionGameArt> {
        art::set_collection_game_art_from_url(&self.paths, &self.http, runner, app_name, kind, image_url, thumb_url)
    }

    /// Back to the default art for `kind`.
    pub fn reset_art(&self, runner: &str, app_name: &str, kind: ArtKind) -> Result<CollectionGameArt> {
        art::clear_collection_game_art(&self.paths, runner, app_name, kind)
    }

    /// `getSteamAppDetails` (Heroic language code, e.g. `pt-BR`).
    pub fn steam_details(&self, steam_app_id: &str, language: Option<&str>) -> Option<SteamAppDetails> {
        steam::get_steam_app_details(&self.paths, &self.http, &self.steam, steam_app_id, language)
    }

    /// `cacheSteamHero`.
    pub fn cache_steam_hero(&self, steam_app_id: &str) -> Option<SteamHeroCacheResult> {
        steam::cache_steam_hero(&self.paths, &self.http, &self.steam, steam_app_id)
    }

    /// `warmSteamHeroes`.
    pub fn warm_steam_heroes(&self, steam_app_ids: &[String]) {
        steam::warm_steam_heroes(&self.paths, &self.http, &self.steam, steam_app_ids);
    }

    /// `refreshMissingLocalCovers`.
    pub fn refresh_missing_local_covers(&self) -> Result<usize> {
        covers::refresh_missing_local_covers(&self.paths, &self.http)
    }

    /// `searchCollectionWebImages`.
    pub fn search_web_images(&self, query: &str, transparent: bool) -> Result<Vec<WebImage>> {
        web_images::search_web_images(&self.http, query, transparent)
    }

    /// HowLongToBeat for a game (cache first; see [`hltb::get_or_fetch_hltb`]).
    pub fn how_long_to_beat(&self, title: &str, store_title: Option<&str>) -> Result<HltbLookup> {
        hltb::get_or_fetch_hltb(&self.paths, &self.http, &self.hltb, title, store_title)
    }
}
