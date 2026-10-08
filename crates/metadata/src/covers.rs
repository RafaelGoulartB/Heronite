//! Covers for games that have none: port of `local-library/covers.ts`
//! (Steam CDN, SteamGridDB when an API key exists, Steam title search,
//! Bottles' SteamGrid mirror) and the SteamGridDB calls of
//! `backend/steamgrid/utils.ts`.

use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{anyhow, Result};
use heronite_data::json_store;
use heronite_data::models::{GameInfo, HeroicConfigFile, LocalGameMeta, LocalMetaFile};
use heronite_data::paths::HeroicPaths;
use parking_lot::Mutex;
use serde_json::Value;

use crate::http::Http;
use crate::util::{encode_uri_component, normalize_title, re};

const SGDB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) HeroicGamesLauncher/2.0";
const ENCRYPTED_PREFIX: &str = "sgdb:v1:";

/// `art_square` / `art_cover` of a game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Covers {
    pub art_square: Option<String>,
    pub art_cover: Option<String>,
}

impl Covers {
    fn has_cover(&self) -> bool {
        self.art_square.as_deref().is_some_and(|s| !s.is_empty()) || self.art_cover.as_deref().is_some_and(|s| !s.is_empty())
    }
}

fn steam_cdn(app_id: &str, file: &str) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/{file}")
}

/// `steamCovers`.
pub fn steam_covers(app_id: &str) -> Covers {
    Covers {
        art_square: Some(steam_cdn(app_id, "library_600x900_2x.jpg")),
        art_cover: Some(steam_cdn(app_id, "header.jpg")),
    }
}

/// `steamGridKey`: the SteamGridDB key from Heroic's settings. Keys that
/// Heroic encrypted with Electron's `safeStorage` (`sgdb:v1:` prefix) cannot
/// be decrypted outside Electron; those give `None`.
pub fn steamgrid_api_key(paths: &HeroicPaths) -> Option<String> {
    let config: HeroicConfigFile = json_store::read(&paths.config()).ok()?;
    let key = config.settings?.get("steamGridDbApiKey")?.as_str()?.to_owned();
    if key.is_empty() {
        return None;
    }
    if key.starts_with(ENCRYPTED_PREFIX) {
        log::info!("SteamGridDB API key is encrypted by Heroic (safeStorage); pass the key explicitly to use it");
        return None;
    }
    Some(key)
}

fn title_queries(title: &str) -> Vec<String> {
    let cleaned = re!(r"\s+").replace_all(&title.replace('_', " "), " ").trim().to_owned();
    let colon = re!(r"\s+-\s+").replace_all(&cleaned, ": ").into_owned();
    let mut out: Vec<String> = Vec::new();
    for q in [cleaned, colon] {
        if q.chars().count() >= 2 && !out.contains(&q) {
            out.push(q);
        }
    }
    out
}

fn is_likely_addon(name: &str, query: &str) -> bool {
    let extra = re!(r"(?i)\b(soundtrack|ost|dlc|pack|bundle|redkit|demo|toolkit|bonus)\b");
    extra.is_match(&normalize_title(name)) && !extra.is_match(&normalize_title(query))
}

/// `scoreSteamName`.
pub fn score_steam_name(query: &str, name: &str) -> i32 {
    let wanted = normalize_title(query);
    let got = normalize_title(name);
    if wanted.is_empty() || got.is_empty() || is_likely_addon(name, query) {
        return 0;
    }
    if wanted == got {
        return 100;
    }
    if got.starts_with(&wanted) || wanted.starts_with(&got) {
        let diff = (got.chars().count() as i32 - wanted.chars().count() as i32).abs();
        return 60.max(90 - diff);
    }
    if got.contains(&wanted) {
        return 45;
    }
    0
}

/// Keeps the best-scoring Steam app (>= 60); the first wins ties.
#[derive(Default)]
struct BestApp(Option<(String, i32)>);

