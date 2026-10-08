//! End-to-end on a synthetic Playnite library built byte by byte (LiteDB v7
//! pages + BSON) and a throw-away Heroic folder.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::models::*;
use heronite_data::{json_store, HeroicPaths};
use heronite_launch::probe::FakeProbe;
use serde_json::{json, Value};

use crate::litedb::testkit::*;
use crate::reader::*;
use crate::types::*;
use crate::Playnite;

const G_STEAM: &str = "11111111-1111-1111-1111-111111111111";
const G_EPIC: &str = "22222222-2222-2222-2222-222222222222";
const G_EPIC_MISS: &str = "33333333-3333-3333-3333-333333333333";
const G_GOG: &str = "44444444-4444-4444-4444-444444444444";
const G_EXE: &str = "55555555-5555-5555-5555-555555555555";
const G_EMU: &str = "66666666-6666-6666-6666-666666666666";
const ST_PLAYED: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const ST_BEATEN: &str = "aaaaaaaa-0000-0000-0000-000000000002";
const ST_WISH: &str = "aaaaaaaa-0000-0000-0000-000000000003";
const EMU: &str = "eeeeeeee-0000-0000-0000-000000000001";
const DEV: &str = "dddddddd-0000-0000-0000-000000000001";
const GENRE: &str = "99999999-0000-0000-0000-000000000001";

struct World {
    _dir: tempfile::TempDir,
    heroic: HeroicPaths,
    library: PathBuf,
    root: PathBuf,
}

fn write_db(library: &Path, name: &str, docs: &[Vec<u8>]) {
    fs::write(library.join(name), simple_db(docs)).unwrap();
}

fn game(id: &str, name: &str, plugin: &str, game_id: Option<&str>, playtime: i64, status: &str, extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut els = vec![guid("_id", id), s("Name", name), guid("PluginId", plugin), i64e("Playtime", playtime), guid("CompletionStatusId", status)];
    if let Some(g) = game_id {
        els.push(s("GameId", g));
    }
    els.push(date("LastActivity", 1_700_000_000_000));
    els.push(date("Added", 1_690_000_000_000));
    els.extend(extra);
    doc(&els)
}

