//! VNDB as a source: its lookups and searches, when it is asked (only as a
//! fallback), adult covers, its back-off, against a local mock serving
//! recorded VNDB answers.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{fixture, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::record::{self, SourceMatch};
use heronite_metadata::service::is_sent_to_review;
use heronite_metadata::*;
use serde_json::{json, Value};

/// The games, none with IGDB keys: (app name, title, Steam id).
const GAMES: [(&str, &str, Option<&str>); 8] = [
    ("sg", "Steins;Gate", None),
    ("sgja", "シュタインズ・ゲート", None),
    ("clannad", "CLANNAD", None),
    ("saya", "The Song of Saya", None),
    ("steam_sg", "Steins;Gate", Some("412830")),
    ("fsn", "Fate/stay night", None),
    ("portal", "Portal", None),
    ("ms", "Midnight Seminar", None),
];

fn make_root(dir: &Path) -> DataPaths {
    let paths = DataPaths::at(dir);
    let overlay: serde_json::Map<String, Value> = GAMES
        .iter()
        .map(|(app, title, steam)| {
            let entry = json!({"appName": app, "runner": "sideload", "title": title, "steamAppId": steam,
                               "playniteId": "", "source": "manual", "launchKind": "executable"});
            (app.to_string(), entry)
        })
        .collect();
    write(&paths.local_meta(), json!({ "games": overlay }));
    write(&paths.collection_settings(), json!({"metadata": {}}));
    paths
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn body(req: &Req) -> Value {
    serde_json::from_str(&req.body).unwrap_or(Value::Null)
}

/// The recorded VNDB answer for a `POST /vn`.
fn vndb_answer(req: &Req) -> Resp {
    let b = body(req);
    let filter = (b["filters"][0].as_str().unwrap_or(""), b["filters"][2].as_str().unwrap_or(""));
    let file = match filter {
        ("search", "Steins;Gate") => "vndb_lookup_steins_gate.json",
        ("search", "シュタインズ・ゲート") => "vndb_lookup_ja_steins_gate.json",
        ("search", "CLANNAD") => "vndb_lookup_clannad.json",
        ("search", "Fate/stay night") => "vndb_lookup_fate_stay_night.json",
        ("search", "The Song of Saya") => "vndb_lookup_song_of_saya.json",
        ("search", "Fate") => "vndb_search_fate.json",
        ("search", "Midnight Seminar") | ("id", "v9") => "vndb_id_v9.json",
        ("id", "v2002") => "vndb_id_v2002.json",
        _ => return Resp::json(json!({"results": [], "more": false})),
    };
    Resp::raw_json(fixture(file))
}

/// VNDB as recorded; Steam's app details for any id; Lutris has Portal
/// and a "Fate/stay night" whose genre is Visual Novel; nothing else.
/// `throttle`: that many VNDB requests are answered 429 first.
fn mock(throttle: Arc<AtomicUsize>) -> Server {
    Server::start(move |req| match req.path() {
        "/vndb/kana/vn" => {
            if throttle.load(Ordering::SeqCst) > 0 {
                throttle.fetch_sub(1, Ordering::SeqCst);
                return Resp::status(429).with_header("Retry-After", "0");
            }
            vndb_answer(req)
        }
        "/steam/api/appdetails" => {
            let id = req.query("appids").unwrap_or_default();
            let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
            Resp::json(json!({ id: all["400"].clone() }))
        }
        "/lutris/api/games" => Resp::json(match req.query("search").as_deref() {
            Some("Portal") => serde_json::from_str::<Value>(&fixture("lutris_search_portal.json")).unwrap(),
            Some("Fate/stay night") => json!({"results": [{"name": "Fate/stay night", "slug": "fsn", "year": 2004}]}),
            Some("Midnight Seminar") => json!({"results": [{"name": "Midnight Seminar", "slug": "midnight-seminar", "year": 2000}]}),
            _ => json!({"results": []}),
        }),
        "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
        "/lutris/api/games/fsn" => Resp::json(
            json!({"name": "Fate/stay night", "slug": "fsn", "year": 2004, "genres": [{"name": "Visual Novel"}], "coverart": "https://lutris/fsn.jpg"}),
        ),
        "/lutris/api/games/midnight-seminar" => {
            Resp::json(json!({"name": "Midnight Seminar", "slug": "midnight-seminar", "year": 2000, "coverart": "https://lutris/ms.jpg"}))
        }
        _ => Resp::status(404),
    })
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
    throttle: Arc<AtomicUsize>,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let throttle = Arc::new(AtomicUsize::new(0));
    let server = mock(throttle.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    Setup { _dir: dir, paths, server, svc, throttle }
}

fn args(app: &str) -> MetadataApplyArgs {
    let (_, title, steam) = GAMES.iter().find(|g| g.0 == app).unwrap();
    MetadataApplyArgs {
        app_name: app.into(),
        runner: "sideload".into(),
        title: title.to_string(),
        steam_app_id: steam.map(Into::into),
        ..Default::default()
    }
}

fn item(app: &str) -> BulkItem {
    let a = args(app);
    BulkItem { app_name: a.app_name, runner: a.runner, title: a.title, steam_app_id: a.steam_app_id }
}

/// VNDB requests whose body names `what` (a title or an id).
fn vndb_asked(server: &Server, what: &str) -> usize {
    server.count(|r| r.path() == "/vndb/kana/vn" && body(r)["filters"][2].as_str() == Some(what))
}

fn vndb_state(preview: &MetadataPreview) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == "vndb").map(|s| s.answer.clone()).unwrap()
}

