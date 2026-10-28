//! Steam: store details cache (`steam-details.ts`), the Steam metadata
//! candidate (`fetchSteamMetadata`) and the cached hero (`heroes.ts`).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::art::{steam_hero_download_urls, steam_library_cover, steam_library_hero};
use heronite_data::focus::steam_language;
use heronite_data::models::CollectionGameMetadata;
use heronite_data::paths::DataPaths;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Http;
use crate::types::{Lookup, SteamHeroCacheResult};
use crate::util::{blank_metadata, clip, is_steam_app_id, local_art_url, now_ms, strip_html, unique_names, unique_names12};

const CACHE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const MISSING_FOR: Duration = Duration::from_secs(30 * 60);

/// `CollectionSteamDetails` + `fetchedAt`, exactly the file Heronite writes
/// (compact JSON, same key order).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SteamAppDetails {
    pub steam_app_id: String,
    pub language: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short_description: String,
    #[serde(default)]
    pub description_html: String,
    #[serde(default)]
    pub developers: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default)]
    pub store_url: String,
    /// The store's header image. Newer apps keep their art under hashed URLs
    /// that only this field gives (the plain CDN paths are 404).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_image: Option<String>,
    /// Milliseconds since the epoch (`Date.now()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<i64>,
}

/// In-process state: negative cache for failed lookups, per-key locks that
/// stand in for the TS `pending` promise map.
#[derive(Default)]
pub struct SteamState {
    missing_until: Mutex<HashMap<String, Instant>>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    missing_heroes: Mutex<HashSet<String>>,
}

impl SteamState {
    fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut m = self.locks.lock();
        if m.len() > 4096 {
            m.retain(|_, v| Arc::strong_count(v) > 1);
        }
        m.entry(key.to_owned()).or_default().clone()
    }
}

