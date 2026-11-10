//! Covers per family against a local mock of the services: SteamGridDB and
//! Steam by family, what a fetch fills (only the kept families), the
//! "download the <family> covers" job with its locks and Undo, the job a
//! view that starts using a family queues, and removing a game.

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use common::{fixture, png, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::DataPaths;
use heronite_metadata::covers::{find_steamgrid_game, steamgrid_pictures};
use heronite_metadata::*;
use indexmap::IndexMap;
use serde_json::{json, Value};

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

/// SteamGridDB's answers, with the picture URLs on this server.
fn sgdb(req: &Req, base: &str) -> Resp {
    let path = req.path().trim_start_matches("/sgdb/api/v2");
    let body = match path {
        "/games/steam/620" => fixture("sgdb_games_steam_620.json"),
        p if p.starts_with("/search/autocomplete/Plain") => fixture("sgdb_autocomplete_plain.json"),
        p if p.starts_with("/grids/game/") => {
            let dims = req.query("dimensions").unwrap_or_default();
            if dims.contains("512x512") {
                fixture("sgdb_grids_square.json")
            } else if dims.contains("460x215") {
                fixture("sgdb_grids_wide.json")
            } else {
                fixture("sgdb_grids_tall.json")
            }
        }
        _ => return Resp::raw_json(r#"{"success":false,"errors":["Game not found"]}"#.into()),
    };
    Resp::raw_json(body.replace("https://cdn2.steamgriddb.com", &format!("{base}/img")))
}

fn mock() -> Server {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let b = base.clone();
    let server = Server::start(move |req| match req.path() {
        "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
        "/igdb/v4/games" if req.body.starts_with("search") => Resp::raw_json(fixture("igdb_search_portal2.json")),
        "/igdb/v4/games" => Resp::raw_json(fixture("igdb_game_steam_620.json")),
        "/steam/api/appdetails" => steam_body(req),
        p if p.starts_with("/sgdb/") => sgdb(req, b.get().map_or("", String::as_str)),
        p if p.starts_with("/img/") => Resp::image(png()),
        _ => Resp::status(404),
    });
    base.set(server.base.clone()).unwrap();
    server
}

/// Portal 2 (on Steam) and Plain Game in the sideload library, IGDB keys,
/// a SteamGridDB key, Steam tags off; `keep`: `keepCoverFamilies`.
fn make_root(dir: &Path, keep: &[&str]) -> DataPaths {
    let paths = DataPaths::at(dir.join("data"));
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    let entry =
        |app: &str, title: &str| json!({"runner": "sideload", "app_name": app, "title": title, "is_installed": false, "install": {}});
    w(paths.sideload_library(), json!({"games": [entry("steam_620", "Portal 2"), entry("plain", "Plain Game")]}));
    let overlay = |app: &str, title: &str, steam: Option<&str>| json!({"appName": app, "runner": "sideload", "title": title, "steamAppId": steam, "playniteId": "", "source": "manual", "launchKind": "executable"});
    w(
        paths.local_meta(),
        json!({"games": {"steam_620": overlay("steam_620", "Portal 2", Some("620")), "plain": overlay("plain", "Plain Game", None)}}),
    );
    w(
        paths.collection_settings(),
        json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret", "downloadImages": false, "steamTags": false,
                            "keepCoverFamilies": keep}}),
    );
    w(paths.global_config(), json!({"defaultSettings": {"steamGridDbApiKey": "K"}, "version": "v0"}));
    paths
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: Arc<MetadataService>,
}

fn setup(keep: &[&str]) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let server = mock();
    let paths = make_root(dir.path(), keep);
    let svc = Arc::new(MetadataService::with_endpoints(paths.clone(), server.endpoints()));
    Setup { _dir: dir, paths, server, svc }
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

fn args(game: &BulkItem) -> MetadataApplyArgs {
    MetadataApplyArgs {
        app_name: game.app_name.clone(),
        runner: game.runner.clone(),
        title: game.title.clone(),
        steam_app_id: game.steam_app_id.clone(),
        ..Default::default()
    }
}

