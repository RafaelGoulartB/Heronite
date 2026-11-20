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

/// The app details of `appids`: the recorded Portal (400), and as Portal 2 for 620.
fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    let mut app = all["400"].clone();
    if id == "620" {
        app["data"]["name"] = json!("Portal 2");
        app["data"]["release_date"]["date"] = json!("18 Apr, 2011");
    }
    Resp::json(json!({ id: app }))
}

/// Steam's store and community search, its app details, IGDB and Lutris;
/// `over` answers first (a request it takes is not the usual one).
fn mock_over(over: impl Fn(&Req) -> Option<Resp> + Send + Sync + 'static) -> Server {
    Server::start(move |req| {
        if let Some(r) = over(req) {
            return r;
        }
        match req.path() {
            p if p.starts_with("/community/actions/SearchApps/") => Resp::raw_json(fixture("steam_searchapps_portal.json")),
            "/steam/api/storesearch/" => Resp::raw_json(fixture("steam_storesearch_portal.json")),
            "/steam/api/appdetails" => steam_body(req),
            "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
            "/igdb/v4/games" if req.body.starts_with("search") => Resp::raw_json(fixture("igdb_search_portal2.json")),
            "/igdb/v4/games" => Resp::raw_json(fixture("igdb_game_steam_620.json")),
            "/lutris/api/games" => Resp::raw_json(fixture("lutris_search_portal.json")),
            "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
            _ => Resp::status(404),
        }
    })
}

fn mock() -> Server {
    mock_over(|_| None)
}

/// The service with the search by name turned on (it is off by default).
fn setup_with(server: Server) -> (tempfile::TempDir, DataPaths, Server, MetadataService) {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    svc.set_source_option("steam", "byName", OptionValue::Switch(true)).unwrap();
    (dir, paths, server, svc)
}

fn setup() -> (tempfile::TempDir, DataPaths, Server, MetadataService) {
    setup_with(mock())
}

/// The service with the search by name turned off.
fn setup_off() -> (tempfile::TempDir, DataPaths, Server, MetadataService) {
    let s = setup();
    s.3.set_source_option("steam", "byName", OptionValue::Switch(false)).unwrap();
    s
}

fn steam_id_of(svc: &MetadataService, app: &str) -> Option<String> {
    svc.get_game_metadata("sideload", app).unwrap().and_then(|md| md.steam_app_id)
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

fn item(app: &str, title: &str) -> BulkItem {
    BulkItem { app_name: app.into(), runner: "sideload".into(), title: title.into(), steam_app_id: None }
}

fn steam_state(preview: &MetadataPreview) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == "steam").map(|s| s.answer.clone()).unwrap()
}

fn appdetails(server: &Server) -> usize {
    server.count(|r| r.path() == "/steam/api/appdetails")
}

#[test]
fn by_default_a_game_not_on_steam_asks_nothing_of_steam() {
    let dir = tempfile::tempdir().unwrap();
    let server = mock();
    let svc = MetadataService::with_endpoints(make_root(dir.path()), server.endpoints());
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    assert_eq!(appdetails(&server) + server.count(|r| r.path().contains("search")), 0, "Steam was not asked");
}

#[test]
fn with_the_search_by_name_off_a_game_not_on_steam_asks_nothing_of_steam() {
    let (_dir, _paths, server, svc) = setup_off();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    assert!(preview.candidates.iter().all(|c| c.source != "steam"));
    assert_eq!(appdetails(&server) + server.count(|r| r.path().contains("search")), 0, "Steam was not asked");

    // An emulated game is not one a Steam page can be linked to; a Steam game is asked as always.
    let emulated = svc.preview_game_metadata(&args("emulated", None)).unwrap();
    assert_eq!(steam_state(&emulated), SourceAnswer::NotAsked { why: NotAsked::NotForThisGame });
    let steam = svc.preview_game_metadata(&args("steam_620", None)).unwrap();
    assert!(matches!(steam_state(&steam), SourceAnswer::Found { pinned: false, .. }));

    // Still linked by hand: the pin is asked whatever the setting.
    let linked = svc.preview_game_metadata(&args("manual", Some(Some("400")))).unwrap();
    assert!(matches!(steam_state(&linked), SourceAnswer::Found { pinned: true, .. }));
}