impl BestApp {
    fn consider(&mut self, id: Option<String>, name: &str, title: &str) {
        let Some(id) = id.filter(|i| !i.is_empty() && i != "0") else { return };
        let score = score_steam_name(title, name);
        if score < 60 {
            return;
        }
        if self.0.as_ref().is_none_or(|(_, s)| score > *s) {
            self.0 = Some((id, score));
        }
    }
    fn done(&self) -> bool {
        self.0.as_ref().is_some_and(|(_, s)| *s >= 90)
    }
}

/// `steamAppIdFromTitle`: Steam community `SearchApps`, then the store search.
pub fn steam_app_id_from_title(http: &Http, title: &str) -> Option<String> {
    let mut best = BestApp::default();
    for query in title_queries(title) {
        let community = http
            .get(format!("{}/actions/SearchApps/{}", http.endpoints.steam_community, encode_uri_component(&query)))
            .send_ok()
            .and_then(|r| r.json::<Value>());
        match community {
            Ok(Value::Array(hits)) => best_from_search_apps(&mut best, &hits, title),
            Ok(_) => {}
            Err(e) => log::warn!("Steam community search failed for {query}: {e}"),
        }
        if best.done() {
            break;
        }

        let store = http
            .get(format!("{}/api/storesearch/", http.endpoints.steam_store))
            .query(&[("term", query.as_str()), ("l", "english"), ("cc", "US")])
            .send_ok()
            .and_then(|r| r.json::<Value>());
        match store {
            Ok(v) => best_from_store_search(&mut best, &v, title),
            Err(e) => log::warn!("Steam store search failed for {query}: {e}"),
        }
        if best.done() {
            break;
        }
    }
    best.0.map(|(id, _)| id)
}

fn best_from_search_apps(best: &mut BestApp, hits: &[Value], title: &str) {
    for item in hits {
        let id = item.get("appid").and_then(|a| a.as_str().map(str::to_owned).or_else(|| a.as_u64().map(|n| n.to_string())));
        best.consider(id, item.get("name").and_then(Value::as_str).unwrap_or(""), title);
    }
}

fn best_from_store_search(best: &mut BestApp, body: &Value, title: &str) {
    for item in body.get("items").and_then(Value::as_array).into_iter().flatten() {
        if item.get("type").and_then(Value::as_str).is_some_and(|t| !t.is_empty() && t != "app") {
            continue;
        }
        let id = item.get("id").and_then(Value::as_u64).map(|n| n.to_string());
        best.consider(id, item.get("name").and_then(Value::as_str).unwrap_or(""), title);
    }
}

/// Parses `SearchApps` + `storesearch` fixtures the same way (for tests).
#[cfg(test)]
fn best_of(title: &str, search_apps: &Value, store: &Value) -> Option<String> {
    let mut best = BestApp::default();
    best_from_search_apps(&mut best, search_apps.as_array().unwrap(), title);
    best_from_store_search(&mut best, store, title);
    best.0.map(|(i, _)| i)
}

fn sgdb_get(http: &Http, api_key: &str, path: &str, params: &[(&str, &str)]) -> Result<Value> {
    let resp = http
        .get(format!("{}{path}", http.endpoints.steamgriddb))
        .query(params)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("User-Agent", SGDB_UA)
        .send_ok()?;
    let body: Value = resp.json()?;
    if body.get("success").and_then(Value::as_bool) != Some(true) {
        let errors = body
            .get("errors")
            .and_then(Value::as_array)
            .map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "request failed".into());
        return Err(anyhow!(errors));
    }
    Ok(body)
}

/// `searchGame` (autocomplete): id of the first hit.
pub fn sgdb_search_first_id(http: &Http, api_key: &str, query: &str) -> Result<Option<i64>> {
    let body = sgdb_get(http, api_key, &format!("/search/autocomplete/{}", encode_uri_component(query)), &[])?;
    Ok(body.get("data").and_then(Value::as_array).and_then(|a| a.first()).and_then(|g| g.get("id")).and_then(Value::as_i64))
}

