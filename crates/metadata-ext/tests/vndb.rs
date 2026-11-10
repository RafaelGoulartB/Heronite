//! The VNDB example (`extensions/metadata/vndb`) through the real install, gather,
//! review and jobs, against recorded VNDB answers: what it fills, how sure
//! each match is, when it is asked (only as a fallback), adult covers, a
//! busy VNDB (429, server errors) and no network.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::record::{self, SourceMatch};
use heronite_metadata::service::{is_offline, is_sent_to_review};
use heronite_metadata::sources::{OptionValue, SourceHealth, SourceStatus};
use heronite_metadata::*;
use heronite_metadata_ext::manifest::Manifest;
use heronite_metadata_ext::ExtensionsExt;
use serde_json::{json, Value};
use support::{endpoints, example, fixture, Req, Resp, Server};

const API: &str = "/api.vndb.org/kana/vn";

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

/// A recorded answer of `heronite-metadata`'s fixtures (Steam, Lutris).
fn metadata_fixture(name: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../metadata/tests/fixtures").join(name)).unwrap()
}

/// Entries written for these tests in the shape of VNDB's answers: the
/// cases the recordings lack (a partial date, formatting codes, every kind
/// of tag, screenshots of every shape and vote).
fn written(id: &str) -> Option<Value> {
    let shot = |url: &str, w: u32, h: u32, sexual: f64, votes: u32| json!({"url": format!("https://t.vndb.org/sf/{url}.jpg"), "dims": [w, h], "sexual": sexual, "violence": 0, "votecount": votes});
    let tag = |name: &str, rating: f64, spoiler: u8, lie: bool, category: &str| json!({"name": name, "rating": rating, "spoiler": spoiler, "lie": lie, "category": category});
    Some(match id {
        "v1" => json!({
            "id": "v1", "title": "Codes", "released": "2012-03", "rating": 71.4,
            "description": "[b]Bold[/b] and [i]it[/i]. [spoiler]She dies.[/spoiler]Read [url=https://x.y/]more[/url].\n\n\n\n[From [url=/v17]VNDB[/url]]",
            "tags": [
                tag("Mystery", 2.5, 0, false, "cont"), tag("Twist", 2.9, 2, false, "cont"), tag("Fake", 2.9, 0, true, "cont"),
                tag("Sexual", 2.9, 0, false, "ero"), tag("ADV", 2.9, 0, false, "tech"), tag("Weak", 1.5, 0, false, "cont"),
                tag("Drama", 2.8, 0, false, "cont"),
            ],
            "screenshots": [shot("tall", 600, 900, 0.0, 9), shot("hot", 1280, 720, 1.8, 30), shot("ok", 1280, 720, 0.2, 12)],
        }),
        // Only an explicit landscape screenshot, and no date yet.
        "v2" => json!({"id": "v2", "title": "Hot", "released": "TBA", "screenshots": [shot("hot2", 1280, 720, 1.8, 30)]}),
        // A portrait is no background; an image nobody voted on is not safe.
        "v3" => json!({"id": "v3", "title": "Tall", "released": "2012",
                       "image": {"url": "https://t.vndb.org/cv/03/3.jpg", "sexual": 0, "violence": 0, "votecount": 0},
                       "screenshots": [shot("tall3", 600, 900, 0.0, 9)]}),
        "v17" => json!({"id": "v17", "title": "Ever17 -the out of infinity-"}),
        _ => return None,
    })
}

/// Every recorded entry by id, the fullest recording of each (the title
/// searches were recorded with every field).
static RECORDED: LazyLock<Vec<Value>> = LazyLock::new(|| {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<Value> = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files.iter().filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("vndb_")) {
        let body: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        for entry in body["results"].as_array().unwrap() {
            let fuller = entry.get("description").is_some();
            match out.iter().position(|e| e["id"] == entry["id"]) {
                Some(i) if fuller && out[i].get("description").is_none() => out[i] = entry.clone(),
                Some(_) => {}
                None => out.push(entry.clone()),
            }
        }
    }
    out
});

