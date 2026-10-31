//! The Collection's game list: which games `Collection/index.tsx` shows and
//! everything joined to each of them.
//!
//! The list is built like the screen does:
//! `[...sideload, ...legendary, ...gog, ...nile, ...zoom]`, minus DLC, minus
//! games hidden in Heroic (`store/config.json` `games.hidden`, by app name),
//! de-duplicated on `runner_appName`. Libraries are read like Heroic's
//! `GlobalState`:
//! - `sideload_apps/library.json` `games`;
//! - `store_cache/legendary_library.json` `library`, `nile_library.json`
//!   `library`, `gog_library.json` `games`, `zoom_library.json` `games` (cache
//!   stores: a list without its `__timestamp.<key>` counts as empty), GOG and
//!   Zoom games get `install` from `gog_store/installed.json` /
//!   `zoom_store/installed.json` and are marked installed;
//! - Zoom only when `store/config.json` `settings.experimentalFeatures.zoomPlatform`;
//! - `store/game-overrides.json` attaches Heroic's custom title/art;
//! - `store_cache/steam_library.json` joins games owned on Steam to the
//!   entries of other sources (`steam_owned`): those get a [`SteamLink`],
//!   and a Steam entry this app added that nobody used yet gives way to them.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use heronite_i18n::tr;
use serde_json::Value;

use crate::collection::{game_facets, GameFacets};
use crate::focus::Details;
use crate::json_store;
use crate::models::*;
use crate::paths::DataPaths;
use crate::settings::EffectiveSettings;
use crate::status::{resolve_status_id, sorted_or_default};
use crate::steam_owned::{self, Candidate, SteamOwnedFile};
use crate::text::DateLocale;

/// Everything the UI needs about one game. Cheap to share: keep it in an
/// `Arc` ([`Game`](crate::collection::Game)).
#[derive(Debug)]
pub struct CollectionGame {
    /// `runner_appName`: the key the screen uses for focus and de-duplication.
    pub key: String,
    pub runner: String,
    pub app_name: String,
    /// `collectionGameTitle`: Collection metadata title, else the Heroic
    /// override, else the store title.
    pub title: String,
    pub title_lower: String,
    /// "Library" of the Details panel: Steam, Emulator, Epic, GOG, Amazon,
    /// Xbox, Local, Other.
    pub source_label: &'static str,
    /// `GameInfo.is_installed` (Heronite keeps it current for Steam, ROMs and
    /// executables when it refreshes its install states), or the Steam copy
    /// is installed ([`via_steam`](Self::via_steam)).
    pub installed: bool,
    /// `completionStatusId` as stored.
    pub status_raw: Option<String>,
    /// Status id used for grouping (`slugForMeta`), against the statuses of
    /// the load.
    pub status_id: String,
    /// `store/timestamp.json` `totalPlayed`.
    pub playtime_minutes: u64,
    pub first_played: Option<String>,
    pub last_played: Option<String>,
    pub favourite: bool,
    pub recent: bool,
    /// In Heroic's hidden list (`games.hidden`). Only present in a
    /// [`Library`] loaded with `include_hidden`.
    pub hidden: bool,
    /// `meta.launchKind`: `steam-uri`, `executable`, `emulator`, `unavailable`.
    pub launch_kind: Option<String>,
    /// `meta.steamAppId` (art, screenshots, Steam details).
    pub steam_app_id: Option<String>,
    /// `steamAppIdFromMeta`: set only for `steam-uri` games (Install/Uninstall
    /// open the Steam client), and for games that play through their Steam copy.
    pub steam_launch_id: Option<String>,
    /// The same game owned on Steam, for an entry of another source.
    pub steam_link: Option<SteamLink>,
    /// Only the Steam copy is installed: the game plays, uninstalls and is
    /// set up through Steam (`launch_kind` reads `steam-uri`), while its
    /// store's own actions treat it as not installed.
    pub via_steam: bool,
    /// Focus panel values without Steam/wiki data; see
    /// [`focus_details`](crate::focus::focus_details) for the full chain.
    pub details: Details,
    pub facets: GameFacets,

    // Raw models, shared with the stores they came from.
    pub game: Arc<GameInfo>,
    pub overrides: Option<GameOverride>,
    pub meta: Option<Arc<LocalGameMeta>>,
    pub metadata: Option<Arc<CollectionGameMetadata>>,
    pub art: Option<Arc<CollectionGameArt>>,
    pub sessions: Option<Arc<Vec<LocalGameSession>>>,
}

/// The Steam copy of a game from another source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamLink {
    pub app_id: String,
    pub installed: bool,
}

pub(crate) struct BuildInputs<'a> {
    pub game: GameInfo,
    pub steam: Option<SteamLink>,
    pub overrides: Option<GameOverride>,
    pub meta: Option<Arc<LocalGameMeta>>,
    pub metadata: Option<Arc<CollectionGameMetadata>>,
    pub art: Option<Arc<CollectionGameArt>>,
    pub timestamp: Option<&'a TimestampEntry>,
    pub sessions: Option<Arc<Vec<LocalGameSession>>>,
    pub favourite: bool,
    pub recent: bool,
    pub hidden: bool,
    pub statuses: &'a [CompletionStatus],
}

/// `libraryLabel` of the Focus panel (the `SOURCE_LABELS` table, then the
/// runner, then "Local").
fn source_label(meta: Option<&LocalGameMeta>, runner: &str) -> &'static str {
    if let Some(source) = meta.map(|m| m.source.as_str()) {
        match source {
            "steam" => return "Steam",
            "epic" => return "Epic",
            "gog" => return "GOG",
            "amazon" => return "Amazon",
            "xbox" => return "Xbox",
            "emulator" => return tr!("Emulator"),
            "heroic" => return "Heroic",
            "lutris" => return "Lutris",
            "bottles" => return "Bottles",
            "faugus" => return "Faugus",
            "manual" => return tr!("Local"),
            "other" => return tr!("Other"),
            other => {
                if let Some(client) = crate::clients::Client::from_id(other) {
                    return client.store();
                }
            }
        }
    }
    match runner {
        "legendary" => "Epic",
        "gog" => "GOG",
        "nile" => "Amazon",
        _ => tr!("Local"),
    }
}