fn put(paths: &DataPaths, md: &CollectionGameMetadata) {
    json_store::update::<MetadataFile, _>(&paths.metadata(), |f| {
        f.games.insert(format!("{}:{}", md.runner, md.app_name), md.clone());
        Ok(())
    })
    .unwrap();
}

fn record(app: &str, f: impl FnOnce(&mut CollectionGameMetadata)) -> CollectionGameMetadata {
    let mut md: CollectionGameMetadata = serde_json::from_value(json!({"appName": app, "runner": "sideload"})).unwrap();
    md.description = Some("d".into());
    md.cover_url = Some("https://images.igdb.com/tall.jpg".into());
    md.field_sources.insert("cover".into(), "igdb".into());
    md.fetched_at = Some("2020-01-01T00:00:00.000Z".into());
    f(&mut md);
    md
}

fn get(svc: &MetadataService, app: &str) -> CollectionGameMetadata {
    svc.get_game_metadata("sideload", app).unwrap().unwrap()
}

fn sgdb_requests(server: &Server) -> usize {
    server.count(|r| r.path().starts_with("/sgdb/"))
}

#[test]
fn steamgriddb_and_steam_give_covers_by_family() {
    let s = setup(&[]);
    let http = &s.svc.http;
    let found = find_steamgrid_game(http, "K", Some("620"), "Portal 2").into_option().unwrap();
    assert_eq!((found.id, found.name.as_deref(), found.year, found.matched_by), (2254, Some("Portal 2"), Some(2011), MatchedBy::SteamId));
    let by_title = find_steamgrid_game(http, "K", None, "Plain Game").into_option().unwrap();
    assert_eq!((by_title.id, by_title.matched_by), (5150, MatchedBy::ExactTitle));
    assert!(matches!(find_steamgrid_game(http, "K", None, "Ghost"), Lookup::NotFound));

    let kinds = [ArtKind::Cover, ArtKind::CoverSquare, ArtKind::CoverWide];
    let pictures = steamgrid_pictures(http, "K", 2254, &kinds).into_option().unwrap();
    let urls: IndexMap<ArtKind, String> = pictures.into_iter().collect();
    assert!(urls[&ArtKind::Cover].ends_with("/grid/tall-600.png"));
    assert!(urls[&ArtKind::CoverSquare].ends_with("/grid/square-1024.png"), "the first SteamGridDB ranks");
    assert!(urls[&ArtKind::CoverWide].ends_with("/grid/wide-920.png"));
    // Each family asked by its dimensions.
    let dims: Vec<String> =
        s.server.log.lock().iter().filter(|r| r.path() == "/sgdb/api/v2/grids/game/2254").filter_map(|r| r.query("dimensions")).collect();
    for want in ["600x900,342x482,660x930", "512x512,1024x1024", "460x215,920x430"] {
        assert!(dims.iter().any(|d| d == want), "{want} in {dims:?}");
    }

    // Steam: the store's header as the wide cover, no square one.
    let mut a = args(&portal());
    a.only_fields = Some(vec!["coverWide".into()]);
    let preview = s.svc.preview_game_metadata(&a).unwrap();
    let steam = preview.candidates.iter().find(|c| c.source == "steam").unwrap();
    assert!(steam.metadata.cover_wide_url.as_deref().unwrap().contains("/apps/400/header.jpg"));
    assert!(steam.metadata.cover_square_url.is_none());
    assert_eq!(preview.proposed.field_sources["coverWide"], "steam", "official art first");
    // The per-game dialog's sources stay the details ones.
    assert!(preview.sources.iter().all(|st| st.source != "steamgriddb"));
}

