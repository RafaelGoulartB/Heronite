//! An unsure match is not saved: the review queue, the user's answers, the
//! match per source (pins), field locks and the games that need attention,
//! against a local mock of the remote services.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{fallback, fixture, Req, Resp, Server, FALLBACK};
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::DataPaths;
use heronite_metadata::record::{self, SourceMatch};
use heronite_metadata::service::{is_sent_to_review, Attention, MatchUnavailable, Resolution, Resolved};
use heronite_metadata::*;
use indexmap::IndexMap;
use serde_json::{json, Value};

/// An IGDB `games` row (release on January 2nd of `year`).
fn igdb_row(id: i64, name: &str, year: i32) -> Value {
    let ts = chrono::NaiveDate::from_ymd_opt(year, 1, 2).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    json!({"id": id, "name": name, "slug": name.to_lowercase().replace(' ', "-"), "first_release_date": ts,
           "summary": format!("About {name}."), "genres": [{"name": "Action"}], "cover": {"image_id": format!("co{id}")},
           "platforms": [{"name": "PC (Microsoft Windows)"}]})
}

fn quoted(body: &str) -> String {
    body.split('"').nth(1).unwrap_or_default().to_owned()
}

fn steam_body(req: &Req) -> Resp {
    let id = req.query("appids").unwrap_or_default();
    let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
    Resp::json(json!({ id: all["400"].clone() }))
}

/// IGDB: "Portal 2" by Steam id 620; "Hollow Knight" is only close to
/// Silksong; "Doom" finds Doom Eternal while Lutris finds Doom 64;
/// IGDB and Lutris both find "The Witcher 3: Wild Hunt" for "Witcher 3";
/// nothing has "Ghost Game".
fn mock() -> Server {
    Server::start(|req| match req.path() {
        "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600})),
        "/igdb/v4/games" => {
            let body = req.body.as_str();
            if body.contains("where external_games.uid") {
                return if body.contains("\"620\"") { Resp::raw_json(fixture("igdb_game_steam_620.json")) } else { Resp::json(json!([])) };
            }
            if let Some(id) = body.split("where id = ").nth(1) {
                let id: i64 = id.trim_end_matches(|c: char| !c.is_ascii_digit()).split(';').next().unwrap().trim().parse().unwrap_or(0);
                return Resp::json(match id {
                    72 => serde_json::from_str::<Value>(&fixture("igdb_game_steam_620.json")).unwrap(),
                    500 => json!([igdb_row(500, "Hollow Knight: Silksong", 2025)]),
                    501 => json!([igdb_row(501, "Hollow Knight", 2017)]),
                    4242 => json!([igdb_row(4242, "Hollow Knight: Voidheart Edition", 2018)]),
                    600 => json!([igdb_row(600, "Doom Eternal", 2020)]),
                    _ => json!([]),
                });
            }
            Resp::json(match quoted(body).as_str() {
                "Portal 2" if body.contains("platforms.name;") => {
                    serde_json::from_str::<Value>(&fixture("igdb_search_portal2.json")).unwrap()
                }
                "Hollow Knight" => json!([igdb_row(500, "Hollow Knight: Silksong", 2025)]),
                "Doom" => json!([igdb_row(600, "Doom Eternal", 2020)]),
                "Witcher 3" => json!([igdb_row(700, "The Witcher 3: Wild Hunt", 2015)]),
                _ => json!([]),
            })
        }
        "/steam/api/appdetails" => steam_body(req),
        "/steam/api/storesearch/" => Resp::raw_json(fixture("steam_storesearch_portal.json")),
        "/lutris/api/games" => Resp::json(match req.query("search").as_deref() {
            Some("Doom") => json!({"results": [{"name": "Doom 64", "slug": "doom-64", "year": 1997}]}),
            Some("Witcher 3") => json!({"results": [{"name": "The Witcher 3: Wild Hunt", "slug": "the-witcher-3", "year": 2015}]}),
            Some("Portal 2") => serde_json::from_str::<Value>(&fixture("lutris_search_portal.json")).unwrap(),
            _ => json!({"results": []}),
        }),
        "/lutris/api/games/doom-64" => {
            Resp::json(json!({"name": "Doom 64", "slug": "doom-64", "year": 1997, "genres": [{"name": "Shooter"}]}))
        }
        "/lutris/api/games/the-witcher-3" => {
            Resp::json(json!({"name": "The Witcher 3: Wild Hunt", "slug": "the-witcher-3", "year": 2015, "genres": [{"name": "RPG"}]}))
        }
        "/lutris/api/games/portal" => Resp::raw_json(fixture("lutris_detail_portal.json")),
        _ => Resp::status(404),
    })
}

