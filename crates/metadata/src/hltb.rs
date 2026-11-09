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
use heronite_data::paths::DataPaths;
use heronite_data::text::{is_word_char, remove_special_characters, title_key};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::http::Http;
use crate::util::re;

// The browser User-Agent and headers are kept from the original port: HLTB's
// search refuses requests that do not look like its own site.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const TOKEN_TTL: Duration = Duration::from_secs(10 * 60);
/// `new CacheStore('wikigameinfo', 60 * 24 * 30)`: 30 days.
const CACHE_LIFESPAN_MINUTES: f64 = 60.0 * 24.0 * 30.0;
/// A title HowLongToBeat did not have is asked again after a week, not at
/// every start.
const NOT_FOUND_RETRY_MINUTES: f64 = 60.0 * 24.0 * 7.0;

/// How the cached entry was matched to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    /// The same name (or one of HLTB's aliases), ignoring case and symbols.
    Exact,
    /// By a HowLongToBeat id (PCGamingWiki's).
    Id,
    /// One name starts with or contains the other.
    Close,
    /// Picked by the user.
    Manual,
}

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
    /// Missing in entries written before matches were told apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<MatchKind>,
}

/// A year, given as a number or a string (`0` and junk: none).
fn de_year<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<i32>, D::Error> {
    let v = Value::deserialize(d)?;
    let n = match &v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    Ok(n.filter(|y| (1950..=2200).contains(y)).map(|y| y as i32))
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
    /// Release year.
    #[serde(default, deserialize_with = "de_year")]
    pub release_world: Option<i32>,
    /// Other names, comma separated.
    #[serde(default)]
    pub game_alias: Option<String>,
}

impl SearchHit {
    fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.game_name.as_str())
            .chain(self.game_alias.as_deref().unwrap_or("").split(','))
            .map(str::trim)
            .filter(|n| !n.is_empty())
    }

    /// A game, not a DLC or mod.
    pub fn is_game(&self) -> bool {
        self.game_type.as_deref().is_none_or(|t| t.is_empty() || t == "game")
    }
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
    title_key(value).replace(' ', "")
}

/// `howLongToBeatSearchTerms`.
pub fn search_terms(title: &str) -> Vec<String> {
    title.split(|c: char| !is_word_char(c)).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

/// `pickHowLongToBeatSearchMatch`, without its "else the first result": a
/// title HLTB has no close name for is not matched to something else.
pub fn pick_search_match<'a>(title: &str, results: &'a [SearchHit]) -> Option<&'a SearchHit> {
    pick_match(title, None, results).map(|(hit, _)| hit)
}