fn vndb_candidate(preview: &MetadataPreview) -> Option<&MetadataCandidate> {
    preview.candidates.iter().find(|c| c.source == "vndb")
}

fn set_metadata(paths: &DataPaths, v: Value) {
    write(&paths.collection_settings(), json!({ "metadata": v }));
}

#[test]
fn a_game_no_other_source_has_is_found_by_its_title() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1, "one request: the search with every field");
    assert_eq!(saved.title.as_deref(), Some("STEINS;GATE"));
    assert_eq!(saved.release_date.as_deref(), Some("2009-10-15"));
    assert_eq!(saved.developers, ["NITRO PLUS", "MAGES.", "MAGES. Inc."]);
    assert_eq!(saved.genres, ["Visual Novel"]);
    assert_eq!(saved.critic_score.map(|n| n.get()), Some(90.0));
    assert!(saved.themes.contains(&"Chuunibyou Protagonist".to_string()));
    assert!(saved.description.as_deref().is_some_and(|d| !d.contains("[url")));
    assert_eq!(saved.cover_url.as_deref(), Some("https://t.vndb.org/cv/19/77819.jpg"));
    assert!(saved.hero_url.as_deref().is_some_and(|u| u.starts_with("https://t.vndb.org/sf/")));
    assert!(["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"]
        .iter()
        .all(|f| saved.field_sources[*f] == "vndb"));
    let m = record::source_match(&saved, "vndb").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by, m.pinned), ("v2002", Some(MatchedBy::ExactTitle), false));

    // It says who it is, and nothing about the user.
    let req = s.server.log.lock().iter().find(|r| r.path() == "/vndb/kana/vn").cloned().unwrap();
    assert!(req.header("user-agent").is_some_and(|ua| ua.starts_with("Heronite/")), "{:?}", req.header("user-agent"));
    assert_eq!(req.method, "POST");
    assert_eq!(body(&req)["sort"], "searchrank");

    // Found again by its id next time: no title search.
    s.svc.apply_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1);
    assert_eq!(vndb_asked(&s.server, "v2002"), 1);
    let again = s.svc.get_game_metadata("sideload", "sg").unwrap().unwrap();
    assert_eq!(record::source_match(&again, "vndb").unwrap().matched_by, Some(MatchedBy::ExactTitle), "how it was first found");
}

#[test]
fn japanese_and_romaji_titles_match_exactly_a_close_one_waits_for_review() {
    let s = setup();
    let preview = s.svc.preview_game_metadata(&args("sgja")).unwrap();
    let c = vndb_candidate(&preview).unwrap();
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("v2002"), Some(MatchedBy::ExactTitle)));
    let preview = s.svc.preview_game_metadata(&args("clannad")).unwrap();
    assert_eq!(vndb_candidate(&preview).and_then(|c| c.match_id.as_deref()), Some("v4"));
    // The title option names the match, not the game.
    set_metadata(&s.paths, json!({"sources": {"vndb": {"options": {"titleOrder": "original"}}}}));
    let saya = s.svc.preview_game_metadata(&args("saya")).unwrap();
    let c = vndb_candidate(&saya).unwrap();
    assert_eq!((c.match_name.as_deref(), c.matched_by), (Some("沙耶の唄"), Some(MatchedBy::FuzzyTitle)));
    // A lone guess is not saved.
    let err = s.svc.apply_game_metadata(&args("saya")).unwrap_err();
    assert!(is_sent_to_review(&err).is_some(), "{err:#}");
    assert!(s.svc.review_queue().iter().any(|q| q.app_name == "saya"));
    assert!(s.svc.get_game_metadata("sideload", "saya").unwrap().is_none_or(|md| md.title.as_deref() != Some("沙耶の唄")));
}

