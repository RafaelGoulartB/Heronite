//! The index built from a few dozen real entries of the database
//! (`tests/fixtures/launchbox/`, trimmed from the published XML: six
//! platforms, alternate names, pictures of several types and regions).

use std::io::Write;
use std::path::{Path, PathBuf};

use heronite_i18n::Lang;

use heronite_data::cover_family::CoverFamily;

use super::index::{ImageKind, Index, LbImage, NameHit};
use super::*;
use crate::launchbox::map::{choices, record, region_order};
use crate::types::ArtKind;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/launchbox").join(name)).unwrap()
}

/// The archive as LaunchBox publishes it, with the fixture's two files.
pub(super) fn fixture_zip() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["Metadata.xml", "Files.xml"] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&fixture(name)).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A data folder whose LaunchBox index is built from the fixture.
fn with_index() -> (tempfile::TempDir, DataPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let files = LaunchBox::files(&paths);
    std::fs::create_dir_all(files.dir()).unwrap();
    let archive = dir.path().join("Metadata.zip");
    std::fs::write(&archive, fixture_zip()).unwrap();
    build(&archive, &files.index(), &DownloadProgress::new()).unwrap();
    (dir, paths)
}

fn emulated(platform: &str, rom: Option<&str>, title: &str) -> SourceGame {
    SourceGame {
        app_name: "a".into(),
        runner: "sideload".into(),
        title: title.into(),
        emulated: true,
        platform: Some(platform.into()),
        rom_file: rom.map(Into::into),
        ..Default::default()
    }
}

fn found(lb: &LaunchBox, paths: &DataPaths, game: &SourceGame) -> Option<(i64, MatchedBy)> {
    lb.lookup(paths, game).into_option().map(|(g, by)| (g.id, by))
}

#[test]
fn the_index_keeps_what_the_source_uses() {
    let (_dir, paths) = with_index();
    let index = Index::open(&LaunchBox::index_path(&paths)).unwrap();
    assert_eq!(index.game_count().unwrap(), 27);
    let zelda = index.game(1318).unwrap().unwrap();
    assert_eq!(zelda.name, "The Legend of Zelda: A Link to the Past");
    assert_eq!(zelda.platform, "Super Nintendo Entertainment System");
    assert_eq!(zelda.released.as_deref(), Some("1991-11-21"));
    assert_eq!((zelda.detail.max_players, zelda.detail.votes), (Some(1), 1379));
    assert!(zelda.detail.overview.as_deref().is_some_and(|o| o.contains("Hyrule") && !o.contains('\n')));
    // Box fronts of each region, one background; no banners, logos or screenshots.
    assert!(zelda.images.iter().filter(|i| i.kind == ImageKind::BoxFront).count() >= 4);
    assert!(zelda.images.iter().filter(|i| i.kind == ImageKind::Background).count() <= 1);
    let names = index.names_of(1318).unwrap();
    assert!(names.contains(&("legendofzeldaalinktothepast".to_string(), false)));
    assert!(names.iter().any(|(k, alt)| *alt && k == "zeldanodensetsukamigaminotriforce"));
    assert_eq!(index.game(999_999_999).unwrap(), None);
    // `Files.xml`: a MAME set name and dump names, by platform.
    let arcade = index.platform_ids(&["Arcade"]).unwrap();
    assert_eq!(index.file_key(&arcade, "SFA2").unwrap().as_deref(), Some("streetfighteralpha2"));
    assert_eq!(index.platform_ids(&["Arcade", "Nintendo 64", "Windows"]).unwrap().len(), 2, "unknown platforms left out");
}

#[test]
fn a_truncated_or_foreign_archive_builds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("index.sqlite");
    let zip = fixture_zip();
    let cut = dir.path().join("cut.zip");
    std::fs::write(&cut, &zip[..zip.len() / 2]).unwrap();
    assert!(build(&cut, &out, &DownloadProgress::new()).is_err());
    let text = dir.path().join("page.zip");
    std::fs::write(&text, b"<html>Too many requests</html>").unwrap();
    assert!(build(&text, &out, &DownloadProgress::new()).is_err());
    // A zip whose XML ends early.
    let mut short = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    short.start_file("Metadata.xml", zip::write::SimpleFileOptions::default()).unwrap();
    let xml = fixture("Metadata.xml");
    short.write_all(&xml[..xml.len() / 3]).unwrap();
    let short_zip = dir.path().join("short.zip");
    std::fs::write(&short_zip, short.finish().unwrap().into_inner()).unwrap();
    assert!(build(&short_zip, &out, &DownloadProgress::new()).is_err());
}

