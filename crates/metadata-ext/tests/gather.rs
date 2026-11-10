//! An extension through the real gather: found, not found, unsure (review),
//! a pinned id, search, adult pictures, fields it does not provide, and a
//! failing extension next to the others.

mod support;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::record;
use heronite_metadata::service::is_sent_to_review;
use heronite_metadata::sources::{SourceHealth, SourceStatus};
use heronite_metadata::*;
use heronite_metadata_ext::manifest::Manifest;
use heronite_metadata_ext::source::ScriptSource;
use heronite_metadata_ext::{ExtensionsExt, Limits};
use serde_json::{json, Value};
use support::{endpoints, manifest, Resp, Server};

/// (app name, title, Steam id).
const GAMES: [(&str, &str, Option<&str>); 5] = [
    ("portal2", "Portal 2", None),
    ("knights", "Portal Knights", None),
    ("nothing", "Nothing Here", None),
    ("steamy", "Some Steam Game", Some("620")),
    ("adult", "Adult Game", None),
];

/// Searches `api.example.org`, which answers the hits itself; reads a game
/// from `/game/<id>`.
const SCRIPT: &str = r#"
export async function find(game, ctx) {
  const r = await ctx.http.get("https://api.example.org/search", { query: { q: game.title, steam: game.ids.steam || "" } });
  if (r.status !== 200) throw new Error("search answered " + r.status);
  return r.json();
}
export async function fetch(id, ctx) {
  const r = await ctx.http.get("https://api.example.org/game/" + encodeURIComponent(id));
  if (r.status === 404) return null;
  return r.json();
}
"#;

fn record_of(id: &str) -> Value {
    match id {
        "p2" => json!({
            "title": "Portal 2", "description": "Think with portals.", "releaseDate": "2011-04-18",
            "developers": ["Valve"], "genres": ["Puzzle"],
            "images": [{ "kind": "cover", "url": "https://api.example.org/p2.jpg" }],
        }),
        "pk" => json!({ "title": "Portal Knights", "description": "Not Portal.", "releaseDate": "2017" }),
        "s620" => json!({ "title": "Portal 2", "description": "By the Steam id." }),
        "adult" => json!({
            "title": "Adult Game", "description": "Grown-ups only.",
            "images": [{ "kind": "cover", "url": "https://api.example.org/a.jpg", "adult": true }],
        }),
        "pinned" => json!({ "title": "The Pinned One", "description": "Chosen by hand." }),
        _ => Value::Null,
    }
}

fn mock(broken: Arc<AtomicBool>) -> Server {
    Server::start(move |req| {
        let path = req.path().to_owned();
        if broken.load(Ordering::SeqCst) && path.starts_with("/api.example.org") {
            return Resp::status(500);
        }
        if path == "/api.example.org/search" {
            if req.query("steam").as_deref() == Some("620") {
                return Resp::json(json!([{ "id": "s620", "title": "Portal 2", "matchedBy": { "id": "steam" } }]));
            }
            return Resp::json(match req.query("q").as_deref() {
                Some("Portal 2") => json!([{ "id": "p2", "title": "Portal 2", "year": 2011 }, { "id": "pk", "title": "Portal Knights" }]),
                Some("Portal Knights") => json!([{ "id": "pk", "title": "Portal Knights: Gold Throne", "year": 2017 }]),
                Some("Adult Game") => json!([{ "id": "adult", "title": "Adult Game" }]),
                Some("Portal") => json!([{ "id": "p2", "title": "Portal 2", "year": 2011, "thumb": "https://api.example.org/p2.jpg" }]),
                _ => json!([]),
            });
        }
        if let Some(id) = path.strip_prefix("/api.example.org/game/") {
            let r = record_of(id);
            return if r.is_null() { Resp::status(404) } else { Resp::json(r) };
        }
        // Built-in sources (Lutris) find nothing.
        if path == "/lutris/api/games" {
            return Resp::json(json!({"results": []}));
        }
        Resp::status(404)
    })
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
    broken: Arc<AtomicBool>,
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn source(id: &str, provides: &[&str], body: &str, script: &str, limits: Limits) -> ScriptSource {
    let m = Manifest::parse(&manifest(id, "1.0.0", "details", provides, body)).unwrap();
    ScriptSource::new(m, script, limits, None).unwrap()
}

const NETWORK: &str = "[network]\nhosts = [\"api.example.org\"]\nrate = \"10/s\"";

fn setup_with(provides: &[&str]) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
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
    let broken = Arc::new(AtomicBool::new(false));
    let server = mock(broken.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &["api.example.org"]));
    svc.add_source(Arc::new(source("exampledb", provides, NETWORK, SCRIPT, Limits::default()))).unwrap();
    Setup { _dir: dir, paths, server, svc, broken }
}

