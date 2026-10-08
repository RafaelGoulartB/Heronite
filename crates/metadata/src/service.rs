//! The metadata service: provider orchestration, preview/apply, bulk jobs and
//! idle auto-fill. Port of `metadata/fetch.ts` plus thin entry points for
//! everything else the Collection UI calls (art, Steam details/heroes,
//! covers, web image search, HowLongToBeat).
//!
//! Everything is blocking. Run calls on worker threads; the service is
//! `Send + Sync`, keep it in an `Arc`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::paths::HeroicPaths;
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
    fetch_playnite_metadata, metadata_from_playnite_game, metadata_from_store_game, PlayniteGame, PlayniteSource,
    StoreLibraries,
};
use crate::steam::{self, SteamAppDetails, SteamState};
use crate::store::{as_runner, current_or_blank, get_game_metadata, list_game_metadata, upsert_game_metadata};
use crate::types::*;
use crate::util::{now_iso, now_ms};
use crate::{covers, web_images};

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Games filled per idle pass.
pub const AUTO_FILL_BATCH: usize = 40;

pub struct MetadataService {
    pub paths: HeroicPaths,
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
    pub fn new(paths: HeroicPaths) -> Self {
        Self::with_endpoints(paths, Endpoints::default())
    }

    /// Custom base URLs (tests point them at a local server).
    pub fn with_endpoints(paths: HeroicPaths, endpoints: Endpoints) -> Self {
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

    // -- settings ----------------------------------------------------------

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

    // -- stored metadata ---------------------------------------------------

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

    // -- candidates --------------------------------------------------------

    fn candidate_for(&self, source: &str, game: &MetadataApplyArgs) -> Option<MetadataCandidate> {
        let (app, runner) = (game.app_name.as_str(), game.runner.as_str());
        let metadata = match source {
            "igdb" => self.igdb.fetch(
                &self.http,
                &self.credentials(),
                app,
                runner,
                &game.title,
                self.steam_id_for(game).as_deref(),
                game.igdb_id,
            ),
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
        let proposed = merge_metadata(MergeArgs {
            app_name: &args.app_name,
            runner: &runner,
            current: current.as_ref(),
            candidates: &merge_candidates,
            priority: &priority,
            overwrite_existing: overwrite,
            preserve_manual,
            field_picks: args.field_picks.as_ref(),
        });
        Ok(MetadataPreview {
            app_name: args.app_name.clone(),
            current,
            proposed,
            candidates,
            igdb_configured: self.igdb_configured(),
        })
    }

    /// `persistImages`.
    fn persist_images(&self, mut md: CollectionGameMetadata, download: bool) -> CollectionGameMetadata {
        md.cover_url = materialize_metadata_image(
            &self.paths, &self.http, &md.runner, &md.app_name, ArtKind::Cover, md.cover_url.as_deref(), download,
        );
        md.hero_url = materialize_metadata_image(
            &self.paths, &self.http, &md.runner, &md.app_name, ArtKind::Hero, md.hero_url.as_deref(), download,
        );
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
        let merged = merge_metadata(MergeArgs {
            app_name,
            runner,
            current: existing.as_ref(),
            candidates: &[MergeCandidate { source: "playnite", metadata: &from_playnite }],
            priority: &self.metadata_settings().field_priority,
            overwrite_existing: false,
            preserve_manual: true,
            field_picks: None,
        });
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

    // -- bulk --------------------------------------------------------------

    /// `getBulkMetadataProgress`.
    pub fn bulk_progress(&self) -> BulkProgress {
        self.bulk.lock().clone()
    }

    fn publish(&self, next: BulkProgress, on_progress: &dyn Fn(&BulkProgress)) {
        *self.bulk.lock() = next.clone();
        on_progress(&next);
    }

    /// `cancelBulkMetadata`: the job stops before the next game.
    pub fn cancel_bulk_metadata(&self, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        if self.bulk_running.load(Ordering::SeqCst) {
            self.bulk_cancel.store(true, Ordering::SeqCst);
            let mut p = self.bulk_progress();
            p.state = BulkState::Cancelling;
            self.publish(p, on_progress);
        }
        self.bulk_progress()
    }

    /// `startBulkMetadata`: runs the job on its own thread and returns the
    /// initial progress; `on_progress` is called after every step (and for
    /// `cancel_bulk_metadata`) from that thread. Ignored while a job runs.
    pub fn start_bulk_metadata(
        self: &Arc<Self>,
        args: BulkArgs,
        on_progress: impl Fn(&BulkProgress) + Send + Sync + 'static,
    ) -> BulkProgress {
        if self.bulk_running.swap(true, Ordering::SeqCst) {
            return self.bulk_progress();
        }
        self.bulk_cancel.store(false, Ordering::SeqCst);
        let initial = BulkProgress {
            state: BulkState::Running,
            total: args.games.len(),
            percent: if args.games.is_empty() { 100 } else { 0 },
            ..Default::default()
        };
        self.publish(initial.clone(), &on_progress);
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("metadata-bulk".into())
            .spawn(move || this.run_bulk(&args, &on_progress))
            .expect("spawn bulk thread");
        initial
    }

    /// Blocking variant of [`start_bulk_metadata`](Self::start_bulk_metadata)
    /// for callers that already have a worker thread (and for tests).
    pub fn run_bulk_metadata(&self, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        if self.bulk_running.swap(true, Ordering::SeqCst) {
            return self.bulk_progress();
        }
        self.bulk_cancel.store(false, Ordering::SeqCst);
        self.run_bulk(args, on_progress)
    }

    /// `runBulkMetadata` (expects `bulk_running` already set).
    fn run_bulk(&self, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        struct Done<'a>(&'a MetadataService);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.bulk_running.store(false, Ordering::SeqCst);
                self.0.bulk_cancel.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(self);

        let total = args.games.len();
        let overwrite = args.overwrite_existing == Some(true);
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let mut errors: Vec<String> = Vec::new();
        let (mut updated, mut skipped, mut failed) = (0usize, 0usize, 0usize);

        if total == 0 {
            let done = BulkProgress { state: BulkState::Done, percent: 100, ..Default::default() };
            self.publish(done.clone(), on_progress);
            return done;
        }

        for (index, game) in args.games.iter().enumerate() {
            if self.bulk_cancel.load(Ordering::SeqCst) {
                break;
            }
            self.publish(
                BulkProgress {
                    state: BulkState::Running,
                    total,
                    processed: index,
                    updated,
                    skipped,
                    failed,
                    percent: (index as f64 / total as f64 * 100.0).round() as u32,
                    current_title: Some(game.title.clone()),
                    errors: errors.iter().take(8).cloned().collect(),
                },
                on_progress,
            );
            let step = || -> Result<Option<CollectionGameMetadata>> {
                let current = get_game_metadata(&self.paths, as_runner(&game.runner), &game.app_name)?;
                if !overwrite && current.as_ref().is_some_and(|c| c.fetched_at.as_deref().is_some_and(|f| !f.is_empty()) && c.fetch_error.is_none()) {
                    return Ok(None);
                }
                self.apply_game_metadata(&MetadataApplyArgs {
                    app_name: game.app_name.clone(),
                    runner: game.runner.clone(),
                    title: game.title.clone(),
                    steam_app_id: game.steam_app_id.clone(),
                    source: Some("auto".into()),
                    overwrite_existing: Some(overwrite),
                    preserve_manual: Some(args.preserve_manual != Some(false)),
                    download_images: Some(download),
                    ..Default::default()
                })
                .map(Some)
            };
            match step() {
                Ok(None) => skipped += 1,
                Ok(Some(saved)) => match saved.fetch_error {
                    Some(err) => {
                        failed += 1;
                        errors.push(format!("{}: {err}", game.title));
                    }
                    None => updated += 1,
                },
                Err(e) => {
                    failed += 1;
                    errors.push(format!("{}: {e}", game.title));
                }
            }
        }

        let processed = updated + skipped + failed;
        let done = BulkProgress {
            state: BulkState::Done,
            total,
            processed,
            updated,
            skipped,
            failed,
            percent: (processed as f64 / total as f64 * 100.0).round() as u32,
            current_title: None,
            errors: errors.into_iter().take(8).collect(),
        };
        self.publish(done.clone(), on_progress);
        done
    }

    // -- idle auto-fill ----------------------------------------------------

    /// `refreshMissingCollectionMetadata`: when `autoFillMissing` is on,
    /// fetches metadata for up to 40 overlay games that have none (or whose
    /// last attempt failed more than a week ago) and returns how many got
    /// data. Call it when the app is idle.
    pub fn refresh_missing_collection_metadata(&self) -> Result<usize> {
        self.refresh_missing_while(&|| false)
    }

    /// Same, checking `should_stop` before each game.
    pub fn refresh_missing_while(&self, should_stop: &dyn Fn() -> bool) -> Result<usize> {
        if self.filling.swap(true, Ordering::SeqCst) {
            return Ok(0);
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&self.filling);

        let settings = self.metadata_settings();
        if !settings.auto_fill_missing {
            return Ok(0);
        }
        let overlay: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        let mut missing = Vec::new();
        for meta in overlay.games.values() {
            let current = get_game_metadata(&self.paths, &meta.runner, &meta.app_name)?;
            let needs = match current {
                None => true,
                Some(c) if c.fetch_error.is_some() => match c.fetched_at.as_deref() {
                    None | Some("") => true,
                    Some(at) => chrono::DateTime::parse_from_rfc3339(at)
                        .map(|t| now_ms() - t.timestamp_millis() > WEEK_MS)
                        .unwrap_or(false),
                },
                Some(_) => false,
            };
            if needs {
                missing.push(meta);
            }
        }
        if missing.is_empty() {
            return Ok(0);
        }
        log::info!("Filling Collection metadata for {} game(s)", missing.len());
        let mut filled = 0;
        for meta in missing.into_iter().take(AUTO_FILL_BATCH) {
            if should_stop() {
                break;
            }
            let saved = self.apply_game_metadata(&MetadataApplyArgs {
                app_name: meta.app_name.clone(),
                runner: meta.runner.clone(),
                title: meta.title.clone(),
                steam_app_id: meta.steam_app_id.clone(),
                source: Some("auto".into()),
                overwrite_existing: Some(false),
                preserve_manual: Some(true),
                download_images: Some(settings.download_images),
                ..Default::default()
            })?;
            if saved.fetch_error.is_none() {
                filled += 1;
            }
        }
        Ok(filled)
    }

    // -- art ---------------------------------------------------------------

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

    // -- Steam -------------------------------------------------------------

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

    // -- web images / HLTB -------------------------------------------------

    /// `searchCollectionWebImages`.
    pub fn search_web_images(&self, query: &str, transparent: bool) -> Result<Vec<WebImage>> {
        web_images::search_web_images(&self.http, query, transparent)
    }

    /// HowLongToBeat for a game (cache first; see [`hltb::get_or_fetch_hltb`]).
    pub fn how_long_to_beat(&self, title: &str, store_title: Option<&str>) -> Result<HltbLookup> {
        hltb::get_or_fetch_hltb(&self.paths, &self.http, &self.hltb, title, store_title)
    }
}