const GAMES: [(&str, &str, Option<&str>); 5] = [
    ("steam_620", "Portal 2", Some("620")),
    ("hk", "Hollow Knight", None),
    ("doom", "Doom", None),
    ("w3", "Witcher 3", None),
    ("ghost", "Ghost Game", None),
];

/// The games above in the overlay (only Portal 2 in the sideload library),
/// IGDB keys, the idle fill on, nothing fetched yet.
fn make_root(dir: &Path) -> DataPaths {
    let paths = DataPaths::at(dir);
    let w = |p: std::path::PathBuf, v: Value| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        json_store::write(&p, &v).unwrap();
    };
    w(
        paths.sideload_library(),
        json!({"games": [{"runner": "sideload", "app_name": "steam_620", "title": "Portal 2", "is_installed": false, "install": {}}]}),
    );
    let overlay: serde_json::Map<String, Value> = GAMES
        .iter()
        .map(|(app, title, steam)| {
            (
                app.to_string(),
                json!({"appName": app, "runner": "sideload", "title": title, "steamAppId": steam,
                                     "playniteId": "", "source": "manual", "launchKind": "executable"}),
            )
        })
        .collect();
    w(paths.local_meta(), json!({ "games": overlay }));
    w(
        paths.collection_settings(),
        json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret", "downloadImages": false,
                            "autoFillMissing": true, "steamTags": false, "fieldPriority": {}}}),
    );
    paths
}

fn item(app: &str) -> BulkItem {
    let (app, title, steam) = GAMES.iter().find(|g| g.0 == app).unwrap();
    BulkItem { app_name: app.to_string(), runner: "sideload".into(), title: title.to_string(), steam_app_id: steam.map(Into::into) }
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let server = mock();
    let paths = make_root(dir.path());
    let svc = MetadataService::with_endpoints(paths.clone(), server.endpoints());
    // A fallback source, as an extension adds one: asked when nothing is sure.
    svc.add_source(fallback()).unwrap();
    Setup { _dir: dir, paths, server, svc }
}

fn get(svc: &MetadataService, app: &str) -> Option<CollectionGameMetadata> {
    svc.get_game_metadata("sideload", app).unwrap()
}

fn put(paths: &DataPaths, md: &CollectionGameMetadata) {
    json_store::update::<MetadataFile, _>(&paths.metadata(), |f| {
        f.games.insert(format!("{}:{}", md.runner, md.app_name), md.clone());
        Ok(())
    })
    .unwrap();
}

fn run(svc: &MetadataService, args: BulkArgs) -> BulkProgress {
    svc.run_bulk_metadata(&args, &|_| {})
}

fn outcomes(p: &BulkProgress) -> Vec<(String, GameOutcome)> {
    p.results.iter().map(|r| (r.app_name.clone(), r.outcome.clone())).collect()
}

fn queued(svc: &MetadataService) -> Vec<String> {
    svc.review_queue().into_iter().map(|i| i.app_name).collect()
}

fn igdb_by_id(server: &Server, id: i64) -> usize {
    server.count(|r| r.path() == "/igdb/v4/games" && r.body.contains(&format!("where id = {id};")))
}

fn igdb_search(server: &Server, title: &str) -> usize {
    server.count(|r| r.path() == "/igdb/v4/games" && r.body.starts_with(&format!("search \"{title}\"")))
}

