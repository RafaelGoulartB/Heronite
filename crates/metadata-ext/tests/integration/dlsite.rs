//! The DLsite extension (`extensions/metadata/dlsite`) through the real
//! install and gather, against recorded DLsite answers: a game found by the
//! product code in its title or folder (sure, whatever its name), what it
//! fills, adult pictures flagged by the product's age category, only asked
//! as a fallback, never asked without a code, and a DLsite that refuses or
//! is busy (unreachable, never a failure).
//!
//! The recordings (`tests/fixtures/dlsite_product_*.json`) are two products
//! rated for all ages, trimmed to the keys the script reads. The adult
//! product is written here, with made-up picture addresses: no adult
//! picture, nor its address, is in the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use crate::support::{endpoints, example, fixture, Req, Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::record;
use heronite_metadata::service::is_offline;
use heronite_metadata::sources::{SourceHealth, SourceStatus};
use heronite_metadata::*;
use heronite_metadata_ext::manifest::Manifest;
use heronite_metadata_ext::ExtensionsExt;
use serde_json::{json, Value};

const API: &str = "/www.dlsite.com/maniax/api/=/product.json";
const HOST: &str = "www.dlsite.com";

/// The games: (app name, title, install folder).
const GAMES: [(&str, &str, &str); 11] = [
    ("drago", "Drago Noka [RJ345679]", "Drago Noka"),
    ("stella", "Stella of the End", "vj015604_Stella"),
    ("adult", "RJ01000001 Night Shift", "Night Shift"),
    ("r15", "RJ01000002 Dusk", "Dusk"),
    ("unrated", "RJ01000003 Mystery", "Mystery"),
    ("all_ages_quirks", "RJ01000004 Indie", "Indie"),
    ("two", "RJ345679 + VJ015604 bundle", "bundle"),
    ("gone", "Lost Work RJ00000002", "Lost Work"),
    ("nocode", "Some Doujin Game", "Some Doujin Game"),
    ("portal", "Portal", "RJ345679"),
    ("portal_plain", "Portal", "Portal"),
];

/// An adult-only product, written for these tests in the shape of DLsite's
/// answer. Its picture addresses are made up.
fn adult_product() -> Value {
    json!([{
        "workno": "RJ01000001", "work_name": "Night Shift", "maker_name": "Test Circle", "regist_date": "2024-05-01 00:00:00",
        "age_category": 3, "age_category_string": "adult", "site_id": "maniax", "work_type": "RPG", "work_type_string": "Role-playing",
        "intro_s": "A test product.", "series_name": "Shifts",
        "image_thumb": "//img.dlsite.jp/test/adult_thumb.jpg",
        "genres": [{"name": "Tag One", "id": 1}],
        "image_main": {"url": "//img.dlsite.jp/test/adult_main.jpg", "width": "560", "height": "420"},
        "image_samples": [{"url": "//img.dlsite.jp/test/adult_sample.jpg", "width": "1280", "height": "720"}]
    }])
}

/// An R-15 product with a hyphenated `site_id` (`girls-pro`) and a sample
/// missing its height before the real landscape sample.
fn r15_product() -> Value {
    json!([{
        "workno": "RJ01000002", "work_name": "Dusk", "maker_name": "Test Circle", "regist_date": "2024-06-01 00:00:00",
        "age_category": 2, "age_category_string": "r15", "site_id": "girls-pro", "work_type": "ADV", "work_type_string": "Adventure",
        "intro_s": "An R-15 test product.",
        "image_thumb": "//img.dlsite.jp/test/r15_thumb.jpg",
        "genres": [{"name": "Tag Two", "id": 2}],
        "image_main": {"url": "//img.dlsite.jp/test/r15_main.jpg", "width": "560", "height": "420"},
        "image_samples": [
            {"url": "//img.dlsite.jp/test/r15_broken.jpg", "width": "560", "height": ""},
            {"url": "//img.dlsite.jp/test/r15_sample.jpg", "width": "1280", "height": "720"}
        ]
    }])
}