fn setup() -> Setup {
    setup_with(&["description", "releaseDate", "genres", "cover"])
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

fn state(preview: &MetadataPreview, id: &str) -> SourceAnswer {
    preview.sources.iter().find(|s| s.source == id).map(|s| s.answer.clone()).unwrap()
}

#[test]
fn a_game_is_found_and_saved_from_the_extension() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("portal2")).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Think with portals."));
    assert_eq!(saved.release_date.as_deref(), Some("2011-04-18"));
    assert_eq!(saved.genres, ["Puzzle"]);
    assert_eq!(
        saved.cover_url.as_deref(),
        Some("https://api.example.org/p2.jpg#heronite-extension"),
        "marked: downloaded from public addresses only"
    );
    for field in ["description", "releaseDate", "genres", "cover"] {
        assert_eq!(saved.field_sources[field], "exampledb", "{field}");
    }
    assert!(saved.developers.is_empty(), "developers are not in its provides");
    let m = record::source_match(&saved, "exampledb").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by, m.pinned), ("p2", Some(MatchedBy::ExactTitle), false));
    // Asked like a built-in: its row is in the per-game dialog's list.
    let preview = s.svc.preview_game_metadata(&args("portal2")).unwrap();
    assert!(matches!(state(&preview, "exampledb"), SourceAnswer::Found { used: true, .. }));
    // The settings order lists it (last, as a source added after the defaults).
    let order = s.svc.field_order(&s.svc.metadata_settings(), "description");
    assert_eq!(order.last().map(String::as_str), Some("exampledb"));
}

#[test]
fn a_game_it_does_not_have_is_not_found() {
    let s = setup();
    let preview = s.svc.preview_game_metadata(&args("nothing")).unwrap();
    assert_eq!(state(&preview, "exampledb"), SourceAnswer::NotFound);
    let saved = s.svc.apply_game_metadata(&args("nothing")).unwrap();
    assert!(saved.fetch_error.is_some() && saved.description.is_none());
}

#[test]
fn an_unsure_match_goes_to_review() {
    let s = setup();
    let e = s.svc.apply_game_metadata(&args("knights")).unwrap_err();
    assert!(is_sent_to_review(&e).is_some(), "{e:#}");
    assert!(s.svc.review_queue().iter().any(|i| i.app_name == "knights"));
    assert!(s.svc.get_game_metadata("sideload", "knights").unwrap().is_none(), "nothing saved");
}

#[test]
fn a_match_by_the_steam_id_is_sure() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("steamy")).unwrap();
    assert_eq!(saved.description.as_deref(), Some("By the Steam id."));
    assert_eq!(record::source_match(&saved, "exampledb").unwrap().matched_by, Some(MatchedBy::SteamId));
}

#[test]
fn a_pinned_game_is_read_by_its_id() {
    let s = setup();
    let mut a = args("knights");
    a.pins = Some([("exampledb".to_string(), Some("pinned".to_string()))].into_iter().collect());
    let saved = s.svc.apply_game_metadata(&a).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Chosen by hand."));
    let m = record::source_match(&saved, "exampledb").unwrap();
    assert!(m.pinned && m.id == "pinned");
    assert_eq!(s.server.count(|r| r.path() == "/api.example.org/search"), 0, "no search for a pinned game");
    // Next time too.
    s.svc.apply_game_metadata(&args("knights")).unwrap();
    assert_eq!(s.server.count(|r| r.path() == "/api.example.org/search"), 0);
    assert_eq!(s.server.count(|r| r.path() == "/api.example.org/game/pinned"), 2);
}