#[test]
fn a_fetch_with_the_defaults_never_asks_steamgriddb() {
    let s = setup(&[]);
    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![portal(), plain()]), &|_| {});
    assert!(done.results.iter().any(|r| r.outcome == GameOutcome::Updated));
    assert_eq!(sgdb_requests(&s.server), 0);
    let md = get(&s.svc, "steam_620");
    assert!(md.cover_square_url.is_none() && md.cover_wide_url.is_none());
    assert!(!md.field_sources.contains_key("coverWide"));
    assert_eq!(md.field_sources["cover"], "igdb", "the tall cover as before");
}

#[test]
fn a_kept_family_is_filled_by_every_fetch() {
    let s = setup(&["square"]);
    s.svc.apply_game_metadata(&args(&portal())).unwrap();
    let md = get(&s.svc, "steam_620");
    assert!(md.cover_square_url.as_deref().unwrap().ends_with("/grid/square-1024.png"));
    assert_eq!(md.field_sources["coverSquare"], "steamgriddb");
    assert_eq!(md.field_sources["cover"], "igdb");
    assert!(md.cover_wide_url.is_none(), "wide is neither in use nor kept");
    // Only the square grids were asked for.
    assert_eq!(s.server.count(|r| r.query("dimensions").is_some_and(|d| d.contains("460x215"))), 0);
    assert_eq!(s.svc.cover_families().kept(), [CoverFamily::Tall, CoverFamily::Square]);
    // The match is kept with the others.
    assert_eq!(heronite_metadata::record::source_match(&md, "steamgriddb").unwrap().id, "2254");
}

#[test]
fn download_square_covers_fills_only_the_games_lacking_them() {
    let s = setup(&[]);
    put(&s.paths, &record("steam_620", |_| {}));
    put(
        &s.paths,
        &record("plain", |m| {
            m.cover_square_url = Some("https://mine/sq.png".into());
            m.field_sources.insert("coverSquare".into(), "steamgriddb".into());
        }),
    );
    let before = s.server.log.lock().len();
    let mut job = BulkArgs::download_covers(vec![portal(), plain()], CoverFamily::Square);
    job.download_images = Some(true);
    assert_eq!(job.kind, JobKind::Covers(CoverFamily::Square));
    let done = s.svc.run_bulk_metadata(&job, &|_| {});
    let r = &done.results[0];
    assert_eq!(r.outcome, GameOutcome::Updated);
    assert_eq!(r.changes, [FieldChange { field: "coverSquare".into(), source: "steamgriddb".into() }]);
    assert_eq!(done.results[1].outcome, GameOutcome::NoChange(NoChangeReason::NothingMissing));
    // Only SteamGridDB (and its picture) was asked.
    assert!(s.server.log.lock()[before..].iter().all(|r| r.path().starts_with("/sgdb/") || r.path().starts_with("/img/")));

    let md = get(&s.svc, "steam_620");
    assert_eq!(md.cover_square_url.as_deref(), Some("localart://metadata-art/sideload_steam_620/cover-square.png"));
    assert_eq!((md.cover_url.as_deref(), md.field_sources["cover"].as_str()), (Some("https://images.igdb.com/tall.jpg"), "igdb"));
    assert_eq!(md.description.as_deref(), Some("d"));
    let dir = s.paths.metadata_art_dir().join("sideload_steam_620");
    assert!(dir.join("cover-square.png").exists() && !dir.join("cover.png").exists());
    assert_eq!(get(&s.svc, "plain").cover_square_url.as_deref(), Some("https://mine/sq.png"));

    // Undo takes the cover and its file away, the rest stays.
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!((report.games, report.files_removed), (1, 1));
    let md = get(&s.svc, "steam_620");
    assert!(md.cover_square_url.is_none() && !md.field_sources.contains_key("coverSquare"));
    assert_eq!(md.cover_url.as_deref(), Some("https://images.igdb.com/tall.jpg"));
    assert!(!dir.join("cover-square.png").exists());

    // A locked family is left alone.
    let locks: IndexMap<String, bool> = [("coverSquare".to_string(), true)].into_iter().collect();
    s.svc.set_field_locks("sideload", "steam_620", &locks).unwrap();
    let done = s.svc.run_bulk_metadata(&BulkArgs::download_covers(vec![portal()], CoverFamily::Square), &|_| {});
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::Locked));
    assert!(get(&s.svc, "steam_620").cover_square_url.is_none());
}

