//! Play time GOG counted for the account's games: `gameplay.gog.com`'s
//! sessions of each game (`time_sum`, minutes: what Heroic reads and posts).
//! Only games never played here take it in: see
//! [`import_store_playtime`](heronite_data::playtime::import_store_playtime).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use anyhow::Result;
use heronite_data::playtime::{import_store_playtime, unplayed_games, StorePlaytime};
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use serde_json::Value;

use crate::api::{self, parallel_map};
use crate::user::Credentials;

pub const GAMEPLAY_BASE: &str = "https://gameplay.gog.com";

/// One request per game never played here, so they are asked once a day.
const CHECK_EVERY_MINUTES: u64 = 24 * 60;
const CACHE: &str = "gog_playtime_import";
const WORKERS: usize = 4;

/// The data folders an import runs for (refreshes may overlap).
static RUNNING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn running() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks an import as running for its data folder until dropped.
struct Running(PathBuf);

impl Drop for Running {
    fn drop(&mut self) {
        running().remove(&self.0);
    }
}

fn sessions_url(base: &str, app_name: &str, user_id: &str) -> String {
    format!("{base}/games/{app_name}/users/{user_id}/sessions")
}

/// `{"time_sum": <minutes>}`.
pub fn parse(doc: &Value) -> Option<u64> {
    doc.get("time_sum")?.as_f64().filter(|m| m.is_finite() && *m >= 1.0).map(|m| m as u64)
}

/// [`import`] on its own thread, so a big library does not hold the refresh
/// up. The Collection notices the new times by itself.
pub fn import_in_background(paths: &DataPaths, creds: &Credentials) {
    let (paths, creds) = (paths.clone(), creds.clone());
    let spawned = std::thread::Builder::new().name("gog-playtime".into()).spawn(move || match import(&paths, &creds) {
        Ok(0) => {}
        Ok(n) => log::info!("GOG play time taken in for {n} game(s) never played here"),
        Err(err) => log::warn!("gog: reading the play time GOG counted: {err:#}"),
    });
    if let Err(err) = spawned {
        log::warn!("gog: could not start the play time import: {err}");
    }
}

/// Gives the GOG games never played here the time GOG counted for them, at
/// most once a day. Blocking, network. Returns how many games got a time.
pub fn import(paths: &DataPaths, creds: &Credentials) -> Result<usize> {
    import_from(paths, creds, GAMEPLAY_BASE)
}

fn import_from(paths: &DataPaths, creds: &Credentials, base: &str) -> Result<usize> {
    let key = paths.timestamps();
    if !running().insert(key.clone()) {
        return Ok(0);
    }
    let _running = Running(key);
    import_once(paths, creds, base)
}

