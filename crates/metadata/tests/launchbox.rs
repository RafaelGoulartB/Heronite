//! The LaunchBox Games Database as a source, against a local server that
//! publishes the fixture's archive (`fixtures/launchbox/`, trimmed from the
//! real XML): nothing asked before the download, the download and its
//! refresh (unchanged, resumed, broken), what an emulated game gets, its
//! place in the order, its box art in the cover family of its shape (ahead
//! of SteamGridDB's made-up art) and in the Covers dialog, removal.

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use parking_lot::Mutex;
use serde_json::{json, Value};

const ARCHIVE: &str = "/launchbox/Metadata.zip";

fn fixture(name: &str) -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/launchbox").join(name)).unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn archive() -> Vec<u8> {
    zip_of(&[("Metadata.xml", &fixture("Metadata.xml")), ("Files.xml", &fixture("Files.xml"))])
}

/// What the server publishes: the archive and its ETag.
struct Published {
    body: Vec<u8>,
    etag: String,
}

/// Super Metroid's North American box in the fixture: landscape (its real
/// size), unless a test says otherwise.
const METROID_NA: &str = "r2_4f09af04-78b8-43e9-97a9-a8d9c4bb0b85.jpg";

/// The start of a JPEG of `w` x `h` as LaunchBox serves its box art: 20 KB
/// of metadata before the frame header (more than one read), then 300 KB
/// of picture the size reader must not wait for.
fn jpeg(w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0xff, 0xd8, 0xff, 0xe1];
    b.extend(20_000u16.to_be_bytes());
    b.extend(vec![0u8; 19_998]);
    b.extend([0xff, 0xc0, 0x00, 0x11, 0x08]);
    b.extend(h.to_be_bytes());
    b.extend(w.to_be_bytes());
    b.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    b.extend(vec![0u8; 300_000]);
    b
}

