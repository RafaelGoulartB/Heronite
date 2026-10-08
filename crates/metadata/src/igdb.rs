//! IGDB provider: port of `metadata/igdb.ts` (Twitch OAuth client-credentials
//! token cache, Apicalypse queries, one request at a time at least 280 ms
//! apart).

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use parking_lot::Mutex;
use serde::Deserialize;

use crate::http::Http;
use crate::types::{IgdbCredentialTest, MetadataSearchHit};
use crate::util::{blank_metadata, clip, pick_best_match, title_queries, unique_names, unique_names12, MIN_MATCH_SCORE};

/// `external_game_source` / old `category` value of Steam.
const STEAM_EXTERNAL: i64 = 1;
const TOKEN_SKEW: Duration = Duration::from_secs(60);
const MIN_GAP: Duration = Duration::from_millis(280);

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Named {
    #[serde(default)]
    pub name: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Image {
    #[serde(default)]
    pub image_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Company {
    #[serde(default)]
    pub developer: bool,
    #[serde(default)]
    pub publisher: bool,
    #[serde(default)]
    pub company: Option<Named>,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Website {
    #[serde(default)]
    pub url: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct External {
    #[serde(default)]
    pub uid: Option<String>,
    /// Deprecated by IGDB (no longer returned); kept for older responses.
    #[serde(default)]
    pub category: Option<i64>,
    #[serde(default)]
    pub external_game_source: Option<i64>,
}

/// One `games` row of the v4 API.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct IgdbGame {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub storyline: Option<String>,
    #[serde(default)]
    pub first_release_date: Option<f64>,
    #[serde(default)]
    pub aggregated_rating: Option<f64>,
    #[serde(default)]
    pub total_rating: Option<f64>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub cover: Option<Image>,
    #[serde(default)]
    pub screenshots: Vec<Image>,
    #[serde(default)]
    pub artworks: Vec<Image>,
    #[serde(default)]
    pub genres: Vec<Named>,
    #[serde(default)]
    pub themes: Vec<Named>,
    #[serde(default)]
    pub game_modes: Vec<Named>,
    #[serde(default)]
    pub platforms: Vec<Named>,
    /// Deprecated singular form.
    #[serde(default)]
    pub collection: Option<Named>,
    #[serde(default)]
    pub collections: Vec<Named>,
    #[serde(default)]
    pub franchises: Vec<Named>,
    #[serde(default)]
    pub involved_companies: Vec<Company>,
    #[serde(default)]
    pub websites: Vec<Website>,
    #[serde(default)]
    pub external_games: Vec<External>,
}

const GAME_FIELDS: &str = "name,slug,summary,storyline,first_release_date,aggregated_rating,total_rating,url,cover.image_id,screenshots.image_id,artworks.image_id,genres.name,themes.name,game_modes.name,platforms.name,collection.name,collections.name,franchises.name,involved_companies.developer,involved_companies.publisher,involved_companies.company.name,websites.url,websites.category,external_games.uid,external_games.category,external_games.external_game_source";

/// `igdbImageUrl`.
pub fn igdb_image_url(image_id: Option<&str>, size: &str) -> Option<String> {
    let id = image_id.filter(|i| !i.is_empty())?;
    Some(format!("https://images.igdb.com/igdb/image/upload/t_{size}/{id}.jpg"))
}

/// `escapeApicalypse`.
pub fn escape_apicalypse(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `releaseDate`: `YYYY-MM-DD` (UTC) of a unix timestamp.
fn release_date(value: Option<f64>) -> Option<String> {
    let v = value.filter(|v| *v != 0.0)?;
    chrono::DateTime::from_timestamp(v as i64, 0).map(|d| d.format("%Y-%m-%d").to_string())
}

fn year_from_unix(value: Option<f64>) -> Option<i32> {
    use chrono::Datelike;
    let v = value.filter(|v| *v != 0.0)?;
    chrono::DateTime::from_timestamp(v as i64, 0).map(|d| d.year())
}

/// `mapGame`: IGDB row to a metadata candidate.
pub fn map_game(app_name: &str, runner: &str, game: &IgdbGame) -> CollectionGameMetadata {
    let companies = |pred: fn(&Company) -> bool| {
        unique_names12(
            game.involved_companies
                .iter()
                .filter(|c| pred(c))
                .map(|c| c.company.as_ref().and_then(|n| n.name.as_deref())),
        )
    };
    let names = |list: &[Named], max: usize| unique_names(list.iter().map(|n| n.name.as_deref()), max);

    let websites = unique_names(
        std::iter::once(game.url.as_deref())
            .chain(game.websites.iter().map(|w| w.url.as_deref()))
            .filter(|u| u.is_some_and(|u| !u.is_empty())),
        4,
    );
    let steam = game.external_games.iter().find(|e| {
        (e.category == Some(STEAM_EXTERNAL) || e.external_game_source == Some(STEAM_EXTERNAL))
            && e.uid.as_deref().is_some_and(|u| !u.is_empty())
    });
    let cover = igdb_image_url(game.cover.as_ref().and_then(|c| c.image_id.as_deref()), "cover_big_2x");
    let hero = igdb_image_url(game.artworks.first().and_then(|a| a.image_id.as_deref()), "720p")
        .or_else(|| igdb_image_url(game.screenshots.first().and_then(|a| a.image_id.as_deref()), "screenshot_huge"));
    let non_empty = |n: &Named| n.name.clone().filter(|s| !s.is_empty());
    let series = game
        .collection
        .as_ref()
        .and_then(non_empty)
        .or_else(|| game.collections.first().and_then(non_empty))
        .or_else(|| game.franchises.first().and_then(non_empty));
    let score = game.aggregated_rating.or(game.total_rating).map(|r| JsNum(r.round()));

    let mut md = blank_metadata(app_name, runner);
    md.igdb_id = Some(JsNum(game.id as f64));
    md.igdb_slug = game.slug.clone();
    md.steam_app_id = steam.and_then(|s| s.uid.clone());
    md.title = game.name.clone();
    md.description = clip(game.summary.as_deref().filter(|s| !s.is_empty()).or(game.storyline.as_deref()));
    md.release_date = release_date(game.first_release_date);
    md.developers = companies(|c| c.developer);
    md.publishers = companies(|c| c.publisher);
    md.genres = names(&game.genres, 12);
    md.themes = names(&game.themes, 12);
    md.game_modes = names(&game.game_modes, 12);
    md.platforms = names(&game.platforms, 10);
    md.series = series;
    md.critic_score = score;
    md.websites = websites;
    md.cover_url = cover;
    md.hero_url = hero;
    md
}

struct Token {
    access: String,
    expires_at: Instant,
    client_id: String,
}

/// IGDB client. Share it (`Arc`) so the token cache and the request queue
/// are common to every thread.
#[derive(Default)]
pub struct Igdb {
    token: Mutex<Option<Token>>,
    /// Held for the whole request: serializes calls; value is the last call.
    queue: Mutex<Option<Instant>>,
}

/// Twitch Client ID / Secret.
#[derive(Debug, Clone, Default)]
pub struct IgdbCredentials {
    pub client_id: String,
    pub client_secret: String,
}

impl IgdbCredentials {
    pub fn new(id: &str, secret: &str) -> Self {
        Self { client_id: id.trim().to_owned(), client_secret: secret.trim().to_owned() }
    }

    pub fn configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }
}

/// Request failure that remembers the HTTP status (for the 401 retry).
#[derive(Debug)]
struct StatusError(u16, String);
impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1)
    }
}
impl std::error::Error for StatusError {}