fn body(req: &Req) -> Value {
    serde_json::from_str(&req.body).unwrap_or(Value::Null)
}

/// The answer to a `POST /vn`, as VNDB recorded it.
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
        ("search", "Midnight Seminar") => "vndb_id_v9.json",
        ("id", id) => {
            let entry = written(id).or_else(|| RECORDED.iter().find(|e| e["id"] == id).cloned());
            return Resp::json(json!({"results": entry.into_iter().collect::<Vec<_>>(), "more": false}));
        }
        _ => return Resp::json(json!({"results": [], "more": false})),
    };
    Resp::raw_json(fixture(file))
}

/// How VNDB answers: as recorded, or busy.
#[derive(Default)]
struct Vndb {
    /// That many requests are answered 429 first.
    throttle: AtomicUsize,
    /// Every request is answered 503.
    down: AtomicBool,
}

/// VNDB; Steam's app details for any id; Lutris has Portal and a
/// "Fate/stay night" whose genre is Visual Novel; nothing else.
fn mock(vndb: Arc<Vndb>) -> Server {
    Server::start(move |req| match req.path() {
        API => {
            if vndb.down.load(Ordering::SeqCst) {
                return Resp::status(503);
            }
            if vndb.throttle.load(Ordering::SeqCst) > 0 {
                vndb.throttle.fetch_sub(1, Ordering::SeqCst);
                return Resp::status(429).with_header("Retry-After", "0");
            }
            vndb_answer(req)
        }
        "/steam/api/appdetails" => {
            let id = req.query("appids").unwrap_or_default();
            let all: Value = serde_json::from_str(&metadata_fixture("steam_appdetails_400.json")).unwrap();
            Resp::json(json!({ id: all["400"].clone() }))
        }
        "/lutris/api/games" => Resp::json(match req.query("search").as_deref() {
            Some("Portal") => serde_json::from_str::<Value>(&metadata_fixture("lutris_search_portal.json")).unwrap(),
            Some("Fate/stay night") => json!({"results": [{"name": "Fate/stay night", "slug": "fsn", "year": 2004}]}),
            Some("Midnight Seminar") => json!({"results": [{"name": "Midnight Seminar", "slug": "midnight-seminar", "year": 2000}]}),
            _ => json!({"results": []}),
        }),
        "/lutris/api/games/portal" => Resp::raw_json(metadata_fixture("lutris_detail_portal.json")),
        "/lutris/api/games/fsn" => Resp::json(
            json!({"name": "Fate/stay night", "slug": "fsn", "year": 2004, "genres": [{"name": "Visual Novel"}], "coverart": "https://lutris/fsn.jpg"}),
        ),
        "/lutris/api/games/midnight-seminar" => {
            Resp::json(json!({"name": "Midnight Seminar", "slug": "midnight-seminar", "year": 2000, "coverart": "https://lutris/ms.jpg"}))
        }
        _ => Resp::status(404),
    })
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

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

/// The extension as published, but at 10 requests a second: its own rate
/// (36 a minute, checked by `it_installs_as_written`) would make these tests
/// wait over a second and a half between requests.
fn fast_copy(parent: &Path) -> PathBuf {
    let dir = parent.join("vndb");
    fs::create_dir_all(&dir).unwrap();
    let manifest = fs::read_to_string(example("vndb").join("source.toml")).unwrap();
    assert!(manifest.contains("rate = \"36/min\""));
    fs::write(dir.join("source.toml"), manifest.replace("rate = \"36/min\"", "rate = \"10/s\"")).unwrap();
    fs::copy(example("vndb").join("source.js"), dir.join("source.js")).unwrap();
    dir
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
    vndb: Arc<Vndb>,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(&dir.path().join("data"));
    let vndb = Arc::new(Vndb::default());
    let server = mock(vndb.clone());
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &["api.vndb.org"]));
    svc.install_extension(&fast_copy(dir.path()), false).unwrap();
    Setup { _dir: dir, paths, server, svc, vndb }
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