fn js_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn str_list(v: &Value, key: &str, field: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|i| if field.is_empty() { i.as_str() } else { i.get(field).and_then(Value::as_str) })
                .map(|s| s.trim().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `mapDetails`: the `data` object of `appdetails` to the cached shape.
pub fn map_details(steam_app_id: &str, language: &str, data: &Value) -> Option<SteamAppDetails> {
    let first = |keys: &[&str]| keys.iter().map(|k| js_str(data, k)).find(|s| !s.is_empty()).unwrap_or_default();
    let description_html = first(&["detailed_description", "about_the_game", "short_description"]).trim().to_owned();
    let short = js_str(data, "short_description");
    if description_html.is_empty() && short.is_empty() {
        return None;
    }
    let features = str_list(data, "categories", "description")
        .into_iter()
        .filter(|n| !n.is_empty() && !n.to_lowercase().starts_with("steam "))
        .collect();
    // `(data.developers ?? []).filter(Boolean)` keeps the strings untrimmed.
    let raw = |key: &str| -> Vec<String> {
        data.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    Some(SteamAppDetails {
        steam_app_id: steam_app_id.to_owned(),
        language: language.to_owned(),
        name: js_str(data, "name"),
        short_description: short,
        description_html,
        developers: raw("developers"),
        publishers: raw("publishers"),
        genres: str_list(data, "genres", "description").into_iter().filter(|s| !s.is_empty()).collect(),
        features,
        release_date: non_empty(data.get("release_date").map(|r| js_str(r, "date")).unwrap_or_default()),
        website: non_empty(js_str(data, "website")),
        store_url: format!("https://store.steampowered.com/app/{steam_app_id}"),
        header_image: non_empty(js_str(data, "header_image")),
        fetched_at: None,
    })
}

fn read_cache(path: &Path) -> Option<SteamAppDetails> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_cache(path: &Path, details: &SteamAppDetails) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&tmp, serde_json::to_vec(details)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `fetchDetails`: asks for `language`, then English.
fn fetch_details(http: &Http, steam_app_id: &str, language: &str) -> Lookup<SteamAppDetails> {
    let mut langs = vec![language];
    if language != "english" {
        langs.push("english");
    }
    for lang in langs {
        let url = format!("{}/api/appdetails", http.endpoints.steam_store);
        let resp = http.get(url).query(&[("appids", steam_app_id), ("l", lang)]).timeout_secs(20).send_ok().and_then(|r| r.json::<Value>());
        match resp {
            Ok(body) => {
                let payload = body.get(steam_app_id);
                let ok = payload.and_then(|p| p.get("success")).and_then(Value::as_bool) == Some(true);
                if let (true, Some(data)) = (ok, payload.and_then(|p| p.get("data"))) {
                    if let Some(mapped) = map_details(steam_app_id, lang, data) {
                        return Lookup::Found(mapped);
                    }
                }
            }
            Err(e) => {
                log::warn!("Steam appdetails failed for {steam_app_id} ({lang}): {e}");
                // English would not answer either.
                if crate::http::is_transient(&e) {
                    return Lookup::Unreachable;
                }
            }
        }
    }
    Lookup::NotFound
}

/// `getSteamAppDetails`: cached for 30 days in
/// `local_library/steam-details/{appId}.{language}.json`; failures are
/// remembered for 30 minutes, except a rate limit or server error (asked again
/// at once). `language` is a Heroic code (`pt-BR`, `en`...).
pub fn get_steam_app_details(
    paths: &DataPaths,
    http: &Http,
    state: &SteamState,
    steam_app_id: &str,
    language: Option<&str>,
) -> Option<SteamAppDetails> {
    lookup_steam_app_details(paths, http, state, steam_app_id, language).into_option()
}

/// [`get_steam_app_details`] that also says when Steam could not be reached.
/// An unreachable Steam is not remembered as missing: the next call asks again.
fn lookup_steam_app_details(
    paths: &DataPaths,
    http: &Http,
    state: &SteamState,
    steam_app_id: &str,
    language: Option<&str>,
) -> Lookup<SteamAppDetails> {
    let steam_app_id = steam_app_id.trim();
    if !is_steam_app_id(steam_app_id) {
        return Lookup::Skipped;
    }
    // Without a language, the interface's (Steam text is then in Portuguese too).
    let language = steam_language(language.unwrap_or(heronite_i18n::current().code()));
    let key = format!("{steam_app_id}:{language}");
    let file = paths.steam_details(steam_app_id, language);
    let fresh = |c: &SteamAppDetails| c.fetched_at.is_some_and(|t| now_ms() - t < CACHE_MS);
    // A stale cache still beats a failed fetch.
    let stale_or = |cached: Option<SteamAppDetails>, miss: Lookup<SteamAppDetails>| match cached {
        Some(c) => Lookup::Found(strip_fetched(c)),
        None => miss,
    };

    let cached = read_cache(&file);
    if let Some(c) = cached.as_ref().filter(|c| fresh(c)) {
        return Lookup::Found(strip_fetched(c.clone()));
    }
    let blocked = state.missing_until.lock().get(&key).is_some_and(|t| *t > Instant::now());
    if blocked {
        return stale_or(cached, Lookup::NotFound);
    }

    let lock = state.lock_for(&key);
    let _guard = lock.lock();
    // Another thread may have finished while we waited.
    if let Some(c) = read_cache(&file).filter(|c| fresh(c)) {
        return Lookup::Found(strip_fetched(c));
    }
    match fetch_details(http, steam_app_id, language) {
        Lookup::Found(mut fetched) => {
            state.missing_until.lock().remove(&key);
            let mut to_write = fetched.clone();
            to_write.fetched_at = Some(now_ms());
            if let Err(e) = write_cache(&file, &to_write) {
                log::warn!("could not write {}: {e}", file.display());
            }
            fetched.fetched_at = None;
            Lookup::Found(fetched)
        }
        Lookup::Unreachable => stale_or(cached, Lookup::Unreachable),
        miss => {
            state.missing_until.lock().insert(key, Instant::now() + MISSING_FOR);
            stale_or(cached, miss)
        }
    }
}

fn strip_fetched(mut d: SteamAppDetails) -> SteamAppDetails {
    d.fetched_at = None;
    d
}

/// `steamCdnCover`.
pub fn steam_cdn_cover(steam_app_id: Option<&str>) -> Option<String> {
    steam_app_id.filter(|s| is_steam_app_id(s)).map(steam_library_cover)
}

/// `steamCdnHero`.
pub fn steam_cdn_hero(steam_app_id: Option<&str>) -> Option<String> {
    steam_app_id.filter(|s| is_steam_app_id(s)).map(steam_library_hero)
}

/// `fetchSteamMetadata` from already loaded details.
pub fn metadata_from_details(app_name: &str, runner: &str, steam_app_id: &str, d: &SteamAppDetails) -> CollectionGameMetadata {
    let mut md = blank_metadata(app_name, runner);
    md.steam_app_id = Some(steam_app_id.to_owned());
    md.title = Some(d.name.clone());
    let text = if d.short_description.is_empty() { &d.description_html } else { &d.short_description };
    md.description = clip(Some(&strip_html(Some(text))));
    md.release_date = d.release_date.clone();
    md.developers = unique_names12(d.developers.iter().map(|s| Some(s.as_str())));
    md.publishers = unique_names12(d.publishers.iter().map(|s| Some(s.as_str())));
    md.genres = unique_names12(d.genres.iter().map(|s| Some(s.as_str())));
    md.features = unique_names12(d.features.iter().map(|s| Some(s.as_str())));
    md.websites = unique_names([d.website.as_deref(), Some(d.store_url.as_str())], 3);
    md.cover_url = steam_cdn_cover(Some(steam_app_id));
    md.hero_url = steam_cdn_hero(Some(steam_app_id));
    if let Some(header) = &d.header_image {
        md.extra.insert(HEADER_URL.into(), Value::String(header.clone()));
    }
    md
}

/// Key in a game's metadata (kept in `extra`, so the file stays Heronite's
/// format) for the Steam store header, the cover of last resort.
pub const HEADER_URL: &str = "headerUrl";

pub fn fetch_steam_metadata(
    paths: &DataPaths,
    http: &Http,
    state: &SteamState,
    app_name: &str,
    runner: &str,
    steam_app_id: Option<&str>,
    language: Option<&str>,
) -> Lookup<CollectionGameMetadata> {
    let Some(id) = steam_app_id.filter(|s| !s.is_empty()) else { return Lookup::Skipped };
    lookup_steam_app_details(paths, http, state, id, language).map(|details| metadata_from_details(app_name, runner, id, &details))
}

// Heroes (`heroes.ts`)

fn download_hero(http: &Http, url: &str, dest: &Path) -> bool {
    let resp = match http.get(url).timeout_secs(25).max_bytes(30 * 1024 * 1024).send() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Steam hero download failed for {}: {e}", crate::http::redact_url(url));
            return false;
        }
    };
    if resp.status != 200 {
        if resp.status != 404 {
            log::warn!("Steam hero download failed for {}: HTTP {}", crate::http::redact_url(url), resp.status);
        }
        return false;
    }
    let small = resp.content_length.is_some_and(|l| l > 0 && l < 8000);
    if !resp.content_type.contains("image") || small || resp.body.is_empty() {
        return false;
    }
    let write = || -> std::io::Result<()> {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = dest.with_extension("jpg.part");
        fs::write(&tmp, &resp.body)?;
        fs::rename(&tmp, dest)
    };
    write().is_ok()
}

