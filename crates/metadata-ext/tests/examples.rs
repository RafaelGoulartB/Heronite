//! The example extensions of `extensions/` against recorded answers, through
//! the real install and gather.

mod support;

use std::fs;
use std::path::Path;

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use heronite_metadata_ext::ExtensionsExt;
use serde_json::{json, Value};
use support::{endpoints, example, fixture, Req, Resp, Server};

const HOSTS: [&str; 3] = ["en.wikipedia.org", "pt.wikipedia.org", "www.pcgamingwiki.com"];

fn cargo_answer(req: &Req) -> Resp {
    let tables = req.query("tables").unwrap_or_default();
    let filter = req.query("where").unwrap_or_default();
    let file = if tables.contains("Input") && filter.contains("\"Portal 2\"") {
        "pcgamingwiki_fetch_portal2.json"
    } else if filter.contains("HOLDS \"620\"") {
        "pcgamingwiki_find_steam_620.json"
    } else if filter.contains("LIKE \"%Portal 2%\"") {
        "pcgamingwiki_find_title_portal2.json"
    } else {
        "pcgamingwiki_empty.json"
    };
    Resp::raw_json(fixture(file))
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
        "/www.pcgamingwiki.com/w/api.php" => cargo_answer(req),
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
    for name in ["wikipedia", "pcgamingwiki"] {
        svc.install_extension(&example(name), false).unwrap();
    }
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
    let pcgw = s.svc.sources().get("pcgamingwiki").cloned().unwrap();
    assert_eq!((pcgw.kind, pcgw.provides.clone()), (SourceKind::Extras, vec!["series".to_string(), "features".to_string()]));
    assert!(s.svc.sources().has_test("wikipedia") && !s.svc.sources().has_test("pcgamingwiki"));
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
fn pcgamingwiki_finds_a_game_by_its_steam_id() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("steam620", Some("620"))).unwrap();
    assert_eq!(saved.features, ["Full controller support"]);
    assert_eq!(saved.series.as_deref(), Some("Portal"));
    assert_eq!(saved.field_sources["features"], "pcgamingwiki");
    let m = heronite_metadata::record::source_match(&saved, "pcgamingwiki").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by), ("Portal 2", Some(MatchedBy::SteamId)));
    assert_eq!(s.server.count(|r| r.query("where").is_some_and(|w| w.contains("LIKE"))), 0, "no title search");
}

#[test]
fn pcgamingwiki_finds_a_game_by_its_name_otherwise() {
    let s = setup();
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    let game = SourceGame { app_name: "portal2".into(), runner: "sideload".into(), title: "Portal 2".into(), ..Default::default() };
    let source = s.svc.sources();
    let pcgw = source.lookups().find(|l| l.descriptor().id == "pcgamingwiki").unwrap();
    let Lookup::Found(c) = pcgw.lookup(&ctx, &game) else { panic!() };
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("Portal 2"), Some(MatchedBy::ExactTitle)));
    assert_eq!(c.metadata.features, ["Full controller support"]);
    assert!(c.metadata.description.is_none(), "it fills only its own fields");
}