/// VNDB requests whose filter names `what` (a title or an id).
fn vndb_asked(server: &Server, what: &str) -> usize {
    server.count(|r| r.path() == API && body(r)["filters"][2].as_str() == Some(what))
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

/// VNDB's entry `id` read by the extension (as a pinned game is).
fn by_id(s: &Setup, id: &str) -> MetadataCandidate {
    let game = SourceGame { app_name: "x".into(), runner: "sideload".into(), title: "x".into(), ..Default::default() };
    match s.svc.sources().lookup_id(&SourceContext::new(&s.paths, &s.svc.http), "vndb", &game, id) {
        Lookup::Found(c) => c,
        other => panic!("{id}: {other:?}"),
    }
}

#[test]
fn it_installs_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let svc = MetadataService::new(make_root(dir.path()));
    let info = svc.inspect_extension(&example("vndb")).unwrap();
    assert_eq!((info.hosts.clone(), info.image_hosts.clone()), (vec!["api.vndb.org".to_string()], vec!["t.vndb.org".to_string()]));
    assert!(info.readme.as_deref().is_some_and(|r| r.starts_with("# VNDB")));
    assert_eq!(info.rate, "36/min");
    let manifest = Manifest::parse(&fs::read_to_string(example("vndb").join("source.toml")).unwrap()).unwrap();
    assert_eq!(manifest.network.rate.interval(), std::time::Duration::from_secs(60) / 36, "under VNDB's 200 requests per 5 minutes");
    svc.install_extension(&example("vndb"), false).unwrap();
    let d = svc.sources().get("vndb").cloned().unwrap();
    assert!(d.is_fallback() && d.kind == SourceKind::Details && d.searchable && !d.extension.as_ref().unwrap().adult);
    assert_eq!(d.provides, ["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"]);
    let kind = d.game_kind.as_ref().unwrap();
    assert_eq!((kind.name.as_str(), kind.signals.clone()), ("Visual novels", vec!["Visual Novel".to_string()]));
    assert!(svc.sources().has_test("vndb"));
    // Its settings, with their defaults, in its row.
    let row = svc.source_rows(&svc.metadata_settings()).into_iter().find(|r| r.descriptor.id == "vndb").unwrap();
    assert_eq!(row.status, SourceStatus::On);
    let options: Vec<(&str, &OptionValue)> = row.options.iter().map(|o| (o.option.key.as_str(), &o.value)).collect();
    assert_eq!(options, [("titleOrder", &OptionValue::Text("latin".into())), ("screenshotBackground", &OptionValue::Switch(true))]);
}

