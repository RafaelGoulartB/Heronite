//! The umu id (`GAMEID`) of a game: what umu and Proton know it by, so
//! Proton builds with protonfixes (GE-Proton, UMU-Proton, Proton-CachyOS...)
//! apply the fixes written for it. `umu-0` is "unknown game": no fixes.
//!
//! umu turns `GAMEID=umu-<n>` into `UMU_ID`, and into `SteamAppId=<n>` for
//! Proton's own per-game settings. protonfixes then looks the fix up in the
//! folder of the store (`STORE`: egs, gog, amazon) or its umu folder.
//!
//! The id comes from, in order:
//! 1. the game's own setting (`heroniteUmuId`), set by the user;
//! 2. the umu database (umu.openwinecomponents.org), by store and codename
//!    (Epic's app name, GOG's product id, Amazon's id);
//! 3. the ids cached by Heroic (`store_cache/umu.json`);
//! 4. the Steam id of the game's metadata, when the database has it;
//! 5. a database entry with exactly the game's title.
//!
//! The database is downloaded at start, at most once a week
//! (`store_cache/umu_database.json`); building a plan only reads it.

use std::fs;
use std::time::{Duration, SystemTime};

use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::net::Net;
use crate::plan::{LaunchPlan, PlanContext, WineRuntime};

/// The game's own id (a Heronite key, not inherited from the defaults).
pub const KEY: &str = "heroniteUmuId";
/// The umu database page, to look an id up by hand.
pub const WEBSITE: &str = "https://umu.openwinecomponents.org/";
/// Every entry of the database (about 1200, ~200 KB).
pub const API_URL: &str = "https://umu.openwinecomponents.org/umu_api.php";
/// Unknown game.
pub const UNKNOWN: &str = "umu-0";
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const CACHE_NAME: &str = "umu_database";

/// One row of the database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub umu_id: String,
    /// The store's id of the game (`none` for standalone installers).
    #[serde(default)]
    pub codename: Option<String>,
    /// `egs`, `gog`, `amazon`, `humble`, `none`...
    #[serde(default)]
    pub store: Option<String>,
}

/// Where the id came from (the settings show it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    User,
    Store,
    HeroicCache,
    SteamId,
    Title,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub source: Source,
    /// The database's title for the id, when it has one.
    pub title: Option<String>,
}

/// What [`resolve`] looks at for one game.
#[derive(Debug, Clone, Copy, Default)]
pub struct GameKeys<'a> {
    pub runner: &'a str,
    pub app_name: &'a str,
    pub title: &'a str,
    /// The Steam id of the game's metadata.
    pub steam_app_id: Option<&'a str>,
    /// `heroniteUmuId`.
    pub user_id: &'a str,
}

/// The database's store code for a runner (`STORE` too).
pub fn store_of(runner: &str) -> Option<&'static str> {
    match runner {
        "legendary" => Some("egs"),
        "gog" => Some("gog"),
        "nile" => Some("amazon"),
        _ => None,
    }
}

/// An id typed by the user: a bare Steam id becomes `umu-<id>`.
pub fn normalize(id: &str) -> String {
    let id = id.trim();
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        return format!("umu-{id}");
    }
    id.to_string()
}