#[test]
fn it_is_searched_like_a_built_in() {
    let s = setup();
    let found = s.svc.search_sources("Portal", Some(&["exampledb".to_string()]));
    assert_eq!(found.asked, ["exampledb"]);
    let hit = &found.choices[0].hits[0];
    assert_eq!((hit.source.as_str(), hit.id.as_deref(), hit.title.as_deref()), ("exampledb", Some("p2"), Some("Portal 2")));
    assert_eq!(hit.thumb_url.as_deref(), Some("https://api.example.org/p2.jpg#heronite-extension"));
    let hits = s.svc.sources().search(&SourceContext::new(&s.paths, &s.svc.http), "exampledb", "Portal");
    assert!(matches!(hits, Lookup::Found(h) if h.len() == 1));
}

#[test]
fn an_adult_picture_is_dropped_unless_allowed() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("adult")).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Grown-ups only."));
    assert_eq!(saved.cover_url, None);
    write(&s.paths.collection_settings(), json!({"metadata": {"allowAdultCovers": true}}));
    let saved = s.svc.apply_game_metadata(&args("adult")).unwrap();
    assert_eq!(saved.cover_url.as_deref(), Some("https://api.example.org/a.jpg#heronite-extension"));
}

#[test]
fn fields_it_does_not_provide_are_discarded() {
    let s = setup_with(&["description"]);
    let saved = s.svc.apply_game_metadata(&args("portal2")).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Think with portals."));
    assert!(saved.release_date.is_none() && saved.genres.is_empty() && saved.cover_url.is_none());
    assert!(!saved.field_sources.contains_key("genres"));
}

#[test]
fn a_switched_off_extension_is_not_asked() {
    let s = setup();
    s.svc.set_source_enabled("exampledb", false).unwrap();
    let preview = s.svc.preview_game_metadata(&args("portal2")).unwrap();
    assert_eq!(state(&preview, "exampledb"), SourceAnswer::NotAsked { why: NotAsked::Off });
    assert_eq!(s.server.count(|r| r.path().starts_with("/api.example.org")), 0);
}

#[test]
fn a_failing_extension_stops_without_harming_the_others() {
    let s = setup();
    // A second extension that always throws.
    let thrower = source(
        "thrower",
        &["description", "genres"],
        "",
        "export function find() { throw new Error('boom'); }\nexport function fetch() {}",
        Limits::default(),
    );
    s.svc.add_source(Arc::new(thrower)).unwrap();
    for _ in 0..5 {
        let saved = s.svc.apply_game_metadata(&args("portal2")).unwrap();
        assert_eq!(saved.description.as_deref(), Some("Think with portals."), "the good one still answers");
    }
    let settings = s.svc.metadata_settings();
    assert_eq!(s.svc.source_status(&settings, "thrower"), Some(SourceStatus::Failing));
    assert!(matches!(s.svc.sources().health("thrower"), SourceHealth::Failing(why) if why.contains("boom")));
    let row = s.svc.source_rows(&settings).into_iter().find(|r| r.descriptor.id == "thrower").unwrap();
    assert!(row.problem.as_deref().is_some_and(|p| p.contains("boom")));
    let preview = s.svc.preview_game_metadata(&args("portal2")).unwrap();
    assert_eq!(state(&preview, "thrower"), SourceAnswer::NotAsked { why: NotAsked::Failing });
    // Switched on again, it is asked again.
    s.svc.set_source_enabled("thrower", true).unwrap();
    assert_eq!(s.svc.source_status(&s.svc.metadata_settings(), "thrower"), Some(SourceStatus::On));

    // A job of several games finishes whatever the extension does.
    s.broken.store(true, Ordering::SeqCst);
    let games: Vec<BulkItem> = ["portal2", "knights", "nothing"]
        .iter()
        .map(|g| {
            let a = args(g);
            BulkItem { app_name: a.app_name, runner: a.runner, title: a.title, steam_app_id: a.steam_app_id }
        })
        .collect();
    let progress = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(games), &|_| {});
    assert_eq!(progress.state, BulkState::Done);
    assert_eq!(progress.processed, 3);
}