#[test]
fn a_game_no_other_source_has_is_found_by_its_title() {
    let s = setup();
    let saved = s.svc.apply_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1, "one search");
    assert_eq!(vndb_asked(&s.server, "v2002"), 1, "then the entry by its id");
    assert_eq!(saved.release_date.as_deref(), Some("2009-10-15"));
    assert_eq!(saved.developers, ["NITRO PLUS", "MAGES.", "MAGES. Inc."]);
    assert_eq!(saved.genres, ["Visual Novel"]);
    assert_eq!(saved.critic_score.map(|n| n.get()), Some(90.0), "VNDB's 10..=100 rating, rounded");
    // Plain text: no codes, the source note kept.
    let text = saved.description.as_deref().unwrap();
    assert!(text.starts_with("-Decide The Fate Of All Mankind- CAN YOU CHANGE"), "{text}");
    assert!(text.ends_with("[From Steam]") && !text.contains("[b]") && !text.contains("[url"), "{text}");
    // Themes: content tags only, no spoilers, highest first, at most 12.
    assert_eq!(saved.themes.len(), 12);
    assert_eq!(saved.themes[0], "Chuunibyou Protagonist");
    assert!(!saved.themes.iter().any(|t| t == "Time Travel"), "a minor spoiler");
    assert!(!saved.themes.iter().any(|t| t == "One True End" || t == "ADV"), "technical tags");
    assert_eq!(saved.cover_url.as_deref(), Some("https://t.vndb.org/cv/19/77819.jpg#heronite-extension"));
    assert!(saved.hero_url.as_deref().is_some_and(|u| u.starts_with("https://t.vndb.org/sf/")));
    for field in ["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"] {
        assert_eq!(saved.field_sources[field], "vndb", "{field}");
    }
    assert!(saved.publishers.is_empty() && saved.platforms.is_empty() && saved.series.is_none() && saved.age_ratings.is_empty());
    let m = record::source_match(&saved, "vndb").unwrap();
    assert_eq!((m.id.as_str(), m.matched_by, m.pinned), ("v2002", Some(MatchedBy::ExactTitle), false));

    // It says who it is, and nothing about the user.
    let req = s.server.log.lock().iter().find(|r| r.path() == API).cloned().unwrap();
    assert!(req.header("user-agent").is_some_and(|ua| ua.starts_with("Heronite/")), "{:?}", req.header("user-agent"));
    assert_eq!(req.method, "POST");
    assert_eq!((body(&req)["sort"].as_str(), body(&req)["results"].as_u64()), (Some("searchrank"), Some(20)));

    // Found again by its id next time: no title search.
    s.svc.apply_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1);
    assert_eq!(vndb_asked(&s.server, "v2002"), 2);
    let again = s.svc.get_game_metadata("sideload", "sg").unwrap().unwrap();
    assert_eq!(record::source_match(&again, "vndb").unwrap().matched_by, Some(MatchedBy::ExactTitle), "how it was first found");
}

#[test]
fn every_field_is_read_the_way_vndb_writes_it() {
    let s = setup();
    let c = by_id(&s, "v1");
    assert_eq!(c.matched_by, Some(MatchedBy::SourceId));
    let md = &c.metadata;
    assert_eq!(md.release_date.as_deref(), Some("2012-03"), "a partial date is kept");
    assert_eq!(md.description.as_deref(), Some("Bold and it. Read more. [From VNDB]"), "codes and spoilers out");
    assert_eq!(md.themes, ["Drama", "Mystery"], "no spoiler, lie, sexual, technical or weak tag");
    assert_eq!(md.critic_score.map(|n| n.get()), Some(71.0));
    assert_eq!(md.hero_url.as_deref(), Some("https://t.vndb.org/sf/ok.jpg#heronite-extension"), "the safe landscape one");
    assert!(c.adult_pictures.is_empty() && md.cover_url.is_none());
    // Only an explicit landscape: kept in the candidate, flagged, and so
    // dropped unless adult covers are allowed. No date yet: none.
    set_metadata(&s.paths, json!({"allowAdultCovers": true}));
    let hot = by_id(&s, "v2");
    assert_eq!(
        (hot.metadata.hero_url.as_deref(), hot.adult_pictures.clone()),
        (Some("https://t.vndb.org/sf/hot2.jpg#heronite-extension"), vec!["hero".to_string()])
    );
    assert_eq!(hot.metadata.release_date, None);
    // A portrait is no background; a cover nobody voted on is not called safe.
    let tall = by_id(&s, "v3");
    assert_eq!(tall.metadata.hero_url, None);
    assert_eq!(tall.adult_pictures, ["cover"]);
    assert_eq!(tall.metadata.release_date.as_deref(), Some("2012"));
    // No screenshot background when the setting is off.
    set_metadata(&s.paths, json!({"sources": {"vndb": {"options": {"screenshotBackground": false}}}}));
    assert_eq!(by_id(&s, "v1").metadata.hero_url, None);
    assert!(by_id(&s, "v2002").metadata.hero_url.is_none());
}