/// A product with no `age_category`, no `site_id`, and PHP-style `{}` for
/// empty lists: treated as non-all-ages without failing.
fn unrated_product() -> Value {
    json!([{
        "workno": "RJ01000003", "work_name": "Mystery", "maker_name": "Test Circle", "regist_date": "2024-07-01 00:00:00",
        "work_type": "ADV", "work_type_string": "Adventure", "intro_s": "No rating.",
        "image_thumb": "//img.dlsite.jp/test/unrated_thumb.jpg",
        "genres": {}, "image_samples": {},
        "image_main": {"url": "//img.dlsite.jp/test/unrated_main.jpg", "width": "560", "height": "420"}
    }])
}

/// An all-ages product with a missing `site_id` and PHP-style `{}` lists:
/// keeps its cover and falls back to the all-ages `home` storefront.
fn all_ages_quirks_product() -> Value {
    json!([{
        "workno": "RJ01000004", "work_name": "Indie", "maker_name": "Test Circle", "regist_date": "2024-08-01 00:00:00",
        "age_category": 1, "site_id": null, "work_type": "SLN", "work_type_string": "Simulation",
        "intro_s": "All-ages quirks.", "genres": {}, "image_samples": {},
        "image_main": {"url": "//img.dlsite.jp/test/indie_main.jpg", "width": "560", "height": "420"}
    }])
}

/// How DLsite answers: as recorded, or with this status (403, 404, 408, 429,
/// 451, 503), or `200` with a page instead of data (`1`), or `200 {}` (`2`).
#[derive(Default)]
struct Dlsite {
    refuse: AtomicU16,
}

/// DLsite's product data; Lutris has Portal; nothing else.
fn mock(dlsite: Arc<Dlsite>) -> Server {
    Server::start(move |req: &Req| match req.path() {
        API => match dlsite.refuse.load(Ordering::SeqCst) {
            0 => match req.query("workno").as_deref() {
                Some("RJ345679") => Resp::raw_json(fixture("dlsite_product_RJ345679.json")),
                Some("VJ015604") => Resp::raw_json(fixture("dlsite_product_VJ015604.json")),
                Some("RJ01000001") => Resp::json(adult_product()),
                Some("RJ01000002") => Resp::json(r15_product()),
                Some("RJ01000003") => Resp::json(unrated_product()),
                Some("RJ01000004") => Resp::json(all_ages_quirks_product()),
                _ => Resp::json(json!([])),
            },
            1 => Resp::html("<html><body>Please wait…</body></html>"),
            2 => Resp::json(json!({"error": "not a list"})),
            status => Resp::status(status),
        },
        "/lutris/api/games" => Resp::json(match req.query("search").as_deref() {
            Some("Portal") => serde_json::from_str::<Value>(&metadata_fixture("lutris_search_portal.json")).unwrap(),
            _ => json!({"results": []}),
        }),
        "/lutris/api/games/portal" => Resp::raw_json(metadata_fixture("lutris_detail_portal.json")),
        _ => Resp::status(404),
    })
}

fn metadata_fixture(name: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../metadata/tests/fixtures").join(name)).unwrap()
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn make_root(dir: &Path) -> DataPaths {
    let paths = DataPaths::at(dir);
    let overlay: serde_json::Map<String, Value> = GAMES
        .iter()
        .map(|(app, title, folder)| {
            let entry = json!({"appName": app, "runner": "sideload", "title": title, "playniteId": "", "source": "manual",
                               "launchKind": "executable", "windowsInstallDirectory": format!("C:\\Games\\{folder}")});
            (app.to_string(), entry)
        })
        .collect();
    write(&paths.local_meta(), json!({ "games": overlay }));
    write(&paths.collection_settings(), json!({"metadata": {}}));
    paths
}

/// The extension as published, but at 10 requests a second: its own rate
/// (20 a minute, checked by `it_installs_as_written`) would make these
/// tests wait three seconds between requests.
fn fast_copy(parent: &Path) -> PathBuf {
    let dir = parent.join("dlsite");
    fs::create_dir_all(&dir).unwrap();
    let manifest = fs::read_to_string(example("dlsite").join("source.toml")).unwrap();
    assert!(manifest.contains("rate = \"20/min\""));
    fs::write(dir.join("source.toml"), manifest.replace("rate = \"20/min\"", "rate = \"10/s\"")).unwrap();
    fs::copy(example("dlsite").join("source.js"), dir.join("source.js")).unwrap();
    dir
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
    dlsite: Arc<Dlsite>,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(&dir.path().join("data"));
    let dlsite = Arc::new(Dlsite::default());
    let server = mock(dlsite.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &[HOST]));
    svc.install_extension(&fast_copy(dir.path()), false).unwrap();
    Setup { _dir: dir, paths, server, svc, dlsite }
}