/// SteamGridDB's made-up art of the game it finds for any title: one grid
/// of the dimensions asked, one hero, all on this server.
fn sgdb(req: &Req, base: &str) -> Resp {
    let path = req.path().trim_start_matches("/sgdb/api/v2");
    if path.starts_with("/search/autocomplete/") {
        return Resp::json(json!({"success": true, "data": [{"id": 5, "name": "Super Metroid"}]}));
    }
    let picture =
        |name: &str, w: u32, h: u32| json!({"success": true, "data": [{"url": format!("{base}/img/{name}"), "width": w, "height": h}]});
    match path {
        "/grids/game/5" => {
            let dims = req.query("dimensions").unwrap_or_default();
            if dims.contains("512x512") {
                Resp::json(picture("sgdb-square.png", 512, 512))
            } else if dims.contains("460x215") {
                Resp::json(picture("sgdb-wide.png", 920, 430))
            } else {
                Resp::json(picture("sgdb-tall.png", 600, 900))
            }
        }
        "/heroes/game/5" => Resp::json(picture("sgdb-hero.png", 1920, 620)),
        _ => Resp::raw_json(r#"{"success":false,"errors":["Game not found"]}"#.into()),
    }
}

/// LaunchBox's archive (with `If-None-Match` and `Range` as its CDN
/// answers them) and its pictures (`sizes`, else tall: 1000 x 1400);
/// Lutris knows "Super Metroid" and "Hollow Knight"; SteamGridDB has
/// made-up art of any title; the rest is not found.
fn mock(published: Arc<Mutex<Published>>, sizes: Arc<Mutex<HashMap<String, (u16, u16)>>>) -> Server {
    let base: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let b = base.clone();
    let server = Server::start(move |req: &Req| match req.path() {
        "/lbimages/gone.jpg" => Resp::status(404),
        "/lbimages/busy.jpg" => Resp::status(503),
        p if p.starts_with("/lbimages/") => {
            let file = p.trim_start_matches("/lbimages/");
            let (w, h) = sizes.lock().get(file).copied().unwrap_or((1000, 1400));
            Resp::image(jpeg(w, h))
        }
        p if p.starts_with("/sgdb/") => sgdb(req, b.get().map_or("", String::as_str)),
        ARCHIVE => {
            let p = published.lock();
            if req.header("If-None-Match") == Some(p.etag.as_str()) {
                return Resp::status(304);
            }
            let range =
                req.header("Range").and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
            let same = req.header("If-Range").is_none_or(|e| e == p.etag);
            match range {
                // Asked from its end: the part is whole.
                Some(from) if same && from >= p.body.len() => Resp::status(416),
                Some(from) if same => {
                    let mut r = Resp::image(p.body[from..].to_vec()).with_header("ETag", &p.etag);
                    r.status = 206;
                    r
                }
                _ => Resp::image(p.body.clone()).with_header("ETag", &p.etag),
            }
        }
        "/lutris/api/games" => {
            let q = req.query("search").unwrap_or_default();
            let slug = q.to_lowercase().replace(' ', "-");
            let year = if slug == "super-metroid" { 1994 } else { 2017 };
            Resp::json(json!({"count": 1, "results": [{"id": 1, "name": q, "slug": slug, "year": year}]}))
        }
        p if p.starts_with("/lutris/api/games/") => {
            let slug = p.trim_start_matches("/lutris/api/games/");
            let (name, year) = if slug == "super-metroid" { ("Super Metroid", 1994) } else { ("Hollow Knight", 2017) };
            Resp::json(json!({"name": name, "slug": slug, "year": year, "description": "What Lutris says.",
                              "genres": [{"name": "Platform"}], "platforms": [], "coverart": "https://lutris.net/c.jpg"}))
        }
        _ => Resp::status(404),
    });
    base.set(server.base.clone()).unwrap();
    server
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    svc: MetadataService,
    server: Server,
    published: Arc<Mutex<Published>>,
    /// The sizes of LaunchBox's pictures, by file name.
    sizes: Arc<Mutex<HashMap<String, (u16, u16)>>>,
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

/// A SNES game added from Playnite (no platform id, the user's emulator
/// name and the dump's file) and a PC game.
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    write(
        &paths.local_meta(),
        json!({"games": {
            "sm": {"appName": "sm", "runner": "sideload", "title": "Super Metroid", "playniteId": "", "source": "emulator",
                   "launchKind": "emulator", "emulatorName": "Snes9x",
                   "roms": [{"path": "{InstallDir}\\Super Metroid (Japan, USA) (En,Ja).sfc"}],
                   "windowsInstallDirectory": "D:\\Roms\\SNES"},
            "hk": {"appName": "hk", "runner": "sideload", "title": "Hollow Knight", "playniteId": "", "source": "manual",
                   "launchKind": "executable"}
        }}),
    );
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let published = Arc::new(Mutex::new(Published { body: archive(), etag: "\"v1\"".into() }));
    let sizes = Arc::new(Mutex::new(HashMap::from([(METROID_NA.to_string(), (4209, 2937))])));
    let server = mock(published.clone(), sizes.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    Setup { _dir: dir, paths, svc, server, published, sizes }
}

fn args(app: &str, title: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.into(), ..Default::default() }
}

fn archive_requests(server: &Server) -> usize {
    server.count(|r| r.path() == ARCHIVE)
}

fn state(preview: &MetadataPreview) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == "launchbox").map(|s| s.answer.clone()).unwrap()
}

fn index_dir(paths: &DataPaths) -> PathBuf {
    paths.source_cache_dir("launchbox")
}