#[test]
fn japanese_and_romaji_titles_and_aliases_match_exactly_a_close_one_waits_for_review() {
    let s = setup();
    // The Japanese name is one of VNDB's aliases of the entry.
    let preview = s.svc.preview_game_metadata(&args("sgja")).unwrap();
    let c = vndb_candidate(&preview).unwrap();
    assert_eq!(
        (c.match_id.as_deref(), c.matched_by, c.match_name.as_deref()),
        (Some("v2002"), Some(MatchedBy::ExactTitle), Some("STEINS;GATE"))
    );
    let preview = s.svc.preview_game_metadata(&args("clannad")).unwrap();
    assert_eq!(vndb_candidate(&preview).map(|c| (c.match_id.as_deref(), c.matched_by)), Some((Some("v4"), Some(MatchedBy::ExactTitle))));
    let saya = s.svc.preview_game_metadata(&args("saya")).unwrap();
    assert_eq!(vndb_candidate(&saya).and_then(|c| c.match_name.as_deref()), Some("Saya no Uta"));
    // The title setting names the match, not the game.
    set_metadata(&s.paths, json!({"sources": {"vndb": {"options": {"titleOrder": "original"}}}}));
    let saya = s.svc.preview_game_metadata(&args("saya")).unwrap();
    let c = vndb_candidate(&saya).unwrap();
    assert_eq!((c.match_name.as_deref(), c.matched_by), (Some("沙耶の唄"), Some(MatchedBy::FuzzyTitle)), "only close to an alias: a guess");
    // A lone guess is not saved.
    let err = s.svc.apply_game_metadata(&args("saya")).unwrap_err();
    assert!(is_sent_to_review(&err).is_some(), "{err:#}");
    assert!(s.svc.review_queue().iter().any(|q| q.app_name == "saya"));
    assert!(s.svc.get_game_metadata("sideload", "saya").unwrap().is_none_or(|md| md.title.as_deref() != Some("沙耶の唄")));
    // A name that is no entry's.
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    let game = SourceGame { app_name: "p".into(), runner: "sideload".into(), title: "Portal 2".into(), ..Default::default() };
    let vndb = s.svc.sources();
    let source = vndb.lookups().find(|l| l.descriptor().id == "vndb").unwrap();
    assert!(matches!(source.lookup(&ctx, &game), Lookup::NotFound));
}

#[test]
fn a_typed_id_or_address_finds_that_entry_and_a_name_finds_several() {
    let s = setup();
    let only = ["vndb".to_string()];
    for query in ["v2002", "https://vndb.org/v2002", "vndb.org/v2002/chars#x", " V2002 "] {
        let found = s.svc.search_sources(query, Some(&only));
        assert_eq!(found.asked, ["vndb"]);
        let hits: Vec<&SourceHit> = found.choices.iter().flat_map(|c| &c.hits).collect();
        assert_eq!(hits.len(), 1, "{query}");
        assert_eq!((hits[0].id.as_deref(), hits[0].title.as_deref()), (Some("v2002"), Some("STEINS;GATE")));
    }
    assert_eq!(vndb_asked(&s.server, "v2002"), 4);
    // A bare number, another kind of VNDB address, a title: searched by name.
    for query in ["1942", "https://vndb.org/r12"] {
        s.svc.search_sources(query, Some(&only));
        assert_eq!(
            s.server.count(|r| r.path() == API && body(r)["filters"][0] == "search" && body(r)["filters"][2] == query),
            1,
            "{query}"
        );
    }
    // A search by name: several entries, with the original title beside.
    let fate = s.svc.search_sources("Fate", Some(&only));
    assert!(fate.choices.len() > 10, "{}", fate.choices.len());
    let extra = fate.choices.iter().find(|c| c.title.as_deref() == Some("Fate/EXTRA")).unwrap();
    assert_eq!((extra.alt_title.as_deref(), extra.year), (Some("フェイト/エクストラ"), Some(2010)));
    let hit = &extra.hits[0];
    assert!(hit.platform.is_some() && hit.thumb_url.as_deref().is_some_and(|u| u.starts_with("https://t.vndb.org/")));
    // Entries without a date have no year; a year alone is one.
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    set_metadata(&s.paths, json!({"allowAdultCovers": true}));
    let Lookup::Found(hits) = s.svc.sources().search(&ctx, "vndb", "Fate") else { panic!() };
    let year = |id: &str| hits.iter().find(|h| h.id.as_deref() == Some(id)).map(|h| h.year);
    assert_eq!((year("v55735"), year("v12779")), (Some(Some(2012)), Some(None)));
    // Explicit thumbnails are flagged, the others not.
    assert!(hits.iter().any(|h| h.thumb_adult) && hits.iter().any(|h| !h.thumb_adult && h.thumb_url.is_some()));
}

