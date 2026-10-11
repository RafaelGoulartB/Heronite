//! Achievements of the Focus panel (`window.api.getAchievements`). Heroic
//! only implements them for GOG (`gog/games.ts` `getAchievements`): the list
//! is cached forever in `store_cache/gog_achievements.json` (a `CacheStore`
//! without lifespan, keyed by the GOG product id) and otherwise fetched from
//! `gameplay.gog.com` with the logged-in user's token. Every other store
//! answers an empty list.
//!
//! The token comes from `gog_store/auth.json` (the file `gogdl auth` keeps,
//! brought over by the Heroic migration). This app has no GOG login and never
//! refreshes it: an expired token simply means no fresh achievements.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::http::Http;

pub const GAMEPLAY_BASE: &str = "https://gameplay.gog.com";
/// GOG Galaxy client id gogdl logs in with (key of `auth.json`).
pub const GALAXY_CLIENT_ID: &str = "46899977096215655";
/// Rows the Focus panel shows (`.slice(0, 8)`).
pub const SHOWN_ACHIEVEMENTS: usize = 8;

/// `GOGAchievement`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Achievement {
    #[serde(default)]
    pub achievement_id: String,
    #[serde(default)]
    pub achievement_key: String,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub image_url_unlocked: String,
    #[serde(default)]
    pub image_url_locked: String,
    #[serde(default)]
    pub rarity: f64,
    #[serde(default)]
    pub date_unlocked: Option<String>,
    #[serde(default)]
    pub rarity_level_description: String,
    #[serde(default)]
    pub rarity_level_slug: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Achievement {
    /// `item.date_unlocked` truthiness.
    pub fn unlocked(&self) -> bool {
        self.date_unlocked.as_deref().is_some_and(|d| !d.is_empty())
    }

    /// Image shown in the list: unlocked or locked artwork.
    pub fn image_url(&self) -> &str {
        if self.unlocked() {
            &self.image_url_unlocked
        } else {
            &self.image_url_locked
        }
    }

    /// Name shown in the list (`item.visible ? item.name : '???'`).
    pub fn display_name(&self) -> &str {
        if self.visible {
            &self.name
        } else {
            "???"
        }
    }
}

/// `unlockedCount` and the rows of the panel: unlocked first, then locked
/// (each in stored order), at most [`SHOWN_ACHIEVEMENTS`].
pub fn panel_rows(items: &[Achievement]) -> (usize, Vec<&Achievement>) {
    let unlocked = items.iter().filter(|a| a.unlocked());
    let locked = items.iter().filter(|a| !a.unlocked());
    let count = items.iter().filter(|a| a.unlocked()).count();
    (count, unlocked.chain(locked).take(SHOWN_ACHIEVEMENTS).collect())
}

// Cache (`store_cache/gog_achievements.json`)

fn cache_path(paths: &DataPaths) -> std::path::PathBuf {
    paths.store_cache("gog_achievements")
}

/// `achievementStore.get(id)`: the cached list, whatever its age.
pub fn read_cache(paths: &DataPaths, product_id: &str) -> Option<Vec<Achievement>> {
    let doc = json_store::read_value(&cache_path(paths)).ok()?;
    serde_json::from_value(doc.get(product_id)?.clone()).ok()
}

/// `achievementStore.set(id, items)`: value plus `__timestamp.<id>` (a JS
/// `Date()` string, nested under `__timestamp` by electron-store's dot paths).
pub fn write_cache(paths: &DataPaths, product_id: &str, items: &[Achievement]) -> Result<()> {
    json_store::update_value(&cache_path(paths), |doc| {
        let obj = doc.as_object_mut().expect("object");
        obj.insert(product_id.to_string(), serde_json::to_value(items)?);
        let stamps = obj.entry("__timestamp").or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(m) = stamps {
            m.insert(product_id.to_string(), Value::String(js_date_now()));
        }
        Ok(())
    })
}

/// Node's `Date()`: `Thu Oct 08 2026 10:30:00 GMT-0300 (Local Time)`.
fn js_date_now() -> String {
    chrono::Local::now().format("%a %b %d %Y %H:%M:%S GMT%z (Local Time)").to_string()
}

// Credentials (`gog_store/auth.json`)

/// What the request needs from `auth.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    pub user_id: String,
    pub access_token: String,
}

/// Parses `auth.json` (`{ "<client id>": { access_token, user_id, expires_in,
/// loginTime } }`); `None` when it is missing, unusable or expired at `now`
/// (unix seconds).
pub fn parse_credentials(doc: &Value, now: f64) -> Option<Credentials> {
    let entry = doc.get(GALAXY_CLIENT_ID).or_else(|| doc.as_object()?.values().next())?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    let (access_token, user_id) = (text("access_token")?, text("user_id")?);
    let number = |key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    // gogdl refreshes when `loginTime + expires_in < now`; keep a minute of margin.
    if number("loginTime") + number("expires_in") - 60.0 < now {
        return None;
    }
    Some(Credentials { user_id, access_token })
}

fn credentials(paths: &DataPaths) -> Option<Credentials> {
    let doc = json_store::read_value(&paths.gog_store("auth")).ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    parse_credentials(&doc, now)
}