#[test]
fn with_the_search_by_name_on_a_game_is_found_on_steam_by_its_exact_name_and_never_by_a_like_one() {
    let (_dir, paths, server, svc) = setup();
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

    // Saved, the match is the record's, not the game's own, and not pinned.
    let saved = svc.apply_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(saved.steam_app_id.as_deref(), Some("620"));
    assert!(!record::source_match(&saved, "steam").unwrap().pinned);
    let overlay = json_store::read_value(&paths.local_meta()).unwrap();
    assert!(overlay["games"]["manual"].get("steamAppId").is_none());
    let game = Library::load(&paths).unwrap();
    let game = game.get("sideload_manual").unwrap();
    assert_eq!((game.steam_app_id.as_deref(), game.steam_launch_id.as_deref()), (None, None));
    assert_eq!(game.launch_kind.as_deref(), Some("executable"));

    // The bulk and idle fills go through the same search.
    let fill = svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![item("other", "Portal 2 Remake")]), &|_| {});
    assert_eq!(steam_id_of(&svc, "other"), None, "{fill:?}");
}

#[test]
fn unlinking_means_not_on_steam_and_sticks_for_every_later_update() {
    let (_dir, _paths, _server, svc) = setup();
    // Found by its name, unpinned.
    svc.apply_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_id_of(&svc, "manual").as_deref(), Some("620"));

    // Unlink: the preview already shows it, and Apply saves the mark with the other fields.
    let unlink = args("manual", Some(None));
    let preview = svc.preview_game_metadata(&unlink).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    let saved = svc.apply_preview(&preview, &unlink).unwrap();
    assert_eq!(saved.steam_app_id, None);
    assert!(record::source_match(&saved, "steam").is_none() && record::is_not_on(&saved, "steam"));
    assert!(saved.description.is_some(), "the fields stay");

    // Nothing finds it again: the dialog, a one-game update, a bulk fill.
    let after = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&after), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    let _ = svc.apply_game_metadata(&args("manual", None));
    let fill = svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![item("manual", "Portal 2")]), &|_| {});
    let refresh = svc.run_bulk_metadata(&BulkArgs::update(vec![item("manual", "Portal 2")], UpdateMode::Refresh, None, None), &|_| {});
    assert_eq!(steam_id_of(&svc, "manual"), None, "{fill:?} {refresh:?}");

    // Linking by hand again clears the mark.
    let link = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: true, .. }));
    let saved = svc.apply_preview(&preview, &link).unwrap();
    assert_eq!(saved.steam_app_id.as_deref(), Some("400"));
    assert!(!record::is_not_on(&saved, "steam"));
}

#[test]
fn unlinking_a_hand_made_link_without_other_sources_only_marks_the_game() {
    let (_dir, _paths, _server, svc) = setup_off();
    let link = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    svc.apply_preview(&preview, &link).unwrap();

    let saved = svc.unlink_sources("sideload", "manual", &["steam".to_owned()]).unwrap().unwrap();
    assert_eq!(saved.steam_app_id, None);
    assert!(record::source_match(&saved, "steam").is_none() && record::is_not_on(&saved, "steam"));
    assert!(saved.description.is_some());
    let after = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&after), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
}

#[test]
fn a_game_with_a_steam_id_of_its_own_is_never_marked_not_on_steam() {
    let (_dir, _paths, _server, svc) = setup();
    // "Automatic" on a Steam game lets Steam find it by its id again.
    let preview = svc.preview_game_metadata(&args("steam_620", Some(None))).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: false, .. }));
    let saved = svc.apply_preview(&preview, &args("steam_620", Some(None))).unwrap();
    assert!(!record::is_not_on(&saved, "steam"));
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

fn searches(server: &Server) -> usize {
    server.count(|r| r.path().contains("SearchApps") || r.path().contains("storesearch"))
}

#[test]
fn a_link_to_a_page_the_account_owns_stays_a_link_and_can_be_taken_away() {
    let (_dir, paths, _server, svc) = setup();
    // The account owns Portal (400), the page the hand-added game is linked to.
    json_store::write(
        &paths.steam_owned(),
        &json!({"games": [{"appId": "400", "name": "Portal Classic"}], "updatedAt": "2026-10-13T00:00:00Z"}),
    )
    .unwrap();
    let link = args("other", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    svc.apply_preview(&preview, &link).unwrap();
    let lib = Library::load(&paths).unwrap();
    let game = lib.get("sideload_other").unwrap();
    assert!(game.steam_link.is_none() && game.steam_app_id.is_none() && !game.via_steam, "not joined to the copy it is linked to");
    assert_eq!(game.tag_steam_id(), Some("400"));

    // Were the id of a joined copy the game's, "Unlink" would be refused: it is not, whatever the
    // dialog passes as the game's id.
    let mut unlink = args("other", Some(None));
    unlink.steam_app_id = Some("400".into());
    let preview = svc.preview_game_metadata(&unlink).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotLinked });
    let saved = svc.apply_preview(&preview, &unlink).unwrap();
    assert!(saved.steam_app_id.is_none() && record::is_not_on(&saved, "steam"));
}

