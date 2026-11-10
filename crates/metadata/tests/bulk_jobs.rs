//! Bulk metadata jobs against a local mock of the remote services: modes,
//! chosen fields and sources, per-game results, Undo, the queue, the idle
//! fills meeting a job, and the counts shown before a job.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{fallback, fixture, png, Req, Resp, Server, FALLBACK};
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use parking_lot::Mutex;
use serde_json::{json, Value};

const ITEMS: &str = "/steamapi/IStoreBrowseService/GetItems/v1/";
const TAG_LIST: &str = "/steamapi/IStoreService/GetTagList/v1/";

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

/// Every service answers (after `delay`); nothing has "Ghost Game".
fn mock(delay: Duration) -> Server {
    Server::start(move |req| {
        std::thread::sleep(delay);
        if req.target.contains("Ghost") || req.body.contains("Ghost") {
            return match req.path() {
                "/igdb/v4/games" => Resp::json(json!([])),
                _ => Resp::status(404),
            };
        }
        match req.path() {
            "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
            "/igdb/v4/games" if req.body.starts_with("search") => Resp::raw_json(fixture("igdb_search_portal2.json")),
            "/igdb/v4/games" => Resp::raw_json(fixture("igdb_game_steam_620.json")),
            "/steam/api/appdetails" => steam_body(req),
            "/lutris/api/games" => Resp::raw_json(fixture("lutris_search_portal.json")),
            "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
            p if p.starts_with("/img/") => Resp::image(png()),
            ITEMS => Resp::raw_json(fixture("steam_getitems_tags.json")),
            TAG_LIST => Resp::raw_json(fixture("steam_taglist_en.json")),
            _ => Resp::status(404),
        }
    })
}

/// Four games in the sideload library and the overlay (only Portal 2 on
/// Steam), IGDB keys, Steam tags off, nothing fetched yet.
fn make_root(dir: &Path, base: &str) -> DataPaths {
    let paths = DataPaths::at(dir);
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    let entry =
        |app: &str, title: &str| json!({"runner": "sideload", "app_name": app, "title": title, "is_installed": false, "install": {}});
    let mut portal = entry("steam_620", "Portal 2");
    portal["art_square"] = json!(format!("{base}/img/portal.png"));
    let mut plain = entry("plain", "Plain Game");
    plain["art_square"] = json!(format!("{base}/img/plain.png"));
    w(paths.sideload_library(), json!({"games": [portal, plain, entry("g3", "Third Game"), entry("g4", "Fourth Game")]}));
    let overlay = |app: &str, title: &str, steam: Option<&str>| json!({"appName": app, "runner": "sideload", "title": title, "steamAppId": steam, "playniteId": "", "source": "manual", "launchKind": "executable"});
    w(
        paths.local_meta(),
        json!({"games": {
            "steam_620": overlay("steam_620", "Portal 2", Some("620")),
            "plain": overlay("plain", "Plain Game", None),
            "g3": overlay("g3", "Third Game", None),
            "g4": overlay("g4", "Fourth Game", None),
        }}),
    );
    w(
        paths.collection_settings(),
        json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret", "downloadImages": false,
                            "autoFillMissing": true, "steamTags": false, "fieldPriority": {}}}),
    );
    paths
}

fn item(app: &str, title: &str, steam: Option<&str>) -> BulkItem {
    BulkItem { app_name: app.into(), runner: "sideload".into(), title: title.into(), steam_app_id: steam.map(Into::into) }
}

fn portal() -> BulkItem {
    item("steam_620", "Portal 2", Some("620"))
}

fn plain() -> BulkItem {
    item("plain", "Plain Game", None)
}

fn ghost() -> BulkItem {
    item("ghost", "Ghost Game", None)
}

/// Portal 2 as a user left it: a description typed by hand, genres from
/// Steam, no release date.
fn mixed_portal() -> CollectionGameMetadata {
    let mut md: CollectionGameMetadata = serde_json::from_value(json!({"appName": "steam_620", "runner": "sideload"})).unwrap();
    md.title = Some("Portal 2".into());
    md.steam_app_id = Some("620".into());
    md.description = Some("hand text".into());
    md.field_sources.insert("description".into(), "manual".into());
    md.genres = vec!["Old".into()];
    md.field_sources.insert("genres".into(), "steam".into());
    md.fetched_at = Some("2020-01-01T00:00:00.000Z".into());
    md
}

fn put(paths: &DataPaths, md: &CollectionGameMetadata) {
    json_store::update::<MetadataFile, _>(&paths.metadata(), |f| {
        f.games.insert(format!("{}:{}", md.runner, md.app_name), md.clone());
        Ok(())
    })
    .unwrap();
}

fn get(svc: &MetadataService, app: &str) -> Option<CollectionGameMetadata> {
    svc.get_game_metadata("sideload", app).unwrap()
}

fn games_json(paths: &DataPaths) -> Value {
    json_store::read_value(&paths.metadata()).unwrap()["games"].clone()
}

fn changed(r: &GameResult) -> Vec<&str> {
    r.changes.iter().map(|c| c.field.as_str()).collect()
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: Arc<MetadataService>,
}