fn build_library(library: &Path, played_secs: i64, with_status: &str) {
    fs::create_dir_all(library).unwrap();
    let games = vec![
        game(G_STEAM, "Portal", STEAM_PLUGIN_ID, Some("400"), played_secs, with_status, vec![
            arr("DeveloperIds", &[guid("", DEV)]),
            arr("GenreIds", &[guid("", GENRE)]),
            s("Description", "<p>Think <b>with</b> portals&nbsp;&amp; more</p>"),
            s("CoverImage", &format!("{G_STEAM}\\cover.png")),
            sub("ReleaseDate", doc(&[i32e("Year", 2007), i32e("Month", 10), i32e("Day", 9)])),
        ]),
        game(G_EPIC, "Control", EPIC_PLUGIN_ID, Some("Calluna"), 7200, ST_BEATEN, vec![]),
        game(G_EPIC_MISS, "Not In Library", EPIC_PLUGIN_ID, Some("zzz"), 60, ST_PLAYED, vec![]),
        game(G_GOG, "The Witcher 3: Wild Hunt", GOG_PLUGIN_ID, Some("999"), 120, ST_WISH, vec![]),
        game(G_EXE, "Indie Thing", EMPTY_GUID, None, 59, ST_PLAYED, vec![
            s("InstallDirectory", "D:\\Games\\Indie"),
            arr("GameActions", &[sub("", doc(&[i32e("Type", 0), s("Name", "Play"), s("Path", "{InstallDir}/indie.exe"), boolean("IsPlayAction", true)]))]),
        ]),
        game(G_EMU, "Castlevania SOTN", EMPTY_GUID, None, 600, ST_PLAYED, vec![
            arr("Roms", &[sub("", doc(&[s("Name", "SOTN"), s("Path", "D:\\roms\\sotn.cue")]))]),
            arr("GameActions", &[sub("", doc(&[
                i32e("Type", 2), boolean("IsPlayAction", true), guid("EmulatorId", EMU), s("EmulatorProfileId", "ps1"),
            ]))]),
        ]),
    ];
    write_db(library, "games.db", &games);
    write_db(library, "companies.db", &[doc(&[guid("_id", DEV), s("Name", "Valve")])]);
    write_db(library, "genres.db", &[doc(&[guid("_id", GENRE), s("Name", "Puzzle")])]);
    write_db(
        library,
        "completionstatuses.db",
        &[
            doc(&[guid("_id", ST_PLAYED), s("Name", "5. Played")]),
            doc(&[guid("_id", ST_BEATEN), s("Name", "1. Beaten")]),
            doc(&[guid("_id", ST_WISH), s("Name", "Wishlist")]),
        ],
    );
    write_db(
        library,
        "emulators.db",
        &[doc(&[
            guid("_id", EMU),
            s("Name", "DuckStation"),
            s("InstallDir", "D:\\emu"),
            arr("CustomProfiles", &[sub("", doc(&[s("Id", "ps1"), s("Name", "PS1"), s("Executable", "D:\\emu\\duckstation.exe"), s("Arguments", "-batch {ImagePath}")]))]),
        ])],
    );
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("playnite");
    let library = root.join("library");
    build_library(&library, 3725, ST_PLAYED);

    // files/ cover and GameActivity sessions
    let cover_dir = library.join("files").join(G_STEAM);
    fs::create_dir_all(&cover_dir).unwrap();
    fs::write(cover_dir.join("cover.png"), b"\x89PNG\r\n\x1a\nfake").unwrap();
    let act = root.join("ExtensionsData").join(GAME_ACTIVITY_ID).join("GameActivity");
    fs::create_dir_all(&act).unwrap();
    fs::write(
        act.join(format!("{G_STEAM}.json")),
        serde_json::to_vec(&json!({"Items": [
            {"DateSession": "2024-03-02T10:00:00Z", "ElapsedSeconds": 1800, "GameActionName": "Play"},
            {"DateSession": "2024-03-01T10:00:00Z", "ElapsedSeconds": 600},
            {"DateSession": "2024-03-03T10:00:00Z", "ElapsedSeconds": 0}
        ]}))
        .unwrap(),
    )
    .unwrap();

    // Heroic data
    let heroic = HeroicPaths::at(dir.path().join("heroic"));
    fs::create_dir_all(heroic.root.join("store_cache")).unwrap();
    json_store::write(&heroic.legendary_library(), &json!({"library": [{"app_name": "Calluna", "title": "Control", "runner": "legendary"}]})).unwrap();
    json_store::write(&heroic.gog_library(), &json!({"games": [{"app_name": "1207658924", "title": "The Witcher 3 - Wild Hunt", "runner": "gog"}]})).unwrap();
    json_store::write(&heroic.timestamps(), &json!({"steam_400": {"firstPlayed": "2030-01-01T00:00:00.000Z", "lastPlayed": "2020-01-01T00:00:00.000Z", "totalPlayed": 5}})).unwrap();

    World { _dir: dir, heroic, library, root }
}

fn playnite(w: &World) -> Playnite {
    let probe = FakeProbe::new("/nonexistent-home").with_path("/mnt/d/Games/Indie/indie.exe").with_path("/mnt/d/emu/duckstation.exe").with_path("/mnt/d/roms/sotn.cue");
    Playnite::new(w.heroic.clone()).with_probe(probe)
}

fn drive_map() -> Vec<DriveRemap> {
    vec![DriveRemap { windows_root: "D:\\".into(), linux_path: "/mnt/d".into() }]
}

fn lib(w: &World) -> String {
    w.library.to_string_lossy().into_owned()
}

fn args(w: &World, merge: bool) -> PlayniteImportArgs {
    PlayniteImportArgs { library_path: lib(w), drive_map: drive_map(), fetch_covers: None, merge_existing: merge }
}

#[test]
fn reads_the_synthetic_library() {
    let w = world();
    let dump = load_playnite_library(&lib(&w)).unwrap();
    assert_eq!(dump.games.len(), 6);
    assert_eq!(dump.playnite_root, w.root.to_string_lossy());
    let portal = dump.games.iter().find(|g| g.id == G_STEAM).unwrap();
    assert_eq!(portal.developers, ["Valve"]);
    assert_eq!(portal.genres, ["Puzzle"]);
    assert_eq!(portal.release_date.as_deref(), Some("2007-10-09"));
    assert_eq!(portal.plugin_id, STEAM_PLUGIN_ID);
    assert_eq!(dump.completion_statuses.len(), 3);
    assert_eq!(dump.emulators[0].profiles[0].id.as_deref(), Some("ps1"));
    assert_eq!(dump.sessions_by_game_id[G_STEAM].len(), 2); // zero-length dropped
    assert_eq!(dump.sessions_by_game_id.len(), 1);
}

