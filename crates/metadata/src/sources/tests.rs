use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::models::LocalGameMeta;
use heronite_data::settings::{default_field_priority, ALL_FIELDS, SOURCE_IDS};
use parking_lot::RwLock;

use super::*;
use crate::igdb::Igdb;
use crate::providers::StoreLibraries;
use crate::steam::SteamState;
use crate::types::{MatchedBy, MetadataApplyArgs};
use heronite_data::source_prefs;

fn registry() -> SourceRegistry {
    SourceRegistry::builtin(Shared {
        igdb: Arc::new(Igdb::new()),
        steam: Arc::new(SteamState::default()),
        stores: Arc::new(StoreLibraries::default()),
        playnite: Arc::new(RwLock::new(None)),
        launchbox: Arc::new(crate::launchbox::LaunchBox::new()),
    })
}

fn ids<'a>(list: impl Iterator<Item = &'a SourceDescriptor>) -> Vec<&'a str> {
    list.map(|d| d.id.as_str()).collect()
}

#[test]
fn lookups_keep_the_provider_order() {
    let reg = registry();
    let lookups: Vec<&str> = reg.lookups().map(|s| s.descriptor().id.as_str()).collect();
    assert_eq!(lookups, ["igdb", "steam", "store", "lutris", "playnite", "launchbox", "steamgriddb", "libretro"]);
    assert_eq!(
        ids(reg.descriptors()),
        [
            "igdb",
            "steam",
            "store",
            "lutris",
            "playnite",
            "launchbox",
            "steamgriddb",
            "libretro",
            "webimages",
            "howlongtobeat",
            "steamtags",
            "protondb"
        ]
    );
    assert_eq!(ids(reg.descriptors()), ids(builtin_descriptors().iter()));
    let unique: BTreeSet<_> = reg.descriptors().map(|d| d.id.as_str()).collect();
    assert_eq!(unique.len(), builtin_descriptors().len());
}

#[test]
fn details_sources_and_two_pictures_sources_have_a_lookup() {
    let reg = registry();
    for d in reg.descriptors() {
        assert_eq!(
            reg.has_lookup(&d.id),
            d.kind == SourceKind::Details || ["steamgriddb", "libretro"].contains(&d.id.as_str()),
            "{}",
            d.id
        );
    }
    for id in ["steamgriddb", "libretro", "webimages"] {
        assert_eq!(reg.get(id).map(|d| d.kind), Some(SourceKind::Pictures));
    }
    for id in ["howlongtobeat", "steamtags", "protondb"] {
        assert_eq!(reg.get(id).map(|d| d.kind), Some(SourceKind::Extras));
    }
    assert!(!reg.has_lookup("manual") && reg.get("manual").is_none());
}

#[test]
fn offered_details_sources_leave_playnite_out() {
    assert_eq!(ids(offered(SourceKind::Details)), ["igdb", "steam", "store", "lutris", "launchbox"]);
    assert!(registry().get("playnite").is_some_and(|d| d.hidden));
    let names: Vec<&str> = offered(SourceKind::Details).map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["IGDB", "Steam", "Store", "Lutris", "LaunchBox"]);
}

#[test]
fn network_sources_are_igdb_steam_lutris_steamgriddb_and_libretro() {
    let reg = registry();
    let online: Vec<&str> = reg.lookups().map(|s| s.descriptor()).filter(|d| d.network).map(|d| d.id.as_str()).collect();
    assert_eq!(online, ["igdb", "steam", "lutris", "steamgriddb", "libretro"]);
}

#[test]
fn cover_families_by_source() {
    let reg = registry();
    let gives = |field: &str| reg.descriptors().filter(|d| d.provides(field)).map(|d| d.id.as_str()).collect::<Vec<_>>();
    assert_eq!(gives("coverSquare"), ["launchbox", "steamgriddb", "libretro"]);
    assert_eq!(gives("coverWide"), ["steam", "store", "steamgriddb", "libretro"]);
    // The tall cover keeps its sources; IGDB and Lutris give only that one.
    assert_eq!(gives("cover"), ["igdb", "steam", "store", "lutris", "playnite", "launchbox", "steamgriddb", "libretro", "webimages"]);
    // No background: its screenshots are 240 to 512 px wide.
    assert!(!gives("hero").contains(&"libretro"));
}