/// `cacheSteamHero`: `local_library/heroes/{appId}.jpg`, trying the Steam CDN
/// hero URLs in order.
pub fn cache_steam_hero(paths: &DataPaths, http: &Http, state: &SteamState, steam_app_id: &str) -> Option<SteamHeroCacheResult> {
    if !is_steam_app_id(steam_app_id) {
        return None;
    }
    let dest = paths.steam_hero(steam_app_id);
    let result = |from_cache| SteamHeroCacheResult {
        url: local_art_url("heroes", &[&format!("{steam_app_id}.jpg")]),
        path: dest.to_string_lossy().into_owned(),
        from_cache,
    };
    if dest.exists() {
        return Some(result(true));
    }
    if state.missing_heroes.lock().contains(steam_app_id) {
        return None;
    }
    let lock = state.lock_for(&format!("hero:{steam_app_id}"));
    let _guard = lock.lock();
    if dest.exists() {
        return Some(result(true));
    }
    let urls: Vec<String> = http.endpoints.steam_hero_urls.clone().unwrap_or_else(|| steam_hero_download_urls(steam_app_id).to_vec());
    for url in urls {
        if download_hero(http, &url, &dest) {
            state.missing_heroes.lock().remove(steam_app_id);
            return Some(result(false));
        }
    }
    state.missing_heroes.lock().insert(steam_app_id.to_owned());
    None
}