fn import_once(paths: &DataPaths, creds: &Credentials, base: &str) -> Result<usize> {
    let checked = CacheStore::new(paths, CACHE, Some(CHECK_EVERY_MINUTES));
    if checked.get("checkedAt").is_some() {
        return Ok(0);
    }
    let mut wanted: Vec<String> = unplayed_games(paths, "gog")?.into_iter().collect();
    wanted.sort();
    let answers = parallel_map(&wanted, WORKERS, |app| {
        api::get_json(&sessions_url(base, app, &creds.user_id), &creds.access_token).map(|doc| parse(&doc))
    });
    let failed = answers.iter().filter(|a| a.is_err()).count();
    if failed > 0 {
        let first = answers.iter().find_map(|a| a.as_ref().err()).map(|e| format!("{e:#}")).unwrap_or_default();
        // Offline or signed out: asked again at the next refresh.
        if failed == wanted.len() {
            anyhow::bail!("{failed} game(s) could not be read: {first}");
        }
        log::warn!("gog: the play time of {failed} game(s) could not be read: {first}");
    }
    let items: Vec<StorePlaytime> = wanted
        .iter()
        .zip(&answers)
        .filter_map(|(app, answer)| Some(StorePlaytime { app_name: app.clone(), minutes: (*answer.as_ref().ok()?)? }))
        .collect();
    let filled = import_store_playtime(paths, &items)?.len();
    checked.set("checkedAt", Value::Bool(true))?;
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::json_store;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A local `gameplay.gog.com`: `time_sum` per game, 401 without the
    /// token, 404 for others. Returns the base and the paths asked.
    fn server(times: &[(&str, u64)]) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let times: Vec<(String, u64)> = times.iter().map(|(a, m)| (a.to_string(), *m)).collect();
        let log = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = [0u8; 4096];
                let n = s.read(&mut req).unwrap_or(0);
                let text = String::from_utf8_lossy(&req[..n]).into_owned();
                let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let found = times.iter().find(|(app, _)| path == format!("/games/{app}/users/42/sessions"));
                let (status, body) = if !text.contains("Bearer tok") {
                    ("401 Unauthorized", String::new())
                } else if let Some((_, m)) = found {
                    ("200 OK", format!("{{\"time_sum\": {m}}}"))
                } else {
                    ("404 Not Found", String::new())
                };
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (format!("http://{addr}"), asked)
    }

    fn write(path: std::path::PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn creds(token: &str) -> Credentials {
        Credentials { access_token: token.into(), user_id: "42".into() }
    }

    /// Owns `fresh`, `zero` (never played anywhere) and `played` (played here).
    fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write(
            paths.gog_library(),
            r#"{"games": [
              {"runner": "gog", "app_name": "fresh", "title": "Fresh", "install": {}},
              {"runner": "gog", "app_name": "zero", "title": "Zero", "install": {}},
              {"runner": "gog", "app_name": "played", "title": "Played", "install": {}}
            ], "__timestamp": {"games": "x"}}"#,
        );
        write(paths.timestamps(), r#"{"played": {"firstPlayed": "2026-01-01T00:00:00.000Z", "lastPlayed": "", "totalPlayed": 3}}"#);
        (dir, paths)
    }

    #[test]
    fn parses_time_sum_minutes() {
        assert_eq!(parse(&serde_json::json!({"time_sum": 125})), Some(125));
        assert_eq!(parse(&serde_json::json!({"time_sum": 0})), None);
        assert_eq!(parse(&serde_json::json!({"time_sum": "5"})), None);
        assert_eq!(parse(&serde_json::json!({})), None);
    }

    #[test]
    fn fills_the_games_never_played_here_once_a_day() {
        let (_d, paths) = setup();
        let (base, asked) = server(&[("fresh", 125), ("zero", 0), ("played", 999)]);
        assert_eq!(import_from(&paths, &creds("tok"), &base).unwrap(), 1);
        let mut paths_asked = asked.lock().unwrap().clone();
        paths_asked.sort();
        // The game played here is not even asked.
        assert_eq!(paths_asked, ["/games/fresh/users/42/sessions", "/games/zero/users/42/sessions"]);
        let ts = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(ts["fresh"]["totalPlayed"], 125);
        assert_eq!(ts["played"]["totalPlayed"], 3);
        assert!(ts.get("zero").is_none());

        // Checked today: no request until tomorrow.
        assert_eq!(import_from(&paths, &creds("tok"), &base).unwrap(), 0);
        assert_eq!(asked.lock().unwrap().len(), 2);
    }

    #[test]
    fn when_every_request_fails_nothing_changes_and_it_is_asked_again() {
        let (_d, paths) = setup();
        let before = std::fs::read_to_string(paths.timestamps()).unwrap();
        let (base, asked) = server(&[("fresh", 125)]);
        let err = import_from(&paths, &creds("zz-old-token"), &base).unwrap_err();
        assert!(!format!("{err:#}").contains("zz-old-token"), "the token never shows in errors");
        assert_eq!(std::fs::read_to_string(paths.timestamps()).unwrap(), before);
        // Not marked as checked: the next refresh asks again.
        assert_eq!(import_from(&paths, &creds("tok"), &base).unwrap(), 1);
        assert_eq!(asked.lock().unwrap().len(), 4);
    }

    #[test]
    fn some_failures_keep_the_answers_that_came() {
        let (_d, paths) = setup();
        // `zero` is unknown to this server (404).
        let (base, _asked) = server(&[("fresh", 30)]);
        assert_eq!(import_from(&paths, &creds("tok"), &base).unwrap(), 1);
        assert_eq!(json_store::read_value(&paths.timestamps()).unwrap()["fresh"]["totalPlayed"], 30);
    }
}