#[test]
fn a_cancelled_build_stops() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("Metadata.zip");
    std::fs::write(&archive, fixture_zip()).unwrap();
    let progress = DownloadProgress::new();
    progress.cancel();
    let err = build(&archive, &dir.path().join("index.sqlite"), &progress).unwrap_err();
    assert!(is_cancelled(&err), "{err:#}");
}

/// Real dump names (No-Intro, Redump, MAME) on their platforms.
#[test]
fn lookups_by_dump_name() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    let cases = [
        ("nintendo_super_nes", "Legend of Zelda, The - A Link to the Past (USA).sfc", 1318),
        ("nintendo_super_nes", "Super Metroid (Japan, USA) (En,Ja).sfc", 299),
        ("sony_playstation", "Castlevania - Symphony of the Night (USA) (Track 1).bin", 511),
        ("sony_playstation", "Metal Gear Solid (USA) (Disc 2) (Rev 1).chd", 196),
        ("sony_playstation", "Final Fantasy VII (USA) (Disc 1).cue", 525),
        ("sega_genesis", "Sonic The Hedgehog 2 (World) (Rev A).md", 108429),
        ("nintendo_gameboyadvance", "Pokemon - FireRed Version (USA, Europe) (Rev 1).gba", 3490),
        ("arcade", "sfa2.zip", 2532),
    ];
    for (platform, rom, id) in cases {
        assert_eq!(found(&lb, &paths, &emulated(platform, Some(rom), "x")), Some((id, MatchedBy::ExactTitle)), "{rom}");
    }
}

#[test]
fn lookups_by_alternate_name_and_by_title() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    // Final Fantasy VI was Final Fantasy III in North America.
    let ff = emulated("nintendo_super_nes", Some("Final Fantasy III (USA) (Rev 1).sfc"), "Final Fantasy III");
    assert_eq!(found(&lb, &paths, &ff), Some((108333, MatchedBy::ExactTitle)));
    let bare = emulated("sega_genesis", Some("Bare Knuckle II - Shitou e no Requiem (Japan).md"), "x");
    assert_eq!(found(&lb, &paths, &bare), Some((694, MatchedBy::ExactTitle)));
    // No ROM name, or one that names nothing: the game's title.
    assert_eq!(found(&lb, &paths, &emulated("sony_playstation", None, "Chrono Trigger")), Some((10848, MatchedBy::ExactTitle)));
    let unnamed = emulated("nintendo_super_nes", Some("rom1.sfc"), "Chrono Trigger");
    assert_eq!(found(&lb, &paths, &unnamed), Some((1255, MatchedBy::ExactTitle)), "the SNES one, not the PlayStation one");
    // A longer name of LaunchBox's: only a guess.
    let sf = emulated("sega_genesis", None, "Street Fighter II' Special");
    assert_eq!(found(&lb, &paths, &sf), Some((7394, MatchedBy::FuzzyTitle)));
}

#[test]
fn lookups_never_cross_platforms() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    // Streets of Rage 2 is a Genesis game; Metroid Fusion a Game Boy Advance one.
    assert!(matches!(
        lb.lookup(&paths, &emulated("nintendo_super_nes", Some("Streets of Rage 2 (USA).sfc"), "Streets of Rage 2")),
        Lookup::NotFound
    ));
    assert!(matches!(lb.lookup(&paths, &emulated("sony_playstation", None, "Metroid Fusion")), Lookup::NotFound));
    // A platform LaunchBox has none of in the index, or one the app does not know.
    assert!(matches!(lb.lookup(&paths, &emulated("nintendo_64", None, "Super Metroid")), Lookup::NotFound));
    assert!(matches!(lb.lookup(&paths, &emulated("no_such_platform", None, "Super Metroid")), Lookup::NotFound));
    let mut unknown = emulated("x", None, "Super Metroid");
    unknown.platform = None;
    assert!(matches!(lb.lookup(&paths, &unknown), Lookup::NotFound));
}