#[test]
fn a_job_never_fills_a_family_that_is_not_kept() {
    let s = setup(&[]);
    let fields = Some(vec!["coverSquare".to_string(), "coverWide".to_string()]);
    let done = s.svc.run_bulk_metadata(&BulkArgs::update(vec![portal()], UpdateMode::Fill, fields, None), &|_| {});
    assert_ne!(done.results[0].outcome, GameOutcome::Updated);
    assert_eq!(sgdb_requests(&s.server), 0);
    assert!(s.svc.get_game_metadata("sideload", "steam_620").unwrap().is_none_or(|m| m.cover_wide_url.is_none()));
}

#[test]
fn a_view_that_starts_using_a_family_queues_one_job() {
    let s = setup(&[]);
    // Off until the user asks for it: nothing is queued.
    assert!(s.svc.cover_family_job(CoverFamily::Square, None).is_none());
    heronite_data::settings::set_fill_cover_families(&s.paths, true).unwrap();
    put(&s.paths, &record("steam_620", |_| {}));
    put(&s.paths, &record("plain", |m| m.cover_square_url = Some("https://mine/sq.png".into())));
    let job = s.svc.cover_family_job(CoverFamily::Square, None).unwrap();
    assert_eq!(job.kind, JobKind::Covers(CoverFamily::Square));
    assert_eq!(job.games.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["steam_620"], "the Collection's games lacking it");
    assert_eq!(job.fields.as_deref(), Some(&["coverSquare".to_string()][..]));
    assert!(s.svc.cover_family_job(CoverFamily::Tall, None).is_none());
    // A picture set by hand counts.
    fs::write(s.paths.local_library().join("pic.png"), png()).unwrap();
    s.svc
        .set_art_from_file("sideload", "steam_620", ArtKind::CoverSquare, &s.paths.local_library().join("pic.png").to_string_lossy())
        .unwrap();
    assert!(s.svc.cover_family_job(CoverFamily::Square, None).is_none());
    assert!(s.svc.cover_family_job(CoverFamily::Wide, Some(vec![plain()])).is_some(), "Steam's and the store's need no key");
    // Off, or no source that can be asked: nothing.
    heronite_data::settings::set_fill_cover_families(&s.paths, false).unwrap();
    assert!(s.svc.cover_family_job(CoverFamily::Wide, None).is_none());
    heronite_data::settings::set_fill_cover_families(&s.paths, true).unwrap();
    s.svc.set_source_enabled("steamgriddb", false).unwrap();
    s.svc.reset_art("sideload", "steam_620", ArtKind::CoverSquare).unwrap();
    assert!(s.svc.cover_family_job(CoverFamily::Square, None).is_none(), "SteamGridDB is off");
    assert_eq!(sgdb_requests(&s.server), 0, "deciding asks no source");
}

#[test]
fn what_a_family_lacks_is_counted_whatever_the_automatic_download_says() {
    let s = setup(&[]);
    put(&s.paths, &record("steam_620", |_| {}));
    put(&s.paths, &record("plain", |m| m.cover_wide_url = Some("https://mine/w.png".into())));
    heronite_data::settings::set_fill_cover_families(&s.paths, false).unwrap();
    let lack = s.svc.family_lack(CoverFamily::Square, None).unwrap();
    let mut names: Vec<_> = lack.games.iter().map(|g| g.app_name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["plain", "steam_620"], "the automatic download is off, a download by hand still finds them");
    assert!(lack.can_download && s.svc.can_download_family(CoverFamily::Square));
    let wide = s.svc.family_lack(CoverFamily::Wide, Some(vec![portal(), plain()])).unwrap();
    assert_eq!(wide.games.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["steam_620"]);
    assert!(s.svc.family_lack(CoverFamily::Tall, None).unwrap().games.is_empty());
    s.svc.set_source_enabled("steamgriddb", false).unwrap();
    assert!(!s.svc.family_lack(CoverFamily::Square, None).unwrap().can_download, "SteamGridDB is off");
    assert_eq!(sgdb_requests(&s.server), 0);
}