impl Igdb {
    pub fn new() -> Self {
        Self::default()
    }

    fn twitch_token(&self, http: &Http, creds: &IgdbCredentials, force: bool) -> Result<String> {
        if !creds.configured() {
            return Err(anyhow!("IGDB credentials are not configured"));
        }
        {
            let guard = self.token.lock();
            if let Some(t) = guard.as_ref() {
                if !force && t.client_id == creds.client_id && t.expires_at > Instant::now() + TOKEN_SKEW {
                    return Ok(t.access.clone());
                }
            }
        }
        let resp = http
            .post(&http.endpoints.twitch_token)
            .query(&[
                ("client_id", creds.client_id.as_str()),
                ("client_secret", creds.client_secret.as_str()),
                ("grant_type", "client_credentials"),
            ])
            .timeout_secs(15)
            .send()?;
        if !resp.ok() {
            return Err(resp.error());
        }
        #[derive(Deserialize)]
        struct Tok {
            access_token: Option<String>,
            expires_in: Option<f64>,
        }
        let tok: Tok = resp.json()?;
        let access = tok.access_token.filter(|a| !a.is_empty()).ok_or_else(|| anyhow!("Twitch did not return an IGDB token"))?;
        *self.token.lock() = Some(Token {
            access: access.clone(),
            client_id: creds.client_id.clone(),
            expires_at: Instant::now() + Duration::from_secs_f64(tok.expires_in.unwrap_or(3600.0).max(0.0)),
        });
        Ok(access)
    }

