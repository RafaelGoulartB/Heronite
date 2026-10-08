//! End-to-end flows against a local mock of every remote service, writing to
//! a temporary Heroic config. Responses are the recorded fixtures.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{fixture, png, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::HeroicPaths;
use heronite_metadata::*;
use serde_json::{json, Value};

/// Minimal Heroic config with one sideloaded Steam game, its overlay entry,
/// IGDB credentials and an unrelated metadata record carrying a field this
/// crate does not model.
fn make_root(dir: &Path) -> HeroicPaths {
    let paths = HeroicPaths::at(dir);
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    w(
        paths.sideload_library(),
        json!({"games": [
            {"runner": "sideload", "app_name": "steam_620", "title": "Portal 2", "developer": "Valve Corp",
             "art_square": "https://store/sq.jpg", "art_cover": "https://store/cv.jpg", "is_installed": false, "install": {},
             "extra": {"genres": ["Puzzle"], "releaseDate": "2011-04-19"}},
            {"runner": "sideload", "app_name": "plain", "title": "Plain Game", "is_installed": false, "install": {}}
        ]}),
    );
    w(
        paths.local_meta(),
        json!({"games": {
            "steam_620": {"appName": "steam_620", "runner": "sideload", "steamAppId": "620", "title": "Portal 2",
                          "playniteId": "", "source": "steam", "launchKind": "steam-uri"},
            "plain": {"appName": "plain", "runner": "sideload", "title": "Plain Game", "playniteId": "", "source": "manual", "launchKind": "executable"}
        }}),
    );
    w(
        paths.collection_settings(),
        json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret", "coverAspect": "steam",
                            "downloadImages": false, "autoFillMissing": true, "fieldPriority": {}}, "futureSetting": 1}),
    );
    w(
        paths.metadata(),
        json!({"games": {"sideload:other": {"appName": "other", "runner": "sideload", "title": "Other",
                "developers": [], "publishers": [], "genres": [], "themes": [], "gameModes": [], "platforms": [],
                "features": [], "websites": [], "fieldSources": {}, "futureField": {"a": 1}}}}),
    );
    paths
}

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

struct Counters {
    tokens: AtomicUsize,
}

fn mock() -> (Server, Arc<Counters>) {
    let counters = Arc::new(Counters { tokens: AtomicUsize::new(0) });
    let c = counters.clone();
    let server = Server::start(move |req| {
        let path = req.path();
        match path {
            "/twitch/oauth2/token" => {
                c.tokens.fetch_add(1, Ordering::SeqCst);
                assert_eq!(req.query("client_id").as_deref(), Some("id"));
                assert_eq!(req.query("grant_type").as_deref(), Some("client_credentials"));
                Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"}))
            }
            "/igdb/v4/games" => {
                assert_eq!(req.header("authorization"), Some("Bearer tok"));
                assert_eq!(req.header("client-id"), Some("id"));
                if req.body.starts_with("search") && req.body.contains("fields name,slug,first_release_date") {
                    Resp::raw_json(fixture("igdb_search_portal2.json"))
                } else {
                    Resp::raw_json(fixture("igdb_game_steam_620.json"))
                }
            }
            "/steam/api/appdetails" => steam_body(req),
            "/lutris/api/games" => Resp::raw_json(fixture("lutris_search_portal.json")),
            "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
            _ => Resp::status(404),
        }
    });
    (server, counters)
}

fn args(app: &str, title: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.into(), ..Default::default() }
}

