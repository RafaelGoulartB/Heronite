//! Play time the Epic Games Launcher counted for the account's games
//! (`library-service`'s playtime, the list Playnite imports). Only games
//! never played here take it in: see
//! [`import_store_playtime`](heronite_data::playtime::import_store_playtime).

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::json_store;
use heronite_data::playtime::{import_store_playtime, unplayed_games, StorePlaytime};
use heronite_data::DataPaths;
use serde_json::Value;

pub const PLAYTIME_BASE: &str = "https://library-service.live.use1a.on.epicgames.com";

/// The account id and access token of legendary's `user.json`. `None` when
/// logged out or when the token expired (legendary renews it on its next
/// run). The token only goes into the request's header.
fn session(data_paths: &DataPaths) -> Option<(String, String)> {
    let doc = json_store::read_value(&data_paths.legendary_config_dir().join("user.json")).ok()?;
    let text = |k: &str| doc.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let expired = text("expires_at").and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok()).is_some_and(|at| at <= chrono::Utc::now());
    if expired {
        return None;
    }
    Some((text("account_id")?, text("access_token")?))
}

/// `[{"artifactId": <app name>, "totalTime": <seconds>}]` as whole minutes.
pub fn parse(doc: &Value) -> Vec<StorePlaytime> {
    let Some(items) = doc.as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let app_name = item.get("artifactId")?.as_str().filter(|s| !s.is_empty())?.to_string();
            let seconds = item.get("totalTime")?.as_f64().filter(|s| s.is_finite() && *s > 0.0)?;
            Some(StorePlaytime { app_name, minutes: (seconds / 60.0) as u64 })
        })
        .filter(|p| p.minutes > 0)
        .collect()
}

fn fetch(base: &str, account_id: &str, token: &str) -> Result<Vec<StorePlaytime>> {
    let url = format!("{base}/library/api/public/playtime/account/{account_id}/all");
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into();
    let mut resp = agent.get(&url).header("Authorization", format!("bearer {token}")).call().map_err(|e| anyhow!("{url}: {e}"))?;
    if !resp.status().is_success() {
        bail!("{url}: HTTP {}", resp.status());
    }
    let body = resp.body_mut().read_to_string().map_err(|e| anyhow!("{url}: {e}"))?;
    let doc: Value = serde_json::from_str(&body).with_context(|| format!("invalid JSON from {url}"))?;
    Ok(parse(&doc))
}

/// Gives the Epic games never played here the time Epic counted for them.
/// Logged out, an expired token or no such game: nothing is asked. Blocking,
/// network. Returns how many games got a time.
pub fn import(data_paths: &DataPaths) -> Result<usize> {
    import_from(data_paths, PLAYTIME_BASE)
}