#[test]
fn nothing_is_fetched_before_the_download() {
    let s = setup();
    let settings = s.svc.metadata_settings();
    assert_eq!(s.svc.source_status(&settings, "launchbox"), Some(sources::SourceStatus::NeedsDownload));
    let row = s.svc.source_rows(&settings).into_iter().find(|r| r.descriptor.id == "launchbox").unwrap();
    let download = row.download.unwrap();
    assert_eq!((download.download.size_mb, download.state.ready), (105, false));

    // Every path that asks sources: a preview, a fetch, the idle fill, a search, a pin.
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotAsked { why: NotAsked::NeedsDownload });
    let _ = s.svc.apply_game_metadata(&args("sm", "Super Metroid"));
    let _ = s.svc.refresh_missing_collection_metadata();
    assert!(!s.svc.search_sources("Metroid", None).asked.contains(&"launchbox".to_string()));
    let mut pinned = args("sm", "Super Metroid");
    pinned.pins = Some([("launchbox".to_string(), Some("299".to_string()))].into_iter().collect());
    assert_eq!(state(&s.svc.preview_game_metadata(&pinned).unwrap()), SourceAnswer::NotAsked { why: NotAsked::NeedsDownload });
    // The idle maintenance never makes the first download.
    assert_eq!(s.svc.refresh_due_source_downloads(), 0);
    assert_eq!(archive_requests(&s.server), 0);
    assert!(!index_dir(&s.paths).exists());
}

#[test]
fn the_download_makes_it_ready_and_an_emulated_game_finds_its_dump() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    assert_eq!(archive_requests(&s.server), 1);
    let state_now = s.svc.source_download_state("launchbox").unwrap();
    assert!(state_now.ready && state_now.size_on_disk > 0 && state_now.refreshed_at.is_some());
    assert!(!index_dir(&s.paths).join("Metadata.zip.part").exists(), "the archive is not kept");
    assert_eq!(s.svc.source_status(&s.svc.metadata_settings(), "launchbox"), Some(sources::SourceStatus::On));
    assert!(s.svc.source_download_progress("launchbox").is_none(), "no download runs");

    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    let SourceAnswer::Found { hit, used, .. } = state(&preview) else { panic!("{:?}", state(&preview)) };
    assert_eq!((hit.id.as_deref(), hit.matched_by, used), (Some("299"), Some(MatchedBy::ExactTitle), true));
    assert_eq!(hit.platform.as_deref(), Some("Super Nintendo Entertainment System"), "Found as shows the platform");
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.field_sources.get("cover").map(String::as_str), Some("launchbox"));
    assert_eq!(saved.critic_score.map(|n| n.get()), Some(94.0));
    assert!(saved.cover_url.is_some());
    assert_eq!(record::source_match(&saved, "launchbox").map(|m| m.id), Some("299".to_string()));

    // A search by hand looks at every platform and shows which.
    let found = s.svc.search_sources("Super Metroid", Some(&["launchbox".to_string()]));
    let platforms: Vec<String> = found.choices.iter().flat_map(|c| c.hits.iter()).filter_map(|h| h.platform.clone()).collect();
    assert!(
        platforms.contains(&"PC (Microsoft Windows)".to_string()) && platforms.contains(&"Super Nintendo Entertainment System".to_string())
    );

    // Not for a PC game, until its option says so.
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotAsked { why: NotAsked::NotForThisGame });
    s.svc.set_source_option("launchbox", "pcGames", sources::OptionValue::Switch(true)).unwrap();
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    assert!(matches!(state(&preview), SourceAnswer::Found { hit, .. } if hit.id.as_deref() == Some("123770")));
}

/// For an emulated game LaunchBox found by its dump, ahead of Lutris (by
/// title); for a PC game, or a field the user ordered, the order as set.
#[test]
fn the_order_rule() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    s.svc.set_source_option("launchbox", "pcGames", sources::OptionValue::Switch(true)).unwrap();
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.field_sources["description"], "launchbox", "emulated: LaunchBox before Lutris");
    assert_ne!(saved.description.as_deref(), Some("What Lutris says."));
    assert_eq!(saved.field_sources["genres"], "launchbox");

    let pc = s.svc.apply_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    assert_eq!(pc.field_sources["description"], "lutris", "a PC game keeps the default order");

    s.svc.set_field_order("description", &["lutris".to_string(), "launchbox".to_string()]).unwrap();
    let mut again = args("sm", "Super Metroid");
    again.overwrite_existing = Some(true);
    let saved = s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(saved.field_sources["description"], "lutris", "the user's order of a field wins");
    assert_eq!(saved.field_sources["genres"], "launchbox", "the other fields still follow the rule");
}