#[test]
fn preview_then_apply_merges_all_providers_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let (server, counters) = mock();
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());

    let preview = svc.preview_game_metadata(&args("steam_620", "Portal 2")).unwrap();
    assert!(preview.igdb_configured);
    let sources: Vec<_> = preview.candidates.iter().map(|c| c.source.as_str()).collect();
    // Playnite has no source configured.
    assert_eq!(sources, ["igdb", "steam", "store", "lutris"]);
    let igdb = &preview.candidates[0];
    assert_eq!(igdb.match_name.as_deref(), Some("Portal 2"));
    assert_eq!(igdb.match_year, Some(2011));
    assert!(preview.current.is_none());
    assert_eq!(preview.proposed.field_sources["description"], "igdb");
    assert_eq!(preview.proposed.field_sources["features"], "steam");
    // Nothing written by a preview.
    assert!(json_store::read::<MetadataFile>(&paths.metadata()).unwrap().games.get("sideload:steam_620").is_none());

    let saved = svc.apply_game_metadata(&args("steam_620", "Portal 2")).unwrap();
    assert_eq!(saved.igdb_id.map(|n| n.get()), Some(72.0));
    assert_eq!(saved.steam_app_id.as_deref(), Some("620"));
    assert_eq!(saved.lutris_slug.as_deref(), Some("portal"));
    assert_eq!(saved.field_sources["genres"], "igdb");
    assert_eq!(saved.field_sources["cover"], "igdb");
    assert!(saved.cover_url.as_deref().unwrap().contains("t_cover_big_2x"));
    assert!(saved.fetched_at.is_some() && saved.fetch_error.is_none());
    // One token for every IGDB query of both runs.
    assert_eq!(counters.tokens.load(Ordering::SeqCst), 1);

    // File: Heronite's schema, foreign records untouched, tab-indented.
    let text = fs::read_to_string(paths.metadata()).unwrap();
    assert!(text.starts_with("{\n\t\"games\": {"));
    let raw: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(raw["games"]["sideload:other"]["futureField"], json!({"a": 1}));
    let typed: MetadataFile = json_store::read(&paths.metadata()).unwrap();
    let again: CollectionGameMetadata = typed.games["sideload:steam_620"].clone();
    assert_eq!(serde_json::to_value(&again).unwrap(), raw["games"]["sideload:steam_620"]);
    assert!(raw["games"]["sideload:steam_620"].get("fetchError").is_none());

    // Steam details were cached in Heronite's file format as a side effect.
    let cached = fs::read_to_string(paths.steam_details("620", "english")).unwrap();
    assert!(cached.starts_with(r#"{"steamAppId":"620","language":"english","name":"Portal""#));
    let n = server.count(|r| r.path() == "/steam/api/appdetails");
    svc.apply_game_metadata(&args("steam_620", "Portal 2")).unwrap();
    assert_eq!(server.count(|r| r.path() == "/steam/api/appdetails"), n, "second run served from the 30 day cache");
}

#[test]
fn manual_edits_survive_refetch_and_picks_override() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let (server, _) = mock();
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    svc.apply_game_metadata(&args("steam_620", "Portal 2")).unwrap();

    svc.update_game_details(&DetailsPatch {
        app_name: "steam_620".into(),
        runner: "sideload".into(),
        description: Some("My own text".into()),
        genres: Some(vec!["Puzzle".into(), "Mine".into()]),
        notes: Some("remember".into()),
        ..Default::default()
    })
    .unwrap();

    // Overwrite everything but keep manual (the bulk/auto default).
    let mut a = args("steam_620", "Portal 2");
    a.overwrite_existing = Some(true);
    a.preserve_manual = Some(true);
    let md = svc.apply_game_metadata(&a).unwrap();
    assert_eq!(md.description.as_deref(), Some("My own text"));
    assert_eq!(md.genres, ["Puzzle", "Mine"]);
    assert_eq!(md.notes.as_deref(), Some("remember"));
    assert_eq!(md.field_sources["description"], "manual");
    assert_eq!(md.field_sources["releaseDate"], "igdb");

    // Explicit pick takes the source even over a manual value.
    let mut a = args("steam_620", "Portal 2");
    a.overwrite_existing = Some(false);
    a.field_picks = Some([("description".to_string(), "steam".to_string())].into_iter().collect());
    let md = svc.apply_game_metadata(&a).unwrap();
    assert_eq!(md.field_sources["description"], "steam");
    assert_ne!(md.description.as_deref(), Some("My own text"));
    assert_eq!(md.genres, ["Puzzle", "Mine"], "no overwrite: untouched fields stay");
}

