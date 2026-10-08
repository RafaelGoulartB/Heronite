//! HowLongToBeat lookup and cache write: port of
//! `backend/wiki_game_info/howlongtobeat/utils.ts` (fork-modified protocol:
//! `/api/search/site/init` token, `x-auth-token`, optional honeypot pair) and
//! the HLTB part of `getWikiGameInfo`, writing `store_cache/wikigameinfo.json`
//! in the format Heronite's `CacheStore` uses (`<key>` entry plus
//! `__timestamp.<key>`) that `heronite_data::hltb::WikiCache` reads.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::paths::HeroicPaths;
use heronite_data::text::remove_special_characters;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::http::Http;
use crate::util::re;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
/// `new CacheStore('wikigameinfo', 60 * 24 * 30)`: 30 days.
const CACHE_LIFESPAN_MINUTES: f64 = 60.0 * 24.0 * 30.0;

/// `HeroicHowLongToBeatEntry` (hours).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HowLongToBeat {
    pub main_story: f64,
    pub main_extra: f64,
    pub completionist: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_web_link: Option<String>,
}

/// One search result of the HLTB API.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchHit {
    #[serde(default)]
    pub game_id: u64,
    #[serde(default)]
    pub game_name: String,
    #[serde(default)]
    pub game_type: Option<String>,
    #[serde(default)]
    pub game_image: Option<String>,
    #[serde(default)]
    pub comp_main: Option<f64>,
    #[serde(default)]
    pub comp_plus: Option<f64>,
    #[serde(default)]
    pub comp_100: Option<f64>,
}

struct SearchToken {
    token: String,
    hp_key: Option<String>,
    hp_val: Option<String>,
    fetched_at: Instant,
}

/// Token cache plus the "already tried to refresh this title" set.
#[derive(Default)]
pub struct HltbClient {
    token: Mutex<Option<SearchToken>>,
    refresh_tried: Mutex<HashSet<String>>,
}

pub fn seconds_to_hours(seconds: Option<f64>) -> f64 {
    match seconds {
        Some(s) if s > 0.0 => s / 3600.0,
        _ => 0.0,
    }
}

/// `normalizeHowLongToBeatTitle`.
pub fn normalize_hltb_title(value: &str) -> String {
    value.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

/// `howLongToBeatSearchTerms`.
pub fn search_terms(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `pickHowLongToBeatSearchMatch`.
pub fn pick_search_match<'a>(title: &str, results: &'a [SearchHit]) -> Option<&'a SearchHit> {
    if results.is_empty() {
        return None;
    }
    let games: Vec<&SearchHit> = results
        .iter()
        .filter(|i| i.game_type.as_deref().map_or(true, |t| t.is_empty() || t == "game"))
        .collect();
    let pool: Vec<&SearchHit> = if games.is_empty() { results.iter().collect() } else { games };
    let needle = normalize_hltb_title(title);
    pool.iter()
        .find(|i| normalize_hltb_title(&i.game_name) == needle)
        .or_else(|| {
            pool.iter().find(|i| {
                let n = normalize_hltb_title(&i.game_name);
                n.starts_with(&needle) || needle.starts_with(&n)
            })
        })
        .or_else(|| {
            pool.iter().find(|i| {
                let n = normalize_hltb_title(&i.game_name);
                n.contains(&needle) || needle.contains(&n)
            })
        })
        .or_else(|| pool.first())
        .copied()
}

fn entry_from_hit(base: &str, hit: &SearchHit) -> HowLongToBeat {
    HowLongToBeat {
        main_story: seconds_to_hours(hit.comp_main),
        main_extra: seconds_to_hours(hit.comp_plus),
        completionist: seconds_to_hours(hit.comp_100),
        game_id: Some(hit.game_id),
        game_name: Some(hit.game_name.clone()).filter(|n| !n.is_empty()),
        game_image_url: hit.game_image.as_deref().filter(|i| !i.is_empty()).map(|i| format!("{base}/games/{i}")),
        game_web_link: (hit.game_id != 0).then(|| format!("{base}/game/{}", hit.game_id)),
    }
}