/// The best search result for `title`, and how close it is. Games come
/// before DLCs and mods; among equally good names, the one released within a
/// year of `year` wins, and a close (not exact) name released years apart is
/// not taken at all.
pub fn pick_match<'a>(title: &str, year: Option<i32>, results: &'a [SearchHit]) -> Option<(&'a SearchHit, MatchKind)> {
    let games: Vec<&SearchHit> = results.iter().filter(|i| i.is_game()).collect();
    let pool: Vec<&SearchHit> = if games.is_empty() { results.iter().collect() } else { games };
    let needle = normalize_hltb_title(title);
    if needle.is_empty() {
        return None;
    }
    // A name with no letter or digit would be a prefix of everything.
    let any_name = |hit: &SearchHit, f: &dyn Fn(&str) -> bool| hit.names().map(normalize_hltb_title).any(|n| !n.is_empty() && f(&n));
    let near = |hit: &SearchHit| match (year, hit.release_world) {
        (Some(y), Some(h)) => (y - h).abs() <= 1,
        _ => true,
    };
    let exact: Vec<&SearchHit> = pool.iter().copied().filter(|h| any_name(h, &|n| n == needle)).collect();
    if let Some(hit) = exact.iter().find(|h| near(h)).or(exact.first()) {
        return Some((hit, MatchKind::Exact));
    }
    let close = |f: &dyn Fn(&str) -> bool| pool.iter().copied().find(|h| near(h) && any_name(h, f));
    close(&|n| n.starts_with(&needle) || needle.starts_with(n))
        .or_else(|| close(&|n| n.contains(&needle) || needle.contains(n)))
        .map(|hit| (hit, MatchKind::Close))
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
        match_kind: None,
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
        *self.token.lock() =
            Some(SearchToken { token: token.clone(), hp_key: hp_key.clone(), hp_val: hp_val.clone(), fetched_at: Instant::now() });
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
        let hits =
            resp.json::<Value>().ok().and_then(|v| v.get("data").cloned()).and_then(|d| serde_json::from_value::<Vec<SearchHit>>(d).ok());
        Ok((resp.status, hits))
    }

    /// The search results for `title`; `Ok(None)` when HLTB answered with an
    /// error (refused token, server error), `Err` when it was not reached.
    fn search_hits(&self, http: &Http, title: &str) -> Result<Option<Vec<SearchHit>>> {
        let terms = search_terms(title);
        if terms.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let mut result = self.search_once(http, &terms, false)?;
        if result.0 == 403 || result.0 == 401 {
            *self.token.lock() = None;
            result = self.search_once(http, &terms, true)?;
        }
        let (status, hits) = result;
        let hits = hits.filter(|_| status == 200);
        if hits.is_none() {
            log::info!("HowLongToBeat search failed for {title} ({status})");
        }
        Ok(hits)
    }

    /// Search results for the "Change match" list: games first, then DLCs
    /// and the rest, each in HLTB's order.
    pub fn search(&self, http: &Http, title: &str) -> Result<Vec<SearchHit>> {
        let Some(mut hits) = self.search_hits(http, title)? else {
            anyhow::bail!("HowLongToBeat did not answer the search");
        };
        hits.retain(|h| h.game_id != 0);
        hits.sort_by_key(|h| !h.is_game());
        Ok(hits)
    }

    /// `searchHowLongToBeatByTitle`: `Err` when HLTB could not be asked.
    fn search_by_title_checked(&self, http: &Http, title: &str, year: Option<i32>) -> Result<Option<HowLongToBeat>> {
        let Some(hits) = self.search_hits(http, title)? else { anyhow::bail!("HowLongToBeat search failed") };
        Ok(pick_match(title, year, &hits)
            .filter(|(m, _)| m.game_id != 0)
            .map(|(m, kind)| HowLongToBeat { match_kind: Some(kind), ..entry_from_hit(Self::base(http), m) }))
    }

    /// `searchHowLongToBeatByTitle`.
    pub fn search_by_title(&self, http: &Http, title: &str) -> Option<HowLongToBeat> {
        match self.search_by_title_checked(http, title, None) {
            Ok(entry) => entry,
            Err(e) => {
                log::error!("Error searching HLTB for {title}: {e}");
                None
            }
        }
    }

    /// `getGameDataById`: scrapes the game page (`/game/{id}`).
    pub fn by_id(&self, http: &Http, game_id: &str) -> Option<HowLongToBeat> {
        self.by_id_checked(http, game_id).unwrap_or_else(|e| {
            log::error!("Error fetching HLTB game data for ID {game_id}: {e}");
            None
        })
    }

    /// [`by_id`](Self::by_id); `Err` when HLTB was not reached or failed
    /// (a missing page is `Ok(None)`).
    fn by_id_checked(&self, http: &Http, game_id: &str) -> Result<Option<HowLongToBeat>> {
        if game_id.is_empty() || !game_id.chars().all(|c| c.is_ascii_digit()) {
            return Ok(None);
        }
        let base = Self::base(http);
        let r = http
            .get(format!("{base}/game/{game_id}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", &format!("{base}/"))
            .timeout_secs(10)
            .max_bytes(8 * 1024 * 1024)
            .send()?;
        match r.status {
            200 => Ok(parse_game_page(base, &r.text()).map(|e| HowLongToBeat { match_kind: Some(MatchKind::Id), ..e })),
            404 => Ok(None),
            status => anyhow::bail!("HowLongToBeat answered {status}"),
        }
    }

    /// `getHowLongToBeat` (without the GOG store-page scrape): by HLTB id when
    /// known, else by title.
    pub fn lookup(&self, http: &Http, title: &str, hltb_id: Option<&str>) -> Option<HowLongToBeat> {
        self.lookup_checked(http, title, None, hltb_id).unwrap_or_else(|e| {
            log::error!("Error searching HLTB for {title}: {e}");
            None
        })
    }

    /// [`lookup`](Self::lookup) with the release year; `Err` when HLTB could
    /// not be asked, so that nothing is cached as "not found".
    pub fn lookup_checked(&self, http: &Http, title: &str, year: Option<i32>, hltb_id: Option<&str>) -> Result<Option<HowLongToBeat>> {
        if let Some(id) = hltb_id.filter(|i| !i.is_empty()) {
            if let Some(found) = self.by_id_checked(http, id)? {
                return Ok(Some(found));
            }
            log::info!("HLTB ID {id} not found for {title}");
        }
        let found = self.search_by_title_checked(http, title, year)?;
        if found.is_none() {
            log::info!("No HowLongToBeat data available for {title}");
        }
        Ok(found)
    }
}

