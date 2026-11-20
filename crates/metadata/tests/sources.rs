//! What the user sets per source (on or off, fields switched off, options)
//! as the service obeys it, against a local mock of the remote services.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use common::{fixture, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::sources::{OptionValue, SourceStatus};
use heronite_metadata::*;
use serde_json::{json, Value};

/// One sideloaded Steam game with its overlay entry and IGDB credentials.
fn make_root(dir: &Path) -> DataPaths {
    let paths = DataPaths::at(dir);
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    w(
        paths.sideload_library(),
        json!({"games": [
            {"runner": "sideload", "app_name": "steam_620", "title": "Portal 2", "developer": "Valve Corp",
             "art_square": "https://store/sq.jpg", "art_cover": "https://store/cv.jpg", "is_installed": false, "install": {},
             "extra": {"genres": ["Puzzle"], "releaseDate": "2011-04-19"}}
        ]}),
    );
    w(
        paths.local_meta(),
        json!({"games": {
            "steam_620": {"appName": "steam_620", "runner": "sideload", "steamAppId": "620", "title": "Portal 2",
                          "playniteId": "", "source": "steam", "launchKind": "steam-uri"}
        }}),
    );
    w(paths.collection_settings(), json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}}));
    paths
}

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

/// Every service answers; Twitch refuses the keys while `refuse` is set.
fn mock(refuse: Arc<AtomicBool>) -> Server {
    Server::start(move |req| match req.path() {
        "/twitch/oauth2/token" if refuse.load(Ordering::SeqCst) => Resp::status(400),
        "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
        "/igdb/v4/games" if req.body.starts_with("search") => Resp::raw_json(fixture("igdb_search_portal2.json")),
        "/igdb/v4/games" => Resp::raw_json(fixture("igdb_game_steam_620.json")),
        "/steam/api/appdetails" => steam_body(req),
        "/lutris/api/games" => Resp::raw_json(fixture("lutris_search_portal.json")),
        "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
        p if p.starts_with("/sgdb/api/v2/search/autocomplete/") => {
            Resp::json(json!({"success": true, "data": [{"id": 77, "name": "Portal 2"}]}))
        }
        "/sgdb/api/v2/grids/game/77" => {
            Resp::json(json!({"success": true, "data": [{"url": "https://cdn2.steamgriddb.com/grid/a.png", "width": 600, "height": 900}]}))
        }
        "/ddg/" => Resp::html(r#"<script>nrje('/d.js?q=x&vqd="4-77"&p=1')</script>"#),
        "/ddg/i.js" => Resp::raw_json(fixture("ddg_images_portal2_cover.json")),
        _ => Resp::status(404),
    })
}

fn setup() -> (tempfile::TempDir, DataPaths, Server, MetadataService) {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = mock(Arc::new(AtomicBool::new(false)));
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    (dir, paths, server, svc)
}

fn args() -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: "steam_620".into(), runner: "sideload".into(), title: "Portal 2".into(), ..Default::default() }
}

fn sources_of(preview: &MetadataPreview) -> Vec<&str> {
    preview.candidates.iter().map(|c| c.source.as_str()).collect()
}