#[test]
fn refreshes_ask_only_for_a_changed_archive() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    let first = s.svc.source_download_state("launchbox").unwrap();
    // Refresh now: unchanged, nothing downloaded again (304), the date moves.
    std::thread::sleep(Duration::from_millis(1100));
    s.svc.download_source("launchbox").unwrap();
    let reqs = s.server.log.lock().clone();
    assert_eq!(reqs.last().and_then(|r| r.header("If-None-Match")), Some("\"v1\""));
    let second = s.svc.source_download_state("launchbox").unwrap();
    assert!(second.refreshed_at > first.refreshed_at && second.size_on_disk == first.size_on_disk);

    // A broken archive (a page instead of the zip, then a zip cut short):
    // the old index stays and keeps answering.
    for broken in [b"<html>busy</html>".to_vec(), archive()[..2000].to_vec()] {
        *s.published.lock() = Published { body: broken, etag: "\"v2\"".into() };
        assert!(s.svc.download_source("launchbox").is_err());
        let after = s.svc.source_download_state("launchbox").unwrap();
        assert!(after.ready && after.refreshed_at == second.refreshed_at);
        assert!(!index_dir(&s.paths).join("Metadata.zip.part").exists(), "a broken archive is not resumed");
        let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
        assert!(matches!(state(&preview), SourceAnswer::Found { .. }));
    }
}

#[test]
fn an_interrupted_download_resumes() {
    let s = setup();
    let body = archive();
    let dir = index_dir(&s.paths);
    fs::create_dir_all(&dir).unwrap();
    // What a download cut halfway left.
    fs::write(dir.join("Metadata.zip.part"), &body[..body.len() / 2]).unwrap();
    fs::write(dir.join("Metadata.zip.part.json"), br#"{"etag":"\"v1\""}"#).unwrap();
    s.svc.download_source("launchbox").unwrap();
    let req = s.server.log.lock().iter().rev().find(|r| r.path() == ARCHIVE).cloned().unwrap();
    assert_eq!(req.header("Range"), Some(format!("bytes={}-", body.len() / 2).as_str()));
    assert!(s.svc.source_download_state("launchbox").unwrap().ready);

    // A part of an archive that changed since is downloaded whole.
    let s = setup();
    let dir = index_dir(&s.paths);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Metadata.zip.part"), b"stale bytes").unwrap();
    fs::write(dir.join("Metadata.zip.part.json"), br#"{"etag":"\"v0\""}"#).unwrap();
    s.svc.download_source("launchbox").unwrap();
    assert!(s.svc.source_download_state("launchbox").unwrap().ready);
}

fn set_refreshed(paths: &DataPaths, days_ago: u64) {
    let path = index_dir(paths).join("state.json");
    let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - days_ago * 86_400;
    v["refreshedAt"] = json!(at);
    fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn the_idle_refresh_is_weekly_and_only_while_on() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    let asked = || archive_requests(&s.server);
    assert_eq!(asked(), 1);
    set_refreshed(&s.paths, 6);
    assert_eq!(s.svc.refresh_due_source_downloads(), 0);
    assert_eq!(asked(), 1, "less than a week: not asked");
    set_refreshed(&s.paths, 8);
    s.svc.set_source_enabled("launchbox", false).unwrap();
    assert_eq!(s.svc.refresh_due_source_downloads(), 0);
    assert_eq!(asked(), 1, "off: not refreshed");
    assert!(s.svc.source_download_state("launchbox").unwrap().ready, "turning it off keeps the database");
    s.svc.set_source_enabled("launchbox", true).unwrap();
    assert_eq!(s.svc.refresh_due_source_downloads(), 1);
    assert_eq!(asked(), 2);
    assert_eq!(s.svc.refresh_due_source_downloads(), 0, "refreshed: not due again");
}

#[test]
fn removing_the_database() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    s.svc.remove_source_download("launchbox").unwrap();
    let after = s.svc.source_download_state("launchbox").unwrap();
    assert!(!after.ready && after.size_on_disk == 0);
    assert_eq!(s.svc.source_status(&s.svc.metadata_settings(), "launchbox"), Some(sources::SourceStatus::NeedsDownload));
    assert!(fs::read_dir(index_dir(&s.paths)).unwrap().next().is_none(), "nothing left");
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotAsked { why: NotAsked::NeedsDownload });
}