#[test]
fn preview_counts_and_destinations() {
    let w = world();
    let p = playnite(&w).preview_import(&lib(&w)).unwrap();
    assert_eq!(p.game_count, 6);
    assert_eq!(p.session_file_count, 1);
    assert_eq!(p.detected_drives, ["D:"]);
    assert_eq!(
        p.counts,
        PlayniteImportCounts { steam: 1, emulator: 1, manual: 1, epic_match: 1, gog_match: 1, skipped_store: 1, ..Default::default() }
    );
    let by = |id: &str| p.games.iter().find(|g| g.playnite_id == id).unwrap();
    assert_eq!(by(G_EPIC).destination, Destination::Legendary);
    assert_eq!(by(G_EPIC).matched_app_name.as_deref(), Some("Calluna"));
    assert_eq!(by(G_GOG).destination, Destination::Gog);
    assert_eq!(by(G_GOG).matched_app_name.as_deref(), Some("1207658924")); // by normalised title
    assert_eq!(by(G_EPIC_MISS).destination, Destination::Skip);
    assert_eq!(by(G_STEAM).session_count, 2);
    assert_eq!(by(G_STEAM).playtime_minutes.get(), 62.0);
    assert_eq!(by(G_STEAM).matched_app_name.as_deref(), Some("steam_400"));
    assert!(!by(G_STEAM).already_imported);

    let missing = playnite(&w).preview_import(w.root.to_str().unwrap()).unwrap();
    assert_eq!(missing.errors, ["games.db was not found in the selected folder"]);
}

fn read<T: serde::de::DeserializeOwned + Default>(p: &Path) -> T {
    json_store::read(p).unwrap()
}

fn assert_round_trip<T: serde::de::DeserializeOwned + serde::Serialize + Default>(p: &Path) {
    let orig = json_store::read_value(p).unwrap();
    let typed: T = json_store::read(p).unwrap();
    assert_eq!(orig, serde_json::to_value(typed).unwrap(), "{}", p.display());
}