#[test]
fn a_source_that_is_off_is_not_asked() {
    let (_dir, paths, server, svc) = setup();
    let igdb_calls = |s: &Server| s.count(|r| r.path().starts_with("/igdb/") || r.path().starts_with("/twitch/"));
    assert_eq!(sources_of(&svc.preview_game_metadata(&args()).unwrap()), ["igdb", "steam", "store", "lutris"]);
    let before = igdb_calls(&server);
    let lutris = server.count(|r| r.path().starts_with("/lutris/"));
    assert!(before > 0 && lutris > 0);

    svc.set_source_enabled("igdb", false).unwrap();
    svc.set_source_enabled("lutris", false).unwrap();
    let preview = svc.preview_game_metadata(&args()).unwrap();
    assert_eq!(sources_of(&preview), ["steam", "store"]);
    assert_eq!(preview.proposed.field_sources["description"], "steam");
    assert!(!preview.proposed.field_sources.values().any(|s| s == "igdb" || s == "lutris"));
    assert_eq!(server.count(|r| r.path().starts_with("/lutris/")), lutris, "only the first preview asked Lutris");

    // The idle fill goes through the same gather.
    assert_eq!(svc.refresh_missing_collection_metadata().unwrap(), 1);
    let saved = svc.get_game_metadata("sideload", "steam_620").unwrap().unwrap();
    assert!(saved.igdb_id.is_none() && saved.field_sources["genres"] == "steam");
    assert_eq!(igdb_calls(&server), before, "IGDB was not asked again");

    // Off is stored under the source; back on leaves nothing behind.
    let file = json_store::read_value(&paths.collection_settings()).unwrap();
    assert_eq!(file["metadata"]["sources"]["igdb"], json!({"enabled": false}));
    svc.set_source_enabled("igdb", true).unwrap();
    svc.set_source_enabled("lutris", true).unwrap();
    let file = json_store::read_value(&paths.collection_settings()).unwrap();
    assert!(file["metadata"].get("sources").is_none());
    assert_eq!(sources_of(&svc.preview_game_metadata(&args()).unwrap()), ["igdb", "steam", "store", "lutris"]);
}

#[test]
fn a_field_switched_off_is_never_taken_from_its_source() {
    let (_dir, _paths, _server, svc) = setup();
    let preview = svc.preview_game_metadata(&args()).unwrap();
    assert_eq!(preview.proposed.field_sources["description"], "igdb");
    assert_eq!(preview.proposed.field_sources["cover"], "igdb");

    svc.set_source_field_enabled("igdb", "description", false).unwrap();
    svc.set_source_field_enabled("igdb", "cover", false).unwrap();
    let s = svc.metadata_settings();
    assert_eq!(svc.field_order(&s, "description"), ["steam", "store", "lutris", "playnite", "launchbox"]);
    let preview = svc.preview_game_metadata(&args()).unwrap();
    assert_eq!(preview.proposed.field_sources["description"], "steam");
    assert_eq!(preview.proposed.field_sources["cover"], "steam");
    assert_eq!(preview.proposed.field_sources["genres"], "igdb", "its other fields still come from it");
    assert!(preview.candidates[0].metadata.description.is_some(), "the candidate itself is shown as found");

    // Not even when it is preferred or picked by hand.
    let mut a = args();
    a.source = Some("igdb".into());
    a.field_picks = Some([("cover".to_string(), "igdb".to_string())].into_iter().collect());
    let md = svc.apply_preview(&preview, &a).unwrap();
    assert_eq!(md.field_sources["description"], "steam");
    assert_eq!(md.field_sources["cover"], "steam");
    assert_eq!(md.field_sources["releaseDate"], "igdb");
}

#[test]
fn moving_a_source_changes_what_the_merge_prefers() {
    let (_dir, _paths, _server, svc) = setup();
    svc.move_source("steam", 0).unwrap();
    let preview = svc.preview_game_metadata(&args()).unwrap();
    assert_eq!(preview.proposed.field_sources["description"], "steam");
    assert_eq!(preview.proposed.field_sources["genres"], "steam");

    // A field the user ordered keeps its order.
    svc.set_field_order("genres", &["igdb".into()]).unwrap();
    let s = svc.metadata_settings();
    assert!(svc.is_field_customised(&s, "genres"));
    assert_eq!(svc.field_order(&s, "genres")[..2], ["igdb", "steam"]);
    let preview = svc.preview_game_metadata(&args()).unwrap();
    assert_eq!(preview.proposed.field_sources["genres"], "igdb");
    let s = svc.reset_field_order("genres").unwrap().metadata;
    assert!(!svc.is_field_customised(&s, "genres"));
    assert_eq!(svc.field_order(&s, "genres")[0], "steam");
    assert!(svc.move_source("playnite", 0).is_err(), "hidden: not a row");
}