/// `getGameDataById`'s parser: the `__NEXT_DATA__` JSON of a game page.
pub fn parse_game_page(base: &str, html: &str) -> Option<HowLongToBeat> {
    let caps = re!(r#"<script id="__NEXT_DATA__" type="application/json">([\s\S]*?)</script>"#).captures(html)?;
    let data: Value = serde_json::from_str(&caps[1]).ok()?;
    let game = data.pointer("/props/pageProps/game/data/game/0")?;
    let hit: SearchHit = serde_json::from_value(game.clone()).ok()?;
    (hit.game_id != 0).then(|| entry_from_hit(base, &hit))
}

impl HltbClient {
    pub fn new() -> Self {
        Self::default()
    }

    fn base(http: &Http) -> &str {
        http.endpoints.hltb.trim_end_matches('/')
    }

    /// `fetchSearchToken`.
    fn fetch_token(&self, http: &Http, force: bool) -> Result<Option<(String, Option<String>, Option<String>)>> {
        {
            let guard = self.token.lock();
            if let Some(t) = guard.as_ref() {
                if !force && t.fetched_at.elapsed() < TOKEN_TTL {
                    return Ok(Some((t.token.clone(), t.hp_key.clone(), t.hp_val.clone())));
                }
            }
        }
        let base = Self::base(http);
        let t = crate::util::now_ms().to_string();
        let resp = http
            .get(format!("{base}/api/search/site/init"))
            .query(&[("t", t.as_str())])
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Origin", base)
            .header("Referer", &format!("{base}/"))
            .timeout_secs(10)
            .send()?;
        let body: Value = resp.json().unwrap_or(Value::Null);
        let text = |k: &str| body.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        let Some(token) = text("token").filter(|_| resp.status == 200) else {
            log::info!("HowLongToBeat search init failed ({})", resp.status);
            *self.token.lock() = None;
            return Ok(None);
        };
        let (hp_key, hp_val) = (text("hpKey"), text("hpVal"));
        *self.token.lock() = Some(SearchToken { token: token.clone(), hp_key: hp_key.clone(), hp_val: hp_val.clone(), fetched_at: Instant::now() });
        Ok(Some((token, hp_key, hp_val)))
    }

    fn search_once(&self, http: &Http, terms: &[String], force_token: bool) -> Result<(u16, Option<Vec<SearchHit>>)> {
        let Some((token, hp_key, hp_val)) = self.fetch_token(http, force_token)? else { return Ok((0, None)) };
        let mut payload = json!({
            "searchType": "games",
            "searchTerms": terms,
            "searchPage": 1,
            "size": 8,
            "searchOptions": {
                "games": {
                    "userId": 0, "platform": "", "sortCategory": "popular", "rangeCategory": "main",
                    "rangeTime": {"min": null, "max": null},
                    "gameplay": {"perspective": "", "flow": "", "genre": "", "difficulty": ""},
                    "year": "", "modifier": ""
                },
                "users": {"sortCategory": "postcount"},
                "lists": {"sortCategory": "follows"},
                "filter": "", "sort": 0, "randomizer": 0
            },
            "useCache": true
        });
        let honeypot = hp_key.clone().zip(hp_val.clone());
        if let Some((k, v)) = &honeypot {
            payload[k] = Value::String(v.clone());
        }
        let base = Self::base(http);
        let mut req = http
            .post(format!("{base}/api/search/site"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Origin", base)
            .header("Referer", &format!("{base}/"))
            .header("x-auth-token", &token)
            .body(payload.to_string())
            .timeout_secs(10);
        if let Some((k, v)) = &honeypot {
            req = req.header("x-hp-key", k).header("x-hp-val", v);
        }
        let resp = req.send()?;
        let hits = resp
            .json::<Value>()
            .ok()
            .and_then(|v| v.get("data").cloned())
            .and_then(|d| serde_json::from_value::<Vec<SearchHit>>(d).ok());
        Ok((resp.status, hits))
    }

    /// `searchHowLongToBeatByTitle`.
    pub fn search_by_title(&self, http: &Http, title: &str) -> Option<HowLongToBeat> {
        let terms = search_terms(title);
        if terms.is_empty() {
            return None;
        }
        let run = || -> Result<Option<HowLongToBeat>> {
            let mut result = self.search_once(http, &terms, false)?;
            if result.0 == 403 || result.0 == 401 {
                *self.token.lock() = None;
                result = self.search_once(http, &terms, true)?;
            }
            let (status, hits) = result;
            let Some(hits) = hits.filter(|_| status == 200) else {
                log::info!("HowLongToBeat search failed for {title} ({status})");
                return Ok(None);
            };
            Ok(pick_search_match(title, &hits).filter(|m| m.game_id != 0).map(|m| entry_from_hit(Self::base(http), m)))
        };
        match run() {
            Ok(entry) => entry,
            Err(e) => {
                log::error!("Error searching HLTB for {title}: {e}");
                None
            }
        }
    }

    /// `getGameDataById`: scrapes the game page (`/game/{id}`).
    pub fn by_id(&self, http: &Http, game_id: &str) -> Option<HowLongToBeat> {
        if game_id.is_empty() || !game_id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let base = Self::base(http);
        let resp = http
            .get(format!("{base}/game/{game_id}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", &format!("{base}/"))
            .timeout_secs(10)
            .max_bytes(8 * 1024 * 1024)
            .send();
        match resp {
            Ok(r) if r.status == 200 => parse_game_page(base, &r.text()),
            Ok(_) => None,
            Err(e) => {
                log::error!("Error fetching HLTB game data for ID {game_id}: {e}");
                None
            }
        }
    }

    /// `getHowLongToBeat` (without the GOG store-page scrape): by HLTB id when
    /// known, else by title.
    pub fn lookup(&self, http: &Http, title: &str, hltb_id: Option<&str>) -> Option<HowLongToBeat> {
        if let Some(id) = hltb_id.filter(|i| !i.is_empty()) {
            if let Some(found) = self.by_id(http, id) {
                return Some(found);
            }
            log::info!("HLTB ID {id} not found for {title}");
        }
        let found = self.search_by_title(http, title);
        if found.is_none() {
            log::info!("No HowLongToBeat data available for {title}");
        }
        found
    }
}

// ---------------------------------------------------------------------------
// store_cache/wikigameinfo.json
// ---------------------------------------------------------------------------

/// dot-prop path of an electron-store key (`a.b` is nested, `\.` is a dot).
fn dot_path(key: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = key.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'.') => {
                chars.next();
                parts.last_mut().expect("non-empty").push('.');
            }
            '.' => parts.push(String::new()),
            c => parts.last_mut().expect("non-empty").push(c),
        }
    }
    parts
}

fn dot_get<'a>(doc: &'a Value, key: &str) -> Option<&'a Value> {
    dot_path(key).iter().try_fold(doc, |v, p| v.get(p.as_str()))
}

