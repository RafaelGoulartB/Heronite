//! The metadata service: provider orchestration, preview/apply, bulk jobs and
//! idle auto-fill. Port of `metadata/fetch.ts` plus thin entry points for
//! everything else the Collection UI calls (art, Steam details/heroes,
//! covers, web image search, HowLongToBeat).
//!
//! Everything is blocking. Run calls on worker threads; the service is
//! `Send + Sync`, keep it in an `Arc`.

mod age_fill;
mod attention;
mod bulk;
mod counts;
mod families;
mod fetch;
mod gather;
mod job_run;
mod launchbox_art;
mod picture_sizes;
mod review;
mod source_settings;
mod tag_fill;
mod undo;

pub use attention::Attention;
pub use bulk::AUTO_FILL_BATCH;
pub use families::FamilyLack;
pub use fetch::{is_sent_to_review, SentToReview};
pub use review::{MatchUnavailable, Resolution, Resolved, SourceSearch};

/// The `fetchError` of a game no source had.
const NO_DATA: &str = "No metadata sources returned data";

/// No online provider could be reached, so nothing was saved: the game stays
/// unfetched and the next fill or manual fetch tries again.
#[derive(Debug)]
pub struct Offline;

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", tr!("Could not reach the metadata services. Check your connection and try again."))
    }
}

impl std::error::Error for Offline {}

/// Whether `err` is [`Offline`].
pub fn is_offline(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Offline>().is_some()
}

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use anyhow::Result;
use heronite_data::field_names::NameRules;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, MetadataSettings};
use heronite_data::paths::DataPaths;
use heronite_i18n::tr;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::art;
use crate::edit;
use crate::hltb::{self, HltbClient, HltbLookup, HltbQuery, HowLongToBeat, SearchHit};
use crate::http::{Endpoints, Http};
use crate::igdb::{Igdb, IgdbCredentials};
use crate::merge::{merge_metadata, MergeArgs, MergeCandidate};
use crate::providers::{metadata_from_playnite_game, PlayniteGame, PlayniteSource, StoreLibraries};
use crate::sources::rows::TestResults;
use crate::sources::{self, ids, Shared, SourceRegistry};
use crate::steam::{self, SteamAppDetails, SteamState};
use crate::store::{as_runner, get_game_metadata, list_game_metadata, upsert_game_metadata};
use crate::types::*;
use crate::{covers, web_images};

pub struct MetadataService {
    pub paths: DataPaths,
    pub http: Http,
    igdb: Arc<Igdb>,
    steam: Arc<SteamState>,
    hltb: HltbClient,
    playnite: Arc<RwLock<Option<Arc<dyn PlayniteSource>>>>,
    /// Swapped whole when an extension is added or removed; readers keep
    /// the one they took.
    sources: RwLock<Arc<SourceRegistry>>,
    tests: TestResults,
    /// Bulk jobs: progress, queue and the last one's undo.
    jobs: Mutex<bulk::Jobs>,
    /// The id of the job asked to stop (ids start at 1).
    stop_job: AtomicU64,
    /// Held by an Undo while it writes and by a job while it starts, so
    /// the two never overlap.
    undo_gate: Mutex<()>,
    filling: AtomicBool,
    rating_fill: AtomicBool,
    /// Bumped on every change of the review queue this service makes.
    review_revision: AtomicU64,
    /// The Steam tags file, read when a fallback source looks for its kind of game.
    steam_tags: gather::SteamTagsCache,
    /// The sources' downloads running now, by source id.
    downloads: source_settings::Downloads,
    /// LaunchBox's index, shared with its source: the Covers dialog asks it too.
    launchbox: Arc<crate::launchbox::LaunchBox>,
    /// The picture sizes read for the dialogs.
    sizes: picture_sizes::Sizes,
}

impl MetadataService {
    pub fn new(paths: DataPaths) -> Self {
        Self::with_endpoints(paths, Endpoints::default())
    }