#[test]
fn counts_say_which_games_show_a_cropped_cover() {
    let s = setup(&["wide"]);
    heronite_data::settings::update_metadata(&s.paths, |m| m.cover_aspect = "square".into()).unwrap();
    put(&s.paths, &record("steam_620", |m| m.cover_square_url = Some("https://sq".into())));
    put(&s.paths, &record("plain", |_| {}));
    let counts = s.svc.bulk_counts(&[portal(), plain()]).unwrap();
    assert_eq!(counts.cropped.get(&CoverFamily::Square), Some(&1));
    assert!(!counts.cropped.contains_key(&CoverFamily::Wide), "kept but not in use: not drawn cropped");
    assert_eq!(counts.no_cover, 0);
    assert_eq!(counts.missing_by_field["coverWide"], 2, "a kept family is filled by \"Fill what is missing\"");
    let attention = s.svc.attention(&[portal(), plain()]).unwrap();
    assert_eq!(attention.cropped[&CoverFamily::Square].len(), 1);
}

#[test]
fn a_cropped_cover_is_not_a_reason_to_need_attention() {
    let s = setup(&[]);
    heronite_data::settings::update_metadata(&s.paths, |m| m.cover_aspect = "square".into()).unwrap();
    put(&s.paths, &record("plain", |_| {}));
    put(&s.paths, &record("steam_620", |_| {}));
    let games = [portal(), plain()];
    let attention = s.svc.attention(&games).unwrap();
    assert_eq!(attention.cropped[&CoverFamily::Square].len(), 2, "both show their tall cover cropped");
    assert!(attention.all().is_empty() && !attention.contains("sideload:plain"));
    assert_eq!(attention.complete(), 2);
    let counts = s.svc.bulk_counts(&games).unwrap();
    assert_eq!((counts.needs_attention, counts.cropped[&CoverFamily::Square]), (0, 2));
    assert!(s.svc.needs_attention(&games).unwrap().is_empty());
}

#[test]
fn removing_a_game_deletes_its_family_files() {
    let s = setup(&[]);
    let pic = s.paths.local_library().join("pic.png");
    fs::write(&pic, png()).unwrap();
    for kind in ArtKind::ALL {
        s.svc.set_art_from_file("sideload", "plain", kind, &pic.to_string_lossy()).unwrap();
    }
    let fetched = s.paths.metadata_art_dir().join("sideload_plain");
    fs::create_dir_all(&fetched).unwrap();
    for name in ["cover.png", "cover-square.png", "cover-wide.png"] {
        fs::write(fetched.join(name), png()).unwrap();
    }
    assert_eq!(fs::read_dir(s.paths.art_dir().join("sideload_plain")).unwrap().count(), 4);
    heronite_metadata::remove::remove_collection_game(&s.paths, &heronite_metadata::remove::ShortcutDirs::none(), "plain", "sideload")
        .unwrap();
    assert_eq!(fs::read_dir(s.paths.art_dir().join("sideload_plain")).unwrap().count(), 0);
    assert!(!fetched.exists());
    assert!(json_store::read_value(&s.paths.art()).unwrap()["games"].get("sideload:plain").is_none());
}