#[test]
fn a_typed_id_or_address_finds_that_entry() {
    let s = setup();
    let only = ["vndb".to_string()];
    for query in ["v2002", "https://vndb.org/v2002"] {
        let found = s.svc.search_sources(query, Some(&only));
        assert_eq!(found.asked, ["vndb"]);
        let hits: Vec<&SourceHit> = found.choices.iter().flat_map(|c| &c.hits).collect();
        assert_eq!(hits.len(), 1, "{query}");
        assert_eq!((hits[0].id.as_deref(), hits[0].title.as_deref()), (Some("v2002"), Some("STEINS;GATE")));
    }
    assert_eq!(vndb_asked(&s.server, "v2002"), 2);
    // A search by name: several entries, with the original title beside.
    let fate = s.svc.search_sources("Fate", Some(&only));
    assert!(fate.choices.len() > 10);
    let extra = fate.choices.iter().find(|c| c.title.as_deref() == Some("Fate/EXTRA")).unwrap();
    assert_eq!(extra.alt_title.as_deref(), Some("フェイト/エクストラ"));
}

#[test]
fn a_pinned_entry_is_looked_up_by_its_id_and_is_sure() {
    let s = setup();
    let mut a = args("portal");
    a.pins = Some([("vndb".to_string(), Some("v2002".to_string()))].into_iter().collect());
    let preview = s.svc.preview_game_metadata(&a).unwrap();
    let c = vndb_candidate(&preview).unwrap();
    assert_eq!(c.matched_by, Some(MatchedBy::SourceId));
    assert!(matches!(vndb_state(&preview), SourceAnswer::Found { pinned: true, .. }));
    assert_eq!(vndb_asked(&s.server, "v2002"), 1);
    assert_eq!(vndb_asked(&s.server, "Portal"), 0, "not searched by title");
    // Saved pinned: every later fetch asks it, whatever the others found.
    s.svc.apply_preview(&preview, &a).unwrap();
    let saved = s.svc.get_game_metadata("sideload", "portal").unwrap().unwrap();
    assert!(record::source_match(&saved, "vndb").unwrap().pinned);
    s.svc.preview_game_metadata(&args("portal")).unwrap();
    assert_eq!(vndb_asked(&s.server, "v2002"), 2);
}

#[test]
fn it_is_asked_only_as_a_fallback() {
    let s = setup();
    // Another source identified the game: no request, and the dialog says why.
    let preview = s.svc.preview_game_metadata(&args("steam_sg")).unwrap();
    assert!(preview.candidates.iter().any(|c| c.source == "steam"));
    assert_eq!(vndb_state(&preview), SourceAnswer::NotAsked { why: NotAsked::Fallback });
    let portal = s.svc.preview_game_metadata(&args("portal")).unwrap();
    assert_eq!(vndb_state(&portal), SourceAnswer::NotAsked { why: NotAsked::Fallback }, "Lutris found it by its title");
    assert_eq!(s.server.count(|r| r.path().starts_with("/vndb/")), 0);

    // No other source found it: asked.
    s.svc.preview_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1);

    // Another source says it is a visual novel: asked after it.
    let fsn = s.svc.preview_game_metadata(&args("fsn")).unwrap();
    assert_eq!(fsn.candidates.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["lutris", "vndb"]);
    assert_eq!(vndb_asked(&s.server, "Fate/stay night"), 1);

    // The record says so: asked with the others.
    json_store::write(
        &s.paths.metadata(),
        &json!({"games": {"sideload:steam_sg": {"appName": "steam_sg", "runner": "sideload", "title": "Steins;Gate",
                "genres": ["Adventure", "Visual Novel"], "fieldSources": {}}}}),
    )
    .unwrap();
    let preview = s.svc.preview_game_metadata(&args("steam_sg")).unwrap();
    assert!(vndb_candidate(&preview).is_some());
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 2);

    // The game's Steam tags say so.
    json_store::write(&s.paths.metadata(), &json!({"games": {}})).unwrap();
    write(
        &s.paths.steam_tags(),
        json!({"apps": {"412830": {"tags": [{"tagid": 3799, "weight": 900}], "fetchedAt": 1}},
               "names": {"english": {"versionHash": "x", "fetchedAt": 1, "names": {"3799": "Visual Novel"}}}}),
    );
    s.svc.preview_game_metadata(&args("steam_sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 3);
}

#[test]
fn a_job_of_only_vndb_asks_it_and_one_that_leaves_another_out_does_not() {
    let s = setup();
    let done = s.svc.run_bulk_metadata(&BulkArgs::only_source(vec![item("steam_sg")], UpdateMode::Fill, "vndb"), &|_| {});
    assert_eq!(done.results.len(), 1);
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1);
    let saved = s.svc.get_game_metadata("sideload", "steam_sg").unwrap().unwrap();
    assert_eq!(saved.field_sources.get("description").map(String::as_str), Some("vndb"));

    let fewer = BulkArgs::update(vec![item("portal")], UpdateMode::Fill, None, Some(vec!["steam".into(), "lutris".into(), "vndb".into()]));
    s.svc.run_bulk_metadata(&fewer, &|_| {});
    assert_eq!(vndb_asked(&s.server, "Portal"), 0);
}