fn import_from(data_paths: &DataPaths, base: &str) -> Result<usize> {
    let Some((account_id, token)) = session(data_paths) else { return Ok(0) };
    let wanted = unplayed_games(data_paths, "legendary")?;
    if wanted.is_empty() {
        return Ok(0);
    }
    let items: Vec<StorePlaytime> = fetch(base, &account_id, &token)?.into_iter().filter(|p| wanted.contains(&p.app_name)).collect();
    Ok(import_store_playtime(data_paths, &items)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Answers one request with `status` and `body`; the request comes back
    /// through the receiver.
    fn serve(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let (status, body) = (status.to_string(), body.to_string());
        std::thread::spawn(move || {
            if let Some(Ok(mut s)) = listener.incoming().next() {
                let mut req = [0u8; 4096];
                let n = s.read(&mut req).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&req[..n]).into_owned());
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (format!("http://{addr}"), rx)
    }

    fn write(path: std::path::PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Logged in with a token valid for `expires_at`, owning `Fresh` (never
    /// played), `Played` (played here) and the DLC `FreshDlc`.
    fn setup(expires_at: &str) -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        write(
            paths.legendary_config_dir().join("user.json"),
            &serde_json::json!({"account_id": "acc1", "access_token": "tok", "expires_at": expires_at, "displayName": "P"}).to_string(),
        );
        write(
            paths.legendary_library(),
            r#"{"library": [
              {"runner": "legendary", "app_name": "Fresh", "title": "Fresh", "install": {}},
              {"runner": "legendary", "app_name": "Played", "title": "Played", "install": {}},
              {"runner": "legendary", "app_name": "FreshDlc", "title": "DLC", "install": {"is_dlc": true}}
            ], "__timestamp": {"library": "x"}}"#,
        );
        write(paths.timestamps(), r#"{"Played": {"firstPlayed": "", "lastPlayed": "2026-01-01T00:00:00.000Z", "totalPlayed": 2}}"#);
        (dir, paths)
    }

    #[test]
    fn parses_seconds_into_minutes() {
        let doc = serde_json::json!([
            {"accountId": "a", "artifactId": "Fall", "totalTime": 70528},
            {"accountId": "a", "artifactId": "Short", "totalTime": 59},
            {"accountId": "a", "artifactId": "", "totalTime": 600},
            {"accountId": "a", "artifactId": "NoTime"},
            {"accountId": "a", "artifactId": "Odd", "totalTime": "600"}
        ]);
        assert_eq!(parse(&doc), [StorePlaytime { app_name: "Fall".into(), minutes: 1175 }]);
        assert!(parse(&serde_json::json!({"errorCode": "x"})).is_empty());
    }

    #[test]
    fn fills_only_the_games_never_played_here() {
        let (_d, paths) = setup("2999-01-01T00:00:00.000Z");
        let (base, rx) = serve(
            "200 OK",
            r#"[{"artifactId": "Fresh", "totalTime": 3600}, {"artifactId": "Played", "totalTime": 99999}, {"artifactId": "FreshDlc", "totalTime": 3600}, {"artifactId": "NotOwned", "totalTime": 3600}]"#,
        );
        assert_eq!(import_from(&paths, &base).unwrap(), 1);
        let req = rx.recv().unwrap();
        assert!(req.starts_with("GET /library/api/public/playtime/account/acc1/all "), "{req}");
        assert!(req.to_lowercase().contains("authorization: bearer tok"), "{req}");
        let ts = json_store::read_value(&paths.timestamps()).unwrap();
        assert_eq!(ts["Fresh"]["totalPlayed"], 60);
        assert_eq!(ts["Played"]["totalPlayed"], 2);
        assert!(ts.get("FreshDlc").is_none() && ts.get("NotOwned").is_none());
    }

    #[test]
    fn nothing_is_asked_without_a_valid_session_or_a_game_to_fill() {
        // Nothing listens there: a request would fail the import.
        let unreachable = "http://127.0.0.1:9";
        let (_d, paths) = setup("2000-01-01T00:00:00.000Z");
        assert_eq!(import_from(&paths, unreachable).unwrap(), 0, "expired token");

        let (_d, paths) = setup("2999-01-01T00:00:00.000Z");
        write(paths.legendary_config_dir().join("user.json"), r#"{"account_id": "acc1", "displayName": "P"}"#);
        assert_eq!(import_from(&paths, unreachable).unwrap(), 0, "no token");

        let (_d, paths) = setup("2999-01-01T00:00:00.000Z");
        write(paths.timestamps(), r#"{"Fresh": {"totalPlayed": 1}, "Played": {"totalPlayed": 2}}"#);
        assert_eq!(import_from(&paths, unreachable).unwrap(), 0, "every game played here");
    }

    #[test]
    fn a_refused_request_is_an_error_and_changes_nothing() {
        let (_d, paths) = setup("2999-01-01T00:00:00.000Z");
        let before = std::fs::read_to_string(paths.timestamps()).unwrap();
        let (base, _rx) =
            serve("401 Unauthorized", r#"{"errorCode": "errors.com.epicgames.common.authentication.token_verification_failed"}"#);
        let err = import_from(&paths, &base).unwrap_err();
        assert!(format!("{err:#}").contains("401"));
        assert!(!format!("{err:#}").contains("tok"), "the token never shows in errors");
        assert_eq!(std::fs::read_to_string(paths.timestamps()).unwrap(), before);
    }
}
