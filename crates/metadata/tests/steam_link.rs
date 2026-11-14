//! A game that is not on Steam, linked to its Steam page by hand or found
//! there by its exact title: what is asked, what the record keeps, and that
//! the game stays what it was (its own Steam id, its launch).

mod common;

use std::fs;
use std::path::Path;

use common::{fixture, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::library::Library;
use heronite_data::paths::DataPaths;
use heronite_metadata::record;
use heronite_metadata::sources::OptionValue;
use heronite_metadata::*;
use indexmap::IndexMap;
use serde_json::{json, Value};

/// A game added by hand, an emulated one with the same title, a Steam game.
fn make_root(dir: &Path) -> DataPaths {
    let paths = DataPaths::at(dir);
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    w(
        paths.sideload_library(),
        json!({"games": [
            {"runner": "sideload", "app_name": "manual", "title": "Portal 2", "is_installed": true, "install": {"executable": "/games/portal2/p2.exe"}},
            {"runner": "sideload", "app_name": "other", "title": "Portal 2 Remake", "is_installed": true, "install": {"executable": "/games/p/p.exe"}},
            {"runner": "sideload", "app_name": "emulated", "title": "Portal 2", "is_installed": true, "install": {}},
            {"runner": "sideload", "app_name": "steam_620", "title": "Portal 2", "is_installed": false, "install": {}}
        ]}),
    );
    w(
        paths.local_meta(),
        json!({"games": {
            "manual": {"appName": "manual", "runner": "sideload", "title": "Portal 2", "playniteId": "", "source": "manual", "launchKind": "executable"},
            "other": {"appName": "other", "runner": "sideload", "title": "Portal 2 Remake", "playniteId": "", "source": "manual", "launchKind": "executable"},
            "emulated": {"appName": "emulated", "runner": "sideload", "title": "Portal 2", "playniteId": "", "source": "emulator", "launchKind": "emulator"},
            "steam_620": {"appName": "steam_620", "runner": "sideload", "steamAppId": "620", "title": "Portal 2", "playniteId": "", "source": "steam", "launchKind": "steam-uri"}
        }}),
    );
    paths
}

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

/// Steam's store and community search, its app details and Lutris.
fn mock() -> Server {
    Server::start(|req| match req.path() {
        p if p.starts_with("/community/actions/SearchApps/") => Resp::raw_json(fixture("steam_searchapps_portal.json")),
        "/steam/api/storesearch/" => Resp::raw_json(fixture("steam_storesearch_portal.json")),
        "/steam/api/appdetails" => steam_body(req),
        "/lutris/api/games" => Resp::raw_json(fixture("lutris_search_portal.json")),
        "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
        _ => Resp::status(404),
    })
}

fn setup() -> (tempfile::TempDir, DataPaths, Server, MetadataService) {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = mock();
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    (dir, paths, server, svc)
}

fn args(app: &str, pin: Option<Option<&str>>) -> MetadataApplyArgs {
    let title = if app == "other" { "Portal 2 Remake" } else { "Portal 2" };
    MetadataApplyArgs {
        app_name: app.into(),
        runner: "sideload".into(),
        title: title.into(),
        source: Some("auto".into()),
        overwrite_existing: Some(true),
        pins: pin.map(|p| IndexMap::from([("steam".to_owned(), p.map(str::to_owned))])),
        ..Default::default()
    }
}

fn steam_state(preview: &MetadataPreview) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == "steam").map(|s| s.answer.clone()).unwrap()
}

fn appdetails(server: &Server) -> usize {
    server.count(|r| r.path() == "/steam/api/appdetails")
}

#[test]
fn a_game_not_on_steam_can_be_linked_but_asks_nothing_of_steam_by_itself() {
    let (_dir, _paths, server, svc) = setup();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    assert!(preview.candidates.iter().all(|c| c.source != "steam"));
    assert_eq!(appdetails(&server) + server.count(|r| r.path().contains("search")), 0, "Steam was not asked");

    // An emulated game is not one a Steam page can be linked to; a Steam game is asked as always.
    let emulated = svc.preview_game_metadata(&args("emulated", None)).unwrap();
    assert_eq!(steam_state(&emulated), SourceAnswer::NotAsked { why: NotAsked::NotForThisGame });
    let steam = svc.preview_game_metadata(&args("steam_620", None)).unwrap();
    assert!(matches!(steam_state(&steam), SourceAnswer::Found { pinned: false, .. }));
}

#[test]
fn steam_is_searched_by_title_address_or_app_id() {
    let (_dir, _paths, _server, svc) = setup();
    let steam = ["steam".to_owned()];
    let found = svc.search_sources("Portal", Some(&steam));
    let hits: Vec<SourceHit> = found.choices.into_iter().flat_map(|c| c.hits).collect();
    assert_eq!(
        hits.iter().filter_map(|h| h.title.as_deref()).take(2).collect::<Vec<_>>(),
        ["Portal", "Portal 2"],
        "the title as typed first"
    );
    assert!(hits.iter().all(|h| h.source == "steam" && h.thumb_url.as_deref().is_some_and(|t| t.contains(h.id.as_deref().unwrap()))));

    // An address or an app id names its game: it comes first, with its year.
    for typed in
        ["https://store.steampowered.com/app/400/Portal/", "store.steampowered.com/app/400", "https://steamcommunity.com/app/400", "400"]
    {
        let found = svc.search_sources(typed, Some(&steam));
        let first = found.choices.into_iter().flat_map(|c| c.hits).next().unwrap();
        assert_eq!((first.id.as_deref(), first.title.as_deref(), first.year), (Some("400"), Some("Portal"), Some(2007)), "{typed}");
    }
}