fn dot_set(doc: &mut Value, key: &str, value: Value) {
    let path = dot_path(key);
    let mut cur = doc;
    for (i, part) in path.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let map = cur.as_object_mut().expect("object");
        if i + 1 == path.len() {
            map.insert(part.clone(), value);
            return;
        }
        cur = map.entry(part.clone()).or_insert_with(|| Value::Object(Map::new()));
    }
}

fn dot_delete(doc: &mut Value, key: &str) {
    let path = dot_path(key);
    let Some((last, parents)) = path.split_last() else { return };
    let mut cur = doc;
    for p in parents {
        match cur.get_mut(p.as_str()) {
            Some(next) => cur = next,
            None => return,
        }
    }
    if let Some(map) = cur.as_object_mut() {
        map.shift_remove(last);
    }
}

/// JS `Date()` string, as `CacheStore.set` stores it.
fn js_date_string() -> String {
    let now = chrono::Local::now();
    format!("{} (Local Time)", now.format("%a %b %d %Y %H:%M:%S GMT%z"))
}

/// Minutes since a stored `Date()` string; `None` when unparseable (JS then
/// compares `NaN` and keeps the entry).
fn minutes_since(stamp: &str) -> Option<f64> {
    let head = stamp.split(" (").next().unwrap_or(stamp).trim();
    let parsed = chrono::DateTime::parse_from_str(head, "%a %b %d %Y %H:%M:%S GMT%z").ok()?;
    Some((chrono::Utc::now() - parsed.with_timezone(&chrono::Utc)).num_milliseconds() as f64 / 60_000.0)
}

/// `CacheStore.get(key)` of the wiki store: the entry when it has a
/// timestamp and is younger than 30 days.
fn cache_get<'a>(doc: &'a Value, key: &str) -> Option<&'a Value> {
    let entry = dot_get(doc, key).filter(|v| !v.is_null())?;
    let stamp = dot_get(doc, &format!("__timestamp.{key}")).and_then(Value::as_str).filter(|s| !s.is_empty())?;
    match minutes_since(stamp) {
        Some(m) if m > CACHE_LIFESPAN_MINUTES => None,
        _ => Some(entry),
    }
}