/// `isLoggedIn()`: `gog_store/config.json` `isLoggedIn`.
pub fn gog_logged_in(paths: &DataPaths) -> bool {
    json_store::read_value(&paths.gog_store("config")).ok().and_then(|d| d.get("isLoggedIn").and_then(Value::as_bool)).unwrap_or(false)
}

/// `GOGGame.getAchievements`: cache, then the gameplay API. Blocking
/// (network): call from a worker thread. Failures give an empty list.
pub fn fetch_gog(paths: &DataPaths, http: &Http, gameplay_base: &str, product_id: &str, lang: &str) -> Vec<Achievement> {
    if let Some(cached) = read_cache(paths, product_id) {
        return cached;
    }
    if !gog_logged_in(paths) {
        return Vec::new();
    }
    let Some(creds) = credentials(paths) else { return Vec::new() };
    let url = format!("{gameplay_base}/clients/{product_id}/users/{}/achievements", creds.user_id);
    let response = http.get(url).header("Authorization", &format!("Bearer {}", creds.access_token)).header("X-Gog-Lc", lang).send();
    let items: Vec<Achievement> = match response {
        Ok(r) if r.ok() => match r.json::<Value>().ok().and_then(|v| serde_json::from_value(v.get("items")?.clone()).ok()) {
            Some(items) => items,
            None => return Vec::new(),
        },
        Ok(r) => {
            log::warn!("GOG achievements for {product_id}: HTTP {}", r.status);
            return Vec::new();
        }
        Err(e) => {
            log::warn!("GOG achievements for {product_id}: {e:#}");
            return Vec::new();
        }
    };
    if let Err(e) = write_cache(paths, product_id, &items) {
        log::warn!("could not cache GOG achievements: {e:#}");
    }
    items
}

/// `getAchievements(appName, runner)`: only GOG has them.
pub fn load_achievements(paths: &DataPaths, http: &Http, runner: &str, app_name: &str) -> Vec<Achievement> {
    if runner != "gog" {
        return Vec::new();
    }
    fetch_gog(paths, http, GAMEPLAY_BASE, app_name, "en-US")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, unlocked: bool, visible: bool) -> Achievement {
        Achievement {
            achievement_id: id.into(),
            name: format!("Name {id}"),
            visible,
            date_unlocked: unlocked.then(|| "2026-01-01T00:00:00+0000".to_string()),
            image_url_unlocked: format!("u{id}"),
            image_url_locked: format!("l{id}"),
            ..Default::default()
        }
    }

    #[test]
    fn panel_puts_unlocked_first_and_caps_at_eight() {
        let items: Vec<_> = (0..12).map(|i| item(&i.to_string(), i % 3 == 0, true)).collect();
        let (unlocked, rows) = panel_rows(&items);
        assert_eq!(unlocked, 4);
        assert_eq!(rows.len(), 8);
        let ids: Vec<_> = rows.iter().map(|a| a.achievement_id.as_str()).collect();
        assert_eq!(ids, ["0", "3", "6", "9", "1", "2", "4", "5"]);
        assert_eq!(rows[0].image_url(), "u0");
        assert_eq!(rows[4].image_url(), "l1");
    }

    #[test]
    fn hidden_achievements_show_question_marks() {
        assert_eq!(item("1", false, false).display_name(), "???");
        assert_eq!(item("1", false, true).display_name(), "Name 1");
        let mut empty_date = item("1", false, true);
        empty_date.date_unlocked = Some(String::new());
        assert!(!empty_date.unlocked());
    }

    #[test]
    fn parses_the_gog_payload_keeping_unknown_fields() {
        let v = json!({"achievement_id":"1","achievement_key":"K","visible":true,"name":"N","description":"D",
            "image_url_unlocked":"u","image_url_locked":"l","rarity":1.5,"date_unlocked":null,
            "rarity_level_description":"Common","rarity_level_slug":"common","future":"x"});
        let a: Achievement = serde_json::from_value(v.clone()).unwrap();
        assert!(!a.unlocked());
        assert_eq!(serde_json::to_value(&a).unwrap(), v);
    }

    #[test]
    fn credentials_need_an_unexpired_token() {
        let doc = json!({ GALAXY_CLIENT_ID: {"access_token":"t","user_id":"7","expires_in":3600,"loginTime":1000.0}});
        assert_eq!(parse_credentials(&doc, 2000.0), Some(Credentials { user_id: "7".into(), access_token: "t".into() }));
        assert_eq!(parse_credentials(&doc, 4600.0), None);
        assert_eq!(parse_credentials(&json!({}), 0.0), None);
        assert_eq!(parse_credentials(&json!({"x": {"user_id": "7"}}), 0.0), None);
    }

    #[test]
    fn cache_round_trip_uses_nested_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store_cache")).unwrap();
        assert!(read_cache(&paths, "1").is_none());
        write_cache(&paths, "1", &[item("a", true, true)]).unwrap();
        write_cache(&paths, "2", &[]).unwrap();
        let doc = json_store::read_value(&cache_path(&paths)).unwrap();
        assert!(doc["__timestamp"]["1"].as_str().unwrap().contains("GMT"));
        assert_eq!(read_cache(&paths, "1").unwrap()[0].achievement_id, "a");
        // An empty list is a valid cache entry (`[]` is truthy in the TS).
        assert_eq!(read_cache(&paths, "2"), Some(vec![]));
    }
}