#[test]
fn an_id_igdb_reports_does_not_bring_back_a_link_that_was_taken_away() {
    let (_dir, paths, _server, svc) = setup();
    json_store::write(&paths.collection_settings(), &json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}})).unwrap();
    let link = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&link).unwrap();
    svc.apply_preview(&preview, &link).unwrap();
    let unlink = args("manual", Some(None));
    let preview = svc.preview_game_metadata(&unlink).unwrap();
    assert!(
        preview.candidates.iter().any(|c| c.source == "igdb" && c.metadata.steam_app_id.as_deref() == Some("620")),
        "IGDB reports a Steam id"
    );
    let saved = svc.apply_preview(&preview, &unlink).unwrap();
    assert_eq!(saved.steam_app_id, None);

    // Later updates, one game or many, and a plain Apply do not write it back.
    let _ = svc.apply_game_metadata(&args("manual", None));
    assert_eq!(steam_id_of(&svc, "manual"), None);
    svc.run_bulk_metadata(&BulkArgs::update(vec![item("manual", "Portal 2")], UpdateMode::Refresh, None, None), &|_| {});
    assert_eq!(steam_id_of(&svc, "manual"), None);
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(preview.proposed.steam_app_id.is_none());
    let saved = svc.apply_preview(&preview, &args("manual", None)).unwrap();
    assert!(saved.steam_app_id.is_none() && record::is_not_on(&saved, "steam"));
    assert!(saved.igdb_id.is_some(), "IGDB's own match is kept");

    // Linking by hand again is the user's word: the id comes back.
    let relink = args("manual", Some(Some("400")));
    let preview = svc.preview_game_metadata(&relink).unwrap();
    assert_eq!(svc.apply_preview(&preview, &relink).unwrap().steam_app_id.as_deref(), Some("400"));
}

#[test]
fn a_game_of_the_same_title_from_another_time_is_not_taken() {
    let (_dir, paths, _server, svc) = setup();
    // The record says 1993; Steam's "Portal 2" is 2011.
    json_store::write(
        &paths.metadata(),
        &json!({"games": {"sideload:manual": {"appName": "manual", "runner": "sideload", "title": "Portal 2", "releaseDate": "1993-12-10",
            "developers": [], "publishers": [], "genres": [], "themes": [], "gameModes": [], "platforms": [], "features": [],
            "websites": [], "fieldSources": {}}}}),
    )
    .unwrap();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound);
    // The same year (or none) is the same game.
    json_store::write(&paths.metadata(), &json!({"games": {}})).unwrap();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { .. }));
}

#[test]
fn an_edition_of_the_title_is_not_the_exact_title() {
    let server = mock_over(|req| match req.path() {
        p if p.starts_with("/community/actions/SearchApps/") => Some(Resp::json(json!([]))),
        "/steam/api/storesearch/" => Some(Resp::json(json!({"items": [{"type": "app", "name": "Portal 2 - Deluxe Edition", "id": 777}]}))),
        _ => None,
    });
    let (_dir, _paths, _server, svc) = setup_with(server);
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound, "the strict search finds the edition; it is not taken");
}

#[test]
fn the_search_by_name_is_remembered_and_a_found_game_is_not_searched_again() {
    let (_dir, paths, server, svc) = setup();
    let first = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(matches!(steam_state(&first), SourceAnswer::Found { .. }));
    let after_first = searches(&server);
    assert!(after_first > 0);
    // A title Steam has no game for is remembered too.
    let miss = svc.preview_game_metadata(&args("other", None)).unwrap();
    assert_eq!(steam_state(&miss), SourceAnswer::NotFound);
    let after_miss = searches(&server);
    svc.preview_game_metadata(&args("manual", None)).unwrap();
    let again = svc.preview_game_metadata(&args("other", None)).unwrap();
    assert_eq!(steam_state(&again), SourceAnswer::NotFound);
    assert_eq!(searches(&server), after_miss, "the second dialog asked no search");
    assert!(titles_cache(&paths).exists());

    // Saved, the game found is asked for by its id: no search even without the memory.
    svc.apply_game_metadata(&args("manual", None)).unwrap();
    std::fs::remove_dir_all(paths.source_cache_dir("steam")).unwrap();
    let before = searches(&server);
    let kept = svc.preview_game_metadata(&args("manual", None)).unwrap();
    let SourceAnswer::Found { hit, pinned, .. } = steam_state(&kept) else { panic!("{:?}", steam_state(&kept)) };
    assert_eq!((hit.id.as_deref(), pinned), (Some("620"), false));
    assert_eq!(searches(&server), before, "no search for a game found before");
    let saved = svc.apply_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(record::source_match(&saved, "steam").unwrap().matched_by, Some(MatchedBy::ExactTitle), "still a match by name, not pinned");
}

