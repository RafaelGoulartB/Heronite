//! ProtonDB tier lookup against a local mock server.

mod common;

use std::fs;
use std::path::Path;

use common::{Resp, Server};
use heronite_metadata::protondb::{cached_steam_id_for_title, cached_tier, steam_id_for_title, tier};
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

fn search_server(community: fn() -> Resp) -> Server {
    Server::start(move |req| match req.path() {
        p if p.starts_with("/community/actions/SearchApps/") => community(),
        "/steam/api/storesearch/" => Resp::json(json!({"total":0,"items":[]})),
        _ => Resp::status(404),
    })
}

fn searches(s: &Server) -> usize {
    s.count(|r| r.path().starts_with("/community/") || r.path().starts_with("/steam/"))
}

#[test]
fn a_failed_title_search_is_not_cached_as_not_on_steam() {
    let dir = tempfile::tempdir().unwrap();
    let mut dead = server().endpoints();
    for url in [&mut dead.steam_community, &mut dead.steam_store] {
        *url = "http://127.0.0.1:1".into();
    }
    assert_eq!(steam_id_for_title(dir.path(), &Http::new(dead), "Hades"), None);
    assert_eq!(cached_steam_id_for_title(dir.path(), "Hades"), None);

    // The next launch, online, finds it.
    let s = search_server(|| Resp::json(json!([{"appid":"1145360","name":"Hades"}])));
    let http = Http::new(s.endpoints());
    assert_eq!(steam_id_for_title(dir.path(), &http, "Hades").as_deref(), Some("1145360"));
    assert_eq!(cached_steam_id_for_title(dir.path(), "Hades"), Some(Some("1145360".into())));
}

#[test]
fn a_server_error_during_the_title_search_is_not_cached_either() {
    let dir = tempfile::tempdir().unwrap();
    let s = search_server(|| Resp::status(503));
    let http = Http::new(s.endpoints());
    assert_eq!(steam_id_for_title(dir.path(), &http, "Hades"), None);
    assert_eq!(cached_steam_id_for_title(dir.path(), "Hades"), None);
    let first = searches(&s);
    assert_eq!(steam_id_for_title(dir.path(), &http, "Hades"), None);
    assert!(searches(&s) > first, "the search is asked again");
}

#[test]
fn a_completed_title_search_without_a_match_is_cached() {
    let dir = tempfile::tempdir().unwrap();
    let s = search_server(|| Resp::json(json!([])));
    let http = Http::new(s.endpoints());
    assert_eq!(steam_id_for_title(dir.path(), &http, "Not A Steam Game"), None);
    assert_eq!(cached_steam_id_for_title(dir.path(), "Not A Steam Game"), Some(None));
    let after_first = searches(&s);
    assert_eq!(steam_id_for_title(dir.path(), &http, "Not A Steam Game"), None);
    assert_eq!(searches(&s), after_first);
}