    fn post_once(&self, http: &Http, creds: &IgdbCredentials, path: &str, body: &str) -> Result<serde_json::Value> {
        let token = self.twitch_token(http, creds, false)?;
        let resp = http
            .post(format!("{}/{path}", http.endpoints.igdb))
            .header("Client-ID", &creds.client_id)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", "application/json")
            .header("Content-Type", "text/plain")
            .body(body)
            .timeout_secs(20)
            .send()?;
        if !resp.ok() {
            return Err(StatusError(resp.status, resp.error().to_string()).into());
        }
        resp.json()
    }

    /// `igdbPost`: one retry with a fresh token on 401.
    fn post(&self, http: &Http, creds: &IgdbCredentials, path: &str, body: &str) -> Result<serde_json::Value> {
        match self.post_once(http, creds, path, body) {
            Err(e) if e.downcast_ref::<StatusError>().is_some_and(|s| s.0 == 401) => {
                self.twitch_token(http, creds, true)?;
                self.post_once(http, creds, path, body)
            }
            other => other,
        }
    }

    /// `queryGames` / `enqueue`: serialized, >= 280 ms between calls.
    pub fn query_games(&self, http: &Http, creds: &IgdbCredentials, body: &str) -> Result<Vec<IgdbGame>> {
        let mut last = self.queue.lock();
        if let Some(at) = *last {
            let wait = MIN_GAP.saturating_sub(at.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        *last = Some(Instant::now());
        let data = self.post(http, creds, "games", body);
        *last = Some(Instant::now());
        let data = data?;
        Ok(match data {
            serde_json::Value::Array(rows) => rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect(),
            _ => Vec::new(),
        })
    }

    /// `testIgdbCredentials`.
    pub fn test_credentials(&self, http: &Http, creds: &IgdbCredentials) -> IgdbCredentialTest {
        if !creds.configured() {
            return IgdbCredentialTest { ok: false, error: Some("Add a Twitch Client ID and Client Secret first.".into()) };
        }
        let result = self
            .twitch_token(http, creds, true)
            .and_then(|_| self.query_games(http, creds, "fields id; limit 1;"));
        match result {
            Ok(_) => IgdbCredentialTest { ok: true, error: None },
            Err(e) => {
                log::warn!("IGDB credential test failed: {e}");
                IgdbCredentialTest { ok: false, error: Some(e.to_string()) }
            }
        }
    }

    /// `searchIgdbGames`: up to 8 hits for the title queries.
    pub fn search(&self, http: &Http, creds: &IgdbCredentials, title: &str) -> Vec<MetadataSearchHit> {
        if !creds.configured() || title.trim().is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<MetadataSearchHit> = Vec::new();
        for query in title_queries(title) {
            let body = format!(
                "search \"{}\"; fields name,slug,first_release_date; limit 8;",
                escape_apicalypse(&query)
            );
            match self.query_games(http, creds, &body) {
                Ok(games) => {
                    for game in games {
                        let id = game.id.to_string();
                        let Some(name) = game.name.clone().filter(|n| !n.is_empty()) else { continue };
                        if hits.iter().any(|h| h.id == id) {
                            continue;
                        }
                        hits.push(MetadataSearchHit {
                            source: "igdb".into(),
                            id,
                            name,
                            year: year_from_unix(game.first_release_date),
                            extra: game.slug.clone(),
                        });
                    }
                    if hits.len() >= 8 {
                        break;
                    }
                }
                Err(e) => log::warn!("IGDB search failed for {query}: {e}"),
            }
        }
        hits.truncate(8);
        hits
    }

    fn by_id(&self, http: &Http, creds: &IgdbCredentials, id: i64) -> Result<Option<IgdbGame>> {
        let body = format!("fields {GAME_FIELDS}; where id = {id}; limit 1;");
        Ok(self.query_games(http, creds, &body)?.into_iter().next())
    }

    /// `gameBySteamId`. IGDB stopped returning `external_games.category`, so
    /// the query uses `external_game_source` first and the old `category`
    /// filter as a fallback.
    fn by_steam_id(&self, http: &Http, creds: &IgdbCredentials, steam_app_id: &str) -> Result<Option<IgdbGame>> {
        let uid = escape_apicalypse(steam_app_id);
        for field in ["external_game_source", "category"] {
            let body = format!(
                "fields {GAME_FIELDS}; where external_games.uid = \"{uid}\" & external_games.{field} = {STEAM_EXTERNAL}; limit 1;"
            );
            if let Some(game) = self.query_games(http, creds, &body)?.into_iter().next() {
                return Ok(Some(game));
            }
        }
        Ok(None)
    }

    fn by_title(&self, http: &Http, creds: &IgdbCredentials, title: &str) -> Result<Option<IgdbGame>> {
        for query in title_queries(title) {
            let body = format!("search \"{}\"; fields {GAME_FIELDS}; limit 8;", escape_apicalypse(&query));
            let games = self.query_games(http, creds, &body)?;
            if let Some(best) = pick_best_match(title, &games, |g| g.name.clone().unwrap_or_default(), MIN_MATCH_SCORE) {
                return Ok(Some(best.clone()));
            }
        }
        Ok(None)
    }

    /// `fetchIgdbMetadata`: by IGDB id, else Steam app id, else title.
    pub fn fetch(
        &self,
        http: &Http,
        creds: &IgdbCredentials,
        app_name: &str,
        runner: &str,
        title: &str,
        steam_app_id: Option<&str>,
        igdb_id: Option<i64>,
    ) -> Option<CollectionGameMetadata> {
        if !creds.configured() {
            return None;
        }
        let result = (|| -> Result<Option<IgdbGame>> {
            let mut game = None;
            if let Some(id) = igdb_id.filter(|i| *i != 0) {
                game = self.by_id(http, creds, id)?;
            }
            if game.is_none() {
                if let Some(s) = steam_app_id.filter(|s| !s.is_empty()) {
                    game = self.by_steam_id(http, creds, s)?;
                }
            }
            if game.is_none() {
                game = self.by_title(http, creds, title)?;
            }
            Ok(game)
        })();
        match result {
            Ok(Some(game)) => Some(map_game(app_name, runner, &game)),
            Ok(None) => None,
            Err(e) => {
                log::warn!("IGDB fetch failed for {title}: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn maps_a_recorded_igdb_game() {
        let rows: Vec<IgdbGame> = serde_json::from_str(&fixture("igdb_game_steam_620.json")).unwrap();
        let md = map_game("steam_620", "sideload", &rows[0]);
        assert_eq!(md.title.as_deref(), Some("Portal 2"));
        assert_eq!(md.igdb_id, Some(JsNum(72.0)));
        assert_eq!(md.igdb_slug.as_deref(), Some("portal-2"));
        assert_eq!(md.steam_app_id.as_deref(), Some("620"));
        assert_eq!(md.release_date.as_deref(), Some("2011-04-18"));
        assert!(md.developers.iter().any(|d| d == "Valve"));
        assert!(md.publishers.iter().any(|d| d == "Valve"));
        assert!(!md.genres.is_empty() && !md.game_modes.is_empty() && !md.platforms.is_empty());
        assert!(md.platforms.len() <= 10);
        assert_eq!(md.series.as_deref(), Some("Portal"));
        assert!(md.critic_score.is_some_and(|s| s.get() >= 80.0));
        let cover = md.cover_url.unwrap();
        assert!(cover.starts_with("https://images.igdb.com/igdb/image/upload/t_cover_big_2x/") && cover.ends_with(".jpg"));
        assert!(md.hero_url.unwrap().contains("/t_720p/"));
        assert!(md.description.is_some_and(|d| !d.contains('\n')));
        assert!(md.websites.len() <= 4 && !md.websites.is_empty());
    }

    #[test]
    fn steam_id_from_legacy_category_also_works() {
        let game: IgdbGame = serde_json::from_str(
            r#"{"id":1,"name":"X","external_games":[{"uid":"99","category":1}]}"#,
        )
        .unwrap();
        assert_eq!(map_game("a", "sideload", &game).steam_app_id.as_deref(), Some("99"));
    }

    #[test]
    fn search_row_shape() {
        let rows: Vec<IgdbGame> = serde_json::from_str(&fixture("igdb_search_portal2.json")).unwrap();
        let p2 = rows.iter().find(|g| g.id == 72).unwrap();
        assert_eq!(year_from_unix(p2.first_release_date), Some(2011));
        assert_eq!(escape_apicalypse(r#"a "b" \c"#), r#"a \"b\" \\c"#);
    }

    #[test]
    fn no_credentials_means_nothing() {
        let http = Http::default();
        let igdb = Igdb::new();
        let none = IgdbCredentials::default();
        assert!(igdb.search(&http, &none, "x").is_empty());
        assert!(igdb.fetch(&http, &none, "a", "sideload", "x", None, None).is_none());
        assert!(!igdb.test_credentials(&http, &none).ok);
    }
}