#[test]
fn a_steam_that_throttles_or_fails_is_unreachable_and_nothing_is_remembered() {
    for status in [429, 503] {
        let server =
            mock_over(move |req| (req.path().contains("SearchApps") || req.path().contains("storesearch")).then(|| Resp::status(status)));
        let (_dir, paths, _server, svc) = setup_with(server);
        let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
        assert_eq!(steam_state(&preview), SourceAnswer::Unreachable, "{status}");
        assert!(!titles_cache(&paths).exists(), "{status}: not read as 'not on Steam'");
    }
}

#[test]
fn a_number_is_an_app_id_after_the_games_of_that_name_and_an_address_comes_first() {
    let server = mock_over(|req| match req.path() {
        p if p.starts_with("/community/actions/SearchApps/") => Some(Resp::json(json!([]))),
        "/steam/api/storesearch/" => Some(Resp::json(json!({"items": [
            {"type": "app", "name": "2048 Hexagonal", "id": 3001}, {"type": "app", "name": "2048", "id": 3002}]}))),
        _ => None,
    });
    let (_dir, _paths, _server, svc) = setup_with(server);
    let steam = ["steam".to_owned()];
    let ids = |q: &str| -> Vec<String> {
        svc.search_sources(q, Some(&steam)).choices.into_iter().flat_map(|c| c.hits).filter_map(|h| h.id).collect()
    };
    // App 2048 does not exist here, 400 does: a game titled "2048" is first, the number after it.
    let by_number = ids("400");
    assert_eq!(by_number.first().map(String::as_str), Some("400"));
    let titled = ids("2048");
    assert_eq!(titled.first().map(String::as_str), Some("3002"), "{titled:?}");
    assert_eq!(ids("https://store.steampowered.com/app/400/Portal/").first().map(String::as_str), Some("400"));
}

/// IGDB's Portal 2, as a game released in 1993.
fn igdb_of_1993() -> Server {
    mock_over(|req| {
        (req.path() == "/igdb/v4/games").then(|| {
            let body =
                if req.body.starts_with("search") { fixture("igdb_search_portal2.json") } else { fixture("igdb_game_steam_620.json") };
            Resp::raw_json(body.replace("1303084800", "735000000"))
        })
    })
}

fn only_search(community: impl Fn() -> Resp + Send + Sync + 'static, store: impl Fn() -> Resp + Send + Sync + 'static) -> Server {
    mock_over(move |req| match req.path() {
        p if p.starts_with("/community/actions/SearchApps/") => Some(community()),
        "/steam/api/storesearch/" => Some(store()),
        _ => None,
    })
}

fn titles_cache(paths: &DataPaths) -> std::path::PathBuf {
    paths.source_cache_dir("steam").join("titles")
}

/// A record of "manual" (Portal 2) with a match by name kept, the year of its date set.
fn kept_match(paths: &DataPaths, release_date: &str) {
    json_store::write(
        &paths.metadata(),
        &json!({"games": {"sideload:manual": {"appName": "manual", "runner": "sideload", "title": "Portal 2", "releaseDate": release_date,
            "steamAppId": "620", "developers": [], "publishers": [], "genres": [], "themes": [], "gameModes": [], "platforms": [],
            "features": [], "websites": [], "fieldSources": {},
            "sourceMatches": {"steam": {"id": "620", "title": "Portal 2", "year": 2011, "matchedBy": "exactTitle"}}}}}),
    )
    .unwrap();
}

#[test]
fn a_match_by_name_kept_in_the_record_is_checked_again_and_dropped_when_it_is_another_game() {
    let (_dir, paths, _server, svc) = setup();
    // Still the game: asked for by its id.
    kept_match(&paths, "2011-04-19");
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: false, .. }));
    assert_eq!(preview.proposed.steam_app_id.as_deref(), Some("620"));

    // The record's year now says 1993 (IGDB filled it): it is not that game. Not asked for it,
    // not found by name either, and the link goes, without a mark: a later search may find another.
    kept_match(&paths, "1993-12-10");
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound);
    assert_eq!(preview.proposed.steam_app_id, None);
    assert!(record::source_match(&preview.proposed, "steam").is_none() && !record::is_not_on(&preview.proposed, "steam"));

    // Renamed: the game of the old title is not this one.
    kept_match(&paths, "2011-04-19");
    let mut renamed = args("manual", None);
    renamed.title = "Portal 3".into();
    let preview = svc.preview_game_metadata(&renamed).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound);
    assert_eq!(preview.proposed.steam_app_id, None);

    // A link the user pinned is theirs: never dropped by a check of names.
    let mut md: Value = json_store::read_value(&paths.metadata()).unwrap();
    md["games"]["sideload:manual"]["sourceMatches"]["steam"] = json!({"id": "620", "title": "Portal 2", "pinned": true});
    json_store::write(&paths.metadata(), &md).unwrap();
    let preview = svc.preview_game_metadata(&renamed).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: true, .. }));
    assert_eq!(preview.proposed.steam_app_id.as_deref(), Some("620"));
}