fn args(app: &str) -> MetadataApplyArgs {
    let (_, title, _) = GAMES.iter().find(|g| g.0 == app).unwrap();
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.to_string(), ..Default::default() }
}

fn set_metadata(paths: &DataPaths, v: Value) {
    write(&paths.collection_settings(), json!({ "metadata": v }));
}

/// DLsite requests for `code`.
fn asked(server: &Server, code: &str) -> usize {
    server.count(|r| r.path() == API && r.query("workno").as_deref() == Some(code))
}

fn state(preview: &MetadataPreview) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == "dlsite").map(|s| s.answer.clone()).unwrap()
}

fn candidate(preview: &MetadataPreview) -> Option<&MetadataCandidate> {
    preview.candidates.iter().find(|c| c.source == "dlsite")
}

#[test]
fn it_installs_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let svc = MetadataService::new(make_root(dir.path()));
    let info = svc.inspect_extension(&example("dlsite")).unwrap();
    assert_eq!((info.hosts.clone(), info.image_hosts.clone()), (vec![HOST.to_string()], vec!["img.dlsite.jp".to_string()]));
    assert_eq!(info.rate, "20/min");
    assert!(info.description.starts_with("Unofficial"), "says plainly it is unofficial: {}", info.description);
    assert_eq!(info.id_example.as_deref(), Some("RJ01234567"));
    let readme = info.readme.as_deref().unwrap();
    assert!(readme.starts_with("# DLsite (unofficial)") && readme.contains("**Maintained by:** lurebat"), "{readme}");
    let manifest = Manifest::parse(&fs::read_to_string(example("dlsite").join("source.toml")).unwrap()).unwrap();
    assert!(manifest.warnings.is_empty(), "{:?}", manifest.warnings);
    assert_eq!(manifest.id_pattern.as_ref().unwrap().as_str(), r"\b(?:RJ|RE|BJ|VJ)[0-9]{6,8}\b");
    assert_eq!(manifest.id_example.as_deref(), Some("RJ01234567"));
    assert!(manifest.network.rate.interval() >= std::time::Duration::from_secs(3), "a low rate");
    svc.install_extension(&example("dlsite"), false).unwrap();
    let d = svc.sources().get("dlsite").cloned().unwrap();
    assert!(d.is_fallback() && d.kind == SourceKind::Details && d.searchable);
    assert!(!d.extension.as_ref().unwrap().adult, "pictures are flagged one by one, by the product's age category");
    assert_eq!(d.provides, ["description", "releaseDate", "developers", "genres", "themes", "series", "coverWide", "hero"]);
    assert!(svc.sources().has_test("dlsite"));
    let row = svc.source_rows(&svc.metadata_settings()).into_iter().find(|r| r.descriptor.id == "dlsite").unwrap();
    assert_eq!(row.status, SourceStatus::On);
}