#[test]
fn import_writes_every_store_and_round_trips() {
    let w = world();
    let pl = playnite(&w);
    let res = pl.import(&args(&w, false)).unwrap();
    assert_eq!(
        res,
        PlayniteImportResult { imported: 3, updated: 0, matched_store: 2, skipped: 1, covers_fetched: 1, sessions_imported: 2, errors: vec![] }
    );
    let p = &w.heroic;

    let meta: LocalMetaFile = read(&p.local_meta());
    assert_eq!(meta.games.len(), 5);
    let steam = &meta.games["steam_400"];
    assert_eq!(steam.launch_kind, "steam-uri");
    assert_eq!(steam.launcher_args.as_deref(), Some("steam://rungameid/400"));
    assert_eq!(steam.playnite_id, G_STEAM);
    assert_eq!(steam.completion_status_id.as_deref(), Some("played"));
    assert_eq!(steam.playnite_completion_status_id.as_deref(), Some(ST_PLAYED));
    // Epic/GOG entries live under the store app name, runner = store
    let control = &meta.games["Calluna"];
    assert_eq!(control.runner, "legendary");
    assert_eq!(control.completion_status_id.as_deref(), Some("beaten"));
    assert_eq!(meta.games["1207658924"].runner, "gog");
    assert_eq!(meta.games["1207658924"].completion_status_id.as_deref(), Some("playnite_aaaaaaaa-0000-0000-0000-000000000003"));
    assert!(!meta.games.values().any(|m| m.playnite_id == G_EPIC_MISS));
    let exe = &meta.games[&format!("playnite_{G_EXE}")];
    assert_eq!(exe.remapped_executable.as_deref(), Some("/mnt/d/Games/Indie/indie.exe"));
    assert_eq!(exe.windows_install_directory.as_deref(), Some("D:\\Games\\Indie"));
    let emu = &meta.games[&format!("playnite_{G_EMU}")];
    assert_eq!(emu.launch_kind, "emulator");
    assert_eq!(emu.roms.as_ref().unwrap()[0].path, "/mnt/d/roms/sotn.cue");

    // sideload entries for steam/exe/emulator only
    let side: SideloadLibraryFile = read(&p.sideload_library());
    let names: Vec<_> = side.games.iter().map(|g| g.app_name.as_str()).collect();
    assert_eq!(names, ["steam_400", &format!("playnite_{G_EXE}"), &format!("playnite_{G_EMU}")]);
    assert_eq!(side.games[0].art_square.as_deref(), Some("https://cdn.cloudflare.steamstatic.com/steam/apps/400/library_600x900_2x.jpg"));
    assert!(!side.games[0].is_installed); // no Steam manifests on the fake machine
    assert_eq!(side.games[0].rest["canRunOffline"], false);
    assert!(side.games[1].is_installed);
    assert_eq!(side.games[1].install.executable.as_deref(), Some("/mnt/d/Games/Indie/indie.exe"));

    // playtime: max minutes, min first / max last
    let ts: TimestampFile = read(&p.timestamps());
    assert_eq!(ts["steam_400"].total_played.get(), 62.0);
    assert_eq!(ts["steam_400"].first_played.as_deref(), Some("2024-03-01T10:00:00.000Z"));
    assert_eq!(ts["steam_400"].last_played.as_deref(), Some("2023-11-14T22:13:20.000Z"));
    assert_eq!(ts["Calluna"].total_played.get(), 120.0);

    // sessions
    let sessions: SessionsFile = read(&p.sessions());
    let s = &sessions.sessions["steam_400"];
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].source, "playnite");

    // statuses learnt from Playnite and the remembered import context
    let st: StatusesFile = read(&p.statuses());
    assert_eq!(st.last_playnite_library_path.as_deref(), Some(lib(&w).as_str()));
    assert_eq!(st.last_playnite_drive_map.as_ref().unwrap()[0]["windowsRoot"], "D:\\");
    let beaten = st.statuses.iter().find(|s| s.id == "beaten").unwrap();
    assert_eq!(beaten.name, "1. Beaten");
    assert_eq!(beaten.playnite_id.as_deref(), Some(ST_BEATEN));

    // metadata seeded from Playnite, cover copied into metadata-art
    let md: MetadataFile = read(&p.metadata());
    let m = &md.games["sideload:steam_400"];
    assert_eq!(m.description.as_deref(), Some("Think with portals & more"));
    assert_eq!(m.developers, ["Valve"]);
    assert_eq!(m.release_date.as_deref(), Some("2007-10-09"));
    assert_eq!(m.field_sources["description"], "playnite");
    assert_eq!(m.cover_url.as_deref(), Some("localart://metadata-art/sideload_steam_400/cover.png"));
    assert!(p.metadata_art_dir().join("sideload_steam_400/cover.png").exists());
    assert!(md.games.contains_key("legendary:Calluna"));

    // launcher args in GamesConfig, emulator registered
    let gc = json_store::read_value(&p.game_config("steam_400")).unwrap();
    assert_eq!(gc["steam_400"]["launcherArgs"], "steam://rungameid/400");
    assert!(!p.game_config(&format!("playnite_{G_EMU}")).exists());
    let emus: heronite_launch::emulators::EmulatorsFile = read(&p.emulators());
    assert_eq!(emus.emulators.len(), 1);
    assert_eq!(emus.emulators[0].definition_id, "duckstation");
    assert_eq!(emu.emulator_id.as_deref(), Some(emus.emulators[0].id.as_str()));
    assert_eq!(emu.emulator_profile_id.as_deref(), Some("ps1"));
    assert_eq!(emu.remapped_executable.as_deref(), Some("/mnt/d/emu/duckstation.exe"));

    assert_round_trip::<LocalMetaFile>(&p.local_meta());
    assert_round_trip::<SessionsFile>(&p.sessions());
    assert_round_trip::<StatusesFile>(&p.statuses());
    assert_round_trip::<MetadataFile>(&p.metadata());
    assert_round_trip::<TimestampFile>(&p.timestamps());
    assert_round_trip::<SideloadLibraryFile>(&p.sideload_library());

    // importing again: everything is "updated", nothing duplicated, sessions deduped
    let again = pl.import(&args(&w, false)).unwrap();
    assert_eq!((again.imported, again.updated, again.matched_store, again.skipped), (0, 3, 2, 1));
    let sessions: SessionsFile = read(&p.sessions());
    assert_eq!(sessions.sessions["steam_400"].len(), 2);
    let side: SideloadLibraryFile = read(&p.sideload_library());
    assert_eq!(side.games.len(), 3);
}

