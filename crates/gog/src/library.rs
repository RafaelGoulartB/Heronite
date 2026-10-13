//! The owned library: the games of the account with their metadata, kept in
//! `store_cache/gog_library.json` (`{"games": [GameInfo...]}`). The Collection
//! reads that file and overlays `gog_store/installed.json` on it.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_epic::util::js_date_now;
use serde_json::{json, Map, Value};

use crate::api::{self, bearer, parallel_map, percent_encode};
use crate::cmd::Gogdl;
use crate::gamesdb::{self, id_text};
use crate::installed::{self, Installed};
use crate::user::{self, Credentials};

const LIBRARY_CACHE: &str = "gog_library";
const API_CACHE: &str = "gog_api_info";
/// Parallel metadata requests.
const WORKERS: usize = 8;
const ATTEMPTS: u32 = 3;

/// A game of the account's release list.
#[derive(Debug, Clone, PartialEq)]
struct Release {
    external_id: String,
    /// Proof of ownership `gamesdb` asks for.
    certificate: String,
}

pub fn releases_url(user_id: &str, page_token: Option<&str>) -> String {
    let base = format!("https://galaxy-library.gog.com/users/{user_id}/releases");
    match page_token {
        Some(token) => format!("{base}?page_token={}", percent_encode(token)),
        None => base,
    }
}

fn gamesdb_url(external_id: &str) -> String {
    format!("https://gamesdb.gog.com/platforms/gog/external_releases/{external_id}")
}

fn product_url(external_id: &str) -> String {
    format!("https://api.gog.com/products/{external_id}?expand=description")
}

/// The GOG releases of one page and the token of the next page.
fn parse_release_page(doc: &Value) -> (Vec<Release>, Option<String>) {
    let releases = doc
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("platform_id").and_then(Value::as_str) == Some("gog"))
                .filter_map(|i| {
                    Some(Release {
                        external_id: id_text(i.get("external_id"))?,
                        certificate: i.get("certificate").and_then(Value::as_str).unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let next = doc.get("next_page_token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
    (releases, next)
}

fn fetch_releases(creds: &Credentials) -> Result<Vec<Release>> {
    let mut all: Vec<Release> = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..500 {
        let doc = api::get_json(&releases_url(&creds.user_id, token.as_deref()), &creds.access_token)?;
        let (page, next) = parse_release_page(&doc);
        for r in page {
            if !all.iter().any(|a| a.external_id == r.external_id) {
                all.push(r);
            }
        }
        match next {
            Some(n) if token.as_deref() != Some(n.as_str()) => token = Some(n),
            _ => return Ok(all),
        }
    }
    bail!("the GOG release list never ended")
}

/// What to do with an answer of the metadata API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Fresh,
    /// 304: the cached copy is current.
    Cached,
    /// 404: GOG has no data for the game.
    NoData,
    Retry,
}

fn verdict(status: u16, has_cache: bool) -> Verdict {
    match status {
        200..=299 => Verdict::Fresh,
        304 if has_cache => Verdict::Cached,
        304 | 404 | 410 => Verdict::NoData,
        _ => Verdict::Retry,
    }
}

fn get_with_retry(url: &str, headers: &[(&str, &str)], has_cache: bool) -> Option<(Verdict, api::Response)> {
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(400 * u64::from(attempt)));
        }
        match api::get(url, headers) {
            Ok(resp) => match verdict(resp.status, has_cache) {
                Verdict::Retry => log::debug!("{url}: HTTP {}", resp.status),
                v => return Some((v, resp)),
            },
            Err(err) => log::debug!("{url}: {err:#}"),
        }
    }
    None
}

/// One game's result: its entry (`None` = not shown) and a fresh cache record.
struct Loaded {
    info: Option<Value>,
    cache: Option<(String, Value)>,
}

fn cache_key(external_id: &str) -> String {
    format!("gog_{external_id}")
}