    /// Custom base URLs (tests point them at a local server).
    pub fn with_endpoints(paths: DataPaths, endpoints: Endpoints) -> Self {
        let igdb = Arc::new(Igdb::new());
        let steam = Arc::new(SteamState::default());
        let playnite = Arc::new(RwLock::new(None));
        let launchbox = Arc::new(crate::launchbox::LaunchBox::new());
        let sources = SourceRegistry::builtin(Shared {
            igdb: igdb.clone(),
            steam: steam.clone(),
            stores: Arc::new(StoreLibraries::default()),
            playnite: playnite.clone(),
            launchbox: launchbox.clone(),
        });
        Self {
            paths,
            http: Http::new(endpoints),
            igdb,
            steam,
            hltb: HltbClient::new(),
            playnite,
            sources: RwLock::new(Arc::new(sources)),
            tests: TestResults::default(),
            jobs: Mutex::new(bulk::Jobs::default()),
            stop_job: AtomicU64::new(0),
            undo_gate: Mutex::new(()),
            filling: AtomicBool::new(false),
            rating_fill: AtomicBool::new(false),
            review_revision: AtomicU64::new(0),
            steam_tags: Default::default(),
            downloads: Default::default(),
            launchbox,
            sizes: Default::default(),
        }
    }

    /// Where the Playnite candidate reads its data (`getCachedPlayniteDump`).
    pub fn set_playnite_source(&self, source: Option<Arc<dyn PlayniteSource>>) {
        *self.playnite.write() = source;
    }

    /// The metadata sources, in the order candidates are listed: the
    /// registry as it is now (an extension added later is not in it).
    pub fn sources(&self) -> Arc<SourceRegistry> {
        self.sources.read().clone()
    }