#[test]
fn a_job_saves_the_sure_queues_the_unsure_and_marks_the_missing() {
    let s = setup();
    let games: Vec<BulkItem> = GAMES.iter().map(|g| item(g.0)).collect();
    let revision = s.svc.review_revision();
    let done = run(&s.svc, BulkArgs::fill_missing(games));
    let ok = |a: &str| (a.to_string(), GameOutcome::Updated);
    assert_eq!(
        outcomes(&done),
        [
            ok("steam_620"),
            ("hk".into(), GameOutcome::ToReview(ReviewReason::LoneGuess)),
            ("doom".into(), GameOutcome::ToReview(ReviewReason::SourcesDisagree)),
            ok("w3"),
            ("ghost".into(), GameOutcome::NotFound),
        ]
    );
    let t = done.tally();
    assert_eq!((t.updated, t.to_review, t.not_found), (2, 2, 1));
    assert_eq!((done.updated, done.skipped, done.failed), (2, 2, 1), "the older counters: queued games were not saved");
    assert!(s.svc.review_revision() > revision);

    // Nothing of the unsure games was written; the queue says why and what was found.
    assert!(get(&s.svc, "hk").is_none() && get(&s.svc, "doom").is_none());
    assert_eq!(queued(&s.svc), ["hk", "doom"]);
    let hk = s.svc.review_entry("sideload:hk").unwrap();
    assert_eq!((hk.title.as_str(), hk.reason), ("Hollow Knight", ReviewReason::LoneGuess));
    let hit = &hk.hits[0];
    assert_eq!((hit.source.as_str(), hit.id.as_deref(), hit.title.as_deref()), ("igdb", Some("500"), Some("Hollow Knight: Silksong")));
    assert_eq!(
        (hit.year, hit.platform.as_deref(), hit.matched_by),
        (Some(2025), Some("PC (Microsoft Windows)"), Some(MatchedBy::FuzzyTitle))
    );
    assert!(hit.thumb_url.as_deref().is_some_and(|u| u.starts_with("https://images.igdb.com/")));
    let doom = s.svc.review_entry("sideload:doom").unwrap().choices();
    assert_eq!(doom.iter().map(|c| c.title.clone().unwrap()).collect::<Vec<_>>(), ["Doom Eternal", "Doom 64"]);
    assert_eq!(doom[1].picks(), [MatchPick { source: "lutris".into(), id: "doom-64".into() }]);
    let raw = json_store::read_value(&s.paths.metadata_review()).unwrap();
    assert_eq!(raw["games"]["sideload:hk"]["reason"], "loneGuess");
    assert_eq!(raw["games"]["sideload:doom"]["hits"][1]["matchedBy"], "fuzzyTitle");

    // Two guesses that agree are saved, and remember what each source matched.
    let w3 = get(&s.svc, "w3").unwrap();
    assert_eq!((w3.description.as_deref(), w3.lutris_slug.as_deref()), (Some("About The Witcher 3: Wild Hunt."), Some("the-witcher-3")));
    let m = record::source_match(&w3, "igdb").unwrap();
    assert_eq!((m.id.as_str(), m.year, m.matched_by, m.pinned), ("700", Some(2015), Some(MatchedBy::FuzzyTitle), false));
    // A sure one: the Steam id.
    let portal = get(&s.svc, "steam_620").unwrap();
    assert_eq!(record::source_match(&portal, "igdb").unwrap().matched_by, Some(MatchedBy::SteamId));

    // Undo takes the queue back too.
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!((report.games, report.reviews), (3, 2));
    assert!(queued(&s.svc).is_empty());
    assert!(get(&s.svc, "w3").is_none());
}

#[test]
fn undo_leaves_what_the_user_reviewed_since_and_puts_back_what_the_job_took_out() {
    let s = setup();
    run(&s.svc, BulkArgs::fill_missing(vec![item("hk"), item("doom")]));
    s.svc.resolve_review(&item("doom"), &Resolution::InNoSource).unwrap();
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!(report.reviews, 1, "only hk, as the job left it");
    assert!(queued(&s.svc).is_empty());
    assert!(get(&s.svc, "doom").is_some_and(|md| record::marked_no_match(&md, "Doom")));

    // A job that saves a queued game takes it out; its undo puts it back.
    run(&s.svc, BulkArgs::fill_missing(vec![item("hk")]));
    assert_eq!(queued(&s.svc), ["hk"]);
    let mut pinned = record_for("hk");
    record::set_source_match(&mut pinned, "igdb", SourceMatch { id: "501".into(), pinned: true, ..Default::default() }, None);
    put(&s.paths, &pinned);
    let done = run(&s.svc, BulkArgs::update(vec![item("hk")], UpdateMode::Refresh, None, None));
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    assert!(queued(&s.svc).is_empty());
    let report = s.svc.undo_bulk_metadata().unwrap().unwrap();
    assert_eq!(report.reviews, 1);
    assert_eq!(queued(&s.svc), ["hk"]);
}

fn record_for(app: &str) -> CollectionGameMetadata {
    serde_json::from_value(json!({"appName": app, "runner": "sideload"})).unwrap()
}

#[test]
fn use_this_match_saves_it_pinned_and_later_updates_use_it() {
    let s = setup();
    run(&s.svc, BulkArgs::fill_missing(vec![item("hk")]));
    let choice = s.svc.review_entry("sideload:hk").unwrap().choices().remove(0);

    // A game the source no longer has: nothing changes.
    let gone = Resolution::UseMatch(vec![MatchPick { source: "igdb".into(), id: "999".into() }]);
    let err = s.svc.resolve_review(&item("hk"), &gone).unwrap_err();
    assert_eq!(err.downcast_ref::<MatchUnavailable>(), Some(&MatchUnavailable(vec!["igdb".into()])));
    assert_eq!(queued(&s.svc), ["hk"]);

    let saved = match s.svc.resolve_review(&item("hk"), &Resolution::UseMatch(choice.picks())).unwrap() {
        Resolved::Saved(md) => md,
        other => panic!("{other:?}"),
    };
    assert_eq!(saved.description.as_deref(), Some("About Hollow Knight: Silksong."));
    let m = record::source_match(&saved, "igdb").unwrap();
    assert_eq!((m.id.as_str(), m.pinned, m.matched_by), ("500", true, Some(MatchedBy::SourceId)));
    assert_eq!(saved.igdb_id.map(|n| n.get()), Some(500.0));
    assert!(queued(&s.svc).is_empty());

    // Later updates ask IGDB for that game, never search, and are sure.
    let (by_id, searches) = (igdb_by_id(&s.server, 500), igdb_search(&s.server, "Hollow Knight"));
    let done = run(&s.svc, BulkArgs::update(vec![item("hk")], UpdateMode::Refresh, None, None));
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::NothingNew));
    assert_eq!((igdb_by_id(&s.server, 500), igdb_search(&s.server, "Hollow Knight")), (by_id + 1, searches));
    assert!(queued(&s.svc).is_empty());
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    let igdb = preview.sources.iter().find(|st| st.source == "igdb").unwrap();
    assert!(matches!(&igdb.answer, SourceAnswer::Found { pinned: true, used: true, hit } if hit.id.as_deref() == Some("500")));
    assert_eq!(preview.review, None);
}