#[test]
fn a_code_in_the_title_finds_the_product_surely_and_fills_its_fields() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("drago")).unwrap();
    assert_eq!(asked(&s.server, "RJ345679"), 1, "find and fetch share one request");
    let m = record::source_match(&saved, "dlsite").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by), ("RJ345679", Some(MatchedBy::SourceId)), "sure, though the names differ");
    assert_eq!(saved.release_date.as_deref(), Some("2022-01-22"));
    assert_eq!(saved.developers, ["GeSEI unkan"]);
    assert_eq!(saved.genres, ["Simulation"]);
    assert_eq!(saved.themes, ["Slice of Life / Daily Living", "Fantasy"]);
    assert!(saved.description.as_deref().unwrap().starts_with("Manage and customize a village"));
    assert_eq!(
        saved.hero_url.as_deref(),
        Some("https://img.dlsite.jp/modpub/images2/work/doujin/RJ346000/RJ345679_img_smp1.jpg#heronite-extension"),
        "the first landscape sample"
    );
    for field in ["description", "releaseDate", "developers", "genres", "themes", "hero"] {
        assert_eq!(saved.field_sources[field], "dlsite", "{field}");
    }
    assert!(saved.series.is_none() && saved.cover_url.is_none());
    // The main picture is its wide cover, which the app takes when asked for the wide covers.
    let mut pt = args("drago");
    pt.language = Some("pt-BR".into());
    let preview = s.svc.preview_game_metadata(&pt).unwrap();
    assert_eq!(
        candidate(&preview).unwrap().metadata.cover_wide_url.as_deref(),
        Some("https://img.dlsite.jp/modpub/images2/work/doujin/RJ346000/RJ345679_img_main.jpg#heronite-extension")
    );
    assert_eq!(saved.websites, ["https://www.dlsite.com/home/work/=/product_id/RJ345679.html"]);

    // It says who it is, asks in the interface's language, and sends nothing but the code.
    let api_reqs: Vec<Req> = s.server.log.lock().iter().filter(|r| r.path() == API).cloned().collect();
    assert!(api_reqs[0].header("user-agent").is_some_and(|ua| ua.starts_with("Heronite/")), "{:?}", api_reqs[0].header("user-agent"));
    assert_eq!((api_reqs[0].method.as_str(), api_reqs[0].query("locale").as_deref()), ("GET", Some("en_US")));
    assert_eq!(api_reqs.last().unwrap().query("locale").as_deref(), Some("pt_BR"));
    assert!(api_reqs.iter().all(|r| r.query("workno").is_some() && r.query("title").is_none()));
}

#[test]
fn a_code_in_the_folder_name_finds_it_too_in_any_case_before_an_underscore() {
    let s = setup();
    let preview = s.svc.preview_game_metadata(&args("stella")).unwrap();
    match state(&preview) {
        SourceAnswer::Found { hit, .. } => assert_eq!(hit.matched_by, Some(MatchedBy::SourceId)),
        other => panic!("{other:?}"),
    }
    let c = candidate(&preview).unwrap();
    assert_eq!(c.metadata.title.as_deref(), Some("終のステラ"));
    assert_eq!(c.metadata.developers, ["Key"]);
    assert_eq!(c.metadata.genres, ["Adventure"]);
    assert_eq!(c.metadata.themes.len(), 6);
    assert_eq!(preview.proposed.field_sources["description"], "dlsite");
}

#[test]
fn no_code_or_two_codes_ask_nothing_and_an_unknown_code_is_not_found() {
    let s = setup();
    for app in ["nocode", "two"] {
        let preview = s.svc.preview_game_metadata(&args(app)).unwrap();
        assert_eq!(state(&preview), SourceAnswer::NotFound, "{app}");
    }
    assert_eq!(s.server.count(|r| r.path() == API), 0, "no request without exactly one code");
    let preview = s.svc.preview_game_metadata(&args("gone")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotFound);
    assert_eq!(asked(&s.server, "RJ00000002"), 1);
    assert_eq!(s.svc.sources().health("dlsite"), SourceHealth::Ok);
}