fn setup_with(delay: Duration) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let server = mock(delay);
    let paths = make_root(dir.path(), &server.base);
    let svc = Arc::new(MetadataService::with_endpoints(paths.clone(), server.endpoints()));
    Setup { _dir: dir, paths, server, svc }
}

fn setup() -> Setup {
    setup_with(Duration::ZERO)
}

fn run(svc: &MetadataService, args: BulkArgs) -> BulkProgress {
    svc.run_bulk_metadata(&args, &|_| {})
}

#[test]
fn each_mode_on_empty_source_filled_and_hand_edited_fields() {
    let s = setup();
    let mode = |m: UpdateMode| {
        put(&s.paths, &mixed_portal());
        let done = run(&s.svc, BulkArgs::update(vec![portal()], m, None, None));
        assert_eq!(done.results.len(), 1);
        (done.results[0].clone(), get(&s.svc, "steam_620").unwrap())
    };

    let (r, md) = mode(UpdateMode::Fill);
    assert_eq!(r.outcome, GameOutcome::Updated);
    assert_eq!((md.description.as_deref(), md.field_sources["description"].as_str()), (Some("hand text"), "manual"));
    assert_eq!((md.genres.as_slice(), md.field_sources["genres"].as_str()), (["Old".to_string()].as_slice(), "steam"));
    assert_eq!(md.field_sources["releaseDate"], "igdb");
    let fields = changed(&r);
    assert!(fields.contains(&"releaseDate") && !fields.contains(&"description") && !fields.contains(&"genres"), "{fields:?}");
    assert!(r.changes.iter().all(|c| !c.source.is_empty()));

    let (r, md) = mode(UpdateMode::Refresh);
    assert_eq!(md.description.as_deref(), Some("hand text"));
    assert_ne!(md.genres, ["Old"]);
    assert_eq!(md.field_sources["genres"], "igdb");
    assert!(changed(&r).contains(&"genres") && !changed(&r).contains(&"description"));

    let (r, md) = mode(UpdateMode::Replace);
    assert_ne!(md.description.as_deref(), Some("hand text"));
    assert_eq!(md.field_sources["description"], "igdb");
    assert!(changed(&r).contains(&"description"));
    let change = r.changes.iter().find(|c| c.field == "description").unwrap();
    assert_eq!(change.source, "igdb");
}

#[test]
fn fields_not_chosen_stay_with_their_source() {
    let s = setup();
    put(&s.paths, &mixed_portal());
    let fields = vec!["description".to_string(), "cover".to_string()];
    let done = run(&s.svc, BulkArgs::update(vec![portal()], UpdateMode::Replace, Some(fields), None));
    assert_eq!(changed(&done.results[0]), ["description", "cover"]);
    let md = get(&s.svc, "steam_620").unwrap();
    assert_eq!(md.field_sources["description"], "igdb");
    assert_eq!((md.genres.as_slice(), md.field_sources["genres"].as_str()), (["Old".to_string()].as_slice(), "steam"));
    assert!(md.release_date.is_none() && !md.field_sources.contains_key("releaseDate"));
    assert!(md.themes.is_empty() && md.age_ratings.is_empty());
    assert!(!md.extra.contains_key("ageRatingsCheckedAt"), "age ratings were not asked");

    // "Fill" with nothing missing in the chosen fields asks no source.
    let before = s.server.log.lock().len();
    let done = run(&s.svc, BulkArgs::update(vec![portal()], UpdateMode::Fill, Some(vec!["description".into()]), None));
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::NothingMissing));
    assert_eq!(s.server.log.lock().len(), before);
}

#[test]
fn only_the_chosen_sources_are_asked() {
    let s = setup();
    put(&s.paths, &mixed_portal());
    let asked = |srv: &Server, prefix: &str| srv.count(|r| r.path().starts_with(prefix));

    let done = run(&s.svc, BulkArgs::update(vec![portal(), plain()], UpdateMode::Replace, None, Some(vec!["steam".into()])));
    let md = get(&s.svc, "steam_620").unwrap();
    assert_eq!(md.field_sources["description"], "steam");
    assert!(done.results[0].changes.iter().all(|c| c.source == "steam"));
    assert_eq!(asked(&s.server, "/igdb/") + asked(&s.server, "/twitch/") + asked(&s.server, "/lutris/"), 0);
    // Steam is not for a game without a Steam id: no source, nothing written.
    assert_eq!(done.results[1].outcome, GameOutcome::Skipped(SkipReason::NoSource));
    assert!(get(&s.svc, "plain").is_none(), "a narrowed job does not mark the game as failed");

    put(&s.paths, &mixed_portal());
    let steam = asked(&s.server, "/steam/");
    let done = run(&s.svc, BulkArgs::only_source(vec![portal()], UpdateMode::Refresh, "lutris"));
    assert!(asked(&s.server, "/lutris/") > 0);
    assert_eq!(asked(&s.server, "/steam/") + asked(&s.server, "/igdb/"), steam);
    let r = &done.results[0];
    assert!(r.changes.iter().all(|c| c.source == "lutris"), "{:?}", r.changes);
    assert_eq!(get(&s.svc, "steam_620").unwrap().description.as_deref(), Some("hand text"), "refresh keeps hand edits");
}