#[test]
fn a_chosen_match_leaves_out_and_forgets_another_sources_wrong_game() {
    let s = setup();
    // Saved before with Lutris' guess.
    let mut md = record_for("doom");
    md.lutris_slug = Some("doom-64".into());
    put(&s.paths, &md);
    let pick = Resolution::UseMatch(vec![MatchPick { source: "igdb".into(), id: "600".into() }]);
    let saved = match s.svc.resolve_review(&item("doom"), &pick).unwrap() {
        Resolved::Saved(md) => md,
        other => panic!("{other:?}"),
    };
    assert_eq!((saved.description.as_deref(), saved.genres.as_slice()), (Some("About Doom Eternal."), ["Action".to_string()].as_slice()));
    assert_eq!(saved.lutris_slug, None);
    assert_eq!(record::source_matches(&saved).keys().collect::<Vec<_>>(), ["igdb"]);
}

fn args(app: &str, title: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.into(), ..Default::default() }
}

#[test]
fn in_no_source_stops_the_automatic_paths_until_the_title_changes() {
    let s = setup();
    // The idle fill: sure and agreeing games saved, the others queued.
    assert_eq!(s.svc.refresh_missing_collection_metadata().unwrap(), 2);
    assert_eq!(queued(&s.svc), ["hk", "doom"]);
    assert!(get(&s.svc, "ghost").unwrap().fetch_error.is_some());
    // A second pass leaves the queued games alone.
    let searches = igdb_search(&s.server, "Hollow Knight");
    assert_eq!(s.svc.refresh_missing_collection_metadata().unwrap(), 0);
    assert_eq!(igdb_search(&s.server, "Hollow Knight"), searches);

    // Skip: it stays.
    assert!(matches!(s.svc.resolve_review(&item("hk"), &Resolution::Skip).unwrap(), Resolved::Skipped));
    assert_eq!(queued(&s.svc), ["hk", "doom"]);

    assert!(matches!(s.svc.resolve_review(&item("hk"), &Resolution::InNoSource).unwrap(), Resolved::InNoSource));
    assert_eq!(queued(&s.svc), ["doom"]);
    let all: Vec<BulkItem> = GAMES.iter().map(|g| item(g.0)).collect();
    let a = s.svc.attention(&all).unwrap();
    assert_eq!(a.in_no_source.iter().collect::<Vec<_>>(), ["sideload:hk"]);
    assert!(!a.without_match().contains("sideload:hk"));
    assert_eq!(a.to_review.iter().collect::<Vec<_>>(), ["sideload:doom"]);

    // Neither a job nor the idle fill asks about it again.
    let done = run(&s.svc, BulkArgs::update(vec![item("hk")], UpdateMode::Replace, None, None));
    assert_eq!(done.results[0].outcome, GameOutcome::Skipped(SkipReason::InNoSource));
    let raw = json_store::read_value(&s.paths.metadata()).unwrap();
    assert_eq!(raw["games"]["sideload:hk"]["noMatch"]["title"], "Hollow Knight");
    json_store::update_value(&s.paths.metadata(), |v| {
        v["games"]["sideload:ghost"]["fetchedAt"] = json!("2020-01-01T00:00:00.000Z");
        Ok(())
    })
    .unwrap();
    s.svc.refresh_missing_collection_metadata().unwrap();
    assert_eq!(igdb_search(&s.server, "Hollow Knight"), searches);

    // Renamed: asked again.
    json_store::update_value(&s.paths.local_meta(), |v| {
        v["games"]["hk"]["title"] = json!("Hollow Knight Voidheart");
        Ok(())
    })
    .unwrap();
    s.svc.refresh_missing_collection_metadata().unwrap();
    assert_eq!(igdb_search(&s.server, "Hollow Knight Voidheart"), 1);
}

