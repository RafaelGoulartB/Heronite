//! Store (Heroic `GameInfo`) and Playnite providers: ports of
//! `fetchStoreMetadata` / `metadataFromPlayniteGame` / `fetchPlayniteMetadata`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, GameInfo, LocalMetaFile};
use heronite_data::paths::DataPaths;
use parking_lot::Mutex;
use serde_json::Value;

use crate::util::{blank_metadata, clip, re, strip_html, unique_names, unique_names12};

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

/// `fetchStoreMetadata` for a game already in memory.
pub fn metadata_from_store_game(app_name: &str, runner: &str, game: &GameInfo) -> CollectionGameMetadata {
    let extra = game.extra.as_ref();
    let at = |path: &[&str]| -> Option<&str> {
        let mut v = extra?;
        for key in path {
            v = v.get(key)?;
        }
        non_empty(v.as_str())
    };
    let description =
        at(&["about", "shortDescription"]).or_else(|| at(&["about", "description"])).or(non_empty(game.description.as_deref()));
    let genres: Vec<&str> = extra
        .and_then(|e| e.get("genres"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut md = blank_metadata(app_name, runner);
    md.title = Some(game.title.clone());
    md.description = clip(Some(&strip_html(description)));
    md.release_date = at(&["releaseDate"]).map(str::to_owned);
    md.developers = unique_names12([game.developer.as_deref()]);
    md.genres = unique_names12(genres);
    md.websites = unique_names([at(&["storeUrl"])], 2);
    md.cover_url = non_empty(game.art_square.as_deref()).or(non_empty(game.art_cover.as_deref())).map(str::to_owned);
    md.hero_url = non_empty(game.art_background.as_deref()).or(non_empty(game.art_cover.as_deref())).map(str::to_owned);
    md
}

type GamesCache = Mutex<HashMap<PathBuf, (SystemTime, u64, Arc<Vec<Value>>)>>;

/// Reads the Heroic game libraries (`storeGames(runner)`), caching each file
/// by mtime so a bulk run does not reparse the Epic library per game.
#[derive(Default)]
pub struct StoreLibraries {
    cache: GamesCache,
}

impl StoreLibraries {
    fn games(&self, path: &Path, key: &str) -> Arc<Vec<Value>> {
        let stamp = std::fs::metadata(path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
        if let Some((mtime, len)) = stamp {
            if let Some((t, l, games)) = self.cache.lock().get(path) {
                if *t == mtime && *l == len {
                    return games.clone();
                }
            }
        }
        let games: Vec<Value> = json_store::read_value(path)
            .ok()
            .and_then(|v| match v.get(key) {
                Some(Value::Array(a)) => Some(a.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let games = Arc::new(games);
        if let Some((mtime, len)) = stamp {
            self.cache.lock().insert(path.to_path_buf(), (mtime, len, games.clone()));
        }
        games
    }

    /// The Heroic `GameInfo` of `app_name` in the store of `runner`.
    pub fn find(&self, paths: &DataPaths, runner: &str, app_name: &str) -> Option<GameInfo> {
        let (file, key) = match runner {
            "legendary" => (paths.legendary_library(), "library"),
            "gog" => (paths.gog_library(), "games"),
            "nile" => (paths.nile_library(), "library"),
            _ => (paths.sideload_library(), "games"),
        };
        let games = self.games(&file, key);
        let found = games.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name))?;
        serde_json::from_value(found.clone()).ok()
    }
}

/// The fields of a Playnite game the metadata uses (`PlayniteGame`).
#[derive(Debug, Clone, Default)]
pub struct PlayniteGame {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub release_date: Option<String>,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub platforms: Vec<String>,
    pub series: Option<String>,
    pub features: Vec<String>,
    /// Relative path inside the Playnite library (`files/...`).
    pub cover_image: Option<String>,
    pub background_image: Option<String>,
}

/// Access to the Playnite library dump (`getCachedPlayniteDump`). Implemented
/// by whoever reads `games.db`; the metadata crate only needs a lookup.
pub trait PlayniteSource: Send + Sync {
    /// The game with this Playnite id (case-insensitive) and the library
    /// folder its image paths are relative to.
    fn game(&self, playnite_id: &str) -> Option<(PlayniteGame, PathBuf)>;
}

fn playnite_file_path(library_path: &Path, relative: Option<&str>) -> Option<PathBuf> {
    let relative = relative.filter(|r| !r.is_empty())?;
    let cleaned = relative.replace('\\', "/");
    let cleaned = re!(r"(?i)^\{PlayniteDir\}/?").replace(&cleaned, "").into_owned();
    let candidates = [library_path.join("files").join(&cleaned), library_path.join(&cleaned)];
    candidates.into_iter().find(|p| p.exists())
}

/// `usablePlayniteFile`: an existing image up to `max_bytes` (2 MB default).
pub fn usable_playnite_file(library_path: &Path, relative: Option<&str>, max_bytes: u64) -> Option<PathBuf> {
    let path = playnite_file_path(library_path, relative)?;
    (std::fs::metadata(&path).ok()?.len() <= max_bytes).then_some(path)
}

/// `metadataFromPlayniteGame`.
pub fn metadata_from_playnite_game(app_name: &str, runner: &str, game: &PlayniteGame, library_path: &Path) -> CollectionGameMetadata {
    let list = |v: &[String]| unique_names12(v.iter().map(|s| Some(s.as_str())));
    let mut md = blank_metadata(app_name, runner);
    md.title = Some(game.name.clone());
    md.description = clip(Some(&strip_html(game.description.as_deref())));
    md.release_date = game.release_date.clone().filter(|d| !d.is_empty());
    md.developers = list(&game.developers);
    md.publishers = list(&game.publishers);
    md.genres = list(&game.genres);
    md.themes = list(&game.tags);
    md.platforms = list(&game.platforms);
    md.series = game.series.clone().filter(|s| !s.is_empty());
    md.features = list(&game.features);
    let max = 2 * 1024 * 1024;
    md.cover_url = usable_playnite_file(library_path, game.cover_image.as_deref(), max).map(|p| p.to_string_lossy().into_owned());
    md.hero_url = usable_playnite_file(library_path, game.background_image.as_deref(), max).map(|p| p.to_string_lossy().into_owned());
    md
}

/// `fetchPlayniteMetadata`: the game's `playniteId` comes from the overlay
/// (`local_library/library.json`), the data from the Playnite dump.
pub fn fetch_playnite_metadata(
    paths: &DataPaths,
    source: Option<&dyn PlayniteSource>,
    app_name: &str,
    runner: &str,
) -> Option<CollectionGameMetadata> {
    let source = source?;
    let file: LocalMetaFile = json_store::read(&paths.local_meta()).ok()?;
    let id = file.games.get(app_name)?.playnite_id.clone();
    if id.is_empty() {
        return None;
    }
    let (game, library_path) = source.game(&id)?;
    Some(metadata_from_playnite_game(app_name, runner, &game, &library_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn store_candidate_reads_extra() {
        let game: GameInfo = serde_json::from_value(json!({
            "runner": "gog", "app_name": "1", "title": "Witcher",
            "developer": "CDPR", "art_square": "https://sq", "art_cover": "https://cv", "art_background": "",
            "description": "<b>desc</b>",
            "extra": {"about": {"shortDescription": "", "description": "Long &amp; text"},
                      "releaseDate": "2015-05-19", "genres": ["RPG", "rpg", "Open World"], "storeUrl": "https://gog/x"}
        }))
        .unwrap();
        let md = metadata_from_store_game("1", "gog", &game);
        assert_eq!(md.description.as_deref(), Some("Long & text"));
        assert_eq!(md.release_date.as_deref(), Some("2015-05-19"));
        assert_eq!(md.developers, ["CDPR"]);
        assert_eq!(md.genres, ["RPG", "Open World"]);
        assert_eq!(md.websites, ["https://gog/x"]);
        assert_eq!(md.cover_url.as_deref(), Some("https://sq"));
        // empty art_background falls back to art_cover
        assert_eq!(md.hero_url.as_deref(), Some("https://cv"));
    }

    #[test]
    fn playnite_files_resolve_inside_the_library() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("files/abc")).unwrap();
        std::fs::write(dir.path().join("files/abc/cover.png"), b"png").unwrap();
        std::fs::write(dir.path().join("files/abc/big.png"), vec![0u8; 3 * 1024 * 1024]).unwrap();
        let g = PlayniteGame {
            id: "abc".into(),
            name: "Game".into(),
            description: Some("<p>Hi</p>".into()),
            tags: vec!["Action".into()],
            cover_image: Some("{PlayniteDir}\\abc\\cover.png".into()),
            background_image: Some("abc/big.png".into()),
            ..Default::default()
        };
        let md = metadata_from_playnite_game("a", "sideload", &g, dir.path());
        assert_eq!(md.themes, ["Action"]);
        assert_eq!(md.description.as_deref(), Some("Hi"));
        // As paths: Windows takes `/` and `\` alike.
        assert_eq!(md.cover_url.as_deref().map(Path::new), Some(dir.path().join("files/abc/cover.png").as_path()));
        // Bigger than 2 MB: not usable.
        assert!(md.hero_url.is_none());
    }

    struct One(PlayniteGame, PathBuf);
    impl PlayniteSource for One {
        fn game(&self, id: &str) -> Option<(PlayniteGame, PathBuf)> {
            self.0.id.eq_ignore_ascii_case(id).then(|| (self.0.clone(), self.1.clone()))
        }
    }

    #[test]
    fn playnite_candidate_needs_overlay_playnite_id() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.local_meta(), r#"{"games":{"x":{"appName":"x","runner":"sideload","playniteId":"ABC-1","title":"X"}}}"#)
            .unwrap();
        let src = One(PlayniteGame { id: "abc-1".into(), name: "Found".into(), ..Default::default() }, dir.path().into());
        let md = fetch_playnite_metadata(&paths, Some(&src), "x", "sideload").unwrap();
        assert_eq!(md.title.as_deref(), Some("Found"));
        assert!(fetch_playnite_metadata(&paths, Some(&src), "nope", "sideload").is_none());
        assert!(fetch_playnite_metadata(&paths, None, "x", "sideload").is_none());
    }
}