#[test]
fn every_game_gets_its_outcome() {
    let s = setup();
    let done = run(&s.svc, BulkArgs::fill_missing(vec![portal(), ghost()]));
    assert_eq!(done.kind, JobKind::FillMissing);
    assert_eq!(done.results.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Portal 2", "Ghost Game"]);
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    assert_eq!(done.results[0].key, "sideload:steam_620");
    assert_eq!(done.results[1].outcome, GameOutcome::NotFound);
    // A full job marks the game as failed, like a fetch of it.
    assert!(get(&s.svc, "ghost").unwrap().fetch_error.is_some());
    // The older counters and error lines are kept.
    assert_eq!((done.updated, done.failed, done.processed, done.percent), (1, 1, 2, 100));
    assert_eq!(done.errors, ["Ghost Game: No metadata sources returned data"]);
    let tally = done.tally();
    assert_eq!((tally.updated, tally.not_found), (1, 1));
    assert_eq!(tally.fields, done.results[0].changes.len());
    assert!(done.started_at_ms > 0 && done.finished_at_ms >= done.started_at_ms);
    assert!(done.can_undo);

    // Offline: nothing is saved.
    let dir = tempfile::tempdir().unwrap();
    let dead = dead_endpoints();
    let svc = MetadataService::with_endpoints(make_root(dir.path(), "http://127.0.0.1:9"), dead);
    let done = run(&svc, BulkArgs::fill_missing(vec![plain()]));
    assert_eq!(done.results[0].outcome, GameOutcome::Offline);
    assert!(get(&svc, "plain").is_none());
    assert_eq!(done.failed, 1);
    assert!(!done.can_undo, "nothing was written");

    // A record that cannot be read fails the game with the reason.
    fs::write(s.paths.metadata(), "{ not json").unwrap();
    let done = run(&s.svc, BulkArgs::fill_missing(vec![portal()]));
    match &done.results[0].outcome {
        GameOutcome::Failed(reason) => assert!(reason.contains("decoding"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

/// Endpoints of a host that drops every connection, like no network.
fn dead_endpoints() -> Endpoints {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            drop(stream);
        }
    });
    Endpoints {
        twitch_token: format!("{base}/twitch/oauth2/token"),
        igdb: format!("{base}/igdb/v4"),
        steam_store: format!("{base}/steam"),
        lutris: format!("{base}/lutris"),
        ..Endpoints::default()
    }
}

#[test]
fn undo_after_fill_puts_every_record_back_as_one() {
    let s = setup();
    put(&s.paths, &mixed_portal());
    let before = games_json(&s.paths);
    let done = run(&s.svc, BulkArgs::fill_missing(vec![portal(), plain(), ghost()]));
    assert!(done.can_undo && s.svc.can_undo_bulk());
    assert!(get(&s.svc, "plain").is_some() && get(&s.svc, "ghost").is_some());

    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!((report.job, report.games, report.kept), (done.job, 3, 0));
    assert_eq!(games_json(&s.paths), before, "Portal 2 as it was, the two new records gone");
    assert!(!s.svc.can_undo_bulk() && !s.svc.bulk_progress().can_undo);

    // Twice in a row: nothing more to undo, nothing written.
    let text = fs::read_to_string(s.paths.metadata()).unwrap();
    assert!(s.svc.undo_bulk_metadata().unwrap().is_none());
    assert_eq!(fs::read_to_string(s.paths.metadata()).unwrap(), text);
}

#[test]
fn undo_after_replace_and_after_a_later_hand_edit() {
    let s = setup();
    put(&s.paths, &mixed_portal());
    run(&s.svc, BulkArgs::update(vec![portal()], UpdateMode::Replace, None, None));
    assert_ne!(get(&s.svc, "steam_620").unwrap().description.as_deref(), Some("hand text"));
    s.svc.undo_bulk_metadata().unwrap().unwrap();
    let md = get(&s.svc, "steam_620").unwrap();
    assert_eq!((md.description.as_deref(), md.field_sources["description"].as_str()), (Some("hand text"), "manual"));
    assert_eq!(serde_json::to_value(&md).unwrap(), serde_json::to_value(mixed_portal()).unwrap());

    // The user edits after the job: their edit stays, the rest goes back.
    run(&s.svc, BulkArgs::update(vec![portal(), plain()], UpdateMode::Replace, None, None));
    let job_platforms = get(&s.svc, "steam_620").unwrap().platforms;
    assert!(!job_platforms.is_empty());
    s.svc
        .update_game_details(&DetailsPatch {
            app_name: "steam_620".into(),
            runner: "sideload".into(),
            genres: Some(vec!["Mine".into()]),
            notes: Some("later".into()),
            ..Default::default()
        })
        .unwrap();
    s.svc
        .update_game_details(&DetailsPatch {
            app_name: "plain".into(),
            runner: "sideload".into(),
            notes: Some("n".into()),
            ..Default::default()
        })
        .unwrap();
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert!(report.kept >= 1);
    let md = get(&s.svc, "steam_620").unwrap();
    assert_eq!((md.genres.as_slice(), md.field_sources["genres"].as_str()), (["Mine".to_string()].as_slice(), "manual"));
    assert_eq!(md.notes.as_deref(), Some("later"));
    assert_eq!(md.description.as_deref(), Some("hand text"));
    assert!(md.platforms.is_empty() && !md.field_sources.contains_key("platforms"));
    // A record the job created and the user then wrote to keeps only the user's part.
    let plain_md = get(&s.svc, "plain").unwrap();
    assert_eq!(plain_md.notes.as_deref(), Some("n"));
    assert!(plain_md.description.is_none() && plain_md.fetched_at.is_none() && plain_md.field_sources.is_empty());
}

#[test]
fn undo_is_offered_for_the_last_job_only() {
    let s = setup();
    run(&s.svc, BulkArgs::fill_missing(vec![plain()]));
    run(&s.svc, BulkArgs::fill_missing(vec![ghost()]));
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!(report.games, 1);
    assert!(get(&s.svc, "plain").is_some(), "the job before is not undone");
    assert!(get(&s.svc, "ghost").is_none());
}

#[test]
fn undo_puts_back_replaced_pictures_and_removes_new_ones() {
    let s = setup();
    let art = s.paths.metadata_art_dir();
    // Portal 2 has a copied cover the job will replace; Plain Game has none.
    fs::create_dir_all(art.join("sideload_steam_620")).unwrap();
    let old = b"old picture, not a real png at all".to_vec();
    fs::write(art.join("sideload_steam_620/cover.png"), &old).unwrap();
    let mut md = mixed_portal();
    md.cover_url = Some("localart://metadata-art/sideload_steam_620/cover.png".into());
    md.field_sources.insert("cover".into(), "store".into());
    put(&s.paths, &md);
    let other = art.join("sideload_steam_620/notes.txt");
    fs::write(&other, b"not the job's").unwrap();

    let args = BulkArgs {
        download_images: Some(true),
        ..BulkArgs::update(vec![portal(), plain()], UpdateMode::Replace, Some(vec!["cover".into()]), Some(vec!["store".into()]))
    };
    let done = run(&s.svc, args);
    assert_eq!(fs::read(art.join("sideload_steam_620/cover.png")).unwrap(), png());
    assert_eq!(get(&s.svc, "plain").unwrap().cover_url.as_deref(), Some("localart://metadata-art/sideload_plain/cover.png"));
    assert_eq!(changed(&done.results[1]), ["cover"]);

    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!((report.files_restored, report.files_removed), (1, 1));
    assert_eq!(fs::read(art.join("sideload_steam_620/cover.png")).unwrap(), old);
    assert!(other.exists(), "a file the job did not create stays");
    assert!(!art.join("sideload_plain").exists());
    assert!(!s.paths.metadata_undo_dir().exists());
}

/// Records every progress a job's callback gets.
fn recorder() -> (Arc<Mutex<Vec<BulkProgress>>>, impl Fn(&BulkProgress) + Send + Sync + 'static) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = events.clone();
    (events, move |p: &BulkProgress| e.lock().push(p.clone()))
}

fn wait_idle(svc: &MetadataService) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while (svc.bulk_progress().is_running() || !svc.bulk_progress().queued.is_empty()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
}

fn last_done(events: &Mutex<Vec<BulkProgress>>) -> Option<BulkProgress> {
    events.lock().iter().rev().find(|p| p.state == BulkState::Done).cloned()
}

#[test]
fn a_job_started_while_one_runs_waits_its_turn() {
    let s = setup_with(Duration::from_millis(60));
    let (a_events, a) = recorder();
    let (b_events, b) = recorder();
    let first = s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![plain(), item("g3", "Third Game", None)]), a);
    assert_eq!((first.state, first.job), (BulkState::Running, 1));
    let queued = s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g4", "Fourth Game", None)]), b);
    assert_eq!(queued.job, 1, "the running job's progress");
    assert_eq!(queued.queued, [QueuedJob { id: 2, kind: JobKind::FillMissing, mode: Some(UpdateMode::Fill), games: 1 }]);
    wait_idle(&s.svc);

    let a_done = last_done(&a_events).unwrap();
    let b_done = last_done(&b_events).unwrap();
    assert_eq!((a_done.job, a_done.results.len()), (1, 2));
    assert_eq!((b_done.job, b_done.results.len()), (2, 1));
    assert!(b_done.started_at_ms >= a_done.finished_at_ms, "one after the other");
    assert!(a_events.lock().iter().all(|p| p.job == 1));
    assert!(b_done.can_undo && !a_done.can_undo, "the next job drops the undo of the one before");
}