#[test]
fn searching_every_source_groups_the_hits_by_game() {
    let s = setup();
    let found = s.svc.search_sources("Portal 2", None);
    assert_eq!(found.asked, ["igdb", "steam", "lutris", FALLBACK], "a search by hand asks the fallback source too");
    assert!(found.unreachable.is_empty() && found.needs_key.is_empty());
    let portal2 = found.choices.iter().find(|c| c.hits.iter().any(|h| h.id.as_deref() == Some("72"))).unwrap();
    let sources: Vec<(&str, &str)> = portal2.hits.iter().map(|h| (h.source.as_str(), h.id.as_deref().unwrap())).collect();
    assert_eq!(sources, [("igdb", "72"), ("steam", "620")]);
    assert_eq!((portal2.title.as_deref(), portal2.year), (Some("Portal 2"), Some(2011)));
    let steam = &portal2.hits[1];
    assert_eq!(steam.thumb_url.as_deref(), Some("https://cdn.cloudflare.steamstatic.com/steam/apps/620/library_600x900_2x.jpg"));
    let lutris = found.choices.iter().find(|c| c.hits.iter().any(|h| h.source == "lutris")).unwrap();
    assert_eq!(lutris.title.as_deref(), Some("Portal"));
    assert!(found.choices.iter().all(|c| c.hits.iter().map(|h| &h.source).collect::<std::collections::HashSet<_>>().len() == c.hits.len()));

    let only = s.svc.search_sources("Portal 2", Some(&["lutris".to_string()]));
    assert_eq!(only.asked, ["lutris"]);
    assert_eq!(only.choices.iter().map(|c| c.hits[0].id.clone().unwrap()).collect::<Vec<_>>(), ["portal", "adventure-portal"]);

    // Without IGDB keys, IGDB says so; a source that cannot search is not asked.
    json_store::write(&s.paths.collection_settings(), &json!({"metadata": {}})).unwrap();
    let keyless = s.svc.search_sources("Portal 2", None);
    assert_eq!(keyless.needs_key, ["igdb"]);
    assert!(s
        .svc
        .sources()
        .descriptors()
        .filter(|d| d.searchable)
        .all(|d| ["igdb", "steam", "lutris", "launchbox", FALLBACK].contains(&d.id.as_str())));
    assert!(!keyless.asked.contains(&"launchbox".to_string()), "searchable, but not asked before its download");
}

#[test]
fn pins_live_in_the_legacy_fields_and_the_new_key() {
    let s = setup();
    // A record from before: an IGDB id and nothing else. Looked up by that id, as before.
    let old: CollectionGameMetadata =
        serde_json::from_value(json!({"appName": "hk", "runner": "sideload", "igdbId": 4242, "fieldSources": {}})).unwrap();
    put(&s.paths, &old);
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    let igdb = &preview.candidates[0];
    assert_eq!((igdb.match_id.as_deref(), igdb.matched_by), (Some("4242"), Some(MatchedBy::SourceId)));
    assert_eq!(preview.review, None, "a game it has an id for is sure");
    assert!(preview.locks.is_empty());

    // Steam pinned in the new key: asked with that id though the game has none.
    let mut md = record_for("hk");
    record::set_source_match(&mut md, "steam", SourceMatch { id: "620".into(), pinned: true, ..Default::default() }, None);
    put(&s.paths, &md);
    let raw = json_store::read_value(&s.paths.metadata()).unwrap();
    assert_eq!(raw["games"]["sideload:hk"]["steamAppId"], "620");
    assert_eq!(raw["games"]["sideload:hk"]["sourceMatches"]["steam"], json!({"id": "620", "pinned": true}));
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    let steam = preview.sources.iter().find(|st| st.source == "steam").unwrap();
    assert!(matches!(&steam.answer, SourceAnswer::Found { pinned: true, .. }), "{steam:?}");
    // The Steam answer is sure: the IGDB guess that is another game is left out.
    assert_eq!(preview.review, None);
    let igdb = preview.sources.iter().find(|st| st.source == "igdb").unwrap();
    assert!(matches!(&igdb.answer, SourceAnswer::Found { used: false, pinned: false, .. }), "{igdb:?}");

    // The per-game dialog: pin a Lutris game, then let it go.
    let mut pin = args("doom", "Doom");
    pin.pins = Some(IndexMap::from([("lutris".to_string(), Some("doom-64".to_string()))]));
    let preview = s.svc.preview_game_metadata(&pin).unwrap();
    assert_eq!(preview.review, None);
    let saved = s.svc.apply_preview(&preview, &pin).unwrap();
    assert_eq!(saved.lutris_slug.as_deref(), Some("doom-64"));
    assert!(record::source_match(&saved, "lutris").unwrap().pinned);
    assert!(saved.description.is_none(), "the IGDB guess (another game) is left out");
    let mut unpin = args("doom", "Doom");
    unpin.pins = Some(IndexMap::from([("lutris".to_string(), None)]));
    let preview = s.svc.preview_game_metadata(&unpin).unwrap();
    let saved = s.svc.apply_preview(&preview, &unpin).unwrap();
    assert!(!record::source_match(&saved, "lutris").unwrap().pinned);
}