#[test]
fn pc_games_by_steam_id_then_title() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    let pc =
        |title: &str, steam: Option<&str>| SourceGame { title: title.into(), steam_app_id: steam.map(Into::into), ..Default::default() };
    assert_eq!(found(&lb, &paths, &pc("Portal Two", Some("620"))), Some((914, MatchedBy::SteamId)));
    assert_eq!(found(&lb, &paths, &pc("Hollow Knight", None)), Some((123770, MatchedBy::ExactTitle)));
    // Its Windows list only: the SNES Super Mario World is not a PC game.
    assert!(matches!(lb.lookup(&paths, &pc("Super Mario World", None)), Lookup::NotFound));
}

#[test]
fn several_games_of_one_name_are_a_guess() {
    let hit = |game, platform, alt| NameHit { game, platform, alt };
    assert_eq!(choose(&[hit(5, 1, false)], &[1]), Some((5, true)));
    assert_eq!(choose(&[hit(5, 1, true), hit(6, 1, false)], &[1]), Some((6, true)), "a game's own name before another's alternate one");
    assert_eq!(choose(&[hit(5, 2, false), hit(6, 1, false)], &[1, 2]), Some((6, false)), "the main platform first, but not sure");
    assert_eq!(choose(&[hit(5, 2, true), hit(6, 1, false)], &[1, 2]), Some((6, true)), "another's alternate name leaves it sure");
    assert_eq!(choose(&[hit(7, 1, false), hit(6, 1, false)], &[1]), Some((6, false)), "two games: not sure");
    assert_eq!(choose(&[], &[1]), None);
}