#[test]
fn stop_ends_the_running_job_and_clear_drops_the_waiting_ones() {
    let s = setup_with(Duration::from_millis(60));
    let games = vec![plain(), item("g3", "Third Game", None), item("g4", "Fourth Game", None)];
    let (a_events, a) = recorder();
    let (b_events, b) = recorder();
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(games.clone()), a);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![ghost()]), b);
    // Stop as soon as the first game runs.
    while s.svc.bulk_progress().current_title.is_none() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let stopping = s.svc.cancel_bulk_metadata(&|_| {});
    assert_eq!(stopping.state, BulkState::Cancelling);
    wait_idle(&s.svc);
    let a_done = last_done(&a_events).unwrap();
    assert_eq!(a_done.results.len(), 3, "every game is listed");
    let stopped = a_done.results.iter().filter(|r| r.outcome == GameOutcome::Skipped(SkipReason::Stopped)).count();
    assert!(stopped >= 1, "{:?}", a_done.results);
    assert_eq!(a_done.tally().stopped, stopped);
    assert_eq!(a_done.processed, 3 - stopped);
    assert!(last_done(&b_events).is_some(), "the waiting job still ran");

    // Clear: the waiting jobs never run.
    let (c_events, c) = recorder();
    let (d_events, d) = recorder();
    let (e_events, e) = recorder();
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g3", "Third Game", None)]), c);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g4", "Fourth Game", None)]), d);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![plain()]), e);
    assert_eq!(s.svc.bulk_progress().queued.len(), 2);
    let cleared = s.svc.clear_bulk_queue(&|_| {});
    assert!(cleared.queued.is_empty());
    wait_idle(&s.svc);
    assert!(last_done(&c_events).is_some());
    assert!(last_done(&d_events).is_none() && last_done(&e_events).is_none());
}