#[test]
fn a_locked_field_stays_in_every_mode_in_the_idle_fill_and_in_a_fetch() {
    let s = setup();
    let portal = || {
        let mut md = record_for("steam_620");
        md.title = Some("Portal 2".into());
        md.description = Some("mine".into());
        md.field_sources.insert("description".into(), "igdb".into());
        record::set_locked(&mut md, "description", true);
        record::set_locked(&mut md, "genres", true);
        md
    };
    for mode in [UpdateMode::Fill, UpdateMode::Refresh, UpdateMode::Replace] {
        put(&s.paths, &portal());
        let done = run(&s.svc, BulkArgs::update(vec![item("steam_620")], mode, None, None));
        assert_eq!(done.results[0].outcome, GameOutcome::Updated, "{mode:?}");
        assert!(done.results[0].changes.iter().all(|c| c.field != "description" && c.field != "genres"), "{mode:?}");
        let md = get(&s.svc, "steam_620").unwrap();
        assert_eq!((md.description.as_deref(), md.genres.len()), (Some("mine"), 0), "{mode:?}");
        assert_eq!(record::locked_fields(&md), ["description", "genres"]);
        assert!(md.release_date.is_some(), "{mode:?}");
    }
    // Only locked fields chosen: nothing asked.
    put(&s.paths, &portal());
    let fields = Some(vec!["description".to_string(), "genres".to_string()]);
    let done = run(&s.svc, BulkArgs::update(vec![item("steam_620")], UpdateMode::Replace, fields, None));
    assert_eq!(done.results[0].outcome, GameOutcome::NoChange(NoChangeReason::Locked));

    // The idle fill of a game that failed long ago.
    let mut failed = record_for("w3");
    failed.fetch_error = Some("No metadata sources returned data".into());
    failed.fetched_at = Some("2020-01-01T00:00:00.000Z".into());
    record::set_locked(&mut failed, "description", true);
    put(&s.paths, &failed);
    s.svc.refresh_missing_collection_metadata().unwrap();
    let w3 = get(&s.svc, "w3").unwrap();
    assert!(w3.description.is_none() && !w3.genres.is_empty() && w3.fetch_error.is_none());

    // A fetch shows the lock and keeps it; the dialog can pick and unlock.
    put(&s.paths, &portal());
    let mut a = args("steam_620", "Portal 2");
    a.overwrite_existing = Some(true);
    a.preserve_manual = Some(false);
    let preview = s.svc.preview_game_metadata(&a).unwrap();
    assert_eq!(preview.locks, ["description", "genres"]);
    assert_eq!(preview.proposed.description.as_deref(), Some("mine"));
    a.field_picks = Some(IndexMap::from([("description".to_string(), "steam".to_string())]));
    let kept = s.svc.apply_preview(&preview, &a).unwrap();
    assert_eq!(kept.description.as_deref(), Some("mine"), "a pick does not open a lock by itself");
    a.locks = Some(IndexMap::from([("description".to_string(), false)]));
    let picked = s.svc.apply_preview(&preview, &a).unwrap();
    assert_ne!(picked.description.as_deref(), Some("mine"));
    assert_eq!(picked.field_sources["description"], "steam");
    assert_eq!(record::locked_fields(&picked), ["genres"]);
    let unlocked = s.svc.set_field_locks("sideload", "steam_620", &IndexMap::from([("genres".to_string(), false)])).unwrap().unwrap();
    assert!(record::locked_fields(&unlocked).is_empty() && !unlocked.extra.contains_key(record::LOCKED_FIELDS));
}