/// Lower-case letters and digits only, so "DOOM: The Dark Ages" and
/// "Doom - The Dark Ages" compare equal.
fn title_key(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The game's id, `None` when nothing knows it (umu then uses `umu-0`).
pub fn resolve(db: &[Entry], heroic_cache: Option<&Value>, keys: GameKeys) -> Option<Resolved> {
    let title_of = |id: &str| db.iter().find(|e| e.umu_id == id).map(|e| e.title.clone()).filter(|t| !t.is_empty());
    let found = |id: String, source: Source| Some(Resolved { title: title_of(&id), id, source });
    let user = normalize(keys.user_id);
    if !user.is_empty() {
        return found(user, Source::User);
    }
    if let Some(store) = store_of(keys.runner) {
        let hit = db
            .iter()
            .find(|e| e.store.as_deref() == Some(store) && e.codename.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(keys.app_name)));
        if let Some(e) = hit.filter(|e| !e.umu_id.is_empty()) {
            return found(e.umu_id.clone(), Source::Store);
        }
        let cached = heroic_cache
            .and_then(|c| c.get(format!("{}_{}", keys.runner, keys.app_name)))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && *s != UNKNOWN);
        if let Some(id) = cached {
            return found(id.to_string(), Source::HeroicCache);
        }
    }
    if let Some(steam) = keys.steam_app_id.filter(|s| !s.is_empty()) {
        let id = format!("umu-{steam}");
        if db.iter().any(|e| e.umu_id == id) {
            return found(id, Source::SteamId);
        }
    }
    let key = title_key(keys.title);
    if !key.is_empty() {
        let mut ids: Vec<&str> = db.iter().filter(|e| title_key(&e.title) == key).map(|e| e.umu_id.as_str()).collect();
        ids.dedup();
        // Two games with the same title: guessing could apply the wrong fixes.
        if let [id] = ids.as_slice() {
            return found(id.to_string(), Source::Title);
        }
    }
    None
}

/// Database games that may be this one, for the user to pick from: titles
/// that start with the game's title ("Control" → "Control Ultimate
/// Edition") or that it starts with (an edition's longer name), never a
/// title that only contains it ("Ground Control"). One per id, at most `limit`.
pub fn suggestions<'a>(db: &'a [Entry], title: &str, limit: usize) -> Vec<&'a Entry> {
    let key = title_key(title);
    if key.len() < 3 {
        return Vec::new();
    }
    let mut out: Vec<&Entry> = Vec::new();
    for e in db {
        let t = title_key(&e.title);
        let related = t.starts_with(&key) || (t.len() >= 4 && key.starts_with(&t));
        if related && !e.umu_id.is_empty() && !out.iter().any(|o| o.umu_id == e.umu_id) {
            out.push(e);
            if out.len() == limit {
                break;
            }
        }
    }
    out
}