#[test]
fn a_linked_game_keeps_the_page_in_its_record_and_stays_a_game_of_its_own() {
    let (_dir, paths, _server, svc) = setup();
    let link = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: true, .. }));
    let saved = svc.apply_preview(&preview, &link).unwrap();
    assert_eq!(saved.steam_app_id.as_deref(), Some("400"));
    assert!(saved.field_sources.values().any(|s| s == "steam"), "Steam's details fill the record");
    let page = record::source_match(&saved, "steam").unwrap();
    assert!(page.pinned && page.id == "400");

    // Its own entry is untouched: no Steam id, still launched as an executable.
    let overlay = json_store::read_value(&paths.local_meta()).unwrap();
    assert!(overlay["games"]["manual"].get("steamAppId").is_none());
    assert_eq!(overlay["games"]["manual"]["launchKind"], "executable");
    let lib = Library::load(&paths).unwrap();
    let game = lib.get("sideload_manual").unwrap();
    assert_eq!(game.steam_app_id, None, "the page is metadata, not an owned Steam game");
    assert_eq!(game.steam_launch_id, None);
    assert_eq!(game.launch_kind.as_deref(), Some("executable"));
    assert!(game.installed && !game.via_steam && game.steam_link.is_none() && game.family.is_none());
    // What reads the record's id (tags, ProtonDB, links) finds it.
    assert_eq!(game.tag_steam_id(), Some("400"));

    // Later updates ask for that page, with nothing pinned in the arguments.
    let again = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(matches!(steam_state(&again), SourceAnswer::Found { pinned: true, .. }));
    assert!(again.candidates.iter().any(|c| c.source == "steam" && c.match_id.as_deref() == Some("400")));
}

#[test]
fn unlinking_takes_the_page_away_but_keeps_what_it_filled() {
    let (_dir, _paths, _server, svc) = setup();
    let link = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    svc.apply_preview(&preview, &link).unwrap();

    // With other sources found, the update that unlinks drops the link.
    let unlink = args("manual", Some(None));
    let preview = svc.preview_game_metadata(&unlink).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    let saved = svc.apply_preview(&preview, &unlink).unwrap();
    assert_eq!(saved.steam_app_id, None);
    assert!(record::source_match(&saved, "steam").is_none());
    assert!(saved.description.is_some(), "the fields stay");

    // Without any, only the link goes.
    let preview = svc.preview_game_metadata(&link).unwrap();
    svc.apply_preview(&preview, &link).unwrap();
    let saved = svc.unlink_sources("sideload", "manual", &["steam".to_owned()]).unwrap().unwrap();
    assert_eq!(saved.steam_app_id, None);
    assert!(record::source_match(&saved, "steam").is_none());
    assert!(saved.description.is_some());
    let after = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&after), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
}

#[test]
fn finding_a_game_on_steam_by_name_is_an_option_and_takes_only_the_same_title() {
    let (_dir, paths, server, svc) = setup();
    svc.set_source_option("steam", "byName", OptionValue::Switch(true)).unwrap();

    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    let SourceAnswer::Found { hit, pinned, .. } = steam_state(&preview) else { panic!("{:?}", steam_state(&preview)) };
    assert_eq!((hit.id.as_deref(), hit.matched_by, pinned), (Some("620"), Some(MatchedBy::ExactTitle), false));
    assert!(appdetails(&server) > 0);

    // A title that is only alike is never taken.
    let alike = svc.preview_game_metadata(&args("other", None)).unwrap();
    assert_eq!(steam_state(&alike), SourceAnswer::NotFound);
    assert!(alike.candidates.iter().all(|c| c.source != "steam"));

    // Still not for an emulated game.
    let emulated = svc.preview_game_metadata(&args("emulated", None)).unwrap();
    assert_eq!(steam_state(&emulated), SourceAnswer::NotAsked { why: NotAsked::NotForThisGame });

    // Saved, the match is the record's, not the game's own.
    let saved = svc.apply_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(saved.steam_app_id.as_deref(), Some("620"));
    assert!(!record::source_match(&saved, "steam").unwrap().pinned);
    let overlay = json_store::read_value(&paths.local_meta()).unwrap();
    assert!(overlay["games"]["manual"].get("steamAppId").is_none());

    // Off again: back to a link to pick.
    svc.set_source_option("steam", "byName", OptionValue::Switch(false)).unwrap();
    let off = svc.preview_game_metadata(&args("other", None)).unwrap();
    assert_eq!(steam_state(&off), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
}

#[test]
fn a_steam_that_cannot_be_reached_is_not_a_game_found_in_no_source() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let server = mock();
    let mut endpoints = server.endpoints();
    endpoints.steam_store = "http://127.0.0.1:9/steam".into();
    endpoints.steam_community = "http://127.0.0.1:9/community".into();
    let svc = MetadataService::with_endpoints(paths, endpoints);
    svc.set_source_option("steam", "byName", OptionValue::Switch(true)).unwrap();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::Unreachable);
}
