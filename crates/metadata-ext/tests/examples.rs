//! The example extensions of `extensions/metadata/` against recorded answers, and an
//! extras source of PC games written for the tests, through the real install
//! and gather.

mod support;

use std::fs;
use std::path::Path;

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use heronite_metadata_ext::ExtensionsExt;
use serde_json::{json, Value};
use support::{endpoints, example, fixture, pc_extras, Req, Resp, Server, PC_EXTRAS_HOST};

const HOSTS: [&str; 3] = ["en.wikipedia.org", "pt.wikipedia.org", PC_EXTRAS_HOST];

/// The service of [`pc_extras`]: Portal 2 by its Steam id or its name.
fn extras_answer(req: &Req) -> Resp {
    let portal2 = json!({"page": "Portal 2", "year": 2011});
    match (req.path().rsplit('/').next(), req.query("steam"), req.query("title"), req.query("page")) {
        (Some("find"), Some(steam), _, _) => Resp::json(if steam == "620" { json!([portal2]) } else { json!([]) }),
        (Some("find"), None, Some(title), _) if title == "Portal 2" => {
            Resp::json(json!([portal2, {"page": "Portal 2: Desolation", "year": 2024}]))
        }
        (Some("game"), _, _, Some(page)) if page == "Portal 2" => {
            Resp::json(json!([{"page": "Portal 2", "series": "Portal", "features": ["Full controller support"]}]))
        }
        _ => Resp::json(json!([])),
    }
}

fn mock() -> Server {
    Server::start(|req| match req.path() {
        "/en.wikipedia.org/w/rest.php/v1/search/title" => match req.query("q").as_deref() {
            Some("Portal 2") => Resp::raw_json(fixture("wikipedia_search_portal2.json")),
            Some("Portal") => Resp::raw_json(fixture("wikipedia_search_portal.json")),
            _ => Resp::raw_json(fixture("wikipedia_search_none.json")),
        },
        "/en.wikipedia.org/api/rest_v1/page/summary/Portal_2" => Resp::raw_json(fixture("wikipedia_summary_en_portal2.json")),
        "/pt.wikipedia.org/api/rest_v1/page/summary/Portal_2" => Resp::raw_json(fixture("wikipedia_summary_pt_portal2.json")),
        "/en.wikipedia.org/w/api.php" if req.query("lllang").as_deref() == Some("pt") => {
            Resp::raw_json(fixture("wikipedia_langlinks_pt_portal2.json"))
        }
        p if p.starts_with(&format!("/{PC_EXTRAS_HOST}/")) => extras_answer(req),
        "/lutris/api/games" => Resp::json(json!({"results": []})),
        _ => Resp::status(404),
    })
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let games = json!({
        "portal2": {"appName": "portal2", "runner": "sideload", "title": "Portal 2", "playniteId": "", "source": "manual", "launchKind": "executable"},
        "steam620": {"appName": "steam620", "runner": "sideload", "title": "Portal 2", "steamAppId": "620", "playniteId": "", "source": "steam", "launchKind": "steam-uri"},
    });
    write(&paths.local_meta(), json!({ "games": games }));
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let server = mock();
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &HOSTS));
    svc.install_extension(&example("wikipedia"), false).unwrap();
    svc.install_extension(&pc_extras(&dir.path().join("packages")), false).unwrap();
    Setup { _dir: dir, paths, server, svc }
}

fn args(app: &str, steam: Option<&str>) -> MetadataApplyArgs {
    MetadataApplyArgs {
        app_name: app.into(),
        runner: "sideload".into(),
        title: "Portal 2".into(),
        steam_app_id: steam.map(Into::into),
        ..Default::default()
    }
}