/// An index of `games` (`(id, name, platform)`), each with a few votes.
fn index_of(games: &[(i64, &str, &str)]) -> (tempfile::TempDir, DataPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let files = LaunchBox::files(&paths);
    std::fs::create_dir_all(files.dir()).unwrap();
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<LaunchBox>\n");
    for (id, name, platform) in games {
        xml.push_str(&format!(
            "<Game><Name>{name}</Name><DatabaseID>{id}</DatabaseID><Platform>{platform}</Platform><CommunityRatingCount>{}</CommunityRatingCount></Game>\n",
            id % 7
        ));
    }
    xml.push_str("</LaunchBox>\n");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("Metadata.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    let archive = dir.path().join("Metadata.zip");
    std::fs::write(&archive, zip.finish().unwrap().into_inner()).unwrap();
    build(&archive, &files.index(), &DownloadProgress::new()).unwrap();
    (dir, paths)
}

const NES: &str = "Nintendo Entertainment System";
const FDS: &str = "Nintendo Famicom Disk System";

/// The NES list holds the Famicom Disk System too: a game of the same name
/// on both is not known for sure from its name.
#[test]
fn a_game_of_the_same_name_on_another_platform_of_the_list_makes_a_guess() {
    let (_dir, paths) = index_of(&[(1, "The Legend of Zelda", NES), (2, "The Legend of Zelda", FDS), (3, "Excitebike", NES)]);
    let lb = LaunchBox::new();
    let nes = emulated("nintendo_nes", Some("Legend of Zelda, The (USA).nes"), "The Legend of Zelda");
    assert_eq!(found(&lb, &paths, &nes), Some((1, MatchedBy::FuzzyTitle)));
    let fds = emulated("nintendo_nes", Some("Legend of Zelda, The (Japan).fds"), "The Legend of Zelda");
    assert_eq!(found(&lb, &paths, &fds), Some((1, MatchedBy::FuzzyTitle)), "a guess: it cannot lead the order");
    assert_eq!(found(&lb, &paths, &emulated("nintendo_nes", None, "Excitebike")), Some((3, MatchedBy::ExactTitle)), "alone: sure");
    assert!(crate::platforms::by_id("nintendo_nes").unwrap().names_in(Database::LaunchBox).contains(&FDS));
}

/// A search by hand: the names that are the query, then those that start
/// with it, then those that hold it, however many come before on other
/// platforms; digits search names and an id.
#[test]
fn a_search_ranks_exact_then_start_then_inside() {
    let mut games: Vec<(i64, String, &str)> =
        (0..400).map(|i| (10_000 + i, format!("Mario Party Collection Volume {i}"), "Arcade")).collect();
    games.extend((0..400).map(|i| (20_000 + i, format!("Tribute to Mario Fans Number {i}"), "Arcade")));
    games.push((5, "Super Mario World".into(), "Super Nintendo Entertainment System"));
    games.push((6, "Mario".into(), "Nintendo Entertainment System"));
    games.push((7, "1942".into(), "Arcade"));
    games.push((1942, "Commando".into(), "Arcade"));
    games.push((8, "19420 Remix".into(), "Arcade"));
    let refs: Vec<(i64, &str, &str)> = games.iter().map(|(id, n, p)| (*id, n.as_str(), *p)).collect();
    let (_dir, paths) = index_of(&refs);
    let lb = LaunchBox::new();
    let hits = lb.search(&paths, "mario").into_option().unwrap();
    assert_eq!(hits.len(), SEARCH_RESULTS);
    assert_eq!((hits[0].id, hits[1].name.starts_with("Mario Party")), (6, true), "the name itself, then names starting with it");
    let hits = lb.search(&paths, "super mario").into_option().unwrap();
    assert_eq!(hits[0].id, 5);
    // Inside a name: the shortest first, past 300 longer ones.
    let hits = lb.search(&paths, "ario world").into_option().unwrap();
    assert_eq!(hits.first().map(|g| (g.id, g.platform.as_str())), Some((5, "Super Nintendo Entertainment System")));
    // Digits: the game named so, the game of that id, then longer names.
    let ids: Vec<i64> = lb.search(&paths, "1942").into_option().unwrap().iter().map(|g| g.id).collect();
    assert_eq!(ids, [7, 1942, 8]);
}

#[test]
fn a_record_with_box_art_of_the_region() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    let zelda = lb.by_id(&paths, "1318").into_option().unwrap();
    let base = "https://images.launchbox-app.com";
    let md = record("a", "sideload", &zelda, &region_order("auto", Lang::En), base, None);
    assert_eq!(md.title.as_deref(), Some("The Legend of Zelda: A Link to the Past"));
    assert_eq!(md.cover_url.as_deref(), Some("https://images.launchbox-app.com/r2_689c3eda-bd3b-452d-997c-c25b5335656e.jpg"));
    assert_eq!(md.platforms, ["Super Nintendo Entertainment System"]);
    assert_eq!(md.developers, ["Nintendo EAD"]);
    assert_eq!(md.critic_score.map(|s| s.get()), Some(95.0));
    assert_eq!(md.game_modes, ["Single player"]);
    assert!(md.description.is_some() && md.release_date.as_deref() == Some("1991-11-21"));
    let br = record("a", "sideload", &zelda, &region_order("auto", Lang::PtBr), base, None);
    assert_eq!(
        br.cover_url.as_deref(),
        Some("https://images.launchbox-app.com/5abfc7aa-e11f-421d-8d7b-3c1ccb6c2437.jpg"),
        "the Brazilian box"
    );
    let jp = record("a", "sideload", &zelda, &region_order("Japan", Lang::En), base, None);
    assert_eq!(jp.cover_url.as_deref(), Some("https://images.launchbox-app.com/1d1c5081-6f2e-427a-ae3f-a05d9d21afeb.jpg"));
    // A rating of one vote is no score.
    let hack = lb.by_id(&paths, "457477").into_option().unwrap();
    assert_eq!((hack.detail.votes, record("a", "s", &hack, &[], base, None).critic_score), (1, None));
    let sor = lb.by_id(&paths, "694").into_option().unwrap();
    assert_eq!(record("a", "s", &sor, &[], base, None).game_modes, ["Single player", "Multiplayer", "Co-operative"]);
}

/// Super Metroid's boxes in the fixture, with their real sizes: the North
/// American one is landscape, the Japanese one tall; LaunchBox's square
/// art (the index keeps one per game) is 1024 x 1024.
const METROID_NA: &str = "r2_4f09af04-78b8-43e9-97a9-a8d9c4bb0b85.jpg";
const METROID_JP: &str = "cc0a933e-52b2-498b-bc3b-3e1d520d70f9.jpg";
const METROID_SQUARE: &str = "r2_321db153-e1cf-425a-bd76-680e84ba85d1.png";