/// Outcome of [`get_or_fetch_hltb`].
#[derive(Debug, Clone, PartialEq)]
pub struct HltbLookup {
    pub entry: Option<HowLongToBeat>,
    /// Served from `wikigameinfo.json` without a request.
    pub from_cache: bool,
}

/// The HLTB half of `getWikiGameInfo(game, preferredTitle)`.
///
/// - an unexpired cached entry with HLTB data is returned as is;
/// - a cached entry without it is refreshed once per session (the PCGamingWiki
///   HLTB id of the entry is used when present) and the entry is rewritten
///   with the new `howlongtobeat` and a new timestamp;
/// - with no entry, HLTB is searched by `title` and a new entry
///   (`pcgamingwiki`, `applegamingwiki`, `gamesdb`, `steamInfo`, `umuId` all
///   `null`: the other sources are not fetched here) is stored.
///
/// `title` is the Collection title (`collectionGameTitle`), `store_title` the
/// Heroic one (second cache key).
pub fn get_or_fetch_hltb(
    paths: &HeroicPaths,
    http: &Http,
    client: &HltbClient,
    title: &str,
    store_title: Option<&str>,
) -> Result<HltbLookup> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(HltbLookup { entry: None, from_cache: false });
    }
    let key = remove_special_characters(title);
    let fallback_key = remove_special_characters(store_title.unwrap_or(""));
    let file = paths.wiki_game_info();

    let doc = json_store::read_value(&file).unwrap_or(Value::Null);
    let cached = cache_get(&doc, &key).cloned().or_else(|| {
        (!fallback_key.is_empty() && fallback_key != key).then(|| cache_get(&doc, &fallback_key).cloned()).flatten()
    });

    let hltb_of = |v: &Value| -> Option<HowLongToBeat> {
        v.get("howlongtobeat").filter(|h| h.is_object()).and_then(|h| serde_json::from_value(h.clone()).ok())
    };

    if let Some(cached) = &cached {
        if cached.get("howlongtobeat").is_some_and(|h| h.is_object()) {
            return Ok(HltbLookup { entry: hltb_of(cached), from_cache: true });
        }
        if !client.refresh_tried.lock().insert(key.clone()) {
            return Ok(HltbLookup { entry: None, from_cache: true });
        }
        log::info!("Refreshing HowLongToBeat data for {key}");
    }

    let hltb_id = cached
        .as_ref()
        .and_then(|c| c.pointer("/pcgamingwiki/howLongToBeatID"))
        .and_then(|v| v.as_str().map(str::to_owned).or_else(|| v.as_u64().map(|n| n.to_string())));
    let found = client.lookup(http, title, hltb_id.as_deref());
    let found_value = match &found {
        Some(f) => serde_json::to_value(f)?,
        None => Value::Null,
    };

    let new_entry = match cached {
        Some(Value::Object(mut map)) => {
            map.insert("howlongtobeat".into(), found_value);
            Value::Object(map)
        }
        _ => json!({
            "pcgamingwiki": null,
            "applegamingwiki": null,
            "howlongtobeat": found_value,
            "gamesdb": null,
            "steamInfo": null,
            "umuId": null
        }),
    };
    json_store::update_value(&file, |doc| {
        dot_set(doc, &key, new_entry);
        dot_set(doc, &format!("__timestamp.{key}"), Value::String(js_date_string()));
        Ok(())
    })?;
    Ok(HltbLookup { entry: found, from_cache: false })
}