/// `warmSteamHeroes`: caches the missing heroes with two workers.
pub fn warm_steam_heroes(paths: &DataPaths, http: &Http, state: &SteamState, ids: &[String]) {
    let mut seen = HashSet::new();
    let needed: Vec<&String> = ids
        .iter()
        .filter(|id| is_steam_app_id(id) && seen.insert(id.as_str()))
        .filter(|id| !paths.steam_hero(id).exists() && !state.missing_heroes.lock().contains(id.as_str()))
        .collect();
    if needed.is_empty() {
        return;
    }
    log::info!("Caching Steam heroes for {} game(s)", needed.len());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..2.min(needed.len()) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(id) = needed.get(i) else { break };
                cache_steam_hero(paths, http, state, id);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn maps_recorded_appdetails() {
        let body: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
        assert_eq!(body["400"]["success"], true);
        let d = map_details("400", "english", &body["400"]["data"]).unwrap();
        assert_eq!(d.name, "Portal");
        assert_eq!(d.store_url, "https://store.steampowered.com/app/400");
        assert_eq!(d.developers, ["Valve"]);
        assert!(d.genres.iter().any(|g| g == "Action"));
        assert!(!d.short_description.is_empty() && !d.description_html.is_empty());
        assert!(d.features.iter().all(|f| !f.to_lowercase().starts_with("steam ")));
        assert!(d.features.iter().any(|f| f == "Single-player"));
        assert!(d.release_date.is_some());
        let md = metadata_from_details("steam_400", "sideload", "400", &d);
        assert_eq!(md.steam_app_id.as_deref(), Some("400"));
        assert_eq!(md.cover_url.as_deref(), Some("https://cdn.cloudflare.steamstatic.com/steam/apps/400/library_600x900_2x.jpg"));
        assert_eq!(md.hero_url.as_deref(), Some("https://cdn.cloudflare.steamstatic.com/steam/apps/400/library_hero_2x.jpg"));
        assert!(md.websites.contains(&"https://store.steampowered.com/app/400".to_string()));
        assert!(!md.description.unwrap().contains('<'));
        // The store header is kept for cards whose CDN art is missing.
        let header = d.header_image.as_deref().expect("fixture has header_image");
        assert_eq!(md.extra.get(HEADER_URL).and_then(Value::as_str), Some(header));
    }

    #[test]
    fn an_app_id_that_is_not_digits_never_reaches_a_path_or_url() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let http = Http::default();
        let state = SteamState::default();
        for bad in ["../../x", "4/0", "4 0", "a"] {
            assert_eq!(steam_cdn_cover(Some(bad)), None, "{bad}");
            assert_eq!(steam_cdn_hero(Some(bad)), None, "{bad}");
            assert!(cache_steam_hero(&paths, &http, &state, bad).is_none(), "{bad}");
            assert!(matches!(lookup_steam_app_details(&paths, &http, &state, bad, Some("english")), Lookup::Skipped), "{bad}");
            assert!(matches!(fetch_steam_metadata(&paths, &http, &state, "a", "sideload", Some(bad), None), Lookup::Skipped), "{bad}");
        }
        warm_steam_heroes(&paths, &http, &state, &["../../x".to_owned()]);
        assert!(!dir.path().join("data").exists(), "nothing was created");
        assert!(steam_cdn_cover(Some("400")).is_some());
    }

    #[test]
    fn cache_file_has_ts_shape() {
        let body: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
        let mut d = map_details("400", "english", &body["400"]["data"]).unwrap();
        d.fetched_at = Some(1_700_000_000_000);
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.starts_with(r#"{"steamAppId":"400","language":"english","name":"Portal","shortDescription":"#));
        assert!(json.ends_with(r#""fetchedAt":1700000000000}"#));
        // The data crate reads it.
        let read: heronite_data::focus::SteamDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(read.name, "Portal");
    }

    #[test]
    fn empty_description_is_not_details() {
        assert!(map_details("1", "english", &serde_json::json!({"name": "x"})).is_none());
    }
}