fn metroid_size(img: &LbImage) -> Option<(u32, u32)> {
    match img.file.as_str() {
        METROID_NA => Some((4209, 2937)),
        METROID_JP => Some((1280, 2302)),
        METROID_SQUARE => Some((1024, 1024)),
        _ => None,
    }
}

/// The box art is the tall cover whatever its shape, and the cover of the
/// family its shape says: a landscape box the wide one, a CD case the
/// square one (ahead of LaunchBox's square art).
#[test]
fn box_art_fills_the_family_of_its_shape() {
    let (_dir, paths) = with_index();
    let metroid = LaunchBox::new().by_id(&paths, "299").into_option().unwrap();
    let order = region_order("auto", Lang::En);
    let url = |file: &str| format!("http://img/{file}");
    let of = |family: Option<CoverFamily>| {
        let md = record("a", "s", &metroid, &order, "http://img", family);
        (md.cover_url, md.cover_square_url, md.cover_wide_url)
    };
    assert_eq!(of(None), (Some(url(METROID_NA)), Some(url(METROID_SQUARE)), None), "shape not read: as before");
    assert_eq!(of(Some(CoverFamily::Tall)), of(None));
    assert_eq!(of(Some(CoverFamily::Wide)), (Some(url(METROID_NA)), Some(url(METROID_SQUARE)), Some(url(METROID_NA))));
    assert_eq!(of(Some(CoverFamily::Square)), (Some(url(METROID_NA)), Some(url(METROID_NA)), None));
}

/// The Covers dialog: every region's box, those of the slot's shape first,
/// then by region; the square art with the square boxes; the background.
#[test]
fn the_dialog_lists_every_box_its_shape_first() {
    let (_dir, paths) = with_index();
    let metroid = LaunchBox::new().by_id(&paths, "299").into_option().unwrap();
    let order = region_order("auto", Lang::En);
    let list = |kind: ArtKind| choices(&metroid, kind, &order, "http://img", metroid_size);
    let files =
        |kind: ArtKind| list(kind).into_iter().map(|c| c.image.image_url.trim_start_matches("http://img/").to_owned()).collect::<Vec<_>>();

    let wide = list(ArtKind::CoverWide);
    assert_eq!(wide.len(), 6, "the six regions' boxes");
    assert_eq!((wide[0].region.as_str(), wide[0].image.width, wide[0].image.height), ("North America", 4209, 2937));
    assert_eq!(files(ArtKind::Cover)[..2], [METROID_JP, METROID_NA], "the tall box first, then by region");
    let square = files(ArtKind::CoverSquare);
    assert_eq!(square[0], METROID_SQUARE, "the only square picture first");
    assert_eq!(square.len(), 7, "the boxes and the square picture");
    assert!(list(ArtKind::Cover).iter().all(|c| c.image.thumb_url == c.image.image_url));
    let hero = list(ArtKind::Hero);
    assert_eq!((hero.len(), hero[0].region.as_str(), hero[0].image.width), (1, "World", 0), "size not read: 0 x 0");
}

#[test]
fn search_by_hand_spans_platforms() {
    let (_dir, paths) = with_index();
    let lb = LaunchBox::new();
    let hits = lb.search(&paths, "link to the past").into_option().unwrap();
    let platforms: Vec<&str> = hits.iter().map(|g| g.platform.as_str()).collect();
    assert_eq!(hits.len(), 2);
    assert!(platforms.contains(&"Windows") && platforms.contains(&"Super Nintendo Entertainment System"));
    assert_eq!(hits[0].id, 1318, "the one more people rated first");
    let hit = map::hit(&hits[0], &region_order("auto", Lang::En), "http://img");
    assert_eq!(hit.platform.as_deref(), Some("Super Nintendo Entertainment System"));
    assert_eq!(hit.id.as_deref(), Some("1318"));
    assert!(hit.thumb_url.is_some_and(|u| u.starts_with("http://img/")));
    assert_eq!(lb.search(&paths, "525").into_option().unwrap()[0].name, "Final Fantasy VII", "a number is an id");
    assert!(lb.search(&paths, "").into_option().unwrap().is_empty());
    assert!(matches!(lb.by_id(&paths, "abc"), Lookup::NotFound));
}