#[test]
fn a_game_only_steamgriddb_finds_counts_as_found_by_no_source() {
    // The idle fill: a game of no store library (imported, emulated) that
    // only SteamGridDB has, by its exact name. No details source found it:
    // marked as failed and tried again in a week; the sure picture found
    // beside is saved with the mark, never as fetched details.
    let elsewhere = item("elsewhere", "Plain Game", None);
    let s = setup(&["square"]);
    let saved = s.svc.apply_game_metadata(&args(&elsewhere)).unwrap();
    assert!(sgdb_requests(&s.server) > 0, "SteamGridDB was asked");
    assert_eq!(saved.fetch_error.as_deref(), Some("No metadata sources returned data"));
    assert!(saved.cover_square_url.is_some() && saved.field_sources["coverSquare"] == "steamgriddb");
    assert!(saved.description.is_none() && !saved.field_sources.contains_key("description"));
    assert_eq!(s.svc.bulk_counts(std::slice::from_ref(&elsewhere)).unwrap().failed_last_time, 1, "failed last time, tried again in a week");

    // A job asking for everything: marked the same way, and the picture is
    // what changed.
    let s = setup(&["square"]);
    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![elsewhere.clone()]), &|_| {});
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    assert_eq!(
        done.results[0].changes.iter().map(|c| (c.field.as_str(), c.source.as_str())).collect::<Vec<_>>(),
        [("coverSquare", "steamgriddb")]
    );
    let md = get(&s.svc, "elsewhere");
    assert_eq!(md.fetch_error.as_deref(), Some("No metadata sources returned data"));
    assert!(md.cover_square_url.is_some());
    // Undone, the game is as before: no record.
    s.svc.undo_bulk_metadata().unwrap();
    assert!(s.svc.get_game_metadata("sideload", "elsewhere").unwrap().is_none());

    // An unsure picture alone (a similar name) is not saved.
    let similar = item("similar", "Plain Gam", None);
    let s = setup(&["square"]);
    let saved = s.svc.apply_game_metadata(&args(&similar)).unwrap();
    assert!(saved.fetch_error.is_some() && saved.cover_square_url.is_none());

    // A job that does not ask every source: a game never fetched gets no
    // record holding only a picture (it would keep the idle fill away).
    let s = setup(&["square"]);
    let some = BulkArgs::update(vec![elsewhere], UpdateMode::Fill, None, Some(vec!["steamgriddb".into(), "igdb".into()]));
    let done = s.svc.run_bulk_metadata(&some, &|_| {});
    assert_eq!(done.results[0].outcome, GameOutcome::NotFound);
    assert!(s.svc.get_game_metadata("sideload", "elsewhere").unwrap().is_none());
}

#[test]
fn a_covers_download_leaves_games_without_details_and_the_fetch_marks_as_they_were() {
    let s = setup(&[]);
    // Details, and the last fetch of them failed: only the picture changes.
    put(&s.paths, &record("steam_620", |m| m.fetch_error = Some("x".into())));
    // No details, found in no source: its sure picture is asked, its mark stays.
    put(
        &s.paths,
        &record("plain", |m| {
            m.description = None;
            m.cover_url = None;
            m.field_sources.clear();
            m.fetch_error = Some("No metadata sources returned data".into());
        }),
    );
    // Never fetched: left to the idle fill.
    let ghost = item("ghost", "Ghost Game", None);
    let lack = s.svc.family_lack(CoverFamily::Square, Some(vec![portal(), plain(), ghost.clone()])).unwrap();
    assert_eq!(lack.games.iter().map(|g| g.app_name.as_str()).collect::<Vec<_>>(), ["steam_620", "plain"], "what the download will do");

    let done = s.svc.run_bulk_metadata(&BulkArgs::download_covers(vec![portal(), plain(), ghost], CoverFamily::Square), &|_| {});
    let outcomes: Vec<&GameOutcome> = done.results.iter().map(|r| &r.outcome).collect();
    assert_eq!(outcomes, [&GameOutcome::Updated, &GameOutcome::Updated, &GameOutcome::Skipped(SkipReason::NoDetails)]);
    let md = get(&s.svc, "steam_620");
    assert!(md.cover_square_url.is_some());
    assert_eq!((md.fetched_at.as_deref(), md.fetch_error.as_deref()), (Some("2020-01-01T00:00:00.000Z"), Some("x")));
    let plain_md = get(&s.svc, "plain");
    assert!(plain_md.cover_square_url.is_some());
    assert_eq!(
        (plain_md.fetched_at.as_deref(), plain_md.fetch_error.as_deref()),
        (Some("2020-01-01T00:00:00.000Z"), Some("No metadata sources returned data")),
        "still found in no source: the idle fill asks it again in a week"
    );
    assert!(s.svc.get_game_metadata("sideload", "ghost").unwrap().is_none(), "no record is made for it");
}