#[test]
fn a_game_found_by_name_of_another_year_than_a_sure_source_says_is_left_out() {
    // IGDB's Portal 2 is from 1993 here; Steam's, found by name, from 2011.
    let server = igdb_of_1993();
    let (_dir, paths, _server, svc) = setup_with(server);
    json_store::write(&paths.collection_settings(), &json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}})).unwrap();
    svc.set_source_option("steam", "byName", OptionValue::Switch(true)).unwrap();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    let igdb = preview.candidates.iter().find(|c| c.source == "igdb").expect("IGDB found it");
    assert_eq!((igdb.match_year, igdb.matched_by), (Some(1993), Some(MatchedBy::ExactTitle)), "{:?}", igdb.metadata.release_date);
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound);
    assert!(preview.candidates.iter().all(|c| c.source != "steam") && preview.proposed.steam_app_id.as_deref() != Some("620"));

    // The same year: the same game.
    let (_dir, paths, _server, svc) = setup();
    json_store::write(&paths.collection_settings(), &json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}})).unwrap();
    svc.set_source_option("steam", "byName", OptionValue::Switch(true)).unwrap();
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { .. }));

    // A link the user made is theirs, whatever the year.
    let server = igdb_of_1993();
    let (_dir, paths, _server, svc) = setup_with(server);
    json_store::write(&paths.collection_settings(), &json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}})).unwrap();
    let preview = svc.preview_game_metadata(&args("manual", Some(Some("620")))).unwrap();
    assert!(matches!(steam_state(&preview), SourceAnswer::Found { pinned: true, .. }));
}

#[test]
fn a_no_is_remembered_only_when_every_request_answered() {
    // The community search fails; the store answers with nothing.
    let server = only_search(|| Resp::status(403), || Resp::json(json!({"items": []})));
    let (_dir, paths, _server, svc) = setup_with(server);
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&preview), SourceAnswer::NotFound);
    assert!(!titles_cache(&paths).exists(), "one request failed: Steam may know the game");

    // A bad answer is not an answer either.
    let server = only_search(|| Resp::raw_json("not json".into()), || Resp::json(json!({"items": []})));
    let (_dir, paths, _server, svc) = setup_with(server);
    svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(!titles_cache(&paths).exists());

    // Both answer: no such game, and it is remembered.
    let server = only_search(|| Resp::json(json!([])), || Resp::json(json!({"items": []})));
    let (_dir, paths, _server, svc) = setup_with(server);
    svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert!(titles_cache(&paths).exists());
}

#[test]
fn a_steam_that_said_to_wait_is_left_alone_for_a_while() {
    let server = only_search(|| Resp::status(429), || Resp::status(429));
    let (_dir, _paths, server, svc) = setup_with(server);
    let first = svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(steam_state(&first), SourceAnswer::Unreachable);
    let asked = searches(&server);
    assert!(asked > 0);
    // Other games of a bulk run, and the dialog again: no request until the wait is over.
    let other = svc.preview_game_metadata(&args("other", None)).unwrap();
    assert_eq!(steam_state(&other), SourceAnswer::Unreachable);
    svc.preview_game_metadata(&args("manual", None)).unwrap();
    assert_eq!(searches(&server), asked);
}

#[test]
fn an_edition_listed_first_does_not_stop_the_search_for_the_exact_name() {
    // The community search lists only the edition; the store has the game itself.
    let server = only_search(
        || Resp::json(json!([{"appid": "777", "name": "Portal 2 - Deluxe Edition"}])),
        || Resp::json(json!({"items": [{"type": "app", "name": "Portal 2", "id": 620}]})),
    );
    let (_dir, _paths, _server, svc) = setup_with(server);
    let preview = svc.preview_game_metadata(&args("manual", None)).unwrap();
    let SourceAnswer::Found { hit, .. } = steam_state(&preview) else { panic!("{:?}", steam_state(&preview)) };
    assert_eq!(hit.id.as_deref(), Some("620"));
}