// store_cache/wikigameinfo.json

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

/// What [`get_or_fetch_hltb_for`] looks a game up by.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HltbQuery {
    /// The Collection title (`collectionGameTitle`), the cache key.
    pub title: String,
    /// The store's own title, the second cache key.
    pub store_title: Option<String>,
    /// Release year, to tell apart games of the same name (remakes).
    pub year: Option<i32>,
}

impl HltbQuery {
    pub fn new(title: &str) -> Self {
        Self { title: title.trim().to_owned(), ..Self::default() }
    }

    fn key(&self) -> String {
        remove_special_characters(&self.title)
    }
}

/// The user's choice for one title in `local_library/hltb_overrides.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HltbOverride {
    /// The HowLongToBeat game to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u64>,
    /// "Not on HowLongToBeat": never searched again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub none: bool,
}

/// The override saved for `title`, if any.
pub fn override_for(paths: &DataPaths, title: &str) -> Option<HltbOverride> {
    let key = remove_special_characters(title.trim());
    let doc = json_store::read_value(&paths.hltb_overrides()).ok()?;
    serde_json::from_value(doc.get(&key)?.clone()).ok()
}

fn save_override(paths: &DataPaths, key: &str, choice: Option<HltbOverride>) -> Result<()> {
    json_store::update_value(&paths.hltb_overrides(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Map::new());
        }
        let map = doc.as_object_mut().expect("object");
        match choice {
            Some(c) => {
                map.insert(key.to_owned(), serde_json::to_value(c)?);
            }
            None => {
                map.shift_remove(key);
            }
        }
        Ok(())
    })
}