#[test]
fn the_idle_fills_give_way_to_a_job() {
    let s = setup_with(Duration::from_millis(80));
    // A job runs: the fills do nothing and return at once.
    let (events, cb) = recorder();
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![plain(), item("g3", "Third Game", None)]), cb);
    let t = Instant::now();
    assert_eq!(s.svc.refresh_missing_collection_metadata().unwrap(), 0);
    assert_eq!(s.svc.fill_missing_age_ratings_while(&|| false).unwrap(), 0);
    assert!(!s.svc.refresh_steam_tags_while(&|| false).unwrap().changed());
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
    wait_idle(&s.svc);
    assert_eq!(last_done(&events).unwrap().results.len(), 2);

    // The fill runs (four games miss metadata): a job stops it after its
    // current game and runs every game of its own.
    let s = setup_with(Duration::from_millis(80));
    let svc = s.svc.clone();
    let fill = std::thread::spawn(move || svc.refresh_missing_collection_metadata().unwrap());
    while s.server.log.lock().is_empty() {
        std::thread::sleep(Duration::from_millis(2));
    }
    let (events, cb) = recorder();
    let asked = Instant::now();
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![ghost()]), cb);
    let filled = fill.join().unwrap();
    assert!(filled <= 1, "the fill stopped for the job: {filled}");
    wait_idle(&s.svc);
    let done = last_done(&events).unwrap();
    assert_eq!(done.results[0].outcome, GameOutcome::NotFound);
    // The job waited for one game of the fill, not for the fill.
    let first_game = events.lock().iter().find(|p| p.current_title.is_some()).map(|p| p.started_at_ms).unwrap();
    assert!(first_game > 0 && asked.elapsed() < Duration::from_secs(5), "{:?}", asked.elapsed());
}

#[test]
fn steam_tags_are_part_of_fill_what_is_missing() {
    let s = setup();
    json_store::update_value(&s.paths.collection_settings(), |v| {
        v["metadata"]["steamTags"] = json!(true);
        Ok(())
    })
    .unwrap();
    let counts = s.svc.bulk_counts(&[portal(), plain()]).unwrap();
    assert_eq!(counts.missing_by_field["steamTags"], 1);

    let done = run(&s.svc, BulkArgs::fill_missing(vec![portal(), plain()]));
    let portal_result = &done.results[0];
    let tag = portal_result.changes.iter().find(|c| c.field == "steamTags").unwrap();
    assert_eq!(tag.source, "steamtags");
    assert_eq!(s.server.count(|r| r.path() == ITEMS), 1);
    assert!(!changed(&done.results[1]).contains(&"steamTags"), "not on Steam");

    // Filled: a second Fill asks Steam nothing.
    let done = run(&s.svc, BulkArgs::update(vec![portal()], UpdateMode::Fill, Some(vec!["steamTags".into()]), None));
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::NothingMissing));
    assert_eq!(s.server.count(|r| r.path() == ITEMS), 1);
    // A refresh asks again; the same tags are no change.
    let done = run(&s.svc, BulkArgs::only_source(vec![portal(), plain()], UpdateMode::Refresh, "steamtags"));
    assert_eq!(s.server.count(|r| r.path() == ITEMS), 2);
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::NothingNew));
    assert_eq!(done.results[1].outcome, GameOutcome::Skipped(SkipReason::NoSource));
}