#[test]
fn it_is_asked_as_a_fallback_or_when_claimed_by_a_code() {
    let s = setup();
    // Lutris finds Portal for sure and there is no DLsite code: DLsite waits.
    let preview = s.svc.preview_game_metadata(&args("portal_plain")).unwrap();
    assert_eq!(state(&preview), SourceAnswer::NotAsked { why: NotAsked::Fallback });
    assert_eq!(s.server.count(|r| r.path() == API), 0);

    // When the folder has a DLsite code, DLsite claims the game and is asked
    // alongside Lutris, while field priority still decides whose values win.
    let preview = s.svc.preview_game_metadata(&args("portal")).unwrap();
    match state(&preview) {
        SourceAnswer::Found { hit, .. } => assert_eq!(hit.matched_by, Some(MatchedBy::SourceId)),
        other => panic!("{other:?}"),
    }
    assert_eq!(asked(&s.server, "RJ345679"), 1);
    assert_eq!(preview.proposed.field_sources.get("description").map(String::as_str), Some("lutris"));

    // A job of DLsite alone asks it and saves its match.
    let a = args("portal");
    let item = BulkItem { app_name: a.app_name, runner: a.runner, title: a.title, steam_app_id: None };
    s.svc.run_bulk_metadata(&BulkArgs::only_source(vec![item], UpdateMode::Fill, "dlsite"), &|_| {});
    assert_eq!(asked(&s.server, "RJ345679"), 2);
    let saved = s.svc.get_game_metadata("sideload", "portal").unwrap().unwrap();
    assert_eq!(record::source_match(&saved, "dlsite").map(|m| m.id), Some("RJ345679".to_string()));
}

#[test]
fn a_typed_code_finds_the_product_in_the_search() {
    let s = setup();
    let found = s.svc.search_sources("RJ345679", Some(&["dlsite".to_string()]));
    let hit = &found.choices[0].hits[0];
    assert_eq!((hit.id.as_deref(), hit.title.as_deref()), (Some("RJ345679"), Some("Drago Noka")));
    assert_eq!(
        hit.thumb_url.as_deref(),
        Some("https://img.dlsite.jp/resize/images2/work/doujin/RJ346000/RJ345679_img_main_240x240.jpg#heronite-extension")
    );
    let found = s.svc.search_sources("Drago Noka", Some(&["dlsite".to_string()]));
    assert!(found.choices.is_empty(), "no search by name");
    assert_eq!(asked(&s.server, "RJ345679"), 1);
}