#[test]
fn a_title_that_looks_like_an_id_is_an_id_only_in_a_search() {
    let s = setup();
    let ids = || s.server.count(|r| r.path() == API && body(r)["filters"][0] == "id");
    // A game titled "V2002", looked up by itself: searched by its name.
    let ctx = SourceContext::new(&s.paths, &s.svc.http);
    let game = SourceGame { app_name: "v".into(), runner: "sideload".into(), title: "V2002".into(), ..Default::default() };
    let sources = s.svc.sources();
    let vndb = sources.lookups().find(|l| l.descriptor().id == "vndb").unwrap();
    assert!(matches!(vndb.lookup(&ctx, &game), Lookup::NotFound));
    assert_eq!(ids(), 0, "never read as VNDB's entry v2002");
    assert_eq!(s.server.count(|r| r.path() == API && body(r)["filters"][0] == "search" && body(r)["filters"][2] == "V2002"), 1);
    // Typed in a search, it is the entry.
    let Lookup::Found(hits) = sources.search(&ctx, "vndb", "V2002") else { panic!() };
    assert_eq!(hits.iter().filter_map(|h| h.id.as_deref()).collect::<Vec<_>>(), ["v2002"]);
    assert_eq!(ids(), 1);
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
    assert_eq!(s.server.count(|r| r.path() == API), 0);

    // No other source found it: asked.
    s.svc.preview_game_metadata(&args("sg")).unwrap();
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 1);

    // Another source says it is a visual novel: asked after it.
    let fsn = s.svc.preview_game_metadata(&args("fsn")).unwrap();
    assert_eq!(fsn.candidates.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["lutris", "vndb"]);
    assert_eq!(vndb_asked(&s.server, "Fate/stay night"), 1);
    assert_eq!(vndb_candidate(&fsn).map(|c| (c.match_id.as_deref(), c.matched_by)), Some((Some("v11"), Some(MatchedBy::ExactTitle))));

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

    // A search by hand asks it like any searchable source.
    let found = s.svc.search_sources("Portal", None);
    assert!(found.asked.contains(&"vndb".to_string()), "{:?}", found.asked);
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
    assert!(found.choices[0].thumb_url.is_none() && found.choices[0].hits[0].thumb_url.is_none(), "no thumbnail");
    // Its other fields still come, and the safe screenshot is the background.
    assert_eq!(off.proposed.field_sources["description"], "vndb");
    assert_eq!(off.proposed.field_sources["hero"], "vndb");

    set_metadata(&s.paths, json!({"fieldPriority": order, "allowAdultCovers": true}));
    let on = s.svc.preview_game_metadata(&args("ms")).unwrap();
    assert_eq!(on.proposed.cover_url.as_deref(), Some("https://t.vndb.org/cv/00/1.jpg#heronite-extension"));
    assert_eq!(on.proposed.field_sources["cover"], "vndb");
    let found = s.svc.search_sources("Midnight Seminar", Some(&["vndb".to_string()]));
    assert_eq!(found.choices[0].thumb_url.as_deref(), Some("https://t.vndb.org/cv.t/00/1.jpg#heronite-extension"));
    // Nothing tried to download a picture.
    assert_eq!(s.server.count(|r| r.path() != API && !r.path().starts_with("/lutris/")), 0);
}