#[test]
fn igdb_401_refreshes_the_token_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let tokens = Arc::new(AtomicUsize::new(0));
    let t = tokens.clone();
    let server = Server::start(move |req| match req.path() {
        "/twitch/oauth2/token" => {
            let n = t.fetch_add(1, Ordering::SeqCst);
            Resp::json(json!({"access_token": format!("tok{n}"), "expires_in": 3600}))
        }
        "/igdb/v4/games" => {
            if req.header("authorization") == Some("Bearer tok0") {
                Resp::status(401)
            } else {
                Resp::raw_json(fixture("igdb_search_portal2.json"))
            }
        }
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(paths, server.endpoints());
    let hits = svc.search_game_metadata("Portal 2");
    assert!(hits.iter().any(|h| h.id == "72" && h.name == "Portal 2" && h.year == Some(2011) && h.extra.as_deref() == Some("portal-2")));
    assert_eq!(tokens.load(Ordering::SeqCst), 2);
    assert!(svc.test_igdb_credentials().ok);
}

#[test]
fn no_provider_answers_stores_a_fetch_error_and_idle_fill_retries_later() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    // Everything fails (also the store lookup: "plain" is the only candidate source and has no data worth... it does)
    let server = Server::start(|_| Resp::status(500));
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    // 'steam_620' resolves from the store (Heroic GameInfo) so make a game that is not in any library.
    let md = svc.apply_game_metadata(&args("ghost", "Ghost Game")).unwrap();
    assert_eq!(md.fetch_error.as_deref(), Some("No metadata sources returned data"));
    assert!(md.fetched_at.is_some());
    let stored = svc.get_game_metadata("sideload", "ghost").unwrap().unwrap();
    assert_eq!(stored.fetch_error, md.fetch_error);

    // Idle fill: both overlay games lack metadata; the store provider answers for them.
    let filled = svc.refresh_missing_collection_metadata().unwrap();
    assert_eq!(filled, 2);
    let stored = svc.get_game_metadata("sideload", "steam_620").unwrap().unwrap();
    assert_eq!(stored.title.as_deref(), Some("Portal 2"));
    assert_eq!(stored.developers, ["Valve Corp"]);
    assert_eq!(stored.field_sources["cover"], "store");
    // Second pass: nothing is missing (a recent fetchError would not be retried within a week either).
    assert_eq!(svc.refresh_missing_collection_metadata().unwrap(), 0);

    // autoFillMissing off disables it.
    let mut settings = json_store::read_value(&paths.collection_settings()).unwrap();
    settings["metadata"]["autoFillMissing"] = json!(false);
    json_store::write(&paths.collection_settings(), &settings).unwrap();
    json_store::update_value(&paths.metadata(), |d| {
        d["games"].as_object_mut().unwrap().shift_remove("sideload:plain");
        Ok(())
    })
    .unwrap();
    assert_eq!(svc.refresh_missing_collection_metadata().unwrap(), 0);
}

#[test]
fn bulk_job_reports_progress_skips_fetched_and_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let (server, _) = mock();
    let svc = Arc::new(MetadataService::with_endpoints(paths.clone(), server.endpoints()));
    svc.apply_game_metadata(&args("plain", "Plain Game")).unwrap();

    let games = vec![
        BulkItem { app_name: "plain".into(), runner: "sideload".into(), title: "Plain Game".into(), steam_app_id: None },
        BulkItem { app_name: "steam_620".into(), runner: "sideload".into(), title: "Portal 2".into(), steam_app_id: Some("620".into()) },
    ];
    let events = Arc::new(parking_lot::Mutex::new(Vec::<BulkProgress>::new()));
    let ev = events.clone();
    let done = svc.run_bulk_metadata(&BulkArgs { games: games.clone(), ..Default::default() }, &move |p| ev.lock().push(p.clone()));
    assert_eq!(done.state, BulkState::Done);
    assert_eq!((done.total, done.processed, done.updated, done.skipped, done.failed, done.percent), (2, 2, 1, 1, 0, 100));
    let events = events.lock();
    assert_eq!(events.first().unwrap().state, BulkState::Running);
    assert_eq!(events.last().unwrap().state, BulkState::Done);
    let titles: Vec<_> = events.iter().filter_map(|e| e.current_title.clone()).collect();
    assert_eq!(titles, ["Plain Game", "Portal 2"]);
    assert_eq!(events.last().unwrap().percent, 100);
    drop(events);

    // Background job via start_bulk_metadata + cancel.
    let (tx, rx) = std::sync::mpsc::channel();
    let svc2 = svc.clone();
    let first = svc.start_bulk_metadata(
        BulkArgs { games: games.clone(), overwrite_existing: Some(true), ..Default::default() },
        move |p| {
            let _ = tx.send(p.clone());
        },
    );
    assert_eq!(first.state, BulkState::Running);
    // Cancel as soon as the job reports its first game.
    let mut last = BulkProgress::default();
    while let Ok(p) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
        if p.current_title.is_some() && p.state == BulkState::Running && p.processed == 0 {
            svc2.cancel_bulk_metadata(&|_| {});
        }
        let end = p.state == BulkState::Done;
        last = p;
        if end {
            break;
        }
    }
    assert_eq!(last.state, BulkState::Done);
    assert!(last.processed <= 2);
    // A new job can start afterwards.
    let again = svc.run_bulk_metadata(&BulkArgs::default(), &|_| {});
    assert_eq!((again.state, again.percent), (BulkState::Done, 100));
}