/// The downloaded database (empty before the first download).
pub fn load(paths: &DataPaths) -> Vec<Entry> {
    fs::read_to_string(paths.store_cache(CACHE_NAME)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Downloads the database when the copy is missing or a week old.
/// `Ok(true)`: it was replaced.
pub fn update_if_needed(paths: &DataPaths, net: &dyn Net) -> Result<bool, String> {
    let target = paths.store_cache(CACHE_NAME);
    let fresh = fs::metadata(&target)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < MAX_AGE);
    if fresh || !net.is_online() {
        return Ok(false);
    }
    let value = net.get_json(API_URL).map_err(|e| format!("umu database: {e}"))?;
    let entries: Vec<Entry> = serde_json::from_value(value).map_err(|e| format!("umu database: {e}"))?;
    // An empty answer must not replace a good copy.
    if entries.is_empty() {
        return Err("umu database: the answer has no games".into());
    }
    heronite_data::json_store::write(&target, &entries).map_err(|e| format!("umu database: {e:#}"))?;
    log::info!("umu database downloaded ({} games)", entries.len());
    Ok(true)
}

/// Sets `GAMEID` on a plan that runs the game through umu; plans without
/// umu (plain Wine or Proton, umu turned off) are left alone.
pub fn attach(plan: &mut LaunchPlan, ctx: &PlanContext, keys: GameKeys) {
    let process = match plan {
        LaunchPlan::Wine { process, runtime: WineRuntime::Umu, .. } => process,
        LaunchPlan::Legendary { process, .. } if process.through_umu => process,
        _ => return,
    };
    if let Some(r) = resolve(&ctx.umu_database, ctx.umu_ids.as_ref(), keys) {
        process.notes.push(format!("umu id {} ({:?})", r.id, r.source));
        process.command.env.insert("GAMEID".into(), r.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use serde_json::json;

    fn entry(title: &str, id: &str, store: &str, codename: &str) -> Entry {
        Entry { title: title.into(), umu_id: id.into(), store: Some(store.into()), codename: Some(codename.into()) }
    }

    fn db() -> Vec<Entry> {
        vec![
            entry("Cyberpunk 2077", "umu-1091500", "egs", "Ginger"),
            entry("Cyberpunk 2077", "umu-1091500", "gog", "1423049311"),
            entry("Borderlands 3", "umu-397540", "egs", "Catnip"),
            entry("Grand Theft Auto V", "umu-271590", "none", "none"),
            entry("Age of Wonders", "umu-61500", "gog", "1207658883"),
            entry("Age of Wonders", "umu-61501", "humble", "x"),
        ]
    }

    fn keys<'a>(runner: &'a str, app: &'a str, title: &'a str) -> GameKeys<'a> {
        GameKeys { runner, app_name: app, title, ..Default::default() }
    }

    #[test]
    fn the_users_id_wins_and_a_bare_steam_id_gets_the_prefix() {
        let k = GameKeys { user_id: " 1234 ", ..keys("legendary", "Ginger", "Cyberpunk 2077") };
        let r = resolve(&db(), None, k).unwrap();
        assert_eq!((r.id.as_str(), r.source), ("umu-1234", Source::User));
        assert_eq!(normalize("umu-genshin"), "umu-genshin");
    }

    #[test]
    fn store_games_by_codename_then_heroics_cache() {
        let r = resolve(&db(), None, keys("legendary", "ginger", "x")).unwrap();
        assert_eq!((r.id.as_str(), r.source, r.title.as_deref()), ("umu-1091500", Source::Store, Some("Cyberpunk 2077")));
        let cache = json!({"legendary_Calluna": "umu-870780", "legendary_Penguin": null});
        let r = resolve(&db(), Some(&cache), keys("legendary", "Calluna", "Control")).unwrap();
        assert_eq!((r.id.as_str(), r.source), ("umu-870780", Source::HeroicCache));
        assert_eq!(resolve(&db(), Some(&cache), keys("legendary", "Penguin", "Some Game")), None);
    }

    #[test]
    fn local_games_by_steam_id_known_to_umu_then_exact_title() {
        let k = GameKeys { steam_app_id: Some("397540"), ..keys("sideload", "abc", "BL3") };
        assert_eq!(resolve(&db(), None, k).unwrap().source, Source::SteamId);
        // A Steam id umu does not know is not guessed.
        let k = GameKeys { steam_app_id: Some("42"), ..keys("sideload", "abc", "Unknown") };
        assert_eq!(resolve(&db(), None, k), None);
        let r = resolve(&db(), None, keys("sideload", "abc", "grand theft auto v")).unwrap();
        assert_eq!((r.id.as_str(), r.source), ("umu-271590", Source::Title));
        // One title, two ids: no guess.
        assert_eq!(resolve(&db(), None, keys("sideload", "abc", "Age of Wonders")), None);
    }

    #[test]
    fn suggestions_share_the_start_of_the_title() {
        let mut d = db();
        d.push(entry("Ground Control", "umu-254820", "gog", "1"));
        d.push(entry("Control Ultimate Edition", "umu-870780", "egs", "Calluna"));
        let ids = |t: &str| suggestions(&d, t, 3).iter().map(|e| e.umu_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("Control"), ["umu-870780"]);
        assert_eq!(ids("cyberpunk"), ["umu-1091500"]);
        assert_eq!(ids("Cyberpunk 2077: Phantom Liberty"), ["umu-1091500"]);
        assert_eq!(ids("Age of Wonders"), ["umu-61500", "umu-61501"]);
        assert!(ids("ab").is_empty());
    }

    #[test]
    fn the_database_is_downloaded_once_a_week() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let body = json!([{"title": "Borderlands 3", "umu_id": "umu-397540", "acronym": "bl3", "codename": "Catnip", "store": "egs", "exe_string": null, "notes": null}]);
        let net = FakeNet::online().with_json(API_URL, body);
        assert_eq!(update_if_needed(&paths, &net), Ok(true));
        assert_eq!(load(&paths)[0].umu_id, "umu-397540");
        assert_eq!(update_if_needed(&paths, &net), Ok(false), "a fresh copy is kept");
        assert_eq!(net.requests().len(), 1);
        let empty = FakeNet::online().with_json(API_URL, json!([]));
        let other = DataPaths::at(dir.path().join("other"));
        assert!(update_if_needed(&other, &empty).is_err());
        assert!(load(&other).is_empty());
    }
}