#[test]
fn the_preview_says_what_each_source_did() {
    let s = setup();
    let preview = s.svc.preview_game_metadata(&args("doom", "Doom")).unwrap();
    assert_eq!(preview.review, Some(ReviewReason::SourcesDisagree));
    let states: Vec<(&str, &SourceAnswer)> = preview.sources.iter().map(|st| (st.source.as_str(), &st.answer)).collect();
    assert_eq!(
        states.iter().map(|s| s.0).collect::<Vec<_>>(),
        ["igdb", "steam", "store", "lutris", "launchbox", FALLBACK],
        "Playnite is hidden"
    );
    assert_eq!(*states[4].1, SourceAnswer::NotAsked { why: NotAsked::NotForThisGame }, "LaunchBox is for emulated games");
    assert!(
        matches!(states[0].1, SourceAnswer::Found { used: false, pinned: false, hit } if hit.matched_by == Some(MatchedBy::FuzzyTitle))
    );
    assert_eq!(*states[1].1, SourceAnswer::NotAsked { why: NotAsked::NotForThisGame });
    assert_eq!(*states[2].1, SourceAnswer::NotFound);
    assert_eq!(*states[5].1, SourceAnswer::NotFound, "nothing sure: the fallback source was asked");
    assert!(preview.proposed.description.is_none() && preview.proposed.genres.is_empty(), "an automatic save takes nothing");

    // Off, and no keys.
    json_store::write(&s.paths.collection_settings(), &json!({"metadata": {"sources": {"lutris": {"enabled": false}}}})).unwrap();
    let preview = s.svc.preview_game_metadata(&args("doom", "Doom")).unwrap();
    let why = |id: &str| preview.sources.iter().find(|st| st.source == id).map(|st| st.answer.clone());
    assert_eq!(why("igdb"), Some(SourceAnswer::NotAsked { why: NotAsked::NeedsKey }));
    assert_eq!(why("lutris"), Some(SourceAnswer::NotAsked { why: NotAsked::Off }));

    // The old dialog: a guess the user picks field by field is saved.
    let s = setup();
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    assert_eq!(preview.review, Some(ReviewReason::LoneGuess));
    run(&s.svc, BulkArgs::fill_missing(vec![item("hk")]));
    let mut a = args("hk", "Hollow Knight");
    a.field_picks = Some(IndexMap::from([("description".to_string(), "igdb".to_string())]));
    let saved = s.svc.apply_preview(&preview, &a).unwrap();
    assert_eq!(saved.description.as_deref(), Some("About Hollow Knight: Silksong."));
    assert!(queued(&s.svc).is_empty(), "fetched by hand: out of the queue");
}

#[test]
fn the_idle_fill_and_a_fetch_send_an_unsure_game_to_review() {
    let s = setup();
    let err = s.svc.apply_game_metadata(&args("hk", "Hollow Knight")).unwrap_err();
    assert_eq!(is_sent_to_review(&err), Some(ReviewReason::LoneGuess));
    assert!(get(&s.svc, "hk").is_none());
    assert_eq!(queued(&s.svc), ["hk"]);
    // Fix matches: a game that is not queued, asked now.
    let entry = s.svc.review_entry_for(&item("steam_620")).unwrap();
    assert_eq!(entry.reason, ReviewReason::Requested);
    assert!(entry.hits.iter().any(|h| h.source == "igdb" && h.id.as_deref() == Some("72")));
    assert_eq!(queued(&s.svc), ["hk"], "asking writes nothing");
}

#[test]
fn files_from_before_load_and_behave_as_before() {
    let s = setup();
    // Today's shape, with a key this build does not know.
    let before = json!({"appName": "w3", "runner": "sideload", "title": "Witcher 3", "igdbId": 700,
        "description": "kept", "developers": [], "publishers": [], "genres": [], "themes": [], "gameModes": [],
        "platforms": [], "features": [], "websites": [], "fieldSources": {"description": "manual"}, "futureField": 1});
    json_store::write(&s.paths.metadata(), &json!({"games": {"sideload:w3": before}})).unwrap();
    assert!(s.svc.review_queue().is_empty(), "no queue file: nothing queued");
    let md = get(&s.svc, "w3").unwrap();
    assert_eq!(record::source_match(&md, "igdb"), Some(SourceMatch { id: "700".into(), ..Default::default() }));
    assert!(record::locked_fields(&md).is_empty() && record::no_match(&md).is_none());

    let done = run(&s.svc, BulkArgs::update(vec![item("w3")], UpdateMode::Refresh, None, None));
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    let raw = json_store::read_value(&s.paths.metadata()).unwrap()["games"]["sideload:w3"].clone();
    assert_eq!((raw["description"].as_str(), raw["futureField"].as_i64()), (Some("kept"), Some(1)));
    assert!(raw.get("lockedFields").is_none() && raw.get("noMatch").is_none());
    assert_eq!(raw["sourceMatches"]["igdb"]["id"], "700");
    assert!(!s.paths.metadata_review().exists(), "nothing queued, no file");
}