#[test]
fn steam_hero_is_cached_where_heronite_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = Server::start(|req| match req.path() {
        "/cdn/hero.jpg" => Resp { status: 200, content_type: "image/jpeg", body: png() },
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    let r = svc.cache_steam_hero("620").unwrap();
    assert!(!r.from_cache);
    assert_eq!(r.url, "localart://heroes/620.jpg");
    assert_eq!(Path::new(&r.path), paths.steam_hero("620"));
    assert!(paths.steam_hero("620").exists());
    assert!(svc.cache_steam_hero("620").unwrap().from_cache);
    assert!(svc.cache_steam_hero("999").is_none() || paths.steam_hero("999").exists());
    // The failed id is remembered: no second round of requests.
    let before = server.log.lock().len();
    svc.warm_steam_heroes(&["999".into(), "620".into()]);
    assert_eq!(server.log.lock().len(), before);
}

#[test]
fn art_from_url_downloads_with_thumb_fallback_and_updates_art_json() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = Server::start(|req| match req.path() {
        "/img/thumb.png" => Resp::image(png()),
        _ => Resp::status(403),
    });
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    let base = &server.base;
    let art = svc
        .set_art_from_url("sideload", "steam_620", ArtKind::Hero, &format!("{base}/img/full.png"), Some(&format!("{base}/img/thumb.png")))
        .unwrap();
    let url = art.hero_url.unwrap();
    assert!(url.starts_with("localart://art/sideload_steam_620/hero-") && url.ends_with(".png"));
    let raw = json_store::read_value(&paths.art()).unwrap();
    assert_eq!(raw["games"]["sideload:steam_620"], json!({"heroUrl": url}));
    assert_eq!(fs::read(paths.resolve_localart(&url, None).unwrap()).unwrap(), png());
    // Heronite's reader accepts the file.
    let lib = heronite_data::library::Library::load(&paths).unwrap();
    let game = lib.get("sideload_steam_620").unwrap();
    assert_eq!(game.art.as_ref().unwrap().hero_url.as_deref(), Some(url.as_str()));

    let err = svc
        .set_art_from_url("sideload", "steam_620", ArtKind::Cover, &format!("{base}/img/full.png"), None)
        .unwrap_err();
    assert!(err.to_string().starts_with("Could not download image"));
    let reset = svc.reset_art("sideload", "steam_620", ArtKind::Hero).unwrap();
    assert!(reset.hero_url.is_none());
    assert!(json_store::read_value(&paths.art()).unwrap()["games"].get("sideload:steam_620").is_none());
}

#[test]
fn metadata_art_downloads_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let (server_base, _c) = mock();
    let img = Server::start(|_| Resp::image(png()));
    // The IGDB cover URL is replaced by one that points at the image server.
    let mut http_paths = server_base.endpoints();
    http_paths.igdb = format!("{}/igdb/v4", server_base.base);
    let svc = MetadataService::with_endpoints(paths.clone(), http_paths);
    let url = format!("{}/c.png", img.base);
    let out = heronite_metadata::images::materialize_metadata_image(
        &paths, &svc.http, "sideload", "steam_620", ArtKind::Cover, Some(&url), true,
    )
    .unwrap();
    assert_eq!(out, "localart://metadata-art/sideload_steam_620/cover.png");
    assert!(paths.metadata_art_dir().join("sideload_steam_620/cover.png").exists());
}