/// Writes the `howlongtobeat` of `key`'s entry (the other sources kept) with
/// a new timestamp.
fn store_entry(paths: &DataPaths, key: &str, existing: Option<Value>, found: Option<&HowLongToBeat>) -> Result<()> {
    let found_value = match found {
        Some(f) => serde_json::to_value(f)?,
        None => Value::Null,
    };
    let new_entry = match existing {
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
    json_store::update_value(&paths.wiki_game_info(), |doc| {
        dot_set(doc, key, new_entry);
        dot_set(doc, &format!("__timestamp.{key}"), Value::String(js_date_string()));
        Ok(())
    })
}

/// The HLTB half of `getWikiGameInfo(game, preferredTitle)`.
///
/// - an unexpired cached entry with HLTB data is returned as is;
/// - a cached entry without it is asked again once a week (the PCGamingWiki
///   HLTB id of the entry is used when present) and the entry is rewritten
///   with the new `howlongtobeat` and a new timestamp;
/// - with no entry, HLTB is searched by `title` and a new entry
///   (`pcgamingwiki`, `applegamingwiki`, `gamesdb`, `steamInfo`, `umuId` all
///   `null`: the other sources are not fetched here) is stored;
/// - a match the user picked ([`set_hltb_match`]) is fetched by its id, and
///   a title they marked as not on HowLongToBeat is never searched.
///
/// When HowLongToBeat can't be reached nothing is written, so the next call
/// asks again.
pub fn get_or_fetch_hltb(
    paths: &DataPaths,
    http: &Http,
    client: &HltbClient,
    title: &str,
    store_title: Option<&str>,
) -> Result<HltbLookup> {
    let query = HltbQuery { store_title: store_title.map(str::to_owned), ..HltbQuery::new(title) };
    get_or_fetch_hltb_for(paths, http, client, &query)
}

/// [`get_or_fetch_hltb`] with the release year.
pub fn get_or_fetch_hltb_for(paths: &DataPaths, http: &Http, client: &HltbClient, query: &HltbQuery) -> Result<HltbLookup> {
    fetch(paths, http, client, query, false)
}

/// Asks HowLongToBeat again for `query` now, whatever the cache holds (a
/// picked match by its id). `Err` when it could not be reached; the cached
/// entry is then kept.
pub fn refresh_hltb(paths: &DataPaths, http: &Http, client: &HltbClient, query: &HltbQuery) -> Result<HltbLookup> {
    client.refresh_tried.lock().remove(&query.key());
    fetch(paths, http, client, query, true)
}

fn fetch(paths: &DataPaths, http: &Http, client: &HltbClient, query: &HltbQuery, force: bool) -> Result<HltbLookup> {
    let title = query.title.trim();
    if title.is_empty() {
        return Ok(HltbLookup { entry: None, from_cache: false });
    }
    let key = query.key();
    let fallback_key = remove_special_characters(query.store_title.as_deref().unwrap_or(""));
    let file = paths.wiki_game_info();

    let doc = json_store::read_value(&file).unwrap_or(Value::Null);
    let fresh = |k: &str| cache_get(&doc, k).cloned();
    let cached = fresh(&key).or_else(|| (!fallback_key.is_empty() && fallback_key != key).then(|| fresh(&fallback_key)).flatten());
    let choice = override_for(paths, title);

    let hltb_of = |v: &Value| -> Option<HowLongToBeat> {
        v.get("howlongtobeat").filter(|h| h.is_object()).and_then(|h| serde_json::from_value(h.clone()).ok())
    };

    if choice.as_ref().is_some_and(|c| c.none) {
        // "Not on HowLongToBeat": the entry says so, nothing is asked.
        if cached.as_ref().is_some_and(|c| c.get("howlongtobeat").is_some_and(Value::is_null)) {
            return Ok(HltbLookup { entry: None, from_cache: true });
        }
        store_entry(paths, &key, dot_get(&doc, &key).cloned(), None)?;
        return Ok(HltbLookup { entry: None, from_cache: false });
    }

    if let (Some(cached), false) = (&cached, force) {
        if cached.get("howlongtobeat").is_some_and(|h| h.is_object()) {
            return Ok(HltbLookup { entry: hltb_of(cached), from_cache: true });
        }
        let stamp = dot_get(&doc, &format!("__timestamp.{key}")).and_then(Value::as_str).and_then(minutes_since);
        let recent = stamp.is_some_and(|m| m < NOT_FOUND_RETRY_MINUTES);
        if recent || !client.refresh_tried.lock().insert(key.clone()) {
            return Ok(HltbLookup { entry: None, from_cache: true });
        }
        log::info!("Refreshing HowLongToBeat data for {key}");
    }

    let found = match choice.and_then(|c| c.game_id) {
        Some(id) => client
            .by_id_checked(http, &id.to_string())?
            .map(|e| HowLongToBeat { match_kind: Some(MatchKind::Manual), ..e })
            // A picked game HLTB removed: keep the times already shown.
            .or_else(|| cached.as_ref().and_then(hltb_of)),
        None => {
            let hltb_id = cached
                .as_ref()
                .and_then(|c| c.pointer("/pcgamingwiki/howLongToBeatID"))
                .and_then(|v| v.as_str().map(str::to_owned).or_else(|| v.as_u64().map(|n| n.to_string())));
            match client.lookup_checked(http, title, query.year, hltb_id.as_deref()) {
                Ok(found) => found,
                Err(e) => {
                    log::info!("HowLongToBeat not reached for {title}: {e}");
                    if force {
                        return Err(e);
                    }
                    return Ok(HltbLookup { entry: cached.as_ref().and_then(hltb_of), from_cache: cached.is_some() });
                }
            }
        }
    };
    store_entry(paths, &key, cached.or_else(|| dot_get(&doc, &key).cloned()), found.as_ref())?;
    Ok(HltbLookup { entry: found, from_cache: false })
}

/// "Change match": `Some(hit)` uses that HowLongToBeat game for `title`
/// from now on (its times are stored right away, from the search result);
/// `None` marks the title as not on HowLongToBeat. Both stay until
/// [`clear_hltb_match`].
pub fn set_hltb_match(paths: &DataPaths, http: &Http, title: &str, hit: Option<&SearchHit>) -> Result<Option<HowLongToBeat>> {
    let key = remove_special_characters(title.trim());
    anyhow::ensure!(!key.is_empty(), "no title");
    let entry = hit
        .filter(|h| h.game_id != 0)
        .map(|h| HowLongToBeat { match_kind: Some(MatchKind::Manual), ..entry_from_hit(HltbClient::base(http), h) });
    let choice = match &entry {
        Some(e) => HltbOverride { game_id: e.game_id, none: false },
        None => HltbOverride { game_id: None, none: true },
    };
    save_override(paths, &key, Some(choice))?;
    let doc = json_store::read_value(&paths.wiki_game_info()).unwrap_or(Value::Null);
    store_entry(paths, &key, dot_get(&doc, &key).cloned(), entry.as_ref())?;
    Ok(entry)
}

/// Back to the automatic match for `title`: the choice and the cached entry
/// are dropped, the next lookup searches again.
pub fn clear_hltb_match(paths: &DataPaths, title: &str) -> Result<()> {
    let key = remove_special_characters(title.trim());
    save_override(paths, &key, None)?;
    forget_cached_title(paths, title)
}

/// `wikiGameInfoStore.delete(key)`: drops a cached title (forces a refetch).
pub fn forget_cached_title(paths: &DataPaths, title: &str) -> Result<()> {
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
    fn accents_and_other_scripts_match_and_empty_names_never_do() {
        let hit = |id: u64, name: &str| SearchHit { game_id: id, game_name: name.into(), ..Default::default() };
        assert_eq!(normalize_hltb_title("Pokémon Red"), "pokemonred");
        assert_eq!(search_terms("Pokémon Red"), ["Pokémon", "Red"]);
        // Decomposed (e + U+0301) the accent stays on its word.
        assert_eq!(search_terms("Poke\u{301}mon Red"), ["Poke\u{301}mon", "Red"]);
        assert_eq!(search_terms("ゼルダの伝説"), ["ゼルダの伝説"]);
        let hits = [hit(1, "Pokemon Blue"), hit(2, "Pokémon Red Version"), hit(3, "Pokemon Red")];
        assert_eq!(pick_search_match("Pokémon Red", &hits).unwrap().game_id, 3);
        let hits = [hit(1, "ゼルダの伝説"), hit(2, "ゼルダの伝説 夢をみる島")];
        assert_eq!(pick_search_match("ゼルダの伝説 夢をみる島", &hits).unwrap().game_id, 2);
        // A name that normalizes to nothing is not a prefix of the title, nor the title of it.
        let hits = [hit(1, "!!!"), hit(2, "Portal 2")];
        assert_eq!(pick_search_match("Portal 2: Sixense", &hits).unwrap().game_id, 2);
        let hits = [hit(1, "Portal"), hit(2, "Portal 2")];
        assert!(pick_search_match("「」", &hits).is_none(), "a title with no letters matches nothing");
        assert!(pick_search_match("Half-Life", &hits).is_none(), "no close name: no match, never the first result");
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
        assert!(minutes_since("Fri Sep 18 2026 21:30:21 GMT-0300 (Brasilia Standard Time)").is_some());
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
        let paths = DataPaths::at(dir.path());
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

    #[test]
    fn release_year_and_aliases_pick_the_right_game() {
        let hit = |id: u64, name: &str, year: Option<i32>| SearchHit {
            game_id: id,
            game_name: name.into(),
            release_world: year,
            ..Default::default()
        };
        // Two games of one name: the one released near the game's year.
        let hits = [hit(1, "Prey", Some(2006)), hit(2, "Prey", Some(2017))];
        assert_eq!(pick_match("Prey", Some(2017), &hits).map(|(h, k)| (h.game_id, k)), Some((2, MatchKind::Exact)));
        assert_eq!(pick_match("Prey", Some(2006), &hits).unwrap().0.game_id, 1);
        assert_eq!(pick_match("Prey", None, &hits).unwrap().0.game_id, 1, "no year: HLTB's order");
        // An exact name released years apart is still that name.
        assert_eq!(pick_match("Prey", Some(1990), &hits).unwrap().0.game_id, 1);
        // A close name released years apart is not taken.
        let hits = [hit(3, "Doom 3", Some(2004))];
        assert!(pick_match("Doom", Some(2016), &hits).is_none());
        assert_eq!(pick_match("Doom", None, &hits).map(|(h, k)| (h.game_id, k)), Some((3, MatchKind::Close)));
        // HLTB's other names count as exact.
        let hits = [SearchHit { game_alias: Some("Biohazard 4, RE4".into()), ..hit(4, "Resident Evil 4", Some(2005)) }];
        assert_eq!(pick_match("RE4", None, &hits).map(|(h, k)| (h.game_id, k)), Some((4, MatchKind::Exact)));
        // Years come as numbers or strings; 0 is none.
        let v: Vec<SearchHit> = serde_json::from_value(json!([
            {"game_id": 1, "game_name": "A", "release_world": 2011},
            {"game_id": 2, "game_name": "B", "release_world": "2013"},
            {"game_id": 3, "game_name": "C", "release_world": 0},
            {"game_id": 4, "game_name": "D"}
        ]))
        .unwrap();
        assert_eq!(v.iter().map(|h| h.release_world).collect::<Vec<_>>(), [Some(2011), Some(2013), None, None]);
    }

    #[test]
    fn picked_match_and_not_on_hltb_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::create_dir_all(paths.wiki_game_info().parent().unwrap()).unwrap();
        // Unreachable: any request fails, so every answer below is offline.
        let http = Http::new(crate::http::Endpoints { hltb: "http://127.0.0.1:9".into(), ..Default::default() });
        let client = HltbClient::new();
        let pick = &hits()[0];
        let saved = set_hltb_match(&paths, &http, "My Portal", Some(pick)).unwrap().unwrap();
        assert_eq!((saved.game_id, saved.match_kind), (Some(7231), Some(MatchKind::Manual)));
        assert_eq!(override_for(&paths, "My Portal"), Some(HltbOverride { game_id: Some(7231), none: false }));
        let got = get_or_fetch_hltb(&paths, &http, &client, "My Portal", None).unwrap();
        assert!(got.from_cache);
        assert_eq!(got.entry.unwrap().game_id, Some(7231));
        let wiki = WikiCache::load(&paths);
        assert!(wiki.hltb("My Portal", "").unwrap().is_manual_match());

        // Not on HowLongToBeat: an empty entry, never searched.
        set_hltb_match(&paths, &http, "Homebrew", None).unwrap();
        let got = get_or_fetch_hltb(&paths, &http, &client, "Homebrew", None).unwrap();
        assert_eq!(got, HltbLookup { entry: None, from_cache: true });
        let e = WikiCache::load(&paths).lookup("Homebrew", "").unwrap();
        assert!(e.hltb_checked && e.hltb.is_none());
        // A refresh keeps the choice and still asks nothing.
        assert!(refresh_hltb(&paths, &http, &client, &HltbQuery::new("Homebrew")).unwrap().entry.is_none());

        // Unreachable: nothing is written, a refresh says so.
        assert_eq!(get_or_fetch_hltb(&paths, &http, &client, "Unknown", None).unwrap().entry, None);
        assert!(WikiCache::load(&paths).lookup("Unknown", "").is_none(), "no 'not found' cached while offline");
        assert!(refresh_hltb(&paths, &http, &client, &HltbQuery::new("Unknown")).is_err());

        // Back to automatic.
        clear_hltb_match(&paths, "My Portal").unwrap();
        assert_eq!(override_for(&paths, "My Portal"), None);
        assert!(WikiCache::load(&paths).lookup("My Portal", "").is_none());
    }
}