#[test]
fn a_cancelled_download_changes_nothing() {
    let s = setup();
    let calls = Arc::new(AtomicUsize::new(0));
    // Cancelled from another thread as soon as it shows.
    let svc = Arc::new(s.svc);
    let watcher = {
        let svc = svc.clone();
        let calls = calls.clone();
        std::thread::spawn(move || {
            for _ in 0..2000 {
                if svc.source_download_progress("launchbox").is_some() {
                    svc.cancel_source_download("launchbox");
                    calls.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_micros(50));
            }
        })
    };
    let result = svc.download_source("launchbox");
    watcher.join().unwrap();
    if calls.load(Ordering::SeqCst) > 0 {
        if let Err(e) = &result {
            assert!(sources::download::is_cancelled(e), "{e:#}");
            assert!(!svc.source_download_state("launchbox").unwrap().ready);
        }
    }
    assert!(svc.source_download_progress("launchbox").is_none());
}

fn stored_state(paths: &DataPaths) -> Value {
    serde_json::from_slice(&fs::read(index_dir(paths).join("state.json")).unwrap()).unwrap()
}

/// Moves the last failed refresh `days` back.
fn set_failed(paths: &DataPaths, days_ago: u64) {
    let path = index_dir(paths).join("state.json");
    let mut v = stored_state(paths);
    v["failedAt"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - days_ago * 86_400);
    fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
}

/// A refresh that fails is recorded: the idle maintenance waits a day, then
/// two, four, a week, instead of downloading 105 MB at every idle pass;
/// the old index answers meanwhile.
#[test]
fn a_failed_refresh_waits_before_the_next_try() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    set_refreshed(&s.paths, 8);
    *s.published.lock() = Published { body: b"<html>busy</html>".to_vec(), etag: "\"v2\"".into() };
    assert_eq!(s.svc.refresh_due_source_downloads(), 0);
    assert_eq!(archive_requests(&s.server), 2);
    assert_eq!(stored_state(&s.paths)["failures"], 1);
    let db = s.svc.source_download_state("launchbox").unwrap();
    let day = Duration::from_secs(86_400);
    assert!(db.ready && db.retry_at.is_some_and(|at| at > SystemTime::now() + day - Duration::from_secs(60)), "{db:?}");
    for _ in 0..3 {
        assert_eq!(s.svc.refresh_due_source_downloads(), 0);
    }
    assert_eq!(archive_requests(&s.server), 2, "not asked again at the next idle passes");
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert!(matches!(state(&preview), SourceAnswer::Found { .. }), "the old index answers");

    // A day later: tried again, failed again, then two days.
    set_failed(&s.paths, 1);
    s.svc.refresh_due_source_downloads();
    assert_eq!((archive_requests(&s.server), stored_state(&s.paths)["failures"].as_u64()), (3, Some(2)));
    set_failed(&s.paths, 1);
    s.svc.refresh_due_source_downloads();
    assert_eq!(archive_requests(&s.server), 3, "the second wait is two days");
    // Mended: the next try works and the failures are forgotten.
    *s.published.lock() = Published { body: archive(), etag: "\"v3\"".into() };
    set_failed(&s.paths, 2);
    assert_eq!(s.svc.refresh_due_source_downloads(), 1);
    let v = stored_state(&s.paths);
    assert!(v.get("failures").is_none() && v.get("failedAt").is_none() && v["etag"] == "\"v3\"");
    assert_eq!(s.svc.source_download_state("launchbox").unwrap().retry_at, None);
}