#[test]
fn adult_pictures_are_flagged_by_the_age_category_and_dropped_unless_allowed() {
    let s = setup();
    for app in ["adult", "r15", "unrated"] {
        let off = s.svc.preview_game_metadata(&args(app)).unwrap();
        let c = candidate(&off).unwrap();
        assert!(c.metadata.cover_wide_url.is_none() && c.metadata.hero_url.is_none(), "{app}: dropped before the merge");
        assert!(c.metadata.themes.is_empty(), "{app}: no genre tags for a product not rated for all ages");
        match state(&off) {
            SourceAnswer::Found { hit, .. } => assert!(hit.thumb_url.is_none(), "{app}"),
            other => panic!("{app}: {other:?}"),
        }
    }
    let off = s.svc.preview_game_metadata(&args("adult")).unwrap();
    let c = candidate(&off).unwrap();
    assert_eq!((c.metadata.series.as_deref(), c.metadata.genres.clone()), (Some("Shifts"), vec!["Role-playing".to_string()]));
    let found = s.svc.search_sources("RJ01000001", Some(&["dlsite".to_string()]));
    assert!(found.choices[0].hits[0].thumb_url.is_none(), "no thumbnail");

    set_metadata(&s.paths, json!({"allowAdultCovers": true}));
    let on = s.svc.preview_game_metadata(&args("adult")).unwrap();
    let c = candidate(&on).unwrap();
    assert_eq!(c.metadata.cover_wide_url.as_deref(), Some("https://img.dlsite.jp/test/adult_main.jpg#heronite-extension"));
    assert_eq!(c.metadata.hero_url.as_deref(), Some("https://img.dlsite.jp/test/adult_sample.jpg#heronite-extension"));
    // R-15 keeps its hyphenated site_id and skips a sample with empty height.
    let r15 = s.svc.preview_game_metadata(&args("r15")).unwrap();
    let c = candidate(&r15).unwrap();
    assert_eq!(c.metadata.websites, ["https://www.dlsite.com/girls-pro/work/=/product_id/RJ01000002.html"]);
    assert_eq!(c.metadata.hero_url.as_deref(), Some("https://img.dlsite.jp/test/r15_sample.jpg#heronite-extension"));
    // Unrated with PHP `{}` lists and missing site_id falls back to maniax.
    let unrated = s.svc.preview_game_metadata(&args("unrated")).unwrap();
    let c = candidate(&unrated).unwrap();
    assert_eq!(c.metadata.websites, ["https://www.dlsite.com/maniax/work/=/product_id/RJ01000003.html"]);
    assert_eq!(c.metadata.cover_wide_url.as_deref(), Some("https://img.dlsite.jp/test/unrated_main.jpg#heronite-extension"));

    // All-ages products keep their pictures with adult covers off, and missing site_id falls back to home.
    set_metadata(&s.paths, json!({}));
    let all_ages = s.svc.preview_game_metadata(&args("drago")).unwrap();
    assert!(candidate(&all_ages).unwrap().adult_pictures.is_empty());
    assert!(candidate(&all_ages).unwrap().metadata.cover_wide_url.is_some());
    let quirks = s.svc.preview_game_metadata(&args("all_ages_quirks")).unwrap();
    let c = candidate(&quirks).unwrap();
    assert!(c.adult_pictures.is_empty() && c.metadata.cover_wide_url.is_some());
    assert_eq!(c.metadata.websites, ["https://www.dlsite.com/home/work/=/product_id/RJ01000004.html"]);
    // Nothing tried to download a picture.
    assert_eq!(s.server.count(|r| r.path() != API && !r.path().starts_with("/lutris/")), 0);
}

#[test]
fn a_refusing_or_busy_dlsite_is_unreachable_not_a_failure() {
    let s = setup();
    set_metadata(&s.paths, json!({"sources": {"lutris": {"enabled": false}}}));
    for refuse in [403u16, 408, 429, 451, 503, 1, 2] {
        s.dlsite.refuse.store(refuse, Ordering::SeqCst);
        for _ in 0..3 {
            let preview = s.svc.preview_game_metadata(&args("drago")).unwrap();
            assert_eq!(state(&preview), SourceAnswer::Unreachable, "{refuse}");
        }
        // With DLsite the only online source asked, the fetch counts as offline and saves nothing.
        let err = s.svc.apply_game_metadata(&args("drago")).unwrap_err();
        assert!(is_offline(&err), "{refuse}: {err:#}");
    }
    assert!(s.svc.get_game_metadata("sideload", "drago").unwrap().is_none());
    // However often: never "Stopped after errors"; its test says what happened.
    assert_eq!(s.svc.sources().health("dlsite"), SourceHealth::Ok);
    assert_eq!(s.svc.source_status(&s.svc.metadata_settings(), "dlsite"), Some(SourceStatus::On));
    s.dlsite.refuse.store(403, Ordering::SeqCst);
    let test = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "dlsite").unwrap();
    assert!(!test.ok && test.error.as_deref().is_some_and(|e| e.contains("DLsite answered 403")), "{test:?}");
    // A 404 means the unofficial endpoint moved or disappeared: test must fail, not pass.
    s.dlsite.refuse.store(404, Ordering::SeqCst);
    let test = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "dlsite").unwrap();
    assert!(!test.ok && test.error.as_deref().is_some_and(|e| e.contains("DLsite answered 404")), "{test:?}");
    s.dlsite.refuse.store(0, Ordering::SeqCst);
    let test = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "dlsite").unwrap();
    assert!(test.ok, "{test:?}");
    let saved = s.svc.apply_game_metadata(&args("drago")).unwrap();
    assert_eq!(record::source_match(&saved, "dlsite").map(|m| m.id), Some("RJ345679".to_string()));
}