fn load_one(creds: &Credentials, release: &Release, cached: Option<&Value>, folder_name: &str) -> Loaded {
    let id = release.external_id.as_str();
    let auth = bearer(&creds.access_token);
    let cached_etag = cached.and_then(|c| c.get("etag")).and_then(Value::as_str).filter(|e| !e.is_empty());
    let cached_data = cached.and_then(|c| c.get("data"));
    let mut headers = vec![("Authorization", auth.as_str()), ("X-GOG-Library-Cert", release.certificate.as_str())];
    if let (Some(etag), Some(_)) = (cached_etag, cached_data) {
        headers.push(("If-None-Match", etag));
    }
    let has_cache = cached_etag.is_some() && cached_data.is_some();
    match get_with_retry(&gamesdb_url(id), &headers, has_cache) {
        Some((Verdict::Fresh, resp)) => match serde_json::from_str::<Value>(&resp.body) {
            Ok(doc) => {
                let info = gamesdb::from_gamesdb(id, &doc, folder_name);
                let entry = json!({"etag": resp.etag.unwrap_or_default(), "data": doc});
                Loaded { info, cache: Some((cache_key(id), entry)) }
            }
            Err(err) => {
                log::warn!("GOG game {id}: bad metadata: {err}");
                from_cache_or_nothing(id, cached_data, folder_name)
            }
        },
        Some((Verdict::Cached, _)) => from_cache_or_nothing(id, cached_data, folder_name),
        Some((_, _)) => Loaded { info: fallback_product(creds, id, folder_name), cache: None },
        None => {
            log::warn!("GOG game {id}: metadata unavailable, using the saved copy");
            from_cache_or_nothing(id, cached_data, folder_name)
        }
    }
}

fn from_cache_or_nothing(id: &str, cached_data: Option<&Value>, folder_name: &str) -> Loaded {
    Loaded { info: cached_data.and_then(|d| gamesdb::from_gamesdb(id, d, folder_name)), cache: None }
}

/// `products` answer for a game `gamesdb` does not know.
fn fallback_product(creds: &Credentials, id: &str, folder_name: &str) -> Option<Value> {
    let auth = bearer(&creds.access_token);
    let (verdict, resp) = get_with_retry(&product_url(id), &[("Authorization", auth.as_str())], false)?;
    if verdict != Verdict::Fresh {
        return None;
    }
    gamesdb::from_product(id, &serde_json::from_str(&resp.body).ok()?, folder_name)
}

/// The saved `gamesdb` answers, by cache key.
fn read_api_cache(paths: &DataPaths) -> Map<String, Value> {
    let doc = json_store::read_value(&paths.store_cache(API_CACHE)).unwrap_or(Value::Null);
    let mut out = Map::new();
    if let Value::Object(map) = doc {
        for (k, v) in map {
            if k.starts_with("gog_") {
                out.insert(k, v);
            }
        }
    }
    out
}

/// Saves many cache records with one write (a refresh covers hundreds).
fn write_api_cache(paths: &DataPaths, records: Vec<(String, Value)>) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    json_store::update_value(&paths.store_cache(API_CACHE), |doc| {
        let obj = doc.as_object_mut().expect("object");
        let now = js_date_now();
        let mut stamps = match obj.remove("__timestamp") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        for (key, value) in records {
            stamps.insert(key.clone(), Value::String(now.clone()));
            obj.insert(key, value);
        }
        obj.insert("__timestamp".into(), Value::Object(stamps));
        Ok(())
    })
}

/// Sets the install fields of a `GameInfo` from `installed.json`'s entry.
pub fn apply_install(game: &mut Value, entry: Option<&Installed>) {
    let Some(obj) = game.as_object_mut() else { return };
    match entry {
        Some(i) => {
            obj.insert("is_installed".into(), json!(true));
            obj.insert("install".into(), i.game_install());
        }
        None => {
            obj.insert("is_installed".into(), json!(false));
            obj.insert("install".into(), json!({"is_dlc": false}));
        }
    }
}