#[test]
fn nothing_is_asked_without_a_download() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let lb = LaunchBox::new();
    assert_eq!(lb.state(&paths), DownloadState::default());
    assert!(matches!(lb.lookup(&paths, &emulated("nintendo_super_nes", None, "Super Metroid")), Lookup::Skipped));
    assert!(matches!(lb.search(&paths, "metroid"), Lookup::Skipped));
}

#[test]
fn removing_frees_the_space() {
    let (dir, paths) = with_index();
    let lb = LaunchBox::new();
    assert!(lb.state(&paths).ready && lb.state(&paths).size_on_disk > 0);
    assert!(lb.lookup(&paths, &emulated("nintendo_super_nes", None, "Super Metroid")).into_option().is_some());
    lb.remove(&paths).unwrap();
    assert_eq!(lb.state(&paths), DownloadState::default());
    assert!(matches!(lb.lookup(&paths, &emulated("nintendo_super_nes", None, "Super Metroid")), Lookup::Skipped));
    assert!(Path::new(&dir.path().join("store_cache/sources/launchbox")).read_dir().unwrap().next().is_none());
}

/// A synthetic index of 150,000 games: a lookup stays far under a millisecond.
#[test]
fn lookups_are_fast_on_a_full_size_index() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let files = LaunchBox::files(&paths);
    std::fs::create_dir_all(files.dir()).unwrap();
    let platforms = ["Super Nintendo Entertainment System", "Sony Playstation", "Sega Genesis", "Windows", "Arcade"];
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<LaunchBox>\n");
    for i in 0..150_000 {
        let platform = platforms[i % platforms.len()];
        xml.push_str(&format!(
            "<Game><Name>Game Number {i} Adventure</Name><DatabaseID>{i}</DatabaseID><Platform>{platform}</Platform><Overview>The story of game {i}.</Overview><CommunityRating>4</CommunityRating><CommunityRatingCount>12</CommunityRatingCount></Game>\n"
        ));
    }
    for i in (0..150_000).step_by(3) {
        xml.push_str(&format!(
            "<GameAlternateName><AlternateName>Other Name {i}</AlternateName><DatabaseID>{i}</DatabaseID></GameAlternateName>\n"
        ));
        xml.push_str(&format!("<GameImage><DatabaseID>{i}</DatabaseID><FileName>{i}.jpg</FileName><Type>Box - Front</Type><Region>World</Region></GameImage>\n"));
    }
    xml.push_str("</LaunchBox>\n");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("Metadata.xml", stored).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    let archive = dir.path().join("Metadata.zip");
    std::fs::write(&archive, zip.finish().unwrap().into_inner()).unwrap();
    assert_eq!(build(&archive, &files.index(), &DownloadProgress::new()).unwrap().games, 150_000);

    let lb = LaunchBox::new();
    let games: Vec<SourceGame> = (0..200)
        .map(|n| {
            let i = n * 743 % 150_000;
            let platform = ["nintendo_super_nes", "sony_playstation", "sega_genesis"][i % 5 % 3];
            let rom = if i % 5 < 3 { format!("Game Number {i} Adventure (USA).bin") } else { "x.bin".into() };
            emulated(platform, Some(&rom), "Nothing like it")
        })
        .collect();
    let _ = lb.lookup(&paths, &games[0]);
    let start = std::time::Instant::now();
    for game in &games {
        let _ = lb.lookup(&paths, game);
    }
    let each = start.elapsed() / games.len() as u32;
    // About 0.1 ms in a release build; a debug build is slower, hence the
    // margin, and the Windows runners' disks and file APIs slower still.
    let bound = std::time::Duration::from_millis(if cfg!(windows) { 25 } else { 5 });
    assert!(each < bound, "{each:?} per lookup");
    let hit = emulated("sony_playstation", Some("Game Number 1 Adventure (Japan).bin"), "x");
    assert_eq!(found(&lb, &paths, &hit), Some((1, MatchedBy::ExactTitle)));
}
