//! GOG achievements against a local mock of `gameplay.gog.com`.

mod common;

use std::fs;

use common::{Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::HeroicPaths;
use heronite_metadata::achievements::*;
use heronite_metadata::{Endpoints, Http};
use serde_json::{json, Value};

fn root(logged_in: bool, token_age_s: f64) -> (tempfile::TempDir, HeroicPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = HeroicPaths::at(dir.path());
    fs::create_dir_all(dir.path().join("store_cache")).unwrap();
    fs::create_dir_all(dir.path().join("gog_store")).unwrap();
    json_store::write(&dir.path().join("gog_store/config.json"), &json!({ "isLoggedIn": logged_in })).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    json_store::write(
        &dir.path().join("gog_store/auth.json"),
        &json!({ GALAXY_CLIENT_ID: {"access_token":"tok-1","user_id":"42","expires_in":3600,"loginTime": now - token_age_s}}),
    )
    .unwrap();
    (dir, paths)
}

fn server() -> Server {
    Server::start(|req| {
        if req.path() == "/clients/1207658924/users/42/achievements"
            && req.header("authorization") == Some("Bearer tok-1")
        {
            return Resp::json(json!({"items":[
                {"achievement_id":"1","achievement_key":"A","visible":true,"name":"First","description":"d",
                 "image_url_unlocked":"https://x/u.png","image_url_locked":"https://x/l.png","rarity":2.5,
                 "date_unlocked":"2026-01-02T03:04:05+0000","rarity_level_description":"Rare","rarity_level_slug":"rare"},
                {"achievement_id":"2","achievement_key":"B","visible":false,"name":"Secret","description":"",
                 "image_url_unlocked":"","image_url_locked":"","rarity":0.1,"date_unlocked":null,
                 "rarity_level_description":"","rarity_level_slug":""}
            ]}));
        }
        Resp::status(404)
    })
}

#[test]
fn fetches_with_the_token_caches_and_then_serves_the_cache() {
    let (_d, paths) = root(true, 10.0);
    let s = server();
    let http = Http::new(Endpoints::default());
    let items = fetch_gog(&paths, &http, &s.base, "1207658924", "en-US");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "First");
    assert_eq!(s.count(|r| r.path().ends_with("/achievements")), 1);
    let sent = s.log.lock()[0].clone();
    assert_eq!(sent.header("x-gog-lc"), Some("en-US"));

    // Second call never reaches the network.
    let again = fetch_gog(&paths, &http, &s.base, "1207658924", "en-US");
    assert_eq!(again, items);
    assert_eq!(s.count(|r| r.path().ends_with("/achievements")), 1);
    let doc: Value = json_store::read_value(&paths.root.join("store_cache/gog_achievements.json")).unwrap();
    assert!(doc["__timestamp"]["1207658924"].is_string());
}

#[test]
fn not_logged_in_expired_token_and_errors_give_nothing_and_cache_nothing() {
    let s = server();
    let http = Http::new(Endpoints::default());

    let (_d, paths) = root(false, 10.0);
    assert!(fetch_gog(&paths, &http, &s.base, "1207658924", "en-US").is_empty());

    let (_d2, paths2) = root(true, 7200.0);
    assert!(fetch_gog(&paths2, &http, &s.base, "1207658924", "en-US").is_empty());

    let (_d3, paths3) = root(true, 10.0);
    assert!(fetch_gog(&paths3, &http, &s.base, "999", "en-US").is_empty());
    assert_eq!(s.count(|r| r.path().ends_with("/achievements")), 1, "only the logged-in call hit the server");
    assert!(read_cache(&paths3, "999").is_none());
}

#[test]
fn other_runners_have_no_achievements() {
    let (_d, paths) = root(true, 10.0);
    let http = Http::new(Endpoints::default());
    assert!(load_achievements(&paths, &http, "sideload", "steam_620").is_empty());
    assert!(load_achievements(&paths, &http, "legendary", "abc").is_empty());
}
