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
//! - `store/game-overrides.json` attaches Heroic's custom title/art.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use serde_json::Value;

use crate::collection::{game_facets, GameFacets};
use crate::focus::Details;
use crate::json_store;
use crate::models::*;
use crate::paths::DataPaths;
use crate::settings::EffectiveSettings;
use crate::status::{resolve_status_id, sorted_or_default};
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
    /// executables when it refreshes its install states).
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
    /// open the Steam client).
    pub steam_launch_id: Option<String>,
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

pub(crate) struct BuildInputs<'a> {
    pub game: GameInfo,
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
            "emulator" => return "Emulator",
            "manual" => return "Local",
            "other" => return "Other",
            _ => {}
        }
    }
    match runner {
        "legendary" => "Epic",
        "gog" => "GOG",
        "nile" => "Amazon",
        _ => "Local",
    }
}

impl CollectionGame {
    pub(crate) fn build(input: BuildInputs<'_>) -> Self {
        let BuildInputs { game, overrides, meta, metadata, art, timestamp, sessions, favourite, recent, hidden, statuses } = input;
        let title = metadata
            .as_deref()
            .and_then(|m| m.title.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .or_else(|| overrides.as_ref().and_then(|o| o.title.clone()).filter(|t| !t.is_empty()))
            .unwrap_or_else(|| game.title.clone());
        let status_raw = meta.as_deref().and_then(|m| m.completion_status_id.clone()).filter(|s| !s.is_empty());
        let steam_app_id = meta.as_deref().and_then(|m| m.steam_app_id.clone()).filter(|s| !s.is_empty());
        let launch_kind = meta.as_deref().map(|m| m.launch_kind.clone()).filter(|s| !s.is_empty());
        let steam_launch_id = if launch_kind.as_deref() == Some("steam-uri") { steam_app_id.clone() } else { None };
        let details = Details::compute(&game, meta.as_deref(), metadata.as_deref(), None, None);
        let facets = game_facets(&game, metadata.as_deref());
        let minutes = timestamp.map(|t| t.total_played.get()).filter(|m| *m > 0.0).unwrap_or(0.0);
        let non_empty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
        Self {
            key: format!("{}_{}", game.runner, game.app_name),
            runner: game.runner.clone(),
            app_name: game.app_name.clone(),
            title_lower: title.to_lowercase(),
            title,
            source_label: source_label(meta.as_deref(), &game.runner),
            installed: game.is_installed,
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
        paths.art(),
        paths.statuses(),
        paths.collection_settings(),
        paths.sessions(),
        paths.timestamps(),
        paths.store_config(),
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
        let settings = EffectiveSettings::from_files(&settings_file, &config);

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
        let mut games: Vec<Arc<CollectionGame>> = Vec::new();
        for game in all {
            let is_hidden = hidden.contains(game.app_name.as_str());
            if game.install.is_dlc == Some(true) || (is_hidden && !include_hidden) {
                continue;
            }
            let key = format!("{}_{}", game.runner, game.app_name);
            if !seen.insert(key) {
                continue;
            }
            let app = game.app_name.clone();
            let runner = game.runner.clone();
            games.push(Arc::new(CollectionGame::build(BuildInputs {
                overrides: overrides.get(&app).cloned(),
                meta: metas.get(&app).cloned(),
                metadata: metadatas.get(&metadata_key(&runner, &app)).cloned(),
                art: arts.get(&format!("{runner}:{app}")).cloned(),
                timestamp: timestamps.get(&app),
                sessions: sessions.get(&app).cloned(),
                favourite: favourites.contains(app.as_str()),
                recent: recent.contains(app.as_str()),
                hidden: is_hidden,
                statuses: &statuses,
                game,
            })));
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
    /// (about 16 `stat` calls; nothing is read).
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
        pub developers: Vec<String>,
        pub genres: Vec<String>,
        pub series: Option<String>,
        pub status: Option<String>,
        pub runner: Option<String>,
    }

    pub fn game(app: &str, title: &str, f: impl FnOnce(&mut Spec)) -> Arc<CollectionGame> {
        let mut spec = Spec::default();
        f(&mut spec);
        let info = GameInfo {
            runner: spec.runner.clone().unwrap_or_else(|| "sideload".into()),
            app_name: app.into(),
            title: title.into(),
            is_installed: spec.installed,
            ..Default::default()
        };
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
            first_played: None,
            last_played: spec.last_played.clone(),
            total_played: JsNum(spec.minutes as f64),
            extra: Default::default(),
        };
        let statuses = crate::status::default_statuses();
        Arc::new(CollectionGame::build(BuildInputs {
            game: info,
            overrides: None,
            meta,
            metadata,
            art: None,
            timestamp: Some(&timestamp),
            sessions: None,
            favourite: false,
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