#[test]
fn web_image_search_uses_ddg_then_falls_back_to_bing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let ddg_blocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let blocked = ddg_blocked.clone();
    let server = Server::start(move |req| match req.path() {
        "/ddg/" => Resp::html(r#"<script>nrje('/d.js?q=x&vqd="4-77"&p=1')</script>"#),
        "/ddg/i.js" => {
            if blocked.load(Ordering::SeqCst) {
                return Resp::status(403);
            }
            assert_eq!(req.query("vqd").as_deref(), Some("4-77"));
            assert_eq!(req.query("q").as_deref(), Some("\"Portal 2\" cover"));
            assert_eq!(req.header("referer"), Some("https://duckduckgo.com/"));
            Resp::raw_json(fixture("ddg_images_portal2_cover.json"))
        }
        "/bing/images/search" => Resp::html(&fixture("bing_images_portal2.html")),
        "/google/search" => Resp::status(302),
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(paths, server.endpoints());
    let q = web_images_query("Portal 2");
    let images = svc.search_web_images(&q, false).unwrap();
    assert_eq!(images.len(), 6);
    assert_eq!(server.count(|r| r.path() == "/bing/images/search"), 0);

    ddg_blocked.store(true, Ordering::SeqCst);
    let images = svc.search_web_images(&q, true).unwrap();
    assert_eq!(images.len(), 5, "Bing fallback");
    assert!(svc.search_web_images("   ", false).unwrap().is_empty());
}

fn web_images_query(title: &str) -> String {
    heronite_metadata::web_images::cover_query(title)
}

#[test]
fn hltb_search_writes_wikigameinfo_in_the_cache_format() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let searches = Arc::new(AtomicUsize::new(0));
    let s = searches.clone();
    let server = Server::start(move |req| match req.path() {
        "/hltb/api/search/site/init" => Resp::json(json!({"token": "T0K"})),
        "/hltb/api/search/site" => {
            s.fetch_add(1, Ordering::SeqCst);
            assert_eq!(req.header("x-auth-token"), Some("T0K"));
            let body: Value = serde_json::from_str(&req.body).unwrap();
            assert_eq!(body["searchTerms"], json!(["Portal", "2"]));
            assert_eq!(body["searchType"], "games");
            Resp::raw_json(fixture("hltb_search_portal2.json"))
        }
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    // An unrelated cached entry that must survive.
    fs::create_dir_all(paths.wiki_game_info().parent().unwrap()).unwrap();
    json_store::write(
        &paths.wiki_game_info(),
        &json!({"Control": {"howlongtobeat": {"mainStory": 1.0, "mainExtra": 2.0, "completionist": 3.0}},
                "__timestamp": {"Control": "Tue Sep 22 2026 01:20:15 GMT-0300 (Brasilia Standard Time)"}}),
    )
    .unwrap();

    let got = svc.how_long_to_beat("Portal 2", None).unwrap();
    assert!(!got.from_cache);
    let entry = got.entry.unwrap();
    assert_eq!(entry.game_id, Some(7231));
    assert_eq!(entry.game_name.as_deref(), Some("Portal 2"));

    let raw = json_store::read_value(&paths.wiki_game_info()).unwrap();
    assert_eq!(raw["Portal 2"]["howlongtobeat"]["gameId"], 7231);
    assert!(raw["Portal 2"]["pcgamingwiki"].is_null());
    assert!(raw["__timestamp"]["Portal 2"].as_str().unwrap().contains("GMT"));
    assert_eq!(raw["Control"]["howlongtobeat"]["mainStory"], 1.0);
    // The data crate reads it, even with the special-character key rules.
    let wiki = heronite_data::hltb::WikiCache::load(&paths);
    let h = wiki.hltb("Portal 2", "").unwrap();
    assert!((h.main_story - 30885.0 / 3600.0).abs() < 1e-9);

    // Second call: cache.
    let again = svc.how_long_to_beat("Portal 2", None).unwrap();
    assert!(again.from_cache);
    assert_eq!(searches.load(Ordering::SeqCst), 1);
}

#[test]
fn covers_use_steam_cdn_or_title_search() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = Server::start(|req| match req.path() {
        "/community/actions/SearchApps/Plain%20Game" => Resp::raw_json(fixture("steam_searchapps_portal.json").replace("Portal 2", "Plain Game").replace("\"620\"", "\"4242\"")),
        "/steam/api/storesearch/" => Resp::json(json!({"items": []})),
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    // `steam_620` has a Steam id and no art; `plain` is found by title.
    let n = svc.refresh_missing_local_covers().unwrap();
    assert_eq!(n, 1);
    let lib: Value = json_store::read_value(&paths.sideload_library()).unwrap();
    let games = lib["games"].as_array().unwrap();
    let portal = games.iter().find(|g| g["app_name"] == "steam_620").unwrap();
    // It already had covers: untouched.
    assert_eq!(portal["art_square"], "https://store/sq.jpg");
    let plain = games.iter().find(|g| g["app_name"] == "plain").unwrap();
    assert_eq!(plain["art_square"], "https://cdn.cloudflare.steamstatic.com/steam/apps/4242/library_600x900_2x.jpg");
    assert_eq!(plain["art_cover"], "https://cdn.cloudflare.steamstatic.com/steam/apps/4242/header.jpg");
    assert_eq!(plain["title"], "Plain Game");
}