#[test]
fn the_collection_is_classified_from_disk_quickly() {
    let s = setup();
    let total = 2000;
    let (mut games, mut records, mut queue, mut art) = (Vec::new(), serde_json::Map::new(), serde_json::Map::new(), serde_json::Map::new());
    for i in 0..total {
        let app = format!("g{i}");
        let title = format!("Game {i}");
        games.push(BulkItem { app_name: app.clone(), runner: "sideload".into(), title: title.clone(), steam_app_id: None });
        let key = format!("sideload:{app}");
        let full = json!({"appName": app, "runner": "sideload", "description": "d", "coverUrl": "https://c", "fieldSources": {}});
        let record = match i % 8 {
            0 => None,
            1 => Some(json!({"appName": app, "runner": "sideload", "fetchError": "x", "fieldSources": {}})),
            2 => {
                queue.insert(
                    key.clone(),
                    json!({"appName": app, "runner": "sideload", "title": title, "reason": "loneGuess", "queuedAt": "t"}),
                );
                None
            }
            3 => Some(json!({"appName": app, "runner": "sideload", "coverUrl": "https://c", "genres": ["x"], "fieldSources": {}})),
            4 => Some(json!({"appName": app, "runner": "sideload", "description": "d", "fieldSources": {}})),
            5 => {
                art.insert(key.clone(), json!({"coverUrl": "localart://art/x"}));
                Some(json!({"appName": app, "runner": "sideload", "description": "d", "fieldSources": {}}))
            }
            6 => Some(json!({"appName": app, "runner": "sideload", "fieldSources": {}, "noMatch": {"title": title, "at": "t"}})),
            _ => Some(full),
        };
        if let Some(r) = record {
            records.insert(key, r);
        }
    }
    json_store::write(&s.paths.metadata(), &json!({ "games": records })).unwrap();
    json_store::write(&s.paths.metadata_review(), &json!({ "games": queue })).unwrap();
    json_store::write(&s.paths.art(), &json!({ "games": art })).unwrap();

    let t = Instant::now();
    let a: Attention = s.svc.attention(&games).unwrap();
    let counts = s.svc.bulk_counts(&games).unwrap();
    let took = t.elapsed();
    assert!(took < Duration::from_secs(3), "{took:?}");

    let n = total / 8;
    assert_eq!((a.to_review.len(), a.no_details.len(), a.failed.len()), (n, 3 * n, n));
    assert_eq!((a.no_description.len(), a.no_cover.len(), a.in_no_source.len()), (n, n, n));
    assert_eq!(a.without_match().len(), 3 * n);
    assert_eq!(a.all().len(), 5 * n);
    assert_eq!(a.complete(), 3 * n, "full, a custom cover, in no source");
    assert_eq!((counts.to_review, counts.without_match, counts.needs_attention, counts.in_no_source), (n, 3 * n, 5 * n, n));
    assert_eq!(s.svc.needs_attention(&games).unwrap().len(), 5 * n);
}

#[test]
fn a_field_picked_from_a_guess_does_not_make_the_guess_sure() {
    let s = setup();
    // IGDB guesses Silksong for Hollow Knight: left out, but the user takes its description.
    let preview = s.svc.preview_game_metadata(&args("hk", "Hollow Knight")).unwrap();
    assert_eq!(preview.review, Some(ReviewReason::LoneGuess));
    let mut a = args("hk", "Hollow Knight");
    a.field_picks = Some(IndexMap::from([("description".to_string(), "igdb".to_string())]));
    let saved = s.svc.apply_preview(&preview, &a).unwrap();
    assert_eq!(saved.description.as_deref(), Some("About Hollow Knight: Silksong."));
    assert!(saved.igdb_id.is_none() && record::source_match(&saved, "igdb").is_none());

    // The next update finds the game again by its name, and is still not sure.
    let done = run(&s.svc, BulkArgs::update(vec![item("hk")], UpdateMode::Refresh, None, None));
    assert_eq!(done.results[0].outcome, GameOutcome::ToReview(ReviewReason::LoneGuess));
    assert_eq!(igdb_by_id(&s.server, 500), 0);

    // A game the user pins is recorded, picked fields or not.
    let mut pinned = a.clone();
    pinned.pins = Some(IndexMap::from([("igdb".to_string(), Some("500".to_string()))]));
    let saved = s.svc.apply_preview(&preview, &pinned).unwrap();
    let m = record::source_match(&saved, "igdb").unwrap();
    assert_eq!((m.id.as_str(), m.pinned), ("500", true));
}

#[test]
fn a_game_in_no_source_renamed_and_still_not_found_waits_a_week() {
    let s = setup();
    assert!(matches!(s.svc.resolve_review(&item("ghost"), &Resolution::InNoSource).unwrap(), Resolved::InNoSource));
    json_store::update_value(&s.paths.local_meta(), |v| {
        v["games"]["ghost"]["title"] = json!("Ghost Game Deluxe");
        Ok(())
    })
    .unwrap();
    s.svc.refresh_missing_collection_metadata().unwrap();
    assert_eq!(igdb_search(&s.server, "Ghost Game Deluxe"), 1, "renamed: asked again");
    let md = get(&s.svc, "ghost").unwrap();
    assert!(md.fetch_error.is_some() && record::no_match(&md).is_none(), "the old title's mark goes");
    s.svc.refresh_missing_collection_metadata().unwrap();
    assert_eq!(igdb_search(&s.server, "Ghost Game Deluxe"), 1, "not asked again before a week");
}