#[test]
fn the_examples_install_as_written() {
    let s = setup();
    let wiki = s.svc.sources().get("wikipedia").cloned().unwrap();
    assert!(wiki.is_fallback() && wiki.kind == SourceKind::Details);
    assert_eq!(wiki.provides, ["description", "releaseDate"]);
    let extras = s.svc.sources().get("pcextras").cloned().unwrap();
    assert_eq!((extras.kind, extras.provides.clone()), (SourceKind::Extras, vec!["series".to_string(), "features".to_string()]));
    assert_eq!(extras.applies_to, sources::AppliesTo::Pc);
    assert!(s.svc.sources().has_test("wikipedia") && !s.svc.sources().has_test("pcextras"));
    let info = s.svc.inspect_extension(&example("wikipedia")).unwrap();
    assert_eq!(info.hosts, ["*.wikipedia.org"]);
    assert!(info.readme.is_some_and(|r| r.starts_with("# Wikipedia")));
}

#[test]
fn wikipedia_gives_the_lead_when_no_other_source_has_the_game() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("portal2", None)).unwrap();
    assert!(saved.description.as_deref().unwrap().starts_with("Portal 2 is a 2011 first person puzzle-platform game"));
    assert_eq!(saved.release_date.as_deref(), Some("2011"));
    assert_eq!(saved.field_sources["description"], "wikipedia");
    let m = heronite_metadata::record::source_match(&saved, "wikipedia").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by), ("Portal_2", Some(MatchedBy::ExactTitle)));
    // Every request says who it is.
    let req = s.server.log.lock().iter().find(|r| r.path().starts_with("/en.wikipedia.org")).cloned().unwrap();
    assert!(req.header("user-agent").is_some_and(|ua| ua.starts_with("Heronite/")));
}

#[test]
fn wikipedia_speaks_the_interface_language() {
    let s = setup();
    let mut a = args("portal2", None);
    a.language = Some("pt-BR".into());
    let saved = s.svc.apply_game_metadata(&a).unwrap();
    assert!(saved.description.as_deref().unwrap().starts_with("Portal 2 é um jogo eletrônico"), "{:?}", saved.description);
    assert_eq!(s.server.count(|r| r.path() == "/pt.wikipedia.org/api/rest_v1/page/summary/Portal_2"), 1);
}

#[test]
fn wikipedia_is_searched_by_name() {
    let s = setup();
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    let Lookup::Found(hits) = s.svc.sources().search(&ctx, "wikipedia", "Portal") else { panic!() };
    let ids: Vec<&str> = hits.iter().filter_map(|h| h.id.as_deref()).collect();
    assert_eq!(ids, ["Portal_(video_game)", "Portal_2"], "games only, the same name first");
    assert_eq!((hits[0].title.as_deref(), hits[0].year), (Some("Portal"), Some(2007)));
    let none = s.svc.sources().search(&ctx, "wikipedia", "Qwzxv Nonexistent Game");
    assert!(matches!(none, Lookup::Found(h) if h.is_empty()));
}

#[test]
fn an_extras_source_finds_a_game_by_its_steam_id() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("steam620", Some("620"))).unwrap();
    assert_eq!(saved.features, ["Full controller support"]);
    assert_eq!(saved.series.as_deref(), Some("Portal"));
    assert_eq!(saved.field_sources["features"], "pcextras");
    let m = heronite_metadata::record::source_match(&saved, "pcextras").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by), ("Portal 2", Some(MatchedBy::SteamId)));
    assert_eq!(s.server.count(|r| r.path().ends_with("/find") && r.query("title").is_some()), 0, "no title search");
}

#[test]
fn an_extras_source_finds_a_game_by_its_name_otherwise() {
    let s = setup();
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    let game = SourceGame { app_name: "portal2".into(), runner: "sideload".into(), title: "Portal 2".into(), ..Default::default() };
    let source = s.svc.sources();
    let extras = source.lookups().find(|l| l.descriptor().id == "pcextras").unwrap();
    let Lookup::Found(c) = extras.lookup(&ctx, &game) else { panic!() };
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("Portal 2"), Some(MatchedBy::ExactTitle)));
    assert_eq!(c.metadata.features, ["Full controller support"]);
    assert!(c.metadata.description.is_none(), "it fills only its own fields");
}