#[test]
fn a_family_download_is_queued_once_while_one_runs_or_waits() {
    let s = setup(&[]);
    heronite_data::settings::set_fill_cover_families(&s.paths, true).unwrap();
    put(&s.paths, &record("steam_620", |_| {}));
    put(&s.paths, &record("plain", |_| {}));
    // The job's thread waits in its first progress until `go` is dropped.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let wait = std::sync::Mutex::new(wait);
    let hold = move |_: &BulkProgress| {
        if std::thread::current().name() == Some("metadata-bulk") {
            let _ = wait.lock().unwrap().recv();
        }
    };
    let first = s.svc.cover_family_job(CoverFamily::Square, None).unwrap();
    s.svc.start_bulk_job(first, hold);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while s.svc.bulk_progress().kind != JobKind::Covers(CoverFamily::Square) && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(s.svc.cover_family_job(CoverFamily::Square, None).is_none(), "running");
    let wide = s.svc.cover_family_job(CoverFamily::Wide, None).unwrap();
    s.svc.start_bulk_job(wide, |_| {});
    assert!(s.svc.cover_family_job(CoverFamily::Wide, None).is_none(), "waiting");
    drop(go);
    while (s.svc.bulk_progress().is_running() || !s.svc.bulk_progress().queued.is_empty()) && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!s.svc.bulk_progress().is_running(), "the jobs ended");
}

/// A pictures source installed as an extension that takes itself out of
/// the service while it is asked, as a removal in the middle of a job does.
struct Leaving {
    d: sources::SourceDescriptor,
    svc: OnceLock<Arc<MetadataService>>,
}

impl sources::MetadataSource for Leaving {
    fn descriptor(&self) -> &sources::SourceDescriptor {
        &self.d
    }

    fn lookup(&self, _ctx: &sources::SourceContext<'_>, game: &sources::SourceGame) -> Lookup<MetadataCandidate> {
        assert!(self.svc.get().unwrap().remove_source(&self.d.id));
        let mut md = heronite_metadata::util::blank_metadata(&game.app_name, &game.runner);
        md.cover_url = Some("https://fanart.example/cover.png".into());
        let mut c = MetadataCandidate::new(&self.d.id, md, Some(MatchedBy::ExactTitle));
        c.match_name = Some(game.title.clone());
        Lookup::Found(c)
    }
}

#[test]
fn a_pictures_source_removed_while_it_is_asked_still_counts_as_one() {
    let s = setup(&[]);
    let mut d = sources::builtin_descriptors().iter().find(|d| d.id == "steamgriddb").unwrap().clone();
    d.id = "fanart".into();
    d.needs = sources::Needs::Nothing;
    d.options.clear();
    d.provides = vec!["cover".into()];
    d.network = false;
    d.extension = Some(sources::ExtensionInfo { version: "1.0.0".into(), author: "a".into(), homepage: None, adult: false });
    let leaving = Arc::new(Leaving { d, svc: OnceLock::new() });
    leaving.svc.set(s.svc.clone()).ok().unwrap();
    s.svc.add_source(leaving).unwrap();
    let saved = s.svc.apply_game_metadata(&args(&item("elsewhere", "Plain Game", None))).unwrap();
    assert!(s.svc.sources().get("fanart").is_none(), "it left during the fetch");
    // Decided with the registry it was asked from: a pictures source alone
    // does not find a game.
    assert_eq!(saved.fetch_error.as_deref(), Some("No metadata sources returned data"));
    assert_eq!(saved.cover_url.as_deref(), Some("https://fanart.example/cover.png"), "a sure picture is kept with the mark");
}