/// An index that cannot be written (here its file's place is taken; a full
/// disk alike) keeps the whole archive: the next try builds from it after
/// one small request, without downloading it again.
#[test]
fn an_index_that_cannot_be_written_keeps_the_archive() {
    let s = setup();
    let dir = index_dir(&s.paths);
    fs::create_dir_all(dir.join("index.sqlite.part").join("x")).unwrap();
    assert!(s.svc.download_source("launchbox").is_err());
    assert!(dir.join("Metadata.zip.part").exists(), "the archive read: kept");
    fs::remove_dir_all(dir.join("index.sqlite.part")).unwrap();
    s.svc.download_source("launchbox").unwrap();
    assert!(s.svc.source_download_state("launchbox").unwrap().ready);
    let last = s.server.log.lock().iter().rev().find(|r| r.path() == ARCHIVE).cloned().unwrap();
    assert_eq!(last.header("Range"), Some(format!("bytes={}-", archive().len()).as_str()), "asked from its end only");
    assert!(!dir.join("Metadata.zip.part").exists(), "dropped once used");
}

/// An index that no longer opens (damaged, or of another schema after an
/// update) is not ready, is not asked, and is downloaded again whole at the
/// next idle pass, whatever the archive's ETag.
#[test]
fn an_index_that_no_longer_opens_is_built_again() {
    let s = setup();
    s.svc.download_source("launchbox").unwrap();
    fs::write(index_dir(&s.paths).join("index.sqlite"), b"not an index").unwrap();
    let db = s.svc.source_download_state("launchbox").unwrap();
    assert!(!db.ready, "{db:?}");
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotAsked { why: NotAsked::NeedsDownload });
    assert_eq!(s.svc.refresh_due_source_downloads(), 1, "due at once");
    let last = s.server.log.lock().iter().rev().find(|r| r.path() == ARCHIVE).cloned().unwrap();
    assert_eq!(last.header("If-None-Match"), None, "the same archive is asked for whole");
    assert!(s.svc.source_download_state("launchbox").unwrap().ready);
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert!(matches!(state(&preview), SourceAnswer::Found { .. }));
}

fn picture_requests(server: &Server) -> usize {
    server.count(|r| r.path().starts_with("/lbimages/"))
}

/// [`setup`] with LaunchBox downloaded, SteamGridDB on (a key), images
/// kept as addresses, and `keep`: `keepCoverFamilies`.
fn with_sgdb(keep: &[&str]) -> Setup {
    let s = setup();
    write(&s.paths.collection_settings(), json!({"metadata": {"downloadImages": false, "steamTags": false, "keepCoverFamilies": keep}}));
    write(&s.paths.global_config(), json!({"defaultSettings": {"steamGridDbApiKey": "K"}, "version": "v0"}));
    s.svc.download_source("launchbox").unwrap();
    s
}

fn lb_url(s: &Setup, file: &str) -> String {
    format!("{}/lbimages/{file}", s.server.base)
}

/// Issue #289: an emulated game whose box is landscape (the SNES's in North
/// America) gets that box as its wide cover, not SteamGridDB's made-up art;
/// its tall cover is the box as before.
#[test]
fn a_landscape_box_is_the_wide_cover() {
    let s = with_sgdb(&["wide"]);
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.cover_wide_url.as_deref(), Some(lb_url(&s, METROID_NA).as_str()));
    assert_eq!(saved.field_sources["coverWide"], "launchbox");
    assert_eq!((saved.cover_url.as_deref(), saved.field_sources["cover"].as_str()), (Some(lb_url(&s, METROID_NA).as_str()), "launchbox"));
    assert_eq!(picture_requests(&s.server), 1, "one picture's size read");
    // Its size is remembered: a fetch again reads nothing.
    let mut again = args("sm", "Super Metroid");
    again.overwrite_existing = Some(true);
    s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(picture_requests(&s.server), 1);
}

/// A CD case (a square box) is the square cover, ahead of SteamGridDB's.
#[test]
fn a_square_box_is_the_square_cover() {
    let s = with_sgdb(&["square"]);
    s.sizes.lock().insert(METROID_NA.into(), (1400, 1240));
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.cover_square_url.as_deref(), Some(lb_url(&s, METROID_NA).as_str()));
    assert_eq!(saved.field_sources["coverSquare"], "launchbox");
    assert_eq!(saved.cover_wide_url, None, "the wide family is not kept: not fetched");
}

