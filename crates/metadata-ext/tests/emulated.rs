//! Installed extensions next to the built-in sources of emulated games, through
//! the real gather: the LaunchBox Games Database (the fixture index of
//! `heronite-metadata`'s tests), Lutris and libretro-thumbnails. What a
//! script's `game` carries for an emulated game, the audiences a manifest
//! declares (`platforms`, `pc`), the fallback rule, each field's order with
//! LaunchBox leading for the emulated games it found surely, and an
//! extension removed while the app runs.

mod support;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::record;
use heronite_metadata::sources::{AppliesTo, SourceStatus};
use heronite_metadata::*;
use heronite_metadata_ext::ExtensionsExt;
use serde_json::{json, Value};
use support::{endpoints, example, folder, manifest, pc_extras, Req, Resp, Server, PC_EXTRAS_HOST};

const HOSTS: [&str; 3] = ["en.wikipedia.org", PC_EXTRAS_HOST, "api.retrodb.org"];
const ARCHIVE: &str = "/launchbox/Metadata.zip";
const SM_ROM: &str = "Super Metroid (Japan, USA) (En,Ja).sfc";

/// A details source of SNES games, written for this test: its `find` sends
/// what the script's `game` carries, so the server sees it.
const RETRODB: &str = r#"
export async function find(game, ctx) {
  const r = await ctx.http.get("https://api.retrodb.org/find", {
    query: { title: game.title, platform: game.platform || "", file: game.fileName || "", year: String(game.year || "") },
  });
  return r.json();
}
export async function fetch(id, ctx) {
  const r = await ctx.http.get("https://api.retrodb.org/game/" + id);
  return r.status === 200 ? r.json() : null;
}
"#;

fn launchbox_archive() -> Vec<u8> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../metadata/tests/fixtures/launchbox");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["Metadata.xml", "Files.xml"] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&fs::read(dir.join(name)).unwrap()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// LaunchBox's archive; Lutris and RetroDB know Super Metroid; Wikipedia
/// knows Super Mario World; the PC extras source and libretro-thumbnails
/// know nothing.
fn mock() -> Server {
    let archive = launchbox_archive();
    Server::start(move |req: &Req| match req.path() {
        ARCHIVE => Resp::image(archive.clone()).with_header("ETag", "\"v1\""),
        "/lutris/api/games" => match req.query("search").as_deref() {
            Some("Super Metroid") => {
                Resp::json(json!({"count": 1, "results": [{"id": 1, "name": "Super Metroid", "slug": "super-metroid", "year": 1994}]}))
            }
            _ => Resp::json(json!({"count": 0, "results": []})),
        },
        "/lutris/api/games/super-metroid" => Resp::json(json!({
            "name": "Super Metroid", "slug": "super-metroid", "year": 1994, "description": "What Lutris says.",
            "genres": [{"name": "Platform"}], "platforms": [], "coverart": "https://lutris.net/c.jpg"
        })),
        "/api.retrodb.org/find" => match req.query("title").as_deref() {
            Some("Super Metroid") => Resp::json(json!([{ "id": "sm1", "title": "Super Metroid", "year": 1994 }])),
            _ => Resp::json(json!([])),
        },
        "/api.retrodb.org/game/sm1" => Resp::json(json!({
            "title": "Super Metroid", "description": "From RetroDB.", "series": "Metroid", "genres": ["Metroidvania"]
        })),
        "/en.wikipedia.org/w/rest.php/v1/search/title" => match req.query("q").as_deref() {
            Some("Super Mario World") => Resp::json(json!({"pages": [
                { "key": "Super_Mario_World", "title": "Super Mario World", "description": "1990 video game" }
            ]})),
            _ => Resp::json(json!({"pages": []})),
        },
        "/en.wikipedia.org/api/rest_v1/page/summary/Super_Mario_World" => Resp::json(json!({
            "type": "standard", "title": "Super Mario World", "description": "1990 video game",
            "extract": "Super Mario World is a 1990 platform game developed and published by Nintendo."
        })),
        p if p.starts_with(&format!("/{PC_EXTRAS_HOST}/")) => Resp::json(json!([])),
        _ => Resp::status(404),
    })
}

struct Setup {
    dir: tempfile::TempDir,
    server: Server,
    svc: MetadataService,
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

/// Super Metroid added from Playnite (no platform id: the emulator and the
/// dump say it), Super Mario World with its platform id, and a PC game;
/// LaunchBox's index downloaded; Wikipedia, an extras source of PC games
/// ([`pc_extras`]) and RetroDB installed, in that order.
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    write(
        &paths.local_meta(),
        json!({"games": {
            "sm": {"appName": "sm", "runner": "sideload", "title": "Super Metroid", "playniteId": "", "source": "emulator",
                   "launchKind": "emulator", "emulatorName": "Snes9x",
                   "roms": [{"path": format!("{{InstallDir}}\\{SM_ROM}")}], "windowsInstallDirectory": "D:\\Roms\\SNES"},
            "smw": {"appName": "smw", "runner": "sideload", "title": "Super Mario World", "playniteId": "", "source": "emulator",
                    "launchKind": "emulator", "platformId": "nintendo_super_nes", "roms": [{"path": "/roms/snes/Super Mario World (USA).sfc"}]},
            "p2": {"appName": "p2", "runner": "sideload", "title": "Portal 2", "playniteId": "", "source": "manual",
                   "launchKind": "executable", "windowsInstallDirectory": "C:\\Games\\Portal 2"}
        }}),
    );
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let server = mock();
    let svc = MetadataService::with_endpoints(paths.clone(), endpoints(&server, &HOSTS));
    svc.download_source("launchbox").unwrap();
    svc.install_extension(&example("wikipedia"), false).unwrap();
    svc.install_extension(&pc_extras(&dir.path().join("packages")), false).unwrap();
    install_retrodb(&svc, dir.path());
    Setup { dir, server, svc }
}