/// `ctx.unreachable`: a script that sees its service busy or down ends the
/// call as unreachable, like a request that got no answer.
#[test]
fn a_busy_service_is_unreachable_and_never_stops_the_extension() {
    let s = setup();
    let script = |body: &str| format!("export function find(game, ctx) {{ {body} }}\nexport function fetch() {{}}");
    let add = |id: &str, body: &str| {
        let ext = source(id, &["description"], "", &script(body), Limits::default());
        s.svc.add_source(Arc::new(ext)).unwrap();
    };
    add("busy", r#"throw ctx.unreachable("The service is busy\n(429)");"#);
    add("quiet", "throw ctx.unreachable();");
    // An error of its own is a failure, even of the same words, and even
    // after it made one with `ctx.unreachable` that it did not throw.
    add("plain", r#"throw new Error("The service is busy (429)");"#);
    add("other", r#"ctx.unreachable("busy"); throw new Error("boom");"#);
    for _ in 0..6 {
        let preview = s.svc.preview_game_metadata(&args("portal2")).unwrap();
        assert_eq!(state(&preview, "busy"), SourceAnswer::Unreachable);
        assert_eq!(state(&preview, "quiet"), SourceAnswer::Unreachable);
        assert!(matches!(state(&preview, "exampledb"), SourceAnswer::Found { .. }));
    }
    let settings = s.svc.metadata_settings();
    for id in ["busy", "quiet"] {
        assert_eq!(s.svc.sources().health(id), SourceHealth::Ok, "{id}");
        assert_eq!(s.svc.source_status(&settings, id), Some(SourceStatus::On), "{id}");
    }
    for id in ["plain", "other"] {
        assert_eq!(s.svc.source_status(&settings, id), Some(SourceStatus::Failing), "{id}");
    }
    // A search says it could not be reached.
    let found = s.svc.search_sources("Portal", Some(&["busy".to_string()]));
    assert_eq!(found.unreachable, ["busy"]);
    // So does its test, with its words on one line.
    let test = source("tested", &["description"], "", "export function find() {}\nexport function fetch() {}\nexport function test(ctx) { throw ctx.unreachable(\"down for\\nmaintenance\"); }", Limits::default());
    s.svc.add_source(Arc::new(test)).unwrap();
    let result = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "tested").unwrap();
    assert_eq!(result.error.as_deref(), Some("unreachable: down for maintenance"));
}

#[test]
fn an_extension_that_runs_too_long_fails_alone() {
    let s = setup();
    let limits = Limits { time: std::time::Duration::from_millis(300), ..Limits::default() };
    let spinner = source("spinner", &["description"], "", "export function find() { while (true) {} }\nexport function fetch() {}", limits);
    s.svc.add_source(Arc::new(spinner)).unwrap();
    let preview = s.svc.preview_game_metadata(&args("portal2")).unwrap();
    assert_eq!(state(&preview, "spinner"), SourceAnswer::NotFound);
    assert!(matches!(state(&preview, "exampledb"), SourceAnswer::Found { .. }));
}

#[test]
fn the_same_extension_answers_several_games_at_once() {
    let s = setup();
    let apps = ["portal2", "steamy", "adult", "nothing"];
    std::thread::scope(|scope| {
        let handles: Vec<_> = apps.iter().map(|app| (app, scope.spawn(|| s.svc.preview_game_metadata(&args(app)).unwrap()))).collect();
        for (app, h) in handles {
            let preview = h.join().unwrap();
            let found = matches!(state(&preview, "exampledb"), SourceAnswer::Found { .. });
            assert_eq!(found, *app != "nothing", "{app}");
        }
    });
}

#[test]
fn a_built_in_id_cannot_be_taken() {
    let s = setup();
    let fake: SourceDescriptor = serde_json::from_value(json!({"id": "igdb", "name": "Fake", "kind": "details"})).unwrap();
    struct Fake(SourceDescriptor);
    impl MetadataSource for Fake {
        fn descriptor(&self) -> &SourceDescriptor {
            &self.0
        }
        fn lookup(&self, _: &SourceContext<'_>, _: &SourceGame) -> Lookup<MetadataCandidate> {
            Lookup::NotFound
        }
    }
    assert!(s.svc.add_source(Arc::new(Fake(fake))).is_err());
    assert_eq!(s.svc.sources().get("igdb").map(|d| d.name.as_str()), Some("IGDB"));
    assert!(!s.svc.remove_source("igdb"));
    assert!(s.svc.sources().get("igdb").is_some());
    // An extension is listed after the built-in sources, with its manifest's facts.
    let ids: Vec<String> = s.svc.sources().descriptors().map(|d| d.id.clone()).collect();
    assert_eq!(ids.last().map(String::as_str), Some("exampledb"));
    assert!(s.svc.installed_extensions().is_empty(), "added in memory, not installed");
}