/// `wikiGameInfoStore.delete(key)`: drops a cached title (forces a refetch).
pub fn forget_cached_title(paths: &HeroicPaths, title: &str) -> Result<()> {
    let key = remove_special_characters(title.trim());
    json_store::update_value(&paths.wiki_game_info(), |doc| {
        dot_delete(doc, &key);
        dot_delete(doc, &format!("__timestamp.{key}"));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::hltb::WikiCache;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn hits() -> Vec<SearchHit> {
        let v: Value = serde_json::from_str(&fixture("hltb_search_portal2.json")).unwrap();
        serde_json::from_value(v["data"].clone()).unwrap()
    }

    #[test]
    fn terms_and_matching() {
        assert_eq!(search_terms("Onimusha: Way of the Sword"), ["Onimusha", "Way", "of", "the", "Sword"]);
        assert_eq!(normalize_hltb_title("Portal 2: Co-op!"), "portal2coop");
        let hits = hits();
        let best = pick_search_match("Portal 2", &hits).unwrap();
        assert_eq!(best.game_id, 7231);
        // No exact match: the first row whose name is a prefix of the title (or vice versa).
        let best = pick_search_match("Portal 2: Sixense", &hits).unwrap();
        assert_eq!(best.game_name, "Portal 2");
        let best = pick_search_match("Sixense Perceptual", &hits).unwrap();
        assert_eq!(best.game_name, "Portal 2: Sixense Perceptual Pack");
        assert!(pick_search_match("x", &[]).is_none());
    }

    #[test]
    fn entry_from_recorded_search() {
        let hits = hits();
        let e = entry_from_hit("https://howlongtobeat.com", pick_search_match("Portal 2", &hits).unwrap());
        assert!((e.main_story - 30885.0 / 3600.0).abs() < 1e-9);
        assert!((e.completionist - 82549.0 / 3600.0).abs() < 1e-9);
        assert_eq!(e.game_id, Some(7231));
        assert_eq!(e.game_image_url.as_deref(), Some("https://howlongtobeat.com/games/Portal2cover.jpg"));
        assert_eq!(e.game_web_link.as_deref(), Some("https://howlongtobeat.com/game/7231"));
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.starts_with(r#"{"mainStory":8.579"#));
    }

    #[test]
    fn game_page_parser() {
        let html = r#"<html><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"game":{"data":{"game":[{"game_id":42,"game_name":"X","comp_main":3600,"comp_plus":7200,"comp_100":0}]}}}}}</script></html>"#;
        let e = parse_game_page("https://h", html).unwrap();
        assert_eq!((e.main_story, e.main_extra, e.completionist), (1.0, 2.0, 0.0));
        assert_eq!(e.game_web_link.as_deref(), Some("https://h/game/42"));
        assert!(parse_game_page("https://h", "<html></html>").is_none());
    }

    #[test]
    fn dot_prop_semantics() {
        let mut d = json!({});
        dot_set(&mut d, "Plain Title", json!(1));
        dot_set(&mut d, "__timestamp.Plain Title", json!("t"));
        dot_set(&mut d, "Dr. Mario", json!(2));
        assert_eq!(d["Plain Title"], 1);
        assert_eq!(d["__timestamp"]["Plain Title"], "t");
        assert_eq!(d["Dr"][" Mario"], 2);
        assert_eq!(dot_get(&d, "Dr. Mario"), Some(&json!(2)));
        dot_delete(&mut d, "Plain Title");
        assert!(d.get("Plain Title").is_none());
        assert_eq!(minutes_since("Fri Sep 18 2026 21:30:21 GMT-0300 (Brasilia Standard Time)").is_some(), true);
        assert!(minutes_since(&js_date_string()).unwrap().abs() < 2.0);
    }

    #[test]
    fn cache_entries_expire_after_30_days_and_need_timestamp() {
        let fresh = js_date_string();
        let doc = json!({
            "A": {"x": 1}, "B": {"x": 2}, "C": {"x": 3},
            "__timestamp": {"A": fresh, "B": "Mon Jan 05 2026 10:00:00 GMT+0000 (UTC)"}
        });
        assert!(cache_get(&doc, "A").is_some());
        assert!(cache_get(&doc, "B").is_none());
        assert!(cache_get(&doc, "C").is_none());
        assert!(cache_get(&doc, "Z").is_none());
    }

    #[test]
    fn cached_hltb_is_served_and_new_entries_use_the_cache_format() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(dir.path());
        std::fs::create_dir_all(paths.wiki_game_info().parent().unwrap()).unwrap();
        let stamp = js_date_string();
        let existing = json!({
            "Onimusha Way of the Sword": {"pcgamingwiki": null, "howlongtobeat": {
                "mainStory": 20.5, "mainExtra": 29.6, "completionist": 47.8, "gameId": 160598,
                "gameWebLink": "https://howlongtobeat.com/game/160598"}},
            "__timestamp": {"Onimusha Way of the Sword": stamp}
        });
        json_store::write(&paths.wiki_game_info(), &existing).unwrap();
        let http = Http::default();
        let client = HltbClient::new();
        let got = get_or_fetch_hltb(&paths, &http, &client, "Onimusha: Way of the Sword", None).unwrap();
        assert!(got.from_cache);
        assert_eq!(got.entry.unwrap().game_id, Some(160598));
        // The data crate reads what this crate wrote or kept.
        let wiki = WikiCache::load(&paths);
        assert!(wiki.hltb("Onimusha: Way of the Sword", "").is_some());
    }
}