#[test]
fn merge_picks_up_new_playtime_and_reports_status_conflicts() {
    let w = world();
    let pl = playnite(&w);
    pl.import(&args(&w, false)).unwrap();

    // Nothing changed yet.
    let quiet = pl.preview_merge().unwrap();
    assert!(quiet.errors.is_empty());
    assert_eq!(quiet.skipped, 1);
    assert_eq!(quiet.unchanged, 5);
    assert!(quiet.new_games.is_empty() && quiet.playtime_updates.is_empty());

    // In Heronite the user marks Portal as beaten; in Playnite it moves to Wishlist
    // and 10 more minutes are played.
    heronite_data::status::set_game_status(&w.heroic, "steam_400", "beaten", &Default::default()).unwrap();
    build_library(&w.library, 3725 + 600, ST_WISH);
    let mp = pl.preview_merge().unwrap();
    assert_eq!(mp.playtime_updates.len(), 1);
    assert_eq!(mp.playtime_updates[0].heroic_minutes.unwrap().get(), 62.0);
    assert_eq!(mp.playtime_updates[0].playnite_minutes.unwrap().get(), 72.0);
    assert_eq!(mp.status_conflicts.len(), 1);
    assert_eq!(mp.status_conflicts[0].title, "Portal");
    assert_eq!(mp.status_conflicts[0].heroic_status.as_deref(), Some("1. Beaten"));
    assert_eq!(mp.status_conflicts[0].playnite_status.as_deref(), Some("Wishlist"));

    let merged = pl.merge().unwrap();
    assert_eq!((merged.imported, merged.updated), (0, 5));
    let meta: LocalMetaFile = read(&w.heroic.local_meta());
    // conflict: the Heronite status wins
    assert_eq!(meta.games["steam_400"].completion_status_id.as_deref(), Some("beaten"));
    assert_eq!(meta.games["steam_400"].playnite_completion_status_id.as_deref(), Some(ST_WISH));
    let ts: TimestampFile = read(&w.heroic.timestamps());
    assert_eq!(ts["steam_400"].total_played.get(), 72.0);
    assert_eq!(meta.games.len(), 5);
}

#[test]
fn export_matches_the_ts_format() {
    let w = world();
    let pl = playnite(&w);
    pl.import(&args(&w, false)).unwrap();
    let target = w.root.join("out");
    let res = pl.export(&target).unwrap();
    assert_eq!(res.game_count, 5);
    assert!(res.path.contains("heroic-playnite-export-") && res.path.ends_with(".json"));
    let text = fs::read_to_string(&res.path).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.starts_with("{\n  \"format\": \"heroic-playnite-export\",\n  \"version\": 1,"));
    let v: Value = serde_json::from_str(&text).unwrap();
    let games = v["games"].as_array().unwrap();
    let portal = games.iter().find(|g| g["heroicAppName"] == "steam_400").unwrap();
    assert_eq!(portal["playniteId"], G_STEAM);
    assert_eq!(portal["steamAppId"], "400");
    assert_eq!(portal["playtimeMinutes"], 62);
    assert_eq!(portal["playtimeSeconds"], 3720);
    assert_eq!(portal["completionStatus"]["slug"], "played");
    assert_eq!(portal["completionStatus"]["playniteId"], ST_PLAYED);
    assert_eq!(portal["lastPlayniteCompletionStatusId"], ST_PLAYED);
    // newest session first
    let s = portal["sessions"].as_array().unwrap();
    assert_eq!(s[0]["startedAt"], "2024-03-02T10:00:00.000Z");
    assert_eq!(s[0]["gameActionName"], "Play");
    assert!(v["statuses"].as_array().unwrap().len() >= 9);
    // an explicit .json path is used as is
    let explicit = w.root.join("my-export.json");
    let res = pl.export(&explicit).unwrap();
    assert_eq!(PathBuf::from(&res.path), explicit);
}