#[test]
fn the_older_dialogs_options_still_work() {
    let s = setup();
    // An older caller's JSON: only the three switches.
    let args: BulkArgs = serde_json::from_value(json!({"games": [
        {"appName": "steam_620", "runner": "sideload", "title": "Portal 2", "steamAppId": "620"},
        {"appName": "plain", "runner": "sideload", "title": "Plain Game"}
    ], "overwriteExisting": false, "preserveManual": true}))
    .unwrap();
    assert!(args.mode.is_none() && args.fields.is_none() && args.kind == JobKind::Custom);
    put(&s.paths, &mixed_portal());
    let done = run(&s.svc, args);
    assert_eq!(done.results[0].outcome, GameOutcome::Skipped(SkipReason::AlreadyFetched));
    assert_eq!((done.updated, done.skipped), (1, 1));

    // Skip games that already have metadata / keep fields edited by hand.
    let done = run(&s.svc, BulkArgs::legacy(vec![portal()], true, true, false));
    assert_eq!(done.skipped, 1);
    let done = run(&s.svc, BulkArgs::legacy(vec![portal()], false, true, false));
    assert_eq!(done.updated, 1);
    assert_eq!(get(&s.svc, "steam_620").unwrap().description.as_deref(), Some("hand text"));
    run(&s.svc, BulkArgs::legacy(vec![portal()], false, false, false));
    assert_ne!(get(&s.svc, "steam_620").unwrap().description.as_deref(), Some("hand text"));
}

#[test]
fn counts_for_two_thousand_games_read_each_file_once() {
    let s = setup();
    let total = 2000;
    let mut games = Vec::new();
    let mut file = serde_json::Map::new();
    for i in 0..total {
        let app = format!("game{i}");
        games.push(item(&app, &format!("Game {i}"), None));
        let mut md = json!({"appName": app, "runner": "sideload", "title": format!("Game {i}"),
                            "developers": [], "publishers": [], "genres": [], "themes": [], "gameModes": [],
                            "platforms": [], "features": [], "websites": [], "fieldSources": {}});
        match i % 4 {
            0 => continue, // no record
            1 => md["fetchError"] = json!("No metadata sources returned data"),
            2 => {
                // Everything a fill could look at, but the hero emptied by hand.
                md["description"] = json!("d");
                md["releaseDate"] = json!("2001");
                md["series"] = json!("s");
                md["criticScore"] = json!(80);
                md["coverUrl"] = json!("https://c");
                md["ageRatings"] = json!([{"system": "esrb", "rating": "T"}]);
                for list in ["developers", "publishers", "genres", "themes", "gameModes", "platforms", "features"] {
                    md[list] = json!(["x"]);
                }
                md["fieldSources"] = json!({"hero": "manual"});
            }
            _ => md["description"] = json!("only a description"),
        }
        file.insert(format!("sideload:{app}"), md);
    }
    json_store::write(&s.paths.metadata(), &json!({"games": file})).unwrap();

    let t = Instant::now();
    let counts = s.svc.bulk_counts(&games).unwrap();
    let attention = s.svc.needs_attention(&games).unwrap();
    let took = t.elapsed();
    // A read of the file per game takes minutes here.
    assert!(took < Duration::from_secs(3), "{took:?}");

    let quarter = total / 4;
    assert_eq!(counts.games, total);
    assert_eq!(counts.no_details, 2 * quarter, "no record, or a failed one with nothing in it");
    assert_eq!(counts.failed_last_time, quarter);
    // Needing attention also counts a missing description or cover: the
    // records with only a description have no cover.
    assert_eq!(attention.len(), 3 * quarter);
    assert!(attention.iter().all(|g| g.app_name.trim_start_matches("game").parse::<usize>().unwrap() % 4 != 2));
    assert_eq!((counts.without_match, counts.no_cover, counts.no_description, counts.to_review), (2 * quarter, quarter, 0, 0));
    assert_eq!(counts.needs_attention, 3 * quarter);
    assert_eq!(counts.games_missing, 3 * quarter);
    let fields = heronite_data::settings::METADATA_FIELDS.len();
    assert_eq!(counts.fields_missing, 2 * quarter * fields + quarter * (fields - 1));
    assert_eq!(counts.missing_by_field["hero"], 3 * quarter);
    assert_eq!(counts.missing_by_field["description"], 2 * quarter);
    assert_eq!(counts.reach["igdb"], total);
    assert_eq!(counts.reach["steam"], 0, "no game has a Steam id");
}

#[test]
fn fill_what_is_missing_does_what_the_counts_say() {
    let s = setup();
    s.svc.add_source(fallback()).unwrap();
    put(&s.paths, &mixed_portal());
    let games = vec![portal(), plain()];
    let counts = s.svc.bulk_counts(&games).unwrap();
    assert_eq!(counts.games_missing, 2);
    let done = run(&s.svc, BulkArgs::fill_missing(games.clone()));
    let looked_at = done.results.iter().filter(|r| r.outcome != GameOutcome::NoChange(NoChangeReason::NothingMissing)).count();
    assert_eq!(looked_at, counts.games_missing);
    let after = s.svc.bulk_counts(&games).unwrap();
    assert!(after.fields_missing < counts.fields_missing);
    assert_eq!(
        s.svc.bulk_sources(&s.svc.metadata_settings()).iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["igdb", "steam", "store", "lutris", "launchbox", FALLBACK],
        "a fallback source is offered too"
    );
    assert_eq!(counts.needs_download, ["launchbox"], "listed, but it reaches no game before its download");
    assert_eq!(counts.reach["launchbox"], 0);
}