#[test]
fn a_429_is_tried_once_more_and_one_that_lasts_is_unreachable_not_a_failure() {
    let s = setup();
    s.vndb.throttle.store(1, Ordering::SeqCst);
    let preview = s.svc.preview_game_metadata(&args("sg")).unwrap();
    assert!(vndb_candidate(&preview).is_some());
    assert_eq!(vndb_asked(&s.server, "Steins;Gate"), 2, "refused once, then answered");

    // Refused every time: with VNDB the only online source asked, the
    // fetch counts as offline and saves nothing.
    set_metadata(&s.paths, json!({"sources": {"lutris": {"enabled": false}}}));
    s.vndb.throttle.store(usize::MAX / 2, Ordering::SeqCst);
    let err = s.svc.apply_game_metadata(&args("clannad")).unwrap_err();
    assert!(is_offline(&err), "{err:#}");
    assert!(s.svc.get_game_metadata("sideload", "clannad").unwrap().is_none());
    assert_eq!(vndb_asked(&s.server, "CLANNAD"), 2, "the request and one more try");
    // However often: never "Stopped after errors".
    for _ in 0..6 {
        let preview = s.svc.preview_game_metadata(&args("clannad")).unwrap();
        assert_eq!(vndb_state(&preview), SourceAnswer::Unreachable);
    }
    assert_eq!(s.svc.sources().health("vndb"), SourceHealth::Ok);
    assert_eq!(s.svc.source_status(&s.svc.metadata_settings(), "vndb"), Some(SourceStatus::On));
    // A server error too; its test says what happened.
    s.vndb.throttle.store(0, Ordering::SeqCst);
    s.vndb.down.store(true, Ordering::SeqCst);
    for _ in 0..6 {
        assert_eq!(vndb_state(&s.svc.preview_game_metadata(&args("clannad")).unwrap()), SourceAnswer::Unreachable);
    }
    assert_eq!(s.svc.sources().health("vndb"), SourceHealth::Ok);
    let test = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "vndb").unwrap();
    assert!(!test.ok && test.error.as_deref().is_some_and(|e| e.contains("VNDB answered 503")), "{test:?}");
    s.vndb.down.store(false, Ordering::SeqCst);
    let test = s.svc.sources().test(&SourceContext::new(&s.paths, &s.svc.http), "vndb").unwrap();
    assert!(test.ok, "{test:?}");
}

/// VNDB throttled while the others answer that they have no such game:
/// nothing is marked as found in no source, the job goes on and says the
/// source did not answer, and the next pass asks again.
#[test]
fn a_throttled_vndb_marks_nothing_as_found_in_no_source() {
    let s = setup();
    s.vndb.throttle.store(usize::MAX / 2, Ordering::SeqCst);
    let err = s.svc.apply_game_metadata(&args("clannad")).unwrap_err();
    assert!(!is_offline(&err), "Lutris answered: {err:#}");
    assert!(s.svc.get_game_metadata("sideload", "clannad").unwrap().is_none(), "not marked as found in no source");

    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![item("clannad"), item("saya"), item("portal")]), &|_| {});
    let outcomes: Vec<&GameOutcome> = done.results.iter().map(|r| &r.outcome).collect();
    assert_eq!(outcomes, [&GameOutcome::Offline, &GameOutcome::Offline, &GameOutcome::Updated], "Portal is Lutris's");
    for app in ["clannad", "saya"] {
        assert!(s.svc.get_game_metadata("sideload", app).unwrap().is_none(), "{app}");
    }
    // The idle fill: left as they were, and asked again once VNDB answers.
    s.svc.refresh_missing_collection_metadata().unwrap();
    assert!(s.svc.get_game_metadata("sideload", "clannad").unwrap().is_none());
    s.vndb.throttle.store(0, Ordering::SeqCst);
    s.svc.refresh_missing_collection_metadata().unwrap();
    let clannad = s.svc.get_game_metadata("sideload", "clannad").unwrap().unwrap();
    assert!(clannad.fetch_error.is_none() && clannad.field_sources["description"] == "vndb");
}