fn install_retrodb(svc: &MetadataService, parent: &Path) {
    let body = "[applies]\nplatforms = [\"snes\"]\n\n[network]\nhosts = [\"api.retrodb.org\"]\nrate = \"10/s\"";
    let text = manifest("retrodb", "1.0.0", "details", &["description", "genres", "series"], body);
    let dir = folder(&parent.join("packages"), "retrodb", &text, RETRODB);
    svc.install_extension(&dir, true).unwrap();
}

fn args(app: &str, title: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.into(), ..Default::default() }
}

fn state(preview: &MetadataPreview, id: &str) -> Option<SourceAnswer> {
    preview.sources.iter().find(|s| s.source == id).map(|s| s.answer.clone())
}

fn found(preview: &MetadataPreview, id: &str) -> Option<MatchedBy> {
    match state(preview, id) {
        Some(SourceAnswer::Found { hit, .. }) => hit.matched_by,
        other => panic!("{id}: {other:?}"),
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn row_ids(svc: &MetadataService) -> Vec<String> {
    svc.source_rows(&svc.metadata_settings()).into_iter().map(|r| r.descriptor.id).collect()
}

#[test]
fn extensions_answer_for_emulated_games_beside_launchbox_and_leave_cleanly() {
    let s = setup();
    let requests = |prefix: &str| s.server.count(|r| r.path().starts_with(prefix));

    // The manifest's `platforms` are read as the app's platform ids.
    let retrodb = s.svc.sources().get("retrodb").cloned().unwrap();
    assert_eq!(retrodb.applies_to, AppliesTo::Platforms(strings(&["nintendo_super_nes"])));
    assert!(retrodb.leads_for.is_none() && retrodb.download.is_none());
    assert_eq!(s.svc.sources().get("pcextras").map(|d| d.applies_to.clone()), Some(AppliesTo::Pc));
    let rows = s.svc.source_rows(&s.svc.metadata_settings());
    let row = rows.iter().find(|r| r.descriptor.id == "retrodb").unwrap();
    assert!(row.status == SourceStatus::On && row.reorderable && row.download.is_none() && row.problem.is_none());

    // Super Metroid: LaunchBox by its dump, Lutris and RetroDB by its title.
    // Lutris found it for sure, so Wikipedia (a fallback) waits.
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(found(&preview, "launchbox"), Some(MatchedBy::ExactTitle));
    assert_eq!(found(&preview, "lutris"), Some(MatchedBy::ExactTitle));
    assert_eq!(found(&preview, "retrodb"), Some(MatchedBy::ExactTitle));
    assert_eq!(state(&preview, "wikipedia"), Some(SourceAnswer::NotAsked { why: NotAsked::Fallback }));
    // The script saw the platform the app worked out and the dump's name, never a path.
    let asked = s.server.log.lock().iter().rev().find(|r| r.path() == "/api.retrodb.org/find").cloned().unwrap();
    assert_eq!(asked.query("platform").as_deref(), Some("nintendo_super_nes"));
    assert_eq!(asked.query("file").as_deref(), Some(SM_ROM));
    assert_eq!(asked.query("year").as_deref(), Some(""), "no record yet");

    // Each field from the first source of its order. The extensions follow
    // the built-in sources; LaunchBox found the emulated game surely, so it
    // goes ahead of the sources that found it by its title (Lutris, RetroDB).
    let settings = s.svc.metadata_settings();
    assert_eq!(
        s.svc.field_order(&settings, "description"),
        strings(&["igdb", "steam", "store", "lutris", "playnite", "launchbox", "wikipedia", "retrodb"])
    );
    assert_eq!(s.svc.field_order(&settings, "series"), strings(&["igdb", "playnite", "pcextras", "retrodb"]));
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    for field in ["description", "genres", "releaseDate", "cover"] {
        assert_eq!(saved.field_sources[field], "launchbox", "{field}");
    }
    assert_eq!((saved.series.as_deref(), saved.field_sources["series"].as_str()), (Some("Metroid"), "retrodb"));
    assert_eq!(record::source_match(&saved, "retrodb").map(|m| m.id), Some("sm1".to_string()));
    // Asked again, the script gets the year the record has now.
    s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    let asked = s.server.log.lock().iter().rev().find(|r| r.path() == "/api.retrodb.org/find").cloned().unwrap();
    assert_eq!(asked.query("year").as_deref(), Some("1994"));

    // A field the user orders keeps their order; the others still follow the rule.
    s.svc.set_field_order("description", &strings(&["retrodb"])).unwrap();
    let mut again = args("sm", "Super Metroid");
    again.overwrite_existing = Some(true);
    let saved = s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!((saved.description.as_deref(), saved.field_sources["description"].as_str()), (Some("From RetroDB."), "retrodb"));
    assert_eq!(saved.field_sources["genres"], "launchbox");

    // Super Mario World: no online details source finds it, so Wikipedia is
    // asked; RetroDB looks and finds nothing; LaunchBox still leads.
    let preview = s.svc.preview_game_metadata(&args("smw", "Super Mario World")).unwrap();
    assert_eq!(found(&preview, "launchbox"), Some(MatchedBy::ExactTitle));
    assert_eq!(found(&preview, "wikipedia"), Some(MatchedBy::ExactTitle));
    assert_eq!(state(&preview, "lutris"), Some(SourceAnswer::NotFound));
    assert_eq!(state(&preview, "retrodb"), Some(SourceAnswer::NotFound));
    let saved = s.svc.apply_game_metadata(&args("smw", "Super Mario World")).unwrap();
    assert_eq!(saved.field_sources["releaseDate"], "launchbox");
    assert!(record::source_match(&saved, "wikipedia").is_some());
    // Without LaunchBox's description, Wikipedia's is next in that field.
    s.svc.set_source_field_enabled("launchbox", "description", false).unwrap();
    s.svc.reset_field_order("description").unwrap();
    let mut again = args("smw", "Super Mario World");
    again.overwrite_existing = Some(true);
    let saved = s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(saved.field_sources["description"], "wikipedia");
    assert!(saved.description.as_deref().unwrap().starts_with("Super Mario World is a 1990 platform game"));
    s.svc.set_source_field_enabled("launchbox", "description", true).unwrap();

    // The extras source is for PC games (`to = "pc"`): never asked about
    // the emulated ones.
    let extras = format!("/{PC_EXTRAS_HOST}/");
    assert_eq!(requests(&extras), 0);
    let preview = s.svc.preview_game_metadata(&args("p2", "Portal 2")).unwrap();
    assert!(requests(&extras) > 0);
    assert_eq!(state(&preview, "retrodb"), Some(SourceAnswer::NotAsked { why: NotAsked::NotForThisGame }));
    assert_eq!(state(&preview, "launchbox"), Some(SourceAnswer::NotAsked { why: NotAsked::NotForThisGame }));

    // RetroDB removed while the app runs: a registry taken before keeps it,
    // the next gather and the rows do not; what the user set for it stays.
    s.svc.set_field_order("description", &strings(&["retrodb"])).unwrap();
    let before = s.svc.sources();
    let rows_before = row_ids(&s.svc);
    s.svc.remove_extension("retrodb", false).unwrap();
    assert!(before.get("retrodb").is_some() && s.svc.sources().get("retrodb").is_none());
    let rows_after = row_ids(&s.svc);
    assert_eq!(rows_after, rows_before.iter().filter(|id| *id != "retrodb").cloned().collect::<Vec<_>>());
    let settings = s.svc.metadata_settings();
    assert_eq!(s.svc.field_order(&settings, "series"), strings(&["igdb", "playnite", "pcextras"]));
    assert_eq!(s.svc.field_order(&settings, "description").first().map(String::as_str), Some("retrodb"), "kept, never answers");
    assert!(!s.svc.bulk_sources(&settings).iter().any(|d| d.id == "retrodb"));
    let asked_before = requests("/api.retrodb.org");
    let preview = s.svc.preview_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(state(&preview, "retrodb"), None);
    assert_eq!(found(&preview, "launchbox"), Some(MatchedBy::ExactTitle));
    let mut again = args("sm", "Super Metroid");
    again.overwrite_existing = Some(true);
    let saved = s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(saved.field_sources["description"], "lutris", "the user's order, without the source that left");
    assert_eq!(saved.field_sources["genres"], "launchbox");
    assert_eq!(requests("/api.retrodb.org"), asked_before);
    assert!(s.svc.search_sources("Super Metroid", None).asked.iter().all(|id| id != "retrodb"));

    // Installed again, it takes its place back, the user's order with it.
    install_retrodb(&s.svc, s.dir.path());
    assert_eq!(row_ids(&s.svc), rows_before);
    let saved = s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(saved.field_sources["description"], "retrodb");

    // Wikipedia removed and forgotten: gone from the rows and every list.
    s.svc.remove_extension("wikipedia", true).unwrap();
    assert!(!row_ids(&s.svc).contains(&"wikipedia".to_string()));
    let settings = s.svc.metadata_settings();
    assert!(settings.field_priority.values().all(|list| !list.iter().any(|id| id == "wikipedia")));
    let preview = s.svc.preview_game_metadata(&args("smw", "Super Mario World")).unwrap();
    assert_eq!(state(&preview, "wikipedia"), None);
    assert_eq!(found(&preview, "launchbox"), Some(MatchedBy::ExactTitle));
}