#[test]
fn a_job_reaches_progress_listeners_in_order() {
    let s = setup();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![portal(), plain()]), &move |p| {
        c.fetch_add(1, Ordering::SeqCst);
        assert!(p.results.len() <= p.total);
        if p.state == BulkState::Running {
            assert_eq!(p.left(), p.total - p.results.len());
        }
    });
    assert!(calls.load(Ordering::SeqCst) >= 3);
    assert_eq!(done.left(), 0);
    assert_eq!(done.result_for("sideload:plain").map(|r| r.title.as_str()), Some("Plain Game"));
}

#[test]
fn fill_asks_only_the_sources_of_the_missing_fields() {
    let s = setup();
    // Everything but the themes, which only IGDB (and Playnite) give.
    let mut md = mixed_portal();
    md.release_date = Some("2011".into());
    md.series = Some("Portal".into());
    md.critic_score = Some(heronite_data::num::JsNum(95.0));
    md.cover_url = Some("https://c".into());
    md.hero_url = Some("https://h".into());
    md.age_ratings = serde_json::from_value(json!([{"system": "esrb", "rating": "E10"}])).unwrap();
    md.developers = vec!["Valve".into()];
    md.publishers = vec!["Valve".into()];
    md.game_modes = vec!["Single".into()];
    md.platforms = vec!["PC".into()];
    md.features = vec!["Co-op".into()];
    put(&s.paths, &md);

    let done = run(&s.svc, BulkArgs::fill_missing(vec![portal()]));
    assert_eq!(changed(&done.results[0]), ["themes"]);
    assert!(s.server.count(|r| r.path().starts_with("/igdb/")) > 0);
    assert_eq!(s.server.count(|r| r.path().starts_with("/steam/") || r.path().starts_with("/lutris/")), 0);
    let after = get(&s.svc, "steam_620").unwrap();
    assert_eq!(after.cover_url.as_deref(), Some("https://c"));
    assert_eq!(after.description.as_deref(), Some("hand text"));
}

/// Writes `bytes` over `path` as the app does: a new file renamed over it.
fn replace_file(path: &Path, bytes: &[u8]) {
    let part = path.with_extension("user.part");
    fs::write(&part, bytes).unwrap();
    fs::rename(&part, path).unwrap();
}

/// Portal 2 with a cover copied to disk (from the store) and a hero picked
/// by hand, both in its `metadata-art` folder.
fn portal_with_pictures(s: &Setup) -> std::path::PathBuf {
    let art = s.paths.metadata_art_dir().join("sideload_steam_620");
    fs::create_dir_all(&art).unwrap();
    fs::write(art.join("cover.png"), b"old cover, not a real png").unwrap();
    fs::write(art.join("hero.png"), b"old hero, not a real png").unwrap();
    let mut md = mixed_portal();
    md.cover_url = Some("localart://metadata-art/sideload_steam_620/cover.png".into());
    md.field_sources.insert("cover".into(), "store".into());
    md.hero_url = Some("localart://metadata-art/sideload_steam_620/hero.png".into());
    md.field_sources.insert("hero".into(), "manual".into());
    put(&s.paths, &md);
    art
}

/// The store's cover of the games, copied to disk.
fn store_cover(games: Vec<BulkItem>, mode: UpdateMode) -> BulkArgs {
    BulkArgs { download_images: Some(true), ..BulkArgs::update(games, mode, Some(vec!["cover".into()]), Some(vec!["store".into()])) }
}

#[test]
fn undo_leaves_the_pictures_written_after_the_job() {
    let s = setup();
    let art = portal_with_pictures(&s);
    run(&s.svc, store_cover(vec![portal()], UpdateMode::Refresh));
    assert_eq!(fs::read(art.join("cover.png")).unwrap(), png());
    // Then the game's own dialog saves another cover and another hero: the
    // record still points at the same files.
    replace_file(&art.join("cover.png"), b"the user's cover");
    replace_file(&art.join("hero.png"), b"the user's hero");

    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!(report.files_restored, 0);
    assert_eq!(fs::read(art.join("cover.png")).unwrap(), b"the user's cover");
    assert_eq!(fs::read(art.join("hero.png")).unwrap(), b"the user's hero", "the job never wrote the hero");
}

#[test]
fn a_picture_replaced_under_the_same_name_is_a_change() {
    let s = setup();
    let art = portal_with_pictures(&s);
    let done = run(&s.svc, store_cover(vec![portal()], UpdateMode::Replace));
    assert_eq!(fs::read(art.join("cover.png")).unwrap(), png());
    assert_eq!(get(&s.svc, "steam_620").unwrap().cover_url.as_deref(), Some("localart://metadata-art/sideload_steam_620/cover.png"));
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    assert_eq!(changed(&done.results[0]), ["cover"]);
    assert_eq!(done.results[0].changes[0].source, "store");
    assert_eq!(done.tally().updated, 1);
    assert!(done.can_undo);

    // The same picture again: nothing new.
    let done = run(&s.svc, store_cover(vec![portal()], UpdateMode::Replace));
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::NothingNew));
}