/// A tall box gives no wide cover: SteamGridDB's fills it as before. And
/// where only the tall family is kept, no picture's size is read at all.
#[test]
fn a_tall_box_leaves_the_other_shapes_to_the_others() {
    let s = with_sgdb(&["wide"]);
    s.sizes.lock().insert(METROID_NA.into(), (1000, 1400));
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.cover_wide_url.as_deref(), Some(format!("{}/img/sgdb-wide.png", s.server.base).as_str()));
    assert_eq!(saved.field_sources["coverWide"], "steamgriddb");

    let s = with_sgdb(&[]);
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.cover_url.as_deref(), Some(lb_url(&s, METROID_NA).as_str()));
    assert_eq!(picture_requests(&s.server), 0, "the tall cover takes the box whatever its shape");
}

/// The Covers dialog lists the game's boxes from LaunchBox first, those of
/// the slot's shape ahead, each with its region and size; then SteamGridDB's.
#[test]
fn the_covers_dialog_offers_every_regions_box() {
    let s = with_sgdb(&[]);
    let game = args("sm", "Super Metroid");
    let found = s.svc.search_game_art_images(&game, "Super Metroid", "\"Super Metroid\" cover", ArtKind::CoverWide).unwrap();
    let lb = &found.launchbox;
    assert_eq!(lb.len(), 6, "six regions' boxes");
    assert_eq!((lb[0].region.as_str(), lb[0].image.image_url.clone()), ("North America", lb_url(&s, METROID_NA)));
    assert_eq!((lb[0].image.width, lb[0].image.height), (4209, 2937));
    assert!(lb[1..].iter().all(|c| (c.image.width, c.image.height) == (1000, 1400)), "the tall ones after");
    assert_eq!(found.steamgrid.len(), 1);
    let tall = s.svc.search_game_art_images(&game, "Super Metroid", "\"Super Metroid\" cover", ArtKind::Cover).unwrap();
    assert_eq!(tall.launchbox.last().map(|c| c.region.as_str()), Some("North America"), "the landscape box last for a tall cover");
    let hero = s.svc.launchbox_pictures(&game, "Super Metroid", ArtKind::Hero);
    assert_eq!(hero.len(), 1);
    // Other words: the game of that name, on the game's platform.
    let other = s.svc.launchbox_pictures(&game, "link to the past", ArtKind::Cover);
    assert!(other.iter().any(|c| c.image.image_url.ends_with("r2_689c3eda-bd3b-452d-997c-c25b5335656e.jpg")), "{other:?}");
    // Not for a PC game (its option off), nor with LaunchBox off.
    assert!(s.svc.launchbox_pictures(&args("hk", "Hollow Knight"), "Hollow Knight", ArtKind::Cover).is_empty());
    s.svc.set_source_enabled("launchbox", false).unwrap();
    assert!(s.svc.launchbox_pictures(&game, "Super Metroid", ArtKind::Cover).is_empty());
}

/// A picture the image server does not have is remembered as such (asked
/// once); one it could not answer for now is asked again.
#[test]
fn a_missing_picture_is_asked_once() {
    let s = setup();
    let lb = launchbox::LaunchBox::new();
    let base = format!("{}/lbimages", s.server.base);
    let asked = |file: &str| s.server.count(|r| r.path() == format!("/lbimages/{file}"));
    for _ in 0..2 {
        assert_eq!(lb.picture_size(&s.svc.http, &base, "gone.jpg"), None);
        assert_eq!(lb.picture_size(&s.svc.http, &base, "busy.jpg"), None);
    }
    assert_eq!((asked("gone.jpg"), asked("busy.jpg")), (1, 2));
    assert_eq!(lb.picture_size(&s.svc.http, &base, METROID_NA), Some((4209, 2937)));
}