#[test]
fn adult_covers_are_dropped_unless_allowed_and_the_next_source_fills_them() {
    let s = setup();
    // VNDB first for the cover, pinned to an entry whose cover is explicit.
    let pin = SourceMatch { id: "v9".into(), title: Some("Midnight Seminar".into()), pinned: true, ..Default::default() };
    let mut md = heronite_metadata::util::blank_metadata("ms", "sideload");
    record::set_source_match(&mut md, "vndb", pin, None);
    json_store::write(&s.paths.metadata(), &json!({"games": {"sideload:ms": md}})).unwrap();
    let order = json!({"cover": ["vndb", "lutris", "igdb", "steam", "playnite", "store"]});
    set_metadata(&s.paths, json!({"fieldPriority": order}));

    let off = s.svc.preview_game_metadata(&args("ms")).unwrap();
    let c = vndb_candidate(&off).unwrap();
    assert!(c.metadata.cover_url.is_none() && c.adult_pictures.is_empty(), "dropped before the merge");
    assert_eq!(off.proposed.cover_url.as_deref(), Some("https://lutris/ms.jpg"));
    assert_eq!(off.proposed.field_sources["cover"], "lutris");
    match vndb_state(&off) {
        SourceAnswer::Found { hit, .. } => assert!(hit.thumb_url.is_none()),
        other => panic!("{other:?}"),
    }
    let found = s.svc.search_sources("Midnight Seminar", Some(&["vndb".to_string()]));
    assert!(found.choices[0].thumb_url.is_none() && found.choices[0].hits[0].thumb_url.is_none());
    // Its other fields still come.
    assert_eq!(off.proposed.field_sources["description"], "vndb");

    set_metadata(&s.paths, json!({"fieldPriority": order, "allowAdultCovers": true}));
    let on = s.svc.preview_game_metadata(&args("ms")).unwrap();
    assert!(on.proposed.cover_url.as_deref().is_some_and(|u| u.starts_with("https://t.vndb.org/")));
    assert_eq!(on.proposed.field_sources["cover"], "vndb");
    let found = s.svc.search_sources("Midnight Seminar", Some(&["vndb".to_string()]));
    assert!(found.choices[0].thumb_url.is_some());
    // Nothing tried to download a picture.
    assert_eq!(s.server.count(|r| !r.path().starts_with("/vndb/") && !r.path().starts_with("/lutris/")), 0);
}

#[test]
fn a_429_is_waited_out_and_one_that_lasts_saves_nothing() {
    let s = setup();
    s.throttle.store(1, Ordering::SeqCst);
    let preview = s.svc.preview_game_metadata(&args("sg")).unwrap();
    assert!(vndb_candidate(&preview).is_some());
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 2, "refused once, then answered");

    // Refused every time: with VNDB the only online source asked, the
    // fetch counts as offline and saves nothing.
    set_metadata(&s.paths, json!({"sources": {"lutris": {"enabled": false}}}));
    s.throttle.store(100, Ordering::SeqCst);
    let err = s.svc.apply_game_metadata(&args("clannad")).unwrap_err();
    assert!(heronite_metadata::service::is_offline(&err), "{err:#}");
    assert!(s.svc.get_game_metadata("sideload", "clannad").unwrap().is_none());
    assert_eq!(vndb_asked(&s.server, "CLANNAD"), 3, "the request and two tries after waiting");
}

#[test]
fn offline_it_is_not_asked_after_the_others_and_a_pinned_game_stays_offline() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(dir.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let dropped = Arc::new(AtomicUsize::new(0));
    let count = dropped.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            count.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    let dead = Endpoints {
        steam_store: format!("{base}/steam"),
        lutris: format!("{base}/lutris"),
        vndb: format!("{base}/vndb"),
        ..Endpoints::default()
    };
    let svc = MetadataService::with_endpoints(paths.clone(), dead);
    let preview = svc.preview_game_metadata(&args("sg")).unwrap();
    assert!(preview.offline);
    assert_eq!(vndb_state(&preview), SourceAnswer::NotAsked { why: NotAsked::Fallback });
    // Lutris' title queries only; VNDB was left alone.
    let after_lutris = dropped.load(Ordering::SeqCst);
    let mut a = args("sg");
    a.pins = Some([("vndb".to_string(), Some("v2002".to_string()))].into_iter().collect());
    let pinned = svc.preview_game_metadata(&a).unwrap();
    assert!(pinned.offline);
    assert_eq!(vndb_state(&pinned), SourceAnswer::Unreachable);
    assert!(dropped.load(Ordering::SeqCst) > after_lutris);
    assert!(heronite_metadata::service::is_offline(&svc.apply_game_metadata(&args("sg")).unwrap_err()));
}
