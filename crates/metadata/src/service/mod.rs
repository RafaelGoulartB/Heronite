//! The metadata service: provider orchestration, preview/apply, bulk jobs and
//! idle auto-fill. Port of `metadata/fetch.ts` plus thin entry points for
//! everything else the Collection UI calls (art, Steam details/heroes,
//! covers, web image search, HowLongToBeat).
//!
//! Everything is blocking. Run calls on worker threads; the service is
//! `Send + Sync`, keep it in an `Arc`.

mod age_fill;
mod bulk;
mod counts;
mod job_run;
mod source_settings;
mod tag_fill;
mod undo;

pub use bulk::AUTO_FILL_BATCH;

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

/// What the providers returned and whether the online ones answered at all.
struct Gathered {
    candidates: Vec<MetadataCandidate>,
    offline: bool,
}

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use anyhow::{bail, Result};
use heronite_data::field_names::NameRules;
use heronite_data::json_store;
use heronite_data::models::{CollectionGameArt, CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::paths::DataPaths;
use heronite_data::settings::METADATA_FIELDS;
use heronite_i18n::tr;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::art;
use crate::edit;
use crate::hltb::{self, HltbClient, HltbLookup, HltbQuery, HowLongToBeat, SearchHit};
use crate::http::{Endpoints, Http};
use crate::igdb::{Igdb, IgdbCredentials};
use crate::images::materialize_metadata_image;
use crate::merge::{clear_field, copy_field, merge_metadata, with_preferred_source, MergeArgs, MergeCandidate};
use crate::providers::{metadata_from_playnite_game, PlayniteGame, PlayniteSource, StoreLibraries};
use crate::sources::rows::TestResults;
use crate::sources::{self, ids, is_field_on, is_on, Shared, SourceContext, SourceDescriptor, SourceGame, SourceRegistry};
use crate::steam::{self, SteamAppDetails, SteamState};
use crate::store::{as_runner, current_or_blank, get_game_metadata, list_game_metadata, upsert_game_metadata};
use crate::types::*;
use crate::util::now_iso;
use crate::{covers, web_images};

pub struct MetadataService {
    pub paths: DataPaths,
    pub http: Http,
    igdb: Arc<Igdb>,
    steam: Arc<SteamState>,
    hltb: HltbClient,
    playnite: Arc<RwLock<Option<Arc<dyn PlayniteSource>>>>,
    sources: SourceRegistry,
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
        let sources = SourceRegistry::builtin(Shared {
            igdb: igdb.clone(),
            steam: steam.clone(),
            stores: Arc::new(StoreLibraries::default()),
            playnite: playnite.clone(),
        });
        Self {
            paths,
            http: Http::new(endpoints),
            igdb,
            steam,
            hltb: HltbClient::new(),
            playnite,
            sources,
            tests: TestResults::default(),
            jobs: Mutex::new(bulk::Jobs::default()),
            stop_job: AtomicU64::new(0),
            undo_gate: Mutex::new(()),
            filling: AtomicBool::new(false),
            rating_fill: AtomicBool::new(false),
        }
    }

    /// Where the Playnite candidate reads its data (`getCachedPlayniteDump`).
    pub fn set_playnite_source(&self, source: Option<Arc<dyn PlayniteSource>>) {
        *self.playnite.write() = source;
    }

    /// The metadata sources, in the order candidates are listed.
    pub fn sources(&self) -> &SourceRegistry {
        &self.sources
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

    /// `game` as the sources see it. The overlay is read only when needed:
    /// for the Steam id the arguments lack, or for a source's audience.
    fn source_game(&self, args: &MetadataApplyArgs) -> SourceGame {
        let has_steam_id = args.steam_app_id.as_deref().is_some_and(|s| !s.is_empty());
        let overlay = (!has_steam_id || self.sources.needs_overlay())
            .then(|| json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok())
            .flatten()
            .and_then(|mut file| file.games.shift_remove(&args.app_name));
        SourceGame::new(args, overlay.as_ref())
    }

    /// `gatherCandidates`: asks every provider that is on and applies to the
    /// game at once (IGDB calls queue among themselves) and keeps those that
    /// answered, in provider order.
    pub fn gather_candidates(&self, game: &MetadataApplyArgs) -> Vec<MetadataCandidate> {
        self.gather(game).candidates
    }

    fn gather(&self, game: &MetadataApplyArgs) -> Gathered {
        let current = get_game_metadata(&self.paths, &game.runner, &game.app_name).ok().flatten();
        let mut with_ids = game.clone();
        with_ids.igdb_id = game.igdb_id.or_else(|| current.as_ref().and_then(|c| c.igdb_id).map(|n| n.get() as i64));
        with_ids.lutris_slug = game.lutris_slug.clone().or_else(|| current.as_ref().and_then(|c| c.lutris_slug.clone()));
        let game_view = self.source_game(&with_ids);
        let ctx = SourceContext::new(&self.paths, &self.http);
        let settings = ctx.settings();
        let lookups: Vec<(&SourceDescriptor, Lookup<MetadataCandidate>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .sources
                .lookups()
                .map(|source| {
                    let (ctx, game_view) = (&ctx, &game_view);
                    let d = source.descriptor();
                    let ask = is_on(settings, &d.id) && d.applies_to.matches(game_view) && Self::wanted(d, game);
                    (d, ask.then(|| scope.spawn(move || source.lookup(ctx, game_view))))
                })
                .collect();
            handles
                .into_iter()
                .map(|(source, h)| match h.map(|h| h.join()) {
                    None => (source, Lookup::Skipped),
                    Some(Ok(l)) => (source, l),
                    Some(Err(_)) => {
                        log::warn!("Metadata source {} failed for {}: panicked", source.id, game.title);
                        (source, Lookup::NotFound)
                    }
                })
                .collect()
        });
        // Offline: every online provider that was asked failed to connect.
        let mut asked = lookups.iter().filter(|(s, l)| s.network && !matches!(l, Lookup::Skipped)).peekable();
        let offline = asked.peek().is_some() && asked.all(|(_, l)| matches!(l, Lookup::Unreachable));
        let candidates = lookups
            .into_iter()
            .filter_map(|(_, l)| match l {
                Lookup::Found(c) => Some(c),
                _ => None,
            })
            .collect();
        Gathered { candidates, offline }
    }

    /// Whether the arguments' `only_sources` and `only_fields` leave `d` to ask.
    fn wanted(d: &SourceDescriptor, args: &MetadataApplyArgs) -> bool {
        args.only_sources.as_ref().is_none_or(|s| s.contains(&d.id))
            && args.only_fields.as_ref().is_none_or(|f| f.iter().any(|field| d.provides(field)))
    }

    /// The candidates the merge may use: those of sources that are on, each
    /// without the fields switched off for its source.
    fn usable(settings: &MetadataSettings, candidates: &[MetadataCandidate]) -> Vec<(String, CollectionGameMetadata)> {
        candidates
            .iter()
            .filter(|c| is_on(settings, &c.source))
            .map(|c| {
                let mut md = c.metadata.clone();
                for field in METADATA_FIELDS.iter().filter(|f| !is_field_on(settings, &c.source, f)) {
                    clear_field(&mut md, field);
                }
                (c.source.clone(), md)
            })
            .collect()
    }

    /// The merge of `candidates` into `current` that a preview proposes.
    fn propose(
        &self,
        args: &MetadataApplyArgs,
        current: Option<&CollectionGameMetadata>,
        candidates: &[MetadataCandidate],
    ) -> CollectionGameMetadata {
        let has_picks = args.field_picks.as_ref().is_some_and(|p| !p.is_empty());
        let overwrite = args.overwrite_existing != Some(false);
        let preserve_manual = if has_picks { false } else { args.preserve_manual.unwrap_or(args.overwrite_existing == Some(false)) };
        let settings = self.metadata_settings();
        let priority = with_preferred_source(&self.sources.field_orders(&settings), args.source.as_deref());
        let usable = Self::usable(&settings, candidates);
        let merge_candidates: Vec<MergeCandidate<'_>> = usable.iter().map(|(source, md)| MergeCandidate { source, metadata: md }).collect();
        let mut proposed = merge_metadata(MergeArgs {
            app_name: &args.app_name,
            runner: &args.runner,
            current,
            candidates: &merge_candidates,
            priority: &priority,
            overwrite_existing: overwrite,
            preserve_manual,
            field_picks: args.field_picks.as_ref(),
            fields: args.only_fields.as_deref(),
        });
        // The merge only fills an empty IGDB id, so a game picked on purpose
        // (re-matching a wrong one) has to be carried over here.
        if args.igdb_id.is_some() {
            if let Some(igdb) = candidates.iter().find(|c| c.source == "igdb").filter(|c| c.metadata.igdb_id.is_some()) {
                proposed.igdb_id = igdb.metadata.igdb_id;
                proposed.igdb_slug = igdb.metadata.igdb_slug.clone();
            }
        }
        // Names the user renamed, merged or removed in the fields manager.
        NameRules::load(&self.paths).apply_metadata(&mut proposed);
        // The rules above touch every field: those not chosen stay as stored.
        if let Some(fields) = &args.only_fields {
            for field in METADATA_FIELDS.iter().filter(|f| !fields.iter().any(|c| c == *f)) {
                copy_field(current, &mut proposed, field);
            }
        }
        proposed
    }

    /// `previewGameMetadata`: what each provider returned and what the merge
    /// would store. Nothing is written.
    pub fn preview_game_metadata(&self, args: &MetadataApplyArgs) -> Result<MetadataPreview> {
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner;
        let Gathered { candidates, offline } = self.gather(&args);
        let proposed = self.propose(&args, current.as_ref(), &candidates);
        Ok(MetadataPreview {
            app_name: args.app_name.clone(),
            current,
            proposed,
            candidates,
            igdb_configured: self.igdb_configured(),
            offline,
        })
    }

    /// `persistImages`.
    fn persist_images(&self, md: CollectionGameMetadata, download: bool) -> CollectionGameMetadata {
        self.persist_chosen_images(md, download, &[ArtKind::Cover, ArtKind::Hero])
    }

    /// `persistImages` of the pictures in `kinds` only.
    fn persist_chosen_images(&self, mut md: CollectionGameMetadata, download: bool, kinds: &[ArtKind]) -> CollectionGameMetadata {
        if kinds.contains(&ArtKind::Cover) {
            md.cover_url = materialize_metadata_image(
                &self.paths,
                &self.http,
                &md.runner,
                &md.app_name,
                ArtKind::Cover,
                md.cover_url.as_deref(),
                download,
            );
        }
        if kinds.contains(&ArtKind::Hero) {
            md.hero_url = materialize_metadata_image(
                &self.paths,
                &self.http,
                &md.runner,
                &md.app_name,
                ArtKind::Hero,
                md.hero_url.as_deref(),
                download,
            );
        }
        md
    }

    /// `applyGameMetadata`: fetch, merge and store. When no provider answers
    /// the record is stored with `fetchError` so the idle fill retries after
    /// a week. When none could be reached at all nothing is stored
    /// ([`Offline`]): local data alone must not count as a fetch.
    pub fn apply_game_metadata(&self, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        let preview = self.preview_game_metadata(args)?;
        if preview.offline {
            return Err(Offline.into());
        }
        if preview.candidates.is_empty() {
            let mut failed = current_or_blank(&self.paths, as_runner(&args.runner), &args.app_name)?;
            failed.fetch_error = Some(NO_DATA.into());
            failed.fetched_at = Some(now_iso());
            return upsert_game_metadata(&self.paths, failed);
        }
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let saved = self.persist_images(preview.proposed, download);
        upsert_game_metadata(&self.paths, saved)
    }

    /// Stores what the user saw in `preview` without asking the providers
    /// again: the preview's candidates are merged with `args` (`field_picks`
    /// and the merge options) over the saved record. Nothing is written, and
    /// an error returned, when the preview holds no online data.
    pub fn apply_preview(&self, preview: &MetadataPreview, args: &MetadataApplyArgs) -> Result<CollectionGameMetadata> {
        if preview.offline {
            return Err(Offline.into());
        }
        if preview.candidates.is_empty() {
            bail!(tr!("No metadata sources returned data"));
        }
        let runner = as_runner(&args.runner).to_owned();
        let current = get_game_metadata(&self.paths, &runner, &args.app_name)?;
        let mut args = args.clone();
        args.runner = runner;
        let proposed = self.propose(&args, current.as_ref(), &preview.candidates);
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        upsert_game_metadata(&self.paths, self.persist_images(proposed, download))
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
            priority: &self.sources.field_orders(&settings),
            overwrite_existing: false,
            preserve_manual: true,
            field_picks: None,
            fields: None,
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

    /// `searchCollectionWebImages`; nothing while web images are off.
    pub fn search_web_images(&self, query: &str, transparent: bool) -> Result<Vec<WebImage>> {
        if !self.source_on(ids::WEB_IMAGES) {
            return Ok(Vec::new());
        }
        web_images::search_web_images(&self.http, query, transparent)
    }

    /// The art picker's image search: SteamGridDB's art when it is on and an
    /// API key is set (not for transparent images, which it does not have)
    /// and the web's (when on), apart. A failing SteamGridDB only leaves its part empty; the web's
    /// results leave out the images SteamGridDB already gave.
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
            let web = self.search_web_images(query, transparent);
            (sgdb.join().unwrap_or_default(), web)
        });
        let web = match web {
            Ok(list) => list,
            Err(e) if sgdb.is_empty() => return Err(e),
            Err(_) => Vec::new(),
        };
        let mut seen: std::collections::HashSet<String> = sgdb.iter().map(|img| img.image_url.clone()).collect();
        let web = web.into_iter().filter(|img| seen.insert(img.image_url.clone())).collect();
        Ok(ArtImages { steamgrid: sgdb, web })
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