#[test]
fn pictures_and_extras_obey_their_switch() {
    let (_dir, paths, server, svc) = setup();
    fs::create_dir_all(paths.global_config().parent().unwrap()).unwrap();
    svc.set_source_option("steamgriddb", "apiKey", OptionValue::Text("K".into())).unwrap();
    let images = svc.search_art_images("\"Portal 2\" cover", ArtKind::Cover, false).unwrap();
    assert_eq!((images.steamgrid.len(), images.web.len()), (1, 6));

    svc.set_source_enabled("steamgriddb", false).unwrap();
    let sgdb = server.count(|r| r.path().starts_with("/sgdb/"));
    let images = svc.search_art_images("\"Portal 2\" cover", ArtKind::Cover, false).unwrap();
    assert_eq!((images.steamgrid.len(), images.web.len()), (0, 6));
    assert_eq!(server.count(|r| r.path().starts_with("/sgdb/")), sgdb);
    assert_eq!(covers::steamgrid_key_if_on(&paths), None);
    assert_eq!(covers::steamgrid_api_key(&paths).as_deref(), Some("K"), "the key itself stays");

    svc.set_source_enabled("webimages", false).unwrap();
    let ddg = server.count(|r| r.path().starts_with("/ddg/"));
    assert!(svc.search_web_images("\"Portal 2\" cover", false).unwrap().is_empty());
    let images = svc.search_art_images("\"Portal 2\" cover", ArtKind::Cover, false).unwrap();
    assert!(images.steamgrid.is_empty() && images.web.is_empty());
    assert_eq!(server.count(|r| r.path().starts_with("/ddg/")), ddg);

    // Steam tags and HowLongToBeat keep their old switches.
    let s = svc.set_source_enabled("steamtags", false).unwrap();
    assert!(!s.metadata.steam_tags && !svc.source_on("steamtags"));
    let s = svc.set_source_enabled("howlongtobeat", false).unwrap();
    assert!(!s.metadata.how_long_to_beat);
    let file = json_store::read_value(&paths.collection_settings()).unwrap();
    assert_eq!((&file["metadata"]["steamTags"], &file["metadata"]["howLongToBeat"]), (&json!(false), &json!(false)));
    assert!(file["metadata"]["sources"].get("steamtags").is_none());
    let s = svc.set_source_option("steamtags", "limit", OptionValue::Text("5".into())).unwrap();
    assert_eq!(s.metadata.steam_tag_limit, 5);
}

#[test]
fn testing_igdb_sets_its_status() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let refuse = Arc::new(AtomicBool::new(true));
    let server = mock(refuse.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    let status = |svc: &MetadataService| svc.source_status(&svc.metadata_settings(), "igdb");
    assert_eq!(status(&svc), Some(SourceStatus::Connected), "keys set, not tested yet");
    assert!(svc.test_source("lutris").is_none(), "no test");

    assert!(!svc.test_source("igdb").unwrap().ok);
    assert_eq!(status(&svc), Some(SourceStatus::TestFailed));
    let rows = svc.source_rows(&svc.metadata_settings());
    assert_eq!(rows.iter().find(|r| r.descriptor.id == "igdb").map(|r| r.status), Some(SourceStatus::TestFailed));

    refuse.store(false, Ordering::SeqCst);
    assert!(svc.test_source("igdb").unwrap().ok);
    assert_eq!(status(&svc), Some(SourceStatus::Connected));

    svc.set_source_option("igdb", "clientSecret", OptionValue::Text(String::new())).unwrap();
    assert_eq!(status(&svc), Some(SourceStatus::NeedsKey));
    assert!(!svc.test_source("igdb").unwrap().ok);
    assert_eq!(status(&svc), Some(SourceStatus::NeedsKey), "a missing key says so before a failed test");
    assert_eq!(svc.source_status(&svc.metadata_settings(), "elsewhere"), None);
}