fn sgdb_first_url(http: &Http, api_key: &str, kind: &str, game_id: i64, dimensions: &str) -> Result<Option<String>> {
    let body = sgdb_get(http, api_key, &format!("/{kind}/game/{game_id}"), &[("dimensions", dimensions)])?;
    Ok(body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|g| g.get("url"))
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// `coversFromSteamGrid`: first 600x900 grid and first 1600x650 hero.
pub fn covers_from_steamgrid(http: &Http, api_key: &str, sgdb_id: i64) -> Result<Covers> {
    let (grid, hero) = std::thread::scope(|s| {
        let g = s.spawn(|| sgdb_first_url(http, api_key, "grids", sgdb_id, "600x900"));
        let h = s.spawn(|| sgdb_first_url(http, api_key, "heroes", sgdb_id, "1600x650"));
        (g.join().expect("grid thread"), h.join().expect("hero thread"))
    });
    let (grid, hero) = (grid?, hero?);
    Ok(Covers { art_square: grid.clone(), art_cover: hero.or(grid) })
}

/// `steamGridIdFromSteamApp`.
pub fn steamgrid_id_from_steam_app(http: &Http, api_key: &str, steam_app_id: &str) -> Option<i64> {
    match sgdb_get(http, api_key, &format!("/games/steam/{steam_app_id}"), &[]) {
        Ok(body) => body.get("data").and_then(|d| d.get("id")).and_then(Value::as_i64).filter(|i| *i != 0),
        Err(e) => {
            log::warn!("SteamGridDB steam id lookup failed for {steam_app_id}: {e}");
            None
        }
    }
}

fn bottles_grid(http: &Http, title: &str) -> Option<String> {
    let resp = http
        .get(format!("{}/api/search/{}", http.endpoints.bottles_steamgrid, encode_uri_component(title)))
        .timeout_secs(5)
        .send_ok();
    match resp {
        Ok(r) => {
            let text = r.text();
            let text = text.trim().trim_matches('"').to_owned();
            text.starts_with("http").then_some(text)
        }
        Err(e) => {
            log::warn!("Bottles SteamGrid search failed for {title}: {e}");
            None
        }
    }
}

/// `fetchCoversForGame`. Falls back step by step (Steam CDN, SteamGridDB,
/// Steam title search, Bottles) and finally keeps the game's own covers.
pub fn fetch_covers_for_game(http: &Http, api_key: Option<&str>, meta: &LocalGameMeta, game: &GameInfo) -> Covers {
    let api_key = api_key.filter(|k| !k.is_empty());

    if let Some(steam_id) = meta.steam_app_id.as_deref().filter(|s| !s.is_empty()) {
        let from_steam = steam_covers(steam_id);
        if let Some(key) = api_key {
            if let Some(sgdb_id) = steamgrid_id_from_steam_app(http, key, steam_id) {
                match covers_from_steamgrid(http, key, sgdb_id) {
                    Ok(sgdb) => {
                        return Covers {
                            art_square: sgdb.art_square.or(from_steam.art_square),
                            art_cover: sgdb.art_cover.or(from_steam.art_cover),
                        }
                    }
                    Err(e) => log::warn!("SteamGridDB covers failed for {steam_id}: {e}"),
                }
            }
        }
        return from_steam;
    }

    if let Some(key) = api_key {
        let found = sgdb_search_first_id(http, key, &meta.title).and_then(|id| match id {
            Some(id) => covers_from_steamgrid(http, key, id).map(Some),
            None => Ok(None),
        });
        match found {
            Ok(Some(sgdb)) if sgdb.has_cover() => {
                return Covers {
                    art_square: sgdb.art_square.or_else(|| game.art_square.clone()),
                    art_cover: sgdb.art_cover.or_else(|| game.art_cover.clone()),
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("SteamGridDB search failed for {}: {e}", meta.title),
        }
    }

    if let Some(id) = steam_app_id_from_title(http, &meta.title) {
        return steam_covers(&id);
    }
    if let Some(grid) = bottles_grid(http, &meta.title) {
        return Covers { art_square: Some(grid.clone()), art_cover: Some(grid) };
    }
    Covers { art_cover: game.art_cover.clone(), art_square: game.art_square.clone() }
}

/// `refreshMissingLocalCovers`: finds sideloaded games with an overlay entry
/// but no `art_cover`/`art_square`, fetches covers with four workers and
/// writes them into `sideload_apps/library.json` (read-modify-write per
/// game, other fields untouched). Returns how many games got covers. The
/// caller should refresh the Heroic library afterwards.
pub fn refresh_missing_local_covers(paths: &HeroicPaths, http: &Http) -> Result<usize> {
    static RUNNING: Mutex<bool> = Mutex::new(false);
    {
        let mut running = RUNNING.lock();
        if *running {
            return Ok(0);
        }
        *running = true;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            *RUNNING.lock() = false;
        }
    }
    let _reset = Reset;

    let library: heronite_data::models::SideloadLibraryFile = json_store::read(&paths.sideload_library())?;
    let overlay: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let missing: Vec<(&GameInfo, &LocalGameMeta)> = library
        .games
        .iter()
        .filter(|g| g.art_cover.as_deref().unwrap_or("").is_empty() && g.art_square.as_deref().unwrap_or("").is_empty())
        .filter_map(|g| overlay.games.get(&g.app_name).map(|m| (g, m)))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    log::info!("Fetching covers for {} local game(s) without artwork", missing.len());
    let api_key = steamgrid_api_key(paths);

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(String, Covers)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4.min(missing.len()) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((game, meta)) = missing.get(i) else { break };
                let covers = fetch_covers_for_game(http, api_key.as_deref(), meta, game);
                if covers.has_cover() {
                    results.lock().push((game.app_name.clone(), covers));
                }
            });
        }
    });
    let results = results.into_inner();
    if results.is_empty() {
        return Ok(0);
    }
    let count = results.len();
    json_store::update_value(&paths.sideload_library(), |doc| {
        let Some(games) = doc.get_mut("games").and_then(Value::as_array_mut) else { return Ok(()) };
        for (app_name, covers) in &results {
            let Some(game) = games.iter_mut().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name)) else { continue };
            if let Some(c) = covers.art_cover.as_deref().filter(|c| !c.is_empty()) {
                game["art_cover"] = Value::String(c.to_owned());
            }
            if let Some(s) = covers.art_square.as_deref().filter(|c| !c.is_empty()) {
                game["art_square"] = Value::String(s.to_owned());
            }
        }
        Ok(())
    })?;
    log::info!("Saved covers for {count} local game(s)");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    }

    #[test]
    fn steam_search_prefers_exact_titles_and_skips_addons() {
        let apps = fixture("steam_searchapps_portal.json");
        let store = fixture("steam_storesearch_portal.json");
        assert_eq!(best_of("Portal", &apps, &store).as_deref(), Some("400"));
        assert_eq!(best_of("Portal 2", &apps, &store).as_deref(), Some("620"));
        assert_eq!(score_steam_name("Portal", "Portal Soundtrack"), 0);
        assert_eq!(score_steam_name("Portal Soundtrack", "Portal Soundtrack"), 100);
        assert_eq!(score_steam_name("Portal", "Portal 2"), 88);
        assert_eq!(score_steam_name("Portal", "The Portal Thing"), 45);
        assert_eq!(title_queries("A_B - C"), ["A B - C", "A B: C"]);
    }

    #[test]
    fn steamgrid_key_ignores_encrypted_values() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        std::fs::create_dir_all(paths.config().parent().unwrap()).unwrap();
        std::fs::write(paths.config(), r#"{"settings":{"steamGridDbApiKey":"abc"}}"#).unwrap();
        assert_eq!(steamgrid_api_key(&paths).as_deref(), Some("abc"));
        std::fs::write(paths.config(), r#"{"settings":{"steamGridDbApiKey":"sgdb:v1:AAAA"}}"#).unwrap();
        assert_eq!(steamgrid_api_key(&paths), None);
        std::fs::write(paths.config(), r#"{"settings":{"steamGridDbApiKey":""}}"#).unwrap();
        assert_eq!(steamgrid_api_key(&paths), None);
    }
}