#[test]
fn a_game_no_source_is_asked_for_is_not_marked_as_not_found() {
    let off = |s: &Setup, keep: &[&str]| {
        for d in sources::builtin_descriptors().iter().filter(|d| !keep.contains(&d.id.as_str())) {
            s.svc.set_source_enabled(&d.id, false).ok();
        }
    };
    let s = setup();
    off(&s, &[]);
    assert_eq!(s.svc.refresh_missing_collection_metadata().unwrap(), 0);
    let done = run(&s.svc, BulkArgs::legacy(vec![plain()], false, true, false));
    assert_eq!(done.results[0].outcome, GameOutcome::Skipped(SkipReason::NoSource));
    let none = ["steam_620", "plain", "g3", "g4"].iter().all(|app| get(&s.svc, app).is_none());
    assert!(none, "nothing written");
    assert!(!done.can_undo);
    assert!(s.server.log.lock().is_empty());

    // Only Steam on: the three games not on Steam are left for later and
    // do not take the place of the one it is for.
    let s = setup();
    off(&s, &["steam"]);
    assert_eq!(s.svc.refresh_missing_collection_metadata().unwrap(), 1);
    assert!(get(&s.svc, "steam_620").is_some_and(|md| md.fetch_error.is_none()));
    assert!(get(&s.svc, "plain").is_none() && get(&s.svc, "g3").is_none());
}

#[test]
fn a_panic_in_a_game_ends_the_job_with_what_it_did() {
    let s = setup();
    let panicked = AtomicUsize::new(0);
    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![portal(), plain()]), &|p| {
        if p.current_title.as_deref() == Some("Plain Game") && panicked.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("a bug while updating Plain Game");
        }
    });
    assert_eq!(done.state, BulkState::Done);
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    assert_eq!(done.results[1].outcome, GameOutcome::Skipped(SkipReason::Stopped));
    assert!(done.can_undo && !s.svc.bulk_progress().is_running());
    s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert!(get(&s.svc, "steam_620").is_none(), "what it wrote is undone");
}

#[test]
fn a_panic_outside_the_games_ends_the_job_and_the_waiting_ones() {
    let s = setup_with(Duration::from_millis(60));
    let (a_events, a) = recorder();
    let (b_events, b_record) = recorder();
    let (c_events, c) = recorder();
    let once = AtomicUsize::new(0);
    // Job 2 panics as it starts (before its first game).
    let b = move |p: &BulkProgress| {
        b_record(p);
        if p.job == 2 && p.state == BulkState::Running && once.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("a bug as job 2 starts");
        }
    };
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![plain()]), a);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g3", "Third Game", None)]), b);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g4", "Fourth Game", None), ghost()]), c);
    wait_idle(&s.svc);

    assert_eq!(last_done(&a_events).unwrap().job, 1);
    assert_eq!(last_done(&b_events).map(|p| p.job), Some(2), "the job that panicked is told it ended");
    let c_done = last_done(&c_events).expect("the waiting job is told it will not run");
    assert_eq!((c_done.job, c_done.results.len()), (3, 2));
    assert!(c_done.results.iter().all(|r| r.outcome == GameOutcome::Skipped(SkipReason::Stopped)));
    assert!(!s.svc.bulk_progress().is_running());
    // Later jobs still run.
    assert_eq!(run(&s.svc, BulkArgs::fill_missing(vec![ghost()])).results[0].outcome, GameOutcome::NotFound);
}

#[test]
fn a_job_started_as_the_one_before_ends_gets_its_own_id() {
    let s = setup_with(Duration::from_millis(60));
    let started: Arc<Mutex<Option<(u64, BulkProgress)>>> = Arc::default();
    let (c_events, c) = recorder();
    let c = Arc::new(Mutex::new(Some(c)));
    let (svc, slot) = (s.svc.clone(), started.clone());
    // Job 3 starts from job 1's last progress: job 2 waits to start then.
    let a = move |p: &BulkProgress| {
        if p.job == 1 && p.state == BulkState::Done {
            if let Some(c) = c.lock().take() {
                *slot.lock() = Some(svc.start_bulk_job(BulkArgs::fill_missing(vec![ghost()]), c));
            }
        }
    };
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![plain()]), a);
    s.svc.start_bulk_metadata(BulkArgs::fill_missing(vec![item("g3", "Third Game", None)]), |_| {});
    wait_idle(&s.svc);

    let (id, now) = started.lock().clone().unwrap();
    assert_eq!(id, 3);
    // What the progress says then: the job before, done.
    assert_eq!((now.job, now.state), (1, BulkState::Done));
    let c_done = last_done(&c_events).unwrap();
    assert_eq!((c_done.job, c_done.results[0].title.as_str()), (3, "Ghost Game"));
}