impl CollectionGame {
    pub(crate) fn build(input: BuildInputs<'_>) -> Self {
        let BuildInputs { game, steam, overrides, meta, metadata, art, timestamp, sessions, favourite, recent, hidden, statuses } = input;
        let title = display_title(&game, overrides.as_ref(), metadata.as_deref());
        let status_raw = meta.as_deref().and_then(|m| m.completion_status_id.clone()).filter(|s| !s.is_empty());
        let via_steam = !game.is_installed && steam.as_ref().is_some_and(|s| s.installed);
        let steam_app_id = meta
            .as_deref()
            .and_then(|m| m.steam_app_id.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| steam.as_ref().map(|s| s.app_id.clone()));
        let launch_kind = if via_steam {
            Some("steam-uri".to_string())
        } else {
            meta.as_deref().map(|m| m.launch_kind.clone()).filter(|s| !s.is_empty())
        };
        let steam_launch_id = match &steam {
            Some(s) if via_steam => Some(s.app_id.clone()),
            _ if launch_kind.as_deref() == Some("steam-uri") => steam_app_id.clone(),
            _ => None,
        };
        let details = Details::compute(&game, meta.as_deref(), metadata.as_deref(), None, None);
        let mut facets = game_facets(&game, metadata.as_deref());
        facets[crate::collection::FacetKind::Store.index()] = crate::collection::store_facet(
            meta.as_deref().map(|m| m.source.as_str()),
            &game.runner,
            meta.as_deref().is_some_and(|m| m.launch_kind == "steam-uri"),
            steam.is_some(),
        );
        let minutes = timestamp.map(|t| t.total_played.get()).filter(|m| *m > 0.0).unwrap_or(0.0);
        let non_empty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
        Self {
            key: format!("{}_{}", game.runner, game.app_name),
            runner: game.runner.clone(),
            app_name: game.app_name.clone(),
            title_lower: title.to_lowercase(),
            title,
            source_label: source_label(meta.as_deref(), &game.runner),
            installed: game.is_installed || via_steam,
            status_id: resolve_status_id(status_raw.as_deref(), statuses),
            status_raw,
            playtime_minutes: minutes.round() as u64,
            first_played: timestamp.and_then(|t| non_empty(&t.first_played)),
            last_played: timestamp.and_then(|t| non_empty(&t.last_played)),
            favourite,
            recent,
            hidden,
            launch_kind,
            steam_app_id,
            steam_launch_id,
            steam_link: steam,
            via_steam,
            details,
            facets,
            game: Arc::new(game),
            overrides,
            meta,
            metadata,
            art,
            sessions,
        }
    }

    /// `collectionMetadataKey`: `runner:appName`, with `zoom` stored as
    /// `sideload`.
    pub fn metadata_key(&self) -> String {
        metadata_key(&self.runner, &self.app_name)
    }

    /// `collectionArtKey`: `runner:appName`.
    pub fn art_key(&self) -> String {
        format!("{}:{}", self.runner, self.app_name)
    }

    /// Label under the card cover ("Not Played", "2h 5m").
    pub fn playtime_label(&self) -> String {
        crate::collection::playtime_label(self.playtime_minutes)
    }

    /// The game's own copy is installed (not only its Steam copy): what its
    /// store's actions (update, verify, move, uninstall) go by.
    pub fn store_installed(&self) -> bool {
        self.installed && !self.via_steam
    }

    /// The store records the game's own install at a folder that is no
    /// longer there (deleted, moved or renamed outside the app). One `stat`.
    pub fn install_folder_missing(&self) -> bool {
        matches!(self.runner.as_str(), "legendary" | "gog" | "nile")
            && self.store_installed()
            && self.game.install.install_path.as_deref().map(str::trim).is_some_and(|p| !p.is_empty() && !std::path::Path::new(p).exists())
    }

    /// The store client (Battle.net, the EA app...) that installs and starts
    /// the game: it is one of the client's Collection entries.
    pub fn client(&self) -> Option<crate::clients::Client> {
        let meta = self.meta.as_deref().filter(|m| m.launch_kind == crate::clients::LAUNCH_KIND)?;
        crate::clients::Client::from_id(&meta.source)
    }

    /// The Steam app id of the game's installed Steam copy, if any (a Steam
    /// entry, or the Steam twin of another entry).
    pub fn steam_installed_id(&self) -> Option<&str> {
        match &self.steam_link {
            Some(l) if l.installed => Some(&l.app_id),
            Some(_) => None,
            None => self.steam_launch_id.as_deref().filter(|_| self.installed),
        }
    }

    /// Both copies are installed: the game plays its own, and Steam's on request.
    pub fn steam_also_playable(&self) -> Option<&str> {
        self.steam_link.as_ref().filter(|l| l.installed && self.store_installed()).map(|l| l.app_id.as_str())
    }

    /// The game is owned on Steam from another entry and Steam's copy is not installed.
    pub fn steam_installable(&self) -> Option<&str> {
        self.steam_link.as_ref().filter(|l| !l.installed).map(|l| l.app_id.as_str())
    }

    /// The `runner` to store in the overlay for this game
    /// (`zoom` becomes `sideload`).
    pub fn overlay_runner(&self) -> &str {
        if self.runner == "zoom" {
            "sideload"
        } else {
            &self.runner
        }
    }
}