#[test]
fn offline_it_is_not_asked_after_the_others_and_a_pinned_game_stays_offline() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(&dir.path().join("data"));
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
        origins: vec![("https://api.vndb.org".into(), format!("{base}/api.vndb.org"))],
        ..Endpoints::default()
    };
    let svc = MetadataService::with_endpoints(paths.clone(), dead);
    svc.install_extension(&fast_copy(dir.path()), false).unwrap();
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
    assert!(is_offline(&svc.apply_game_metadata(&args("sg")).unwrap_err()));
    assert_eq!(svc.sources().health("vndb"), SourceHealth::Ok, "no network is no failure");
}

/// What a settings file says of `vndb` (its order in a field, its settings,
/// a game's match) applies to the extension once it is installed.
#[test]
fn what_the_settings_say_of_vndb_applies_to_the_extension() {
    let dir = tempfile::tempdir().unwrap();
    let paths = make_root(&dir.path().join("data"));
    let vndb = Arc::new(Vndb::default());
    let server = mock(vndb);
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &["api.vndb.org"]));
    set_metadata(
        &paths,
        json!({
            // The defaults a build with VNDB built in saved, and a list the user set.
            "fieldPriority": {"themes": ["igdb", "playnite", "vndb"], "hero": ["igdb", "steam", "lutris", "playnite", "vndb", "launchbox"],
                              "cover": ["vndb", "igdb", "steam"]},
            "sources": {"vndb": {"options": {"titleOrder": "original"}, "fieldsOff": ["criticScore"]}}
        }),
    );
    let pin = SourceMatch { id: "v97".into(), pinned: true, ..Default::default() };
    let mut md = heronite_metadata::util::blank_metadata("portal", "sideload");
    record::set_source_match(&mut md, "vndb", pin, None);
    json_store::write(&paths.metadata(), &json!({"games": {"sideload:portal": md}})).unwrap();
    // Not installed: the id stays and never answers.
    let s = svc.metadata_settings();
    assert_eq!(svc.field_order(&s, "cover"), ["vndb", "igdb", "steam", "launchbox", "libretro"]);
    assert!(svc.preview_game_metadata(&args("portal")).unwrap().sources.iter().all(|a| a.source != "vndb"));

    svc.install_extension(&fast_copy(dir.path()), false).unwrap();
    let s = svc.metadata_settings();
    assert_eq!(svc.field_order(&s, "cover"), ["vndb", "igdb", "steam", "launchbox", "libretro"], "where the user put it");
    assert_eq!(svc.field_order(&s, "themes"), ["igdb", "playnite", "vndb"], "a default of then is the default");
    assert_eq!(svc.field_order(&s, "hero"), ["igdb", "steam", "lutris", "playnite", "launchbox", "vndb"]);
    assert!(!svc.field_order(&s, "criticScore").contains(&"vndb".to_string()), "a field switched off stays off");
    let preview = svc.preview_game_metadata(&args("portal")).unwrap();
    let c = vndb_candidate(&preview).unwrap();
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("v97"), Some(MatchedBy::SourceId)), "the pinned entry");
    assert_eq!(c.match_name.as_deref(), Some("沙耶の唄"), "its setting");
    assert_eq!(preview.proposed.field_sources["themes"], "vndb");
}