/// The library file's games (as saved, install state included).
fn saved_games(paths: &DataPaths) -> Vec<Value> {
    let doc = json_store::read_value(&paths.gog_library()).unwrap_or(Value::Null);
    doc.get("games").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn app_name_of(game: &Value) -> &str {
    game.get("app_name").and_then(Value::as_str).unwrap_or_default()
}

/// Fetches the library and rewrites `store_cache/gog_library.json`. Network.
/// Returns the number of games.
pub fn refresh(paths: &DataPaths, gogdl: &Gogdl) -> Result<usize> {
    let creds = user::credentials(paths, gogdl)?;
    let releases = fetch_releases(&creds)?;
    let cache = read_api_cache(paths);
    let folders: HashMap<String, String> =
        saved_games(paths).iter().filter_map(|g| Some((app_name_of(g).to_string(), g.get("folder_name")?.as_str()?.to_string()))).collect();

    let loaded = parallel_map(&releases, WORKERS, |rel| {
        let folder = folders.get(&rel.external_id).map_or("", String::as_str);
        load_one(&creds, rel, cache.get(&cache_key(&rel.external_id)), folder)
    });

    let installed = installed::load(paths);
    let mut games = Vec::new();
    let mut records = Vec::new();
    for item in loaded {
        records.extend(item.cache);
        if let Some(mut game) = item.info {
            let name = app_name_of(&game).to_string();
            apply_install(&mut game, installed.iter().find(|i| i.app_name == name));
            games.push(game);
        }
    }
    if let Err(err) = write_api_cache(paths, records) {
        log::warn!("saving GOG metadata: {err:#}");
    }
    let count = games.len();
    CacheStore::new(paths, LIBRARY_CACHE, None).set("games", Value::Array(games))?;
    Ok(count)
}

/// The library entry of a game with its install state applied.
pub fn game_info(paths: &DataPaths, app_name: &str) -> Option<Value> {
    let mut game = saved_games(paths).into_iter().find(|g| app_name_of(g) == app_name)?;
    apply_install(&mut game, installed::get(paths, app_name).as_ref());
    Some(game)
}

/// Whether the library has the game.
pub(crate) fn owns(paths: &DataPaths, app_name: &str) -> bool {
    saved_games(paths).iter().any(|g| app_name_of(g) == app_name)
}

/// Rewrites a game's install fields in the library file after an install,
/// update, import, move or uninstall; `folder_name` is stored when known.
pub(crate) fn sync_install_state(paths: &DataPaths, app_name: &str, folder_name: Option<&str>) {
    if !paths.gog_library().exists() {
        return;
    }
    let entry = installed::get(paths, app_name);
    let result = json_store::update_value(&paths.gog_library(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(Value::as_array_mut) else { return Ok(()) };
        if let Some(game) = games.iter_mut().find(|g| app_name_of(g) == app_name) {
            apply_install(game, entry.as_ref());
            if let (Some(folder), Some(obj)) = (folder_name, game.as_object_mut()) {
                obj.insert("folder_name".into(), json!(folder));
            }
        }
        Ok(())
    });
    if let Err(err) = result {
        log::warn!("updating the GOG library entry of {app_name}: {err:#}");
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub owned: usize,
    pub installed: usize,
}

pub fn stats(paths: &DataPaths) -> Stats {
    let games = saved_games(paths);
    let installed = installed::load(paths);
    Stats { owned: games.len(), installed: games.iter().filter(|g| installed.iter().any(|i| i.app_name == app_name_of(g))).count() }
}

/// Library games without an install, for the "import" picker.
pub fn not_installed(paths: &DataPaths) -> Vec<Value> {
    let installed = installed::load(paths);
    saved_games(paths).into_iter().filter(|g| !installed.iter().any(|i| i.app_name == app_name_of(g))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamesdb::sample_release;

    #[test]
    fn release_pages_are_followed_by_token() {
        assert_eq!(releases_url("42", None), "https://galaxy-library.gog.com/users/42/releases");
        assert_eq!(releases_url("42", Some("a b+c=")), "https://galaxy-library.gog.com/users/42/releases?page_token=a%20b%2Bc%3D");
        let page = json!({
            "items": [
                {"platform_id": "gog", "external_id": "1", "certificate": "c1"},
                {"platform_id": "steam", "external_id": "2", "certificate": "c2"},
                {"platform_id": "gog", "external_id": 3, "certificate": "c3"},
                {"platform_id": "gog", "certificate": "none"}
            ],
            "next_page_token": "tok"
        });
        let (releases, next) = parse_release_page(&page);
        assert_eq!(
            releases,
            [Release { external_id: "1".into(), certificate: "c1".into() }, Release { external_id: "3".into(), certificate: "c3".into() }]
        );
        assert_eq!(next.as_deref(), Some("tok"));
        assert_eq!(parse_release_page(&json!({"items": []})), (vec![], None));
    }

    #[test]
    fn answers_of_the_metadata_api() {
        assert_eq!(verdict(200, false), Verdict::Fresh);
        assert_eq!(verdict(304, true), Verdict::Cached);
        assert_eq!(verdict(304, false), Verdict::NoData);
        assert_eq!(verdict(404, true), Verdict::NoData);
        assert_eq!(verdict(500, true), Verdict::Retry);
        assert_eq!(verdict(429, false), Verdict::Retry);
    }

    fn write_library(paths: &DataPaths) {
        let witcher = gamesdb::from_gamesdb("1207658924", &sample_release(), "The Witcher").unwrap();
        let other = gamesdb::from_gamesdb("55", &sample_release(), "").unwrap();
        CacheStore::new(paths, LIBRARY_CACHE, None).set("games", json!([witcher, other])).unwrap();
    }

    fn installed_entry() -> Installed {
        let mut i = Installed::new("1207658924");
        i.platform = "windows".into();
        i.install_path = "/games/The Witcher".into();
        i.install_size = "1.5 GiB".into();
        i.version = "1.0".into();
        i
    }

    #[test]
    fn the_collection_sees_the_library_and_the_install() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_library(&paths);
        installed::upsert(&paths, &installed_entry()).unwrap();

        let lib = heronite_data::Library::load(&paths).unwrap();
        let game = lib.get("gog_1207658924").expect("the game is in the Collection");
        assert!(game.installed && game.game.is_installed);
        assert_eq!(game.game.install.install_path.as_deref(), Some("/games/The Witcher"));
        assert_eq!(game.title, "The Witcher: Enhanced Edition");
        let other = lib.get("gog_55").expect("not installed games are listed too");
        assert!(!other.installed);
    }

    #[test]
    fn queries_overlay_the_install_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(stats(&paths), Stats::default());
        write_library(&paths);
        assert_eq!(stats(&paths), Stats { owned: 2, installed: 0 });
        assert!(game_info(&paths, "1207658924").unwrap()["is_installed"] == json!(false));
        assert!(game_info(&paths, "nope").is_none());

        installed::upsert(&paths, &installed_entry()).unwrap();
        assert_eq!(stats(&paths), Stats { owned: 2, installed: 1 });
        let info = game_info(&paths, "1207658924").unwrap();
        assert_eq!(info["is_installed"], true);
        assert_eq!(info["install"]["install_path"], "/games/The Witcher");
        assert_eq!(info["install"]["version"], "1.0");
        let rest = not_installed(&paths);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0]["app_name"], "55");
        assert!(owns(&paths, "55") && !owns(&paths, "56"));
    }

    #[test]
    fn syncing_rewrites_one_entry_and_keeps_the_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_library(&paths);
        installed::upsert(&paths, &installed_entry()).unwrap();
        sync_install_state(&paths, "1207658924", Some("Witcher Folder"));
        let doc = json_store::read_value(&paths.gog_library()).unwrap();
        assert_eq!(doc["games"][0]["is_installed"], true);
        assert_eq!(doc["games"][0]["folder_name"], "Witcher Folder");
        assert_eq!(doc["games"][1]["is_installed"], false);
        assert!(doc["__timestamp"]["games"].is_string());

        installed::remove(&paths, "1207658924").unwrap();
        sync_install_state(&paths, "1207658924", None);
        let doc = json_store::read_value(&paths.gog_library()).unwrap();
        assert_eq!(doc["games"][0]["is_installed"], false);
        assert_eq!(doc["games"][0]["install"], json!({"is_dlc": false}));
        assert_eq!(doc["games"][0]["folder_name"], "Witcher Folder");
    }

    #[test]
    fn metadata_records_are_saved_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write_api_cache(
            &paths,
            vec![("gog_1".into(), json!({"etag": "e", "data": {}})), ("gog_2".into(), json!({"etag": "f", "data": {}}))],
        )
        .unwrap();
        let cache = read_api_cache(&paths);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache["gog_1"]["etag"], "e");
        let doc = json_store::read_value(&paths.store_cache(API_CACHE)).unwrap();
        assert!(doc["__timestamp"]["gog_2"].is_string());
        assert!(CacheStore::new(&paths, API_CACHE, Some(360)).get("gog_1").is_some());
    }
}