    /// Effective (normalized) `settings.metadata`.
    pub fn metadata_settings(&self) -> MetadataSettings {
        sources::load_metadata_settings(&self.paths)
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
        let settings = self.metadata_settings();
        let from_playnite = MetadataCandidate::new(
            "playnite",
            metadata_from_playnite_game(app_name, runner, game, library_path),
            Some(MatchedBy::SourceId),
        );
        let usable = Self::usable(&settings, std::slice::from_ref(&from_playnite));
        let Some((_, from_playnite)) = usable.first() else {
            return Ok(existing.unwrap_or_else(|| crate::util::blank_metadata(app_name, runner)));
        };
        let mut merged = merge_metadata(MergeArgs {
            app_name,
            runner,
            current: existing.as_ref(),
            candidates: &[MergeCandidate { source: "playnite", metadata: from_playnite }],
            priority: &self.sources().field_orders(&settings),
            overwrite_existing: false,
            preserve_manual: true,
            field_picks: None,
            fields: None,
        });
        NameRules::load(&self.paths).apply_metadata(&mut merged);
        for field in existing.as_ref().map(crate::record::locked_fields).unwrap_or_default() {
            crate::merge::copy_field(existing.as_ref(), &mut merged, &field);
        }
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

    /// Custom art from a local file (or a screenshot): `kind` is the
    /// background or a cover family ([`ArtKind::cover`]; `Cover` is the tall
    /// one).
    pub fn set_art_from_file(&self, runner: &str, app_name: &str, kind: ArtKind, source_path: &str) -> Result<CollectionGameArt> {
        art::set_collection_game_art(&self.paths, runner, app_name, kind, source_path)
    }

    /// Custom art of `kind` downloaded from a URL (`thumb_url` is the
    /// fallback).
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

    /// Back to the default art for `kind` (one cover family, or the
    /// background); its file is removed.
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

    /// `searchCollectionWebImages`; nothing while web images are off.
    pub fn search_web_images(&self, query: &str, transparent: bool) -> Result<Vec<WebImage>> {
        self.search_web_images_shaped(query, transparent, None)
    }

    /// [`search_web_images`](Self::search_web_images) favouring pictures of
    /// a cover family's shape (`None`, or the tall family: no filter, as
    /// the search always was).
    pub fn search_web_images_shaped(&self, query: &str, transparent: bool, family: Option<CoverFamily>) -> Result<Vec<WebImage>> {
        if !self.source_on(ids::WEB_IMAGES) {
            return Ok(Vec::new());
        }
        web_images::search_web_images_shaped(&self.http, query, transparent, family)
    }

    /// The art picker's image search: SteamGridDB's art when it is on and an
    /// API key is set (not for transparent images, which it does not have)
    /// and the web's (when on), apart. A failing SteamGridDB only leaves its part empty; the web's
    /// results leave out the images SteamGridDB already gave. `kind` names
    /// the cover family ([`ArtKind::cover`]): SteamGridDB gives grids of
    /// that shape and the web search favours it.
    pub fn search_art_images(&self, query: &str, kind: ArtKind, transparent: bool) -> Result<ArtImages> {
        let key = if transparent { None } else { covers::steamgrid_key_if_on(&self.paths) };
        let title = web_images::title_of_query(query);
        let (sgdb, web) = std::thread::scope(|s| {
            let sgdb = s.spawn(|| match key.as_deref() {
                Some(key) if !title.is_empty() => covers::steamgrid_images(&self.http, key, &title, kind).unwrap_or_else(|e| {
                    log::warn!("SteamGridDB image search failed for \"{title}\": {e:#}");
                    Vec::new()
                }),
                _ => Vec::new(),
            });
            let web = self.search_web_images_shaped(query, transparent, kind.family());
            (sgdb.join().unwrap_or_default(), web)
        });
        let web = match web {
            Ok(list) => list,
            Err(e) if sgdb.is_empty() => return Err(e),
            Err(_) => Vec::new(),
        };
        let mut seen: std::collections::HashSet<String> = sgdb.iter().map(|img| img.image_url.clone()).collect();
        let web = web.into_iter().filter(|img| seen.insert(img.image_url.clone())).collect();
        Ok(ArtImages { steamgrid: sgdb, web, ..Default::default() })
    }

    /// HowLongToBeat for a game (cache first; see [`hltb::get_or_fetch_hltb`]).
    pub fn how_long_to_beat(&self, title: &str, store_title: Option<&str>) -> Result<HltbLookup> {
        hltb::get_or_fetch_hltb(&self.paths, &self.http, &self.hltb, title, store_title)
    }

    /// [`how_long_to_beat`](Self::how_long_to_beat) with the release year.
    pub fn how_long_to_beat_for(&self, query: &HltbQuery) -> Result<HltbLookup> {
        hltb::get_or_fetch_hltb_for(&self.paths, &self.http, &self.hltb, query)
    }

    /// Asks HowLongToBeat again now (see [`hltb::refresh_hltb`]).
    pub fn refresh_how_long_to_beat(&self, query: &HltbQuery) -> Result<HltbLookup> {
        hltb::refresh_hltb(&self.paths, &self.http, &self.hltb, query)
    }

    /// HowLongToBeat search results, for "Change match".
    pub fn search_how_long_to_beat(&self, title: &str) -> Result<Vec<SearchHit>> {
        self.hltb.search(&self.http, title)
    }

    /// Saves the user's HowLongToBeat match for `title` (`None`: not on
    /// HowLongToBeat).
    pub fn set_how_long_to_beat_match(&self, title: &str, hit: Option<&SearchHit>) -> Result<Option<HowLongToBeat>> {
        hltb::set_hltb_match(&self.paths, &self.http, title, hit)
    }

    /// The HowLongToBeat match the user picked for `title`, if any.
    pub fn how_long_to_beat_choice(&self, title: &str) -> Option<hltb::HltbOverride> {
        hltb::override_for(&self.paths, title)
    }

    /// Back to the automatic HowLongToBeat match for `title`.
    pub fn clear_how_long_to_beat_match(&self, title: &str) -> Result<()> {
        hltb::clear_hltb_match(&self.paths, title)
    }
}