/// `collectionGameTitle`: Collection metadata title, else the Heroic
/// override, else the store title.
fn display_title(game: &GameInfo, overrides: Option<&GameOverride>, metadata: Option<&CollectionGameMetadata>) -> String {
    metadata
        .and_then(|m| m.title.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .or_else(|| overrides.and_then(|o| o.title.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| game.title.clone())
}

pub fn metadata_key(runner: &str, app_name: &str) -> String {
    format!("{}:{app_name}", if runner == "zoom" { "sideload" } else { runner })
}

/// The fields of Heroic's `GameInfo` the Collection reads. Deserializing only
/// these (no `#[serde(flatten)]` buffering, no copy of `dlcList`) halves the
/// time to read the Epic library and keeps its memory small, so
/// `GameInfo::rest` / `InstallInfo::rest` are empty for games from [`Library`].
#[derive(Debug, Default, serde::Deserialize)]
struct RawGame {
    #[serde(default)]
    runner: String,
    #[serde(default)]
    app_name: String,
    #[serde(default)]
    title: String,
    art_cover: Option<String>,
    art_square: Option<String>,
    art_background: Option<String>,
    art_logo: Option<String>,
    developer: Option<String>,
    #[serde(default)]
    is_installed: bool,
    #[serde(default)]
    install: RawInstall,
    folder_name: Option<String>,
    is_linux_native: Option<bool>,
    description: Option<String>,
    extra: Option<Value>,
    /// Kept in `GameInfo::rest` (the game settings show the Cloud saves tab).
    cloud_save_enabled: Option<bool>,
    /// Kept in `GameInfo::rest`: EA app / Ubisoft Connect games install
    /// differently.
    #[serde(rename = "thirdPartyManagedApp")]
    third_party_managed_app: Option<String>,
    /// Kept in `GameInfo::rest`: the View menu's "Run offline".
    #[serde(rename = "canRunOffline")]
    can_run_offline: Option<bool>,
    /// Kept in `GameInfo::rest`: a Linux game started by a command
    /// (`crate::sideload::COMMAND_KEY`).
    #[serde(rename = "heroniteCommand")]
    command: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawInstall {
    executable: Option<String>,
    platform: Option<String>,
    is_dlc: Option<bool>,
    install_path: Option<String>,
    install_size: Option<String>,
}

impl From<RawGame> for GameInfo {
    fn from(g: RawGame) -> Self {
        GameInfo {
            runner: g.runner,
            app_name: g.app_name,
            title: g.title,
            art_cover: g.art_cover,
            art_square: g.art_square,
            art_background: g.art_background,
            art_logo: g.art_logo,
            developer: g.developer,
            is_installed: g.is_installed,
            install: InstallInfo {
                executable: g.install.executable,
                platform: g.install.platform,
                is_dlc: g.install.is_dlc,
                install_path: g.install.install_path,
                install_size: g.install.install_size,
                rest: Default::default(),
            },
            folder_name: g.folder_name,
            is_linux_native: g.is_linux_native,
            description: g.description,
            extra: g.extra,
            rest: {
                let mut rest = crate::models::Extra::new();
                if let Some(b) = g.cloud_save_enabled {
                    rest.insert("cloud_save_enabled".to_string(), Value::Bool(b));
                }
                if let Some(app) = g.third_party_managed_app.filter(|s| !s.is_empty()) {
                    rest.insert("thirdPartyManagedApp".to_string(), Value::String(app));
                }
                if let Some(b) = g.can_run_offline {
                    rest.insert("canRunOffline".to_string(), Value::Bool(b));
                }
                if let Some(c) = g.command.filter(|s| !s.trim().is_empty()) {
                    rest.insert(crate::sideload::COMMAND_KEY.to_string(), Value::String(c));
                }
                rest
            },
        }
    }
}

/// `{ library: [...], games: [...], __timestamp: {...} }` store files.
#[derive(Debug, Default, serde::Deserialize)]
struct RawStore {
    #[serde(default)]
    library: Vec<RawGame>,
    #[serde(default)]
    games: Vec<RawGame>,
    #[serde(default, rename = "__timestamp")]
    timestamp: Option<Value>,
}

/// Modification time and size of every input file, to notice that Heronite
/// (or the user) changed something.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp(Vec<Option<(SystemTime, u64)>>);

fn stamp_files(paths: &DataPaths) -> Vec<PathBuf> {
    vec![
        paths.sideload_library(),
        paths.legendary_library(),
        paths.gog_library(),
        paths.nile_library(),
        paths.zoom_library(),
        paths.gog_installed(),
        paths.zoom_installed(),
        paths.game_overrides(),
        paths.local_meta(),
        paths.metadata(),
        paths.field_names(),
        paths.art(),
        paths.statuses(),
        paths.collection_settings(),
        paths.sessions(),
        paths.timestamps(),
        paths.store_config(),
        paths.steam_owned(),
    ]
}

fn take_stamp(files: &[PathBuf]) -> FileStamp {
    FileStamp(files.iter().map(|p| std::fs::metadata(p).ok().and_then(|m| Some((m.modified().ok()?, m.len())))).collect())
}

/// The loaded Collection.
pub struct Library {
    pub paths: DataPaths,
    /// In screen order (sideload, Epic, GOG, Amazon, Zoom), not sorted.
    pub games: Vec<Arc<CollectionGame>>,
    /// Display order (`sortOrder`, then name).
    pub statuses: Vec<CompletionStatus>,
    pub settings: EffectiveSettings,
    pub date_locale: DateLocale,
    index: HashMap<String, usize>,
    files: Vec<PathBuf>,
    stamp: FileStamp,
}

/// JavaScript truthiness of a JSON value (`!!value`).
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn cache_list(file: RawStore, key: &str) -> Vec<GameInfo> {
    // `CacheStore.get(key)` answers the fallback (empty) without `__timestamp.<key>`.
    let stamped = file.timestamp.as_ref().and_then(|t| t.get(key)).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if !stamped {
        return Vec::new();
    }
    let list = if key == "library" { file.library } else { file.games };
    list.into_iter().map(GameInfo::from).collect()
}

fn apply_installed(games: &mut [GameInfo], installed: Vec<InstalledGame>) {
    if installed.is_empty() {
        return;
    }
    let by_name: HashMap<&str, &InstalledGame> = installed.iter().map(|i| (i.app_name.as_str(), i)).collect();
    for game in games {
        if let Some(i) = by_name.get(game.app_name.as_str()) {
            game.install = i.info.clone();
            game.is_installed = true;
        }
    }
}

fn arcs<T>(map: indexmap::IndexMap<String, T>) -> HashMap<String, Arc<T>> {
    map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect()
}

/// How the Steam library joins the list (see `steam_owned`).
#[derive(Default)]
struct SteamJoins {
    /// Entry key → its Steam copy.
    links: HashMap<String, SteamLink>,
    /// Steam entries added by this app, never used, whose game has a twin.
    gone: HashSet<String>,
}

/// The Steam app id a Steam entry (`steam-uri`) launches.
fn steam_entry_id<'a>(game: &'a GameInfo, meta: Option<&'a LocalGameMeta>) -> Option<&'a str> {
    let meta = meta.filter(|m| m.launch_kind == "steam-uri")?;
    meta.steam_app_id.as_deref().filter(|s| !s.is_empty()).or_else(|| game.app_name.strip_prefix("steam_"))
}

fn steam_joins(
    games: &[GameInfo],
    owned: &SteamOwnedFile,
    metas: &HashMap<String, Arc<LocalGameMeta>>,
    metadatas: &HashMap<String, Arc<CollectionGameMetadata>>,
    overrides: &indexmap::IndexMap<String, GameOverride>,
    timestamps: &TimestampFile,
    sessions: &HashMap<String, Arc<Vec<LocalGameSession>>>,
    favourites: &HashSet<&str>,
) -> SteamJoins {
    if !owned.known() {
        return SteamJoins::default();
    }
    let titles: Vec<String> = games
        .iter()
        .map(|g| display_title(g, overrides.get(&g.app_name), metadatas.get(&metadata_key(&g.runner, &g.app_name)).map(|m| &**m)))
        .collect();
    let keys: Vec<String> = games.iter().map(|g| format!("{}_{}", g.runner, g.app_name)).collect();
    let candidates: Vec<Candidate<'_>> = games
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let meta = metas.get(&g.app_name).map(|m| &**m);
            let metadata = metadatas.get(&metadata_key(&g.runner, &g.app_name));
            let steam_ids = [meta.and_then(|m| m.steam_app_id.as_deref()), metadata.and_then(|m| m.steam_app_id.as_deref())]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect();
            let browser = g.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser"));
            Candidate {
                key: &keys[i],
                title: &titles[i],
                steam_ids,
                steam_entry: steam_entry_id(g, meta),
                joinable: !browser && meta.is_none_or(|m| m.launch_kind != "emulator"),
            }
        })
        .collect();
    let mut twins = steam_owned::twins(&candidates, owned);
    let mut gone = HashSet::new();
    for (g, c) in games.iter().zip(&candidates) {
        let Some(id) = c.steam_entry.filter(|id| twins.contains_key(*id)) else { continue };
        let auto = metas.get(&g.app_name).is_some_and(|m| m.extra.get(steam_owned::AUTO_KEY) == Some(&Value::Bool(true)));
        let played = timestamps.get(&g.app_name).is_some_and(|t| t.total_played.get() > 0.0)
            || sessions.get(&g.app_name).is_some_and(|s| !s.is_empty())
            || favourites.contains(g.app_name.as_str());
        if auto && !played {
            gone.insert(c.key.to_string());
        } else {
            // A Steam entry in use stays the Steam copy; the other entry stays apart.
            twins.remove(id);
        }
    }
    let links = twins
        .into_iter()
        .map(|(id, key)| {
            let installed = owned.get(&id).is_some_and(|g| g.installed);
            (key, SteamLink { app_id: id, installed })
        })
        .collect();
    SteamJoins { links, gone }
}

impl Library {
    /// Reads every store and builds the Collection list (hidden games left out).
    pub fn load(paths: &DataPaths) -> Result<Library> {
        Self::load_with(paths, false)
    }

    /// Like [`Library::load`]; with `include_hidden` the games hidden in
    /// Heroic stay in the list, flagged [`CollectionGame::hidden`] ("Show
    /// hidden games").
    pub fn load_with(paths: &DataPaths, include_hidden: bool) -> Result<Library> {
        let files = stamp_files(paths);
        let stamp = take_stamp(&files);

        // The two big files (Epic library, sessions/metadata) are parsed on
        // their own threads while the small ones are read here.
        let (legendary, metadata_file, sessions_file, local_meta, rest) = std::thread::scope(|scope| {
            let legendary = scope.spawn(|| json_store::read::<RawStore>(&paths.legendary_library()));
            let metadata_file = scope.spawn(|| json_store::read::<MetadataFile>(&paths.metadata()));
            let sessions_file = scope.spawn(|| json_store::read::<SessionsFile>(&paths.sessions()));
            let local_meta = scope.spawn(|| json_store::read::<LocalMetaFile>(&paths.local_meta()));
            let rest = (|| -> Result<_> {
                let config: StoreConfigFile = json_store::read(&paths.store_config())?;
                let sideload: RawStore = json_store::read(&paths.sideload_library())?;
                let nile: RawStore = json_store::read(&paths.nile_library())?;
                let gog: RawStore = json_store::read(&paths.gog_library())?;
                let gog_installed: InstalledGamesFile = json_store::read(&paths.gog_installed())?;
                let overrides: GameOverridesFile = json_store::read(&paths.game_overrides())?;
                let art: ArtFile = json_store::read(&paths.art())?;
                let timestamps: TimestampFile = json_store::read(&paths.timestamps())?;
                let statuses: StatusesFile = json_store::read(&paths.statuses())?;
                let settings: CollectionSettingsFile = json_store::read(&paths.collection_settings())?;
                Ok((config, sideload, nile, gog, gog_installed, overrides, art, timestamps, statuses, settings))
            })();
            (
                legendary.join().expect("legendary thread"),
                metadata_file.join().expect("metadata thread"),
                sessions_file.join().expect("sessions thread"),
                local_meta.join().expect("library thread"),
                rest,
            )
        });
        let (config, sideload, nile, gog, mut gog_installed, overrides, art, timestamps, statuses_file, settings_file) = rest?;
        let legendary = legendary?;
        let metadata_file = metadata_file?;
        let sessions_file = sessions_file?;
        let local_meta = local_meta?;

        let zoom_enabled =
            config.settings.as_ref().and_then(|s| s.get("experimentalFeatures")).and_then(|e| e.get("zoomPlatform")).is_some_and(js_truthy);

        let mut gog_games = cache_list(gog, "games");
        apply_installed(&mut gog_games, std::mem::take(&mut gog_installed.installed));
        let mut zoom_games = Vec::new();
        if zoom_enabled {
            let zoom: RawStore = json_store::read(&paths.zoom_library())?;
            let zoom_installed: InstalledGamesFile = json_store::read(&paths.zoom_installed())?;
            zoom_games = cache_list(zoom, "games");
            apply_installed(&mut zoom_games, zoom_installed.installed);
        }

        let statuses = sorted_or_default(statuses_file.statuses);
        let settings = EffectiveSettings::for_paths(paths, &settings_file, &config);

        let config_games = config.games.unwrap_or_default();
        let hidden: HashSet<&str> = config_games.hidden.iter().map(|g| g.app_name.as_str()).collect();
        let favourites: HashSet<&str> = config_games.favourites.iter().map(|g| g.app_name.as_str()).collect();
        let recent: HashSet<&str> = config_games.recent.iter().map(|g| g.app_name.as_str()).collect();

        let metas = arcs(local_meta.games);
        let metadatas = arcs(metadata_file.games);
        let arts = arcs(art.games);
        let sessions = arcs(sessions_file.sessions);
        let overrides = overrides.overrides;

        let all = sideload
            .games
            .into_iter()
            .map(GameInfo::from)
            .chain(cache_list(legendary, "library"))
            .chain(gog_games)
            .chain(cache_list(nile, "library"))
            .chain(zoom_games);

        let mut seen: HashSet<String> = HashSet::new();
        let kept: Vec<GameInfo> = all
            .filter(|game| {
                let is_hidden = hidden.contains(game.app_name.as_str());
                game.install.is_dlc != Some(true)
                    && (!is_hidden || include_hidden)
                    && seen.insert(format!("{}_{}", game.runner, game.app_name))
            })
            .collect();
        let owned = SteamOwnedFile::read(paths);
        let joins = steam_joins(&kept, &owned, &metas, &metadatas, &overrides, &timestamps, &sessions, &favourites);

        let names = crate::field_names::NameRules::load(paths);
        let mut games: Vec<Arc<CollectionGame>> = Vec::new();
        for game in kept {
            let key = format!("{}_{}", game.runner, game.app_name);
            if joins.gone.contains(&key) {
                continue;
            }
            let app = game.app_name.clone();
            let runner = game.runner.clone();
            let mut built = CollectionGame::build(BuildInputs {
                steam: joins.links.get(&key).cloned(),
                overrides: overrides.get(&app).cloned(),
                meta: metas.get(&app).cloned(),
                metadata: metadatas.get(&metadata_key(&runner, &app)).cloned(),
                art: arts.get(&format!("{runner}:{app}")).cloned(),
                timestamp: timestamps.get(&app),
                sessions: sessions.get(&app).cloned(),
                favourite: favourites.contains(app.as_str()),
                recent: recent.contains(app.as_str()),
                hidden: hidden.contains(app.as_str()),
                statuses: &statuses,
                game,
            });
            names.apply_facets(&mut built.facets);
            games.push(Arc::new(built));
        }

        let index = games.iter().enumerate().map(|(i, g)| (g.key.clone(), i)).collect();
        Ok(Library {
            paths: paths.clone(),
            date_locale: DateLocale::from_language(&settings.language),
            games,
            statuses,
            settings,
            index,
            files,
            stamp,
        })
    }

    /// A game by its `runner_appName` key.
    pub fn get(&self, key: &str) -> Option<&Arc<CollectionGame>> {
        self.index.get(key).map(|i| &self.games[*i])
    }

    /// True when any file the library was built from changed since [`Library::load`]
    /// (about 18 `stat` calls; nothing is read).
    pub fn changed_on_disk(&self) -> bool {
        take_stamp(&self.files) != self.stamp
    }
}

/// Builders for the unit tests of the other modules.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::num::JsNum;

    /// A game with every optional model present (empty), tweaked by `f`.
    pub fn game_with(
        app: &str,
        runner: &str,
        f: impl FnOnce(&mut GameInfo, &mut CollectionGameArt, &mut CollectionGameMetadata, &mut LocalGameMeta, &mut GameOverride),
    ) -> Arc<CollectionGame> {
        let mut info = GameInfo { runner: runner.into(), app_name: app.into(), title: app.into(), ..Default::default() };
        let mut art = CollectionGameArt::default();
        let mut metadata = CollectionGameMetadata { app_name: app.into(), runner: runner.into(), ..Default::default() };
        let mut meta = LocalGameMeta { app_name: app.into(), runner: runner.into(), ..Default::default() };
        let mut overrides = GameOverride::default();
        f(&mut info, &mut art, &mut metadata, &mut meta, &mut overrides);
        let statuses = crate::status::default_statuses();
        Arc::new(CollectionGame::build(BuildInputs {
            game: info,
            steam: None,
            overrides: Some(overrides),
            meta: Some(Arc::new(meta)),
            metadata: Some(Arc::new(metadata)),
            art: Some(Arc::new(art)),
            timestamp: None,
            sessions: None,
            favourite: false,
            recent: false,
            hidden: false,
            statuses: &statuses,
        }))
    }

    #[derive(Default)]
    pub struct Spec {
        pub installed: bool,
        pub minutes: u64,
        pub last_played: Option<String>,
        pub first_played: Option<String>,
        pub developers: Vec<String>,
        pub genres: Vec<String>,
        pub series: Option<String>,
        pub status: Option<String>,
        pub runner: Option<String>,
        pub favourite: bool,
        /// `canRunOffline` of the GameInfo.
        pub offline: Option<bool>,
        /// The store's own title, when the Collection shows another one.
        pub store_title: Option<String>,
    }

    pub fn game(app: &str, title: &str, f: impl FnOnce(&mut Spec)) -> Arc<CollectionGame> {
        let mut spec = Spec::default();
        f(&mut spec);
        let mut info = GameInfo {
            runner: spec.runner.clone().unwrap_or_else(|| "sideload".into()),
            app_name: app.into(),
            title: spec.store_title.clone().unwrap_or_else(|| title.into()),
            is_installed: spec.installed,
            ..Default::default()
        };
        if let Some(offline) = spec.offline {
            info.rest.insert("canRunOffline".into(), offline.into());
        }
        // The Collection title wins over the store's, as metadata would.
        let overrides = spec.store_title.is_some().then(|| GameOverride { title: Some(title.into()), ..Default::default() });
        let metadata = (!spec.developers.is_empty() || !spec.genres.is_empty() || spec.series.is_some()).then(|| {
            Arc::new(CollectionGameMetadata {
                app_name: app.into(),
                runner: "sideload".into(),
                developers: spec.developers.clone(),
                genres: spec.genres.clone(),
                series: spec.series.clone(),
                ..Default::default()
            })
        });
        let meta = spec.status.clone().map(|s| {
            Arc::new(LocalGameMeta { app_name: app.into(), runner: "sideload".into(), completion_status_id: Some(s), ..Default::default() })
        });
        let timestamp = TimestampEntry {
            first_played: spec.first_played.clone(),
            last_played: spec.last_played.clone(),
            total_played: JsNum(spec.minutes as f64),
            extra: Default::default(),
        };
        let statuses = crate::status::default_statuses();
        Arc::new(CollectionGame::build(BuildInputs {
            game: info,
            steam: None,
            overrides,
            meta,
            metadata,
            art: None,
            timestamp: Some(&timestamp),
            sessions: None,
            favourite: spec.favourite,
            recent: false,
            hidden: false,
            statuses: &statuses,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn fixture() -> Option<DataPaths> {
        std::env::var_os("HERONITE_FIXTURE").map(DataPaths::at)
    }

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            // Artwork is large and not needed to test the data layer.
            if matches!(entry.file_name().to_str(), Some("art" | "heroes" | "metadata-art" | "steam-details")) {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    #[test]
    fn empty_directory_loads_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::load(&DataPaths::at(dir.path())).unwrap();
        assert!(lib.games.is_empty());
        assert_eq!(lib.statuses.len(), 9);
        assert!(!lib.changed_on_disk());
    }

    #[test]
    fn a_client_game_knows_its_client() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        fs::create_dir_all(paths.local_meta().parent().unwrap()).unwrap();
        fs::write(
            paths.sideload_library(),
            r#"{"games":[
              {"runner":"sideload","app_name":"battlenet_wow","title":"WoW","is_installed":true,"install":{}},
              {"runner":"sideload","app_name":"lutris_7","title":"Heroes","is_installed":true,"install":{}},
              {"runner":"sideload","app_name":"by_hand","title":"By hand","is_installed":true,"install":{}}]}"#,
        )
        .unwrap();
        fs::write(
            paths.local_meta(),
            r#"{"games":{
              "battlenet_wow":{"appName":"battlenet_wow","runner":"sideload","source":"battlenet","launchKind":"client","launcherArgs":"--exec=\"launch WoW\""},
              "lutris_7":{"appName":"lutris_7","runner":"sideload","source":"lutris","launchKind":"launcher"},
              "by_hand":{"appName":"by_hand","runner":"sideload","source":"battlenet","launchKind":"executable"}}}"#,
        )
        .unwrap();
        let lib = Library::load(&paths).unwrap();
        let client = |key: &str| lib.get(key).unwrap().client();
        assert_eq!(client("sideload_battlenet_wow"), Some(crate::clients::Client::BattleNet));
        assert_eq!(client("sideload_lutris_7"), None);
        assert_eq!(client("sideload_by_hand"), None, "a game added by hand is not the client's, whatever its source");
        assert_eq!(lib.get("sideload_battlenet_wow").unwrap().source_label, "Battle.net");
    }

    #[test]
    fn a_stores_client_names_its_games_library() {
        let meta = |source: &str| LocalGameMeta { source: source.into(), ..Default::default() };
        let labels: Vec<_> = crate::clients::Client::ALL.iter().map(|c| source_label(Some(&meta(c.id())), "sideload")).collect();
        assert_eq!(labels, ["Battle.net", "EA", "Ubisoft", "Rockstar"]);
        assert_eq!(source_label(Some(&meta("lutris")), "sideload"), "Lutris");
        assert_eq!(source_label(Some(&meta("")), "legendary"), "Epic");
    }

    #[test]
    fn the_flags_filters_and_launches_read_survive_loading() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.sideload_library().parent().unwrap()).unwrap();
        fs::write(
            paths.sideload_library(),
            r#"{"games":[
              {"runner":"sideload","app_name":"port","title":"Port","is_installed":true,"canRunOffline":true,
               "heroniteCommand":"./start.sh","install":{"install_path":"/g/port","platform":"linux"}},
              {"runner":"sideload","app_name":"web","title":"Web","canRunOffline":false,"install":{}}]}"#,
        )
        .unwrap();
        let lib = Library::load(&paths).unwrap();
        let port = lib.get("sideload_port").unwrap();
        assert!(crate::collection::runs_offline(port));
        assert_eq!(crate::sideload::command_of(&port.game), Some(("./start.sh", "/g/port")));
        assert!(!crate::collection::runs_offline(lib.get("sideload_web").unwrap()));
    }

    #[test]
    fn small_library_rules() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let w = |p: PathBuf, s: &str| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, s).unwrap();
        };
        w(
            paths.sideload_library(),
            r#"{"games":[
              {"runner":"sideload","app_name":"a","title":"A","is_installed":true,"install":{}},
              {"runner":"sideload","app_name":"a","title":"A dup","install":{}},
              {"runner":"sideload","app_name":"h","title":"Hidden","install":{}},
              {"runner":"sideload","app_name":"d","title":"Dlc","install":{"is_dlc":true}}]}"#,
        );
        w(
            paths.legendary_library(),
            r#"{"library":[{"runner":"legendary","app_name":"a","title":"Epic A","install":{}}],
                "__timestamp":{"library":"Fri Sep 18 2026"}}"#,
        );
        // No __timestamp: Heroic's CacheStore answers empty.
        w(paths.nile_library(), r#"{"library":[{"runner":"nile","app_name":"n","title":"N","install":{}}]}"#);
        w(paths.gog_library(), r#"{"games":[{"runner":"gog","app_name":"g","title":"G","install":{}}],"__timestamp":{"games":"x"}}"#);
        w(paths.gog_installed(), r#"{"installed":[{"appName":"g","install_path":"/games/g","platform":"linux"}]}"#);
        w(paths.store_config(), r#"{"games":{"hidden":[{"appName":"h","title":"Hidden"}],"favourites":[{"appName":"a","title":"A"}]}}"#);
        w(paths.game_overrides(), r#"{"overrides":{"a":{"title":"Custom A"}}}"#);
        let lib = Library::load(&paths).unwrap();
        let keys: Vec<_> = lib.games.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["sideload_a", "legendary_a", "gog_g"]);
        assert_eq!(lib.games[0].title, "Custom A");
        assert!(lib.games[0].favourite);
        assert!(lib.games[0].installed);
        assert_eq!(lib.games[1].source_label, "Epic");
        assert!(lib.games[2].installed);
        assert_eq!(lib.games[2].details.install_path.as_deref(), Some("/games/g"));
        assert_eq!(lib.games[2].source_label, "GOG");
        assert!(lib.get("gog_g").is_some());
        assert!(lib.get("sideload_h").is_none());
        assert!(lib.games.iter().all(|g| !g.hidden));
        // "Show hidden games": the hidden one is back, in its place, flagged.
        let with_hidden = Library::load_with(&paths, true).unwrap();
        let keys: Vec<_> = with_hidden.games.iter().map(|g| (g.key.as_str(), g.hidden)).collect();
        assert_eq!(keys, [("sideload_a", false), ("sideload_h", true), ("legendary_a", false), ("gog_g", false)]);
        assert!(!lib.changed_on_disk());
        w(paths.store_config(), r#"{"games":{"hidden":[]}}"#);
        assert!(lib.changed_on_disk());
    }

    #[test]
    fn steam_twins_join_other_sources() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let w = |p: PathBuf, s: &str| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, s).unwrap();
        };
        w(
            paths.sideload_library(),
            r#"{"games":[
              {"runner":"sideload","app_name":"steam_1","title":"Control","is_installed":true,"install":{}},
              {"runner":"sideload","app_name":"steam_2","title":"Hades","install":{}},
              {"runner":"sideload","app_name":"steam_3","title":"Celeste","install":{}},
              {"runner":"sideload","app_name":"manual","title":"Celeste","install":{}},
              {"runner":"sideload","app_name":"portal","title":"Portal","is_installed":true,"install":{}}]}"#,
        );
        w(
            paths.legendary_library(),
            r#"{"library":[{"runner":"legendary","app_name":"ctl","title":"Control Ultimate Edition","install":{}},
                           {"runner":"legendary","app_name":"hds","title":"HADES™","install":{}}],
                "__timestamp":{"library":"x"}}"#,
        );
        w(
            paths.local_meta(),
            r#"{"games":{
              "steam_1":{"appName":"steam_1","runner":"sideload","launchKind":"steam-uri","steamAppId":"1","heroniteSteamAuto":true},
              "steam_2":{"appName":"steam_2","runner":"sideload","launchKind":"steam-uri","steamAppId":"2","heroniteSteamAuto":true},
              "steam_3":{"appName":"steam_3","runner":"sideload","launchKind":"steam-uri","steamAppId":"3"},
              "ctl":{"appName":"ctl","runner":"legendary","steamAppId":"1"}}}"#,
        );
        w(
            paths.steam_owned(),
            r#"{"games":[{"appId":"1","name":"Control","installed":true},{"appId":"2","name":"Hades"},
                         {"appId":"3","name":"Celeste"},{"appId":"4","name":"Portal","installed":false}],
                "updatedAt":"2026-10-13T00:00:00Z"}"#,
        );
        let lib = Library::load(&paths).unwrap();
        let keys: Vec<_> = lib.games.iter().map(|g| g.key.as_str()).collect();
        // The untouched Steam entries of Control and Hades give way to the Epic games.
        assert_eq!(keys, ["sideload_steam_3", "sideload_manual", "sideload_portal", "legendary_ctl", "legendary_hds"]);
        let ctl = lib.get("legendary_ctl").unwrap();
        assert_eq!(ctl.steam_link, Some(SteamLink { app_id: "1".into(), installed: true }));
        assert!(ctl.via_steam && ctl.installed && !ctl.store_installed());
        assert_eq!(ctl.launch_kind.as_deref(), Some("steam-uri"));
        assert_eq!(ctl.steam_launch_id.as_deref(), Some("1"));
        // Joined by its title.
        let hds = lib.get("legendary_hds").unwrap();
        assert_eq!(hds.steam_link, Some(SteamLink { app_id: "2".into(), installed: false }));
        assert!(!hds.installed && hds.steam_launch_id.is_none());
        assert_eq!(hds.steam_app_id.as_deref(), Some("2"));
        // A Steam entry not added by this app stays the Steam copy.
        assert!(lib.get("sideload_manual").unwrap().steam_link.is_none());
        // The local game keeps its own copy and gets Steam's install.
        let portal = lib.get("sideload_portal").unwrap();
        assert!(portal.installed && !portal.via_steam && portal.store_installed());
        assert_eq!(portal.steam_link.as_ref().map(|l| l.app_id.as_str()), Some("4"));

        // The Store filter: own source, plus Steam for a game also owned there.
        let stores = |key: &str| {
            lib.get(key).unwrap().facets[crate::collection::FacetKind::Store.index()].iter().map(|i| i.value.clone()).collect::<Vec<_>>()
        };
        assert_eq!(stores("legendary_ctl"), ["Epic Games", "Steam"]);
        assert_eq!(stores("sideload_steam_3"), ["Steam"]);
        assert_eq!(stores("sideload_portal"), ["Added manually", "Steam"]);
        let only_manual = crate::collection::filter_games(
            &lib.games,
            &crate::collection::ViewQuery {
                facets: crate::collection::FacetFilters::default().toggled(crate::collection::FacetKind::Store, "added manually"),
                ..Default::default()
            },
        );
        assert_eq!(only_manual.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["sideload_manual", "sideload_portal"]);

        // Played through its Steam entry: that one stays, and so does the Epic game.
        w(paths.timestamps(), r#"{"steam_2":{"totalPlayed":5}}"#);
        let lib = Library::load(&paths).unwrap();
        assert!(lib.get("sideload_steam_2").is_some());
        assert!(lib.get("legendary_hds").unwrap().steam_link.is_none());
    }

    #[test]
    fn zoom_needs_the_experimental_flag() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.store_config().parent().unwrap()).unwrap();
        fs::create_dir_all(paths.zoom_library().parent().unwrap()).unwrap();
        fs::write(
            paths.zoom_library(),
            r#"{"games":[{"runner":"zoom","app_name":"z","title":"Z","install":{}}],"__timestamp":{"games":"x"}}"#,
        )
        .unwrap();
        fs::write(paths.store_config(), r#"{"settings":{}}"#).unwrap();
        assert!(Library::load(&paths).unwrap().games.is_empty());
        fs::write(paths.store_config(), r#"{"settings":{"experimentalFeatures":{"zoomPlatform":true}}}"#).unwrap();
        let lib = Library::load(&paths).unwrap();
        assert_eq!(lib.games.len(), 1);
        assert_eq!(lib.games[0].metadata_key(), "sideload:z");
        assert_eq!(lib.games[0].art_key(), "zoom:z");
    }

    #[test]
    fn real_library_matches_the_collection_screen() {
        let Some(paths) = fixture() else { return };
        let start = std::time::Instant::now();
        let lib = Library::load(&paths).unwrap();
        let first = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..10 {
            Library::load(&paths).unwrap();
        }
        let avg = start.elapsed() / 10;
        eprintln!("games={} load first={first:?} avg10={avg:?}", lib.games.len());

        // Expected by hand from the files: sideload + non-DLC Epic, minus hidden.
        let sideload: SideloadLibraryFile = json_store::read(&paths.sideload_library()).unwrap();
        let epic: StoreLibraryFile = json_store::read(&paths.legendary_library()).unwrap();
        let cfg: StoreConfigFile = json_store::read(&paths.store_config()).unwrap();
        let hidden: HashSet<_> = cfg.games.unwrap().hidden.into_iter().map(|g| g.app_name).collect();
        let expected = sideload
            .games
            .iter()
            .chain(epic.library.iter())
            .filter(|g| g.install.is_dlc != Some(true) && !hidden.contains(&g.app_name))
            .count();
        assert_eq!(lib.games.len(), expected);
        assert!(lib.games.len() > 100);

        let keys: HashSet<_> = lib.games.iter().map(|g| g.key.clone()).collect();
        assert_eq!(keys.len(), lib.games.len());
        assert!(lib.games.iter().any(|g| g.runner == "legendary"));
        assert!(lib.games.iter().any(|g| g.installed));
        let sources: HashSet<_> = lib.games.iter().map(|g| g.source_label).collect();
        eprintln!("source labels: {sources:?}");
        assert!(sources.contains("Steam") && sources.contains("Epic"));
        assert!(!lib.changed_on_disk());
    }

    #[test]
    fn real_games_show_expected_details() {
        let Some(paths) = fixture() else { return };
        let lib = Library::load(&paths).unwrap();
        let ow = lib.get("sideload_steam_2357570").expect("Overwatch");
        assert_eq!(ow.title, "Overwatch®");
        assert_eq!(ow.source_label, "Steam");
        assert_eq!(ow.status_id, "endless");
        assert_eq!(ow.playtime_minutes, 6669);
        assert_eq!(ow.steam_launch_id.as_deref(), Some("2357570"));
        assert_eq!(ow.details.developers, ["Blizzard Entertainment"]);
        assert!(!ow.details.description.is_empty());
        eprintln!("{} | {}", ow.title, ow.details.meta_line(lib.date_locale));
    }

    #[test]
    fn load_is_read_only() {
        let Some(paths) = fixture() else { return };
        let dir = tempfile::tempdir().unwrap();
        copy_tree(&paths.root, dir.path());
        let copy = DataPaths::at(dir.path());
        let lib = Library::load(&copy).unwrap();
        assert!(!lib.changed_on_disk());
        crate::status::set_game_status(&copy, "steam_400", "beaten", &Default::default()).unwrap();
        assert!(lib.changed_on_disk());
        let reloaded = Library::load(&copy).unwrap();
        assert_eq!(reloaded.get("sideload_steam_400").unwrap().status_id, "beaten");
        assert_eq!(reloaded.games.len(), lib.games.len());
    }
}