/// `heronite_data::settings` keeps the ids of the default priorities and of
/// the saved-priority filter; they must be the registered details sources.
#[test]
fn data_crate_ids_match_the_details_sources() {
    let reg = registry();
    let details: BTreeSet<&str> = reg.descriptors().filter(|d| d.kind == SourceKind::Details).map(|d| d.id.as_str()).collect();
    let saved: BTreeSet<&str> = SOURCE_IDS.iter().copied().filter(|id| *id != "manual").collect();
    assert_eq!(details, saved);
    for field in ALL_FIELDS {
        for id in default_field_priority(field) {
            assert!(reg.has_lookup(&id), "default priority of {field} names {id}, not a details source");
        }
    }
    for d in reg.descriptors().filter(|d| d.kind == SourceKind::Details) {
        assert!(d.provides.iter().all(|f| ALL_FIELDS.contains(&f.as_str())), "{}", d.id);
    }
}

fn args(steam: Option<&str>) -> MetadataApplyArgs {
    MetadataApplyArgs {
        app_name: "a".into(),
        runner: "sideload".into(),
        title: "Game".into(),
        steam_app_id: steam.map(Into::into),
        ..Default::default()
    }
}

fn overlay(f: impl FnOnce(&mut LocalGameMeta)) -> LocalGameMeta {
    let mut m: LocalGameMeta = serde_json::from_str(r#"{"appName":"a","runner":"sideload"}"#).unwrap();
    f(&mut m);
    m
}

#[test]
fn source_game_takes_the_steam_id_from_the_overlay_when_missing() {
    let o = overlay(|m| m.steam_app_id = Some("620".into()));
    assert_eq!(SourceGame::new(&args(None), Some(&o), &[]).steam_app_id.as_deref(), Some("620"));
    assert_eq!(SourceGame::new(&args(Some("")), Some(&o), &[]).steam_app_id.as_deref(), Some("620"));
    assert_eq!(SourceGame::new(&args(Some("400")), Some(&o), &[]).steam_app_id.as_deref(), Some("400"));
    assert_eq!(SourceGame::new(&args(Some("")), None, &[]).steam_app_id, None);
}

#[test]
fn applies_to_answers_for_a_game() {
    let plain = SourceGame::new(&args(None), None, &[]);
    let steam = SourceGame::new(&args(Some("620")), None, &[]);
    let rom = overlay(|m| {
        m.source = "emulator".into();
        m.launch_kind = "emulator".into();
        m.platform_id = Some("snes".into());
    });
    let emulated = SourceGame::new(&args(None), Some(&rom), &[]);
    let epic = SourceGame { runner: "legendary".into(), ..plain.clone() };

    assert!(AppliesTo::All.matches(&plain));
    assert!(!AppliesTo::HasSteamId.matches(&plain) && AppliesTo::HasSteamId.matches(&steam));
    assert!(AppliesTo::Emulated.matches(&emulated) && !AppliesTo::Emulated.matches(&steam));
    let snes = AppliesTo::Platforms(vec!["snes".into()]);
    assert!(snes.matches(&emulated) && !snes.matches(&plain));
    let stores = AppliesTo::Stores(vec!["legendary".into(), "emulator".into()]);
    assert!(stores.matches(&epic) && stores.matches(&emulated) && !stores.matches(&plain));

    assert!(AppliesTo::Pc.matches(&plain) && !AppliesTo::Pc.matches(&emulated));

    let reg = registry();
    let steam_source = reg.get("steam").unwrap();
    assert!(!steam_source.applies_to.matches(&plain) && steam_source.applies_to.matches(&steam));
    // LaunchBox is for emulated games: the overlay says which.
    assert!(reg.needs_overlay());
}

/// Real Collection entries: a Playnite import without a platform id, a scan
/// with `other`, a game that is not emulated.
#[test]
fn emulated_games_carry_their_platform_and_rom() {
    let sotn = overlay(|m| {
        m.source = "emulator".into();
        m.launch_kind = "emulator".into();
        m.emulator_name = Some("Duckstation".into());
        m.windows_install_directory = Some("D:\\Games\\_PSS\\PS_Games\\SOTN".into());
        m.roms = Some(vec![heronite_data::models::LocalGameRom {
            name: None,
            path: "{InstallDir}\\Castlevania - Symphony of the Night (USA).cue".into(),
            extra: Default::default(),
        }]);
    });
    let game = SourceGame::new(&args(None), Some(&sotn), &[]);
    assert!(game.emulated);
    assert_eq!(game.platform.as_deref(), Some("sony_playstation"));
    assert_eq!(game.rom_file.as_deref(), Some("Castlevania - Symphony of the Night (USA).cue"));
    assert!(game.rom_hashes.is_empty(), "nothing is hashed for a lookup");
    let psx = AppliesTo::Platforms(vec!["sony_playstation".into()]);
    assert!(psx.matches(&game) && AppliesTo::Emulated.matches(&game));

    // `source: emulator` with an executable launch is still emulated; `other` is no platform.
    let scanned = overlay(|m| {
        m.source = "emulator".into();
        m.launch_kind = "executable".into();
        m.platform_id = Some("other".into());
    });
    let game = SourceGame::new(&args(None), Some(&scanned), &["PlayStation 2".to_string()]);
    assert!(game.emulated);
    assert_eq!(game.platform.as_deref(), Some("sony_playstation2"), "the record says it");
    assert_eq!(SourceGame::new(&args(None), Some(&scanned), &[]).platform, None);

    // A PC game has no platform even when its record names some.
    let pc = SourceGame::new(&args(None), Some(&overlay(|m| m.source = "steam".into())), &["PlayStation 2".to_string()]);
    assert!(!pc.emulated && pc.platform.is_none() && pc.rom_file.is_none());
}

/// `file_name`, what an extension's script gets: never a path.
#[test]
fn the_file_name_is_the_roms_else_the_install_folders() {
    let rom = overlay(|m| {
        m.launch_kind = "emulator".into();
        m.windows_install_directory = Some("/roms/snes".into());
        m.selected_rom_path = Some("{InstallDir}/Super Metroid (Japan, USA) (En,Ja).sfc".into());
    });
    let game = SourceGame::new(&args(None), Some(&rom), &[]);
    assert_eq!(game.file_name, game.rom_file);
    assert_eq!(game.file_name.as_deref(), Some("Super Metroid (Japan, USA) (En,Ja).sfc"));
    let pc = overlay(|m| m.windows_install_directory = Some("C:\\Games\\Portal 2\\".into()));
    let game = SourceGame::new(&args(None), Some(&pc), &[]);
    assert_eq!((game.file_name.as_deref(), game.rom_file), (Some("Portal 2"), None));
    assert_eq!(SourceGame::new(&args(None), None, &[]).file_name, None);
}

#[test]
fn libretro_is_for_the_emulated_games_of_platforms_with_a_folder() {
    let reg = registry();
    let d = reg.get("libretro").unwrap();
    assert_eq!((d.kind, d.needs, d.network, d.searchable, d.hidden), (SourceKind::Pictures, Needs::Nothing, true, false, false));
    assert!(d.options.is_empty() && d.download.is_none() && d.also_for.is_none() && !d.manual_only && d.asked == Asked::Always);
    assert_eq!(d.provides, ["cover", "coverSquare", "coverWide"], "no background: its screenshots are small");
    assert_eq!(d.finds_by, [MatchedBy::ExactTitle, MatchedBy::FuzzyTitle]);
    assert_eq!(d.leads_for, Some(AppliesTo::Emulated));
    let game = |platform: Option<&str>, emulated: bool| SourceGame {
        emulated,
        platform: platform.map(Into::into),
        ..SourceGame::new(&args(None), None, &[])
    };
    assert!(d.applies_to.matches(&game(Some("nintendo_super_nes"), true)));
    assert!(d.applies_to.matches(&game(Some("sony_playstation"), true)));
    assert!(!d.applies_to.matches(&game(Some("sony_playstation3"), true)), "a platform without a folder");
    assert!(!d.applies_to.matches(&game(Some("arcade"), true)));
    assert!(!d.applies_to.matches(&game(None, true)), "no platform");
    assert!(!d.applies_to.matches(&game(None, false)), "a PC game");
    let AppliesTo::Platforms(list) = &d.applies_to else { panic!("a platform list") };
    assert_eq!(list.len(), 43);
    assert!(!reg.get("libretro").unwrap().searchable && reg.has_lookup("libretro"));
}

#[test]
fn launchbox_is_for_emulated_games_and_pc_games_on_request() {
    let reg = registry();
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let lb = reg.get("launchbox").unwrap();
    assert_eq!((lb.needs, lb.network, lb.searchable), (Needs::Download, false, true));
    assert_eq!(lb.download.as_ref().map(|d| d.refresh_days), Some(7));
    let rom = overlay(|m| {
        m.source = "emulator".into();
        m.launch_kind = "emulator".into();
    });
    let emulated = SourceGame::new(&args(None), Some(&rom), &[]);
    let pc = SourceGame::new(&args(Some("620")), None, &[]);
    let mut m = settings::normalize_metadata(None);
    assert!(reg.applies(&paths, &m, lb, &emulated));
    assert!(!reg.applies(&paths, &m, lb, &pc), "not for PC games by default");
    source_prefs::set_option(&mut m, "launchbox", "pcGames", Some(serde_json::json!(true)));
    assert!(reg.applies(&paths, &m, lb, &pc));
    // Not downloaded: it cannot be asked, and says so.
    assert!(!reg.downloaded(&paths, "launchbox") && reg.downloaded(&paths, "igdb"));
    assert_eq!(reg.download_state(&paths, "launchbox"), Some(DownloadState::default()));
    assert_eq!(reg.download_state(&paths, "igdb"), None);
}

#[test]
fn no_built_in_source_is_a_fallback_and_an_extensions_kind_is_read() {
    assert!(registry().descriptors().all(|d| !d.is_fallback() && d.game_kind.is_none()));
    let reg = SourceRegistry::for_tests_with_fallback();
    let fallbacks: Vec<&str> = reg.descriptors().filter(|d| d.is_fallback()).map(|d| d.id.as_str()).collect();
    assert_eq!(fallbacks, [TEST_FALLBACK]);
    let kind = reg.get(TEST_FALLBACK).unwrap().game_kind.as_ref().unwrap();
    assert!(kind.said_by(&["Adventure".to_string(), "visual novel".to_string()]));
    assert!(!kind.said_by(&["Novel".to_string()]));
    // Named by a request of fallback sources only.
    let only = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(reg.names_fallback(TEST_FALLBACK, Some(&only(&[TEST_FALLBACK]))));
    assert!(reg.names_fallback(TEST_FALLBACK, Some(&only(&[TEST_FALLBACK, "steamtags"]))));
    assert!(!reg.names_fallback(TEST_FALLBACK, Some(&only(&["igdb", TEST_FALLBACK]))));
    assert!(!reg.names_fallback(TEST_FALLBACK, None));
    // A manifest says so the same way.
    let d: SourceDescriptor = serde_json::from_str(
        r#"{"id":"x","name":"X","kind":"details","asked":"asFallback","gameKind":{"name":"Visual novels","signals":["Visual Novel"]}}"#,
    )
    .unwrap();
    assert!(d.is_fallback() && d.game_kind.is_some());
}

#[test]
fn a_manifest_descriptor_takes_defaults() {
    let d: SourceDescriptor = serde_json::from_str(r#"{"id":"exampledb","name":"ExampleDB","kind":"details"}"#).unwrap();
    assert_eq!(d.applies_to, AppliesTo::All);
    assert_eq!(d.needs, Needs::Nothing);
    assert!(d.provides.is_empty() && d.finds_by.is_empty() && !d.network && !d.hidden);
    assert!(d.options.is_empty() && d.only_on.is_empty() && !d.manual_only && d.available_here());
    assert!(!d.is_fallback() && d.game_kind.is_none() && d.about.is_none());

    let d: SourceDescriptor = serde_json::from_str(
        r#"{"id":"x","name":"X","kind":"extras","provides":["timeToBeat"],"appliesTo":{"platforms":["snes"]},
            "findsBy":["romHash","fuzzyTitle"],"needs":"download","network":true}"#,
    )
    .unwrap();
    assert!(d.provides(extra_fields::TIME_TO_BEAT));
    assert_eq!(d.applies_to, AppliesTo::Platforms(vec!["snes".into()]));
    assert_eq!(d.finds_by, [MatchedBy::RomHash, MatchedBy::FuzzyTitle]);
    assert_eq!(d.needs, Needs::Download);

    for d in builtin_descriptors() {
        let back: SourceDescriptor = serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(&back, d);
    }
    assert_eq!(serde_json::to_value(AppliesTo::HasSteamId).unwrap(), "hasSteamId");
}

/// An extension-like source: a descriptor with `extension`, a health.
struct Ext(SourceDescriptor, SourceHealth);

impl MetadataSource for Ext {
    fn descriptor(&self) -> &SourceDescriptor {
        &self.0
    }
    fn lookup(&self, _: &SourceContext<'_>, _: &SourceGame) -> crate::types::Lookup<crate::types::MetadataCandidate> {
        crate::types::Lookup::NotFound
    }
    fn health(&self) -> SourceHealth {
        self.1.clone()
    }
}

fn ext(id: &str, version: &str, kind: &str, provides: &[&str], health: SourceHealth) -> Arc<dyn MetadataSource> {
    let d: SourceDescriptor = serde_json::from_value(serde_json::json!({
        "id": id, "name": id, "kind": kind, "provides": provides, "network": true, "searchable": true,
        "extension": {"version": version, "author": "x"}
    }))
    .unwrap();
    Arc::new(Ext(d, health))
}

#[test]
fn extensions_are_added_after_the_built_ins_and_never_take_their_ids() {
    let reg = registry();
    let reg = reg.with_source(ext("mydb", "1.0.0", "details", &["description", "series"], SourceHealth::Ok)).unwrap();
    let reg = reg.with_source(ext("pics", "1.0.0", "pictures", &["coverSquare"], SourceHealth::Ok)).unwrap();
    assert_eq!(ids(reg.descriptors()).last().copied(), Some("pics"));
    // An update takes the place of the old one.
    let reg = reg.with_source(ext("mydb", "2.0.0", "details", &["description", "series"], SourceHealth::Ok)).unwrap();
    assert_eq!(reg.descriptors().filter(|d| d.id == "mydb").count(), 1);
    assert_eq!(reg.get("mydb").unwrap().extension.as_ref().unwrap().version, "2.0.0");
    for taken in ["igdb", "steamgriddb", "protondb", "manual", "keep", "auto"] {
        assert!(is_reserved_id(taken), "{taken}");
        assert!(reg.with_source(ext(taken, "1.0.0", "details", &["description"], SourceHealth::Ok)).is_err(), "{taken}");
        assert!(reg.without_source(taken).is_none());
    }
    // In each field it fills, after the sources already there; the user's lists too.
    let mut m = heronite_data::settings::normalize_metadata(None);
    assert_eq!(reg.field_order(&m, "series"), ["igdb", "playnite", "mydb"]);
    assert_eq!(reg.field_order(&m, "coverSquare"), ["steamgriddb", "launchbox", "libretro", "pics"]);
    m.field_priority.insert("series".into(), vec!["playnite".into()]);
    assert_eq!(reg.field_order(&m, "series"), ["playnite", "mydb"]);
    assert!(!reg.is_order_customised(&m));
    // Moved first among the details sources, it leads every field it fills.
    m.source_order = reg.order_after_move(&m, "mydb", 0).unwrap();
    assert_eq!(reg.field_order(&m, "description").first().map(String::as_str), Some("mydb"));
    let reg = reg.without_source("mydb").unwrap();
    assert!(reg.get("mydb").is_none() && reg.without_source("mydb").is_none());
}

#[test]
fn a_source_that_is_not_healthy_shows_why_and_is_not_searched() {
    let (dir, http) = (tempfile::tempdir().unwrap(), crate::http::Http::default());
    let paths = heronite_data::paths::DataPaths::at(dir.path());
    let broken = SourceHealth::LoadFailed("api = 9".into());
    let reg = registry()
        .with_source(ext("broken", "1.0.0", "details", &["description"], broken.clone()))
        .unwrap()
        .with_source(ext("tired", "1.0.0", "extras", &["features"], SourceHealth::Failing("boom".into())))
        .unwrap();
    assert_eq!(reg.health("broken"), broken);
    assert!(reg.health("igdb").is_ok() && reg.health("unknown").is_ok());
    let m = heronite_data::settings::normalize_metadata(None);
    let rows = reg.rows(&paths, &m, &rows::TestResults::default());
    let row = |id: &str| rows.iter().find(|r| r.descriptor.id == id).unwrap().clone();
    assert_eq!((row("broken").status, row("broken").problem.as_deref()), (SourceStatus::LoadFailed, Some("api = 9")));
    let tired = row("tired");
    assert_eq!((tired.status, tired.problem.as_deref()), (SourceStatus::Failing, Some("boom")));
    assert!(tired.reorderable && tired.fields.iter().all(|f| f.switchable), "an extension's fields feed the merge");
    assert!(row("igdb").problem.is_none());
    let ctx = SourceContext::new(&paths, &http);
    assert!(matches!(reg.search(&ctx, "tired", "x"), crate::types::Lookup::Skipped));
    // Off wins over failing; a load failure shows even when off.
    let mut off = m.clone();
    heronite_data::source_prefs::set_on(&mut off, "tired", false);
    heronite_data::source_prefs::set_on(&mut off, "broken", false);
    let rows = reg.rows(&paths, &off, &rows::TestResults::default());
    let status = |id: &str| rows.iter().find(|r| r.descriptor.id == id).unwrap().status;
    assert_eq!((status("tired"), status("broken")), (SourceStatus::Off, SourceStatus::LoadFailed));
}
