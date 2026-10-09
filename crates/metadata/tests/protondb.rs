//! ProtonDB tier lookup against a local mock server.

mod common;

use std::fs;
use std::path::Path;

use common::{Resp, Server};
use heronite_metadata::protondb::{cached_tier, tier};
use heronite_metadata::Http;
use serde_json::{json, Value};

const PATH: &str = "/protondb/api/v1/reports/summaries/1245620.json";

fn server() -> Server {
    Server::start(|req| match req.path() {
        PATH => Resp::json(json!({"tier":"Gold","trendingTier":"platinum","confidence":"strong","score":0.71,"total":12})),
        "/protondb/api/v1/reports/summaries/500.json" => Resp::status(500),
        _ => Resp::status(404),
    })
}

fn hits(s: &Server) -> usize {
    s.count(|r| r.path().starts_with("/protondb/"))
}

fn backdate(dir: &Path, id: &str, days: i64) {
    let file = dir.join(format!("{id}.json"));
    let mut v: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    let old = chrono::Utc::now() - chrono::Duration::days(days);
    v["fetchedAt"] = json!(old.to_rfc3339());
    fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn fetches_once_then_serves_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let s = server();
    let http = Http::new(s.endpoints());
    assert_eq!(tier(dir.path(), &http, "1245620").as_deref(), Some("gold"));
    assert_eq!(tier(dir.path(), &http, "1245620").as_deref(), Some("gold"));
    assert_eq!(hits(&s), 1);
    assert_eq!(cached_tier(dir.path(), "1245620"), Some(("gold".into(), true)));
}

#[test]
fn unknown_games_are_not_fetched_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = server();
    let http = Http::new(s.endpoints());
    assert_eq!(tier(dir.path(), &http, "42"), None);
    assert_eq!(tier(dir.path(), &http, "42"), None);
    assert_eq!(hits(&s), 1);
    assert!(dir.path().join("42.json").exists());
}

#[test]
fn stale_cache_is_refreshed_and_kept_when_the_server_fails() {
    let dir = tempfile::tempdir().unwrap();
    let s = server();
    let http = Http::new(s.endpoints());
    tier(dir.path(), &http, "1245620");
    backdate(dir.path(), "1245620", 8);
    assert_eq!(cached_tier(dir.path(), "1245620"), Some(("gold".into(), false)));
    assert_eq!(tier(dir.path(), &http, "1245620").as_deref(), Some("gold"));
    assert_eq!(hits(&s), 2);
    assert_eq!(cached_tier(dir.path(), "1245620").map(|c| c.1), Some(true));

    // An unreachable server and an HTTP 500 both fall back to the stale tier.
    fs::write(
        dir.path().join("500.json"),
        json!({"tier":"silver","fetchedAt":(chrono::Utc::now() - chrono::Duration::days(9)).to_rfc3339()}).to_string(),
    )
    .unwrap();
    assert_eq!(tier(dir.path(), &http, "500").as_deref(), Some("silver"));
    let mut dead = s.endpoints();
    dead.protondb = "http://127.0.0.1:1".into();
    backdate(dir.path(), "1245620", 8);
    assert_eq!(tier(dir.path(), &Http::new(dead), "1245620").as_deref(), Some("gold"));
    assert_eq!(cached_tier(dir.path(), "1245620").map(|c| c.1), Some(false));
}

#[test]
fn non_numeric_ids_never_hit_the_network() {
    let dir = tempfile::tempdir().unwrap();
    let s = server();
    let http = Http::new(s.endpoints());
    assert_eq!(tier(dir.path(), &http, "../etc"), None);
    assert_eq!(tier(dir.path(), &http, ""), None);
    assert_eq!(hits(&s), 0);
}
