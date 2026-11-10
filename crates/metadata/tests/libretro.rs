//! libretro-thumbnails as a source, against a local server that answers as
//! the real one does (`fixtures/libretro/answers.json`): a `Range` GET of a
//! PNG's first 32 bytes is a 206, a missing file a 404. The names asked
//! and how many requests a game costs, the cover family by the picture's
//! size, when it is not asked, how it sits in the order, and offline.

mod common;

use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use common::{png_of, Req, Resp, Server};
use heronite_data::cover_family::CoverFamily;
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::sources::{SourceContext, SourceGame};
use heronite_metadata::*;
use parking_lot::Mutex;
use serde_json::{json, Value};

const PREFIX: &str = "/libretro/";
const SNES: &str = "Nintendo - Super Nintendo Entertainment System";
const GENESIS: &str = "Sega - Mega Drive - Genesis";

/// The pictures the server has: `System/Name` → size.
type Files = Arc<Mutex<HashMap<String, (u32, u32)>>>;

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// `System/Name` of a request for a box art, or `None` for anything else.
fn asked(req: &Req) -> Option<String> {
    let rest = unescape(req.path().strip_prefix(PREFIX)?);
    let (system, name) = rest.split_once("/Named_Boxarts/")?;
    Some(format!("{system}/{}", name.strip_suffix(".png")?))
}

struct Mock {
    server: Server,
    files: Files,
    /// A status every picture request gets instead (0: the normal answers).
    outage: Arc<AtomicU16>,
    /// That many picture requests are answered 429 with `Retry-After: 1` first.
    throttled: Arc<AtomicU16>,
}

impl Mock {
    fn requests(&self) -> Vec<String> {
        self.server.log.lock().iter().filter_map(asked).collect()
    }
}

/// The real server's answers; Lutris knows "Sonic The Hedgehog" and
/// "Super Metroid" by title, with a cover of its own.
fn mock(files: &[(&str, u32, u32)]) -> Mock {
    let files: Files = Arc::new(Mutex::new(files.iter().map(|(n, w, h)| (n.to_string(), (*w, *h))).collect()));
    let outage = Arc::new(AtomicU16::new(0));
    let throttled = Arc::new(AtomicU16::new(0));
    let (f, o, t) = (files.clone(), outage.clone(), throttled.clone());
    let server = Server::start(move |req: &Req| {
        if let Some(key) = asked(req) {
            if t.load(Ordering::SeqCst) > 0 {
                t.fetch_sub(1, Ordering::SeqCst);
                return Resp::status(429).with_header("Retry-After", "1");
            }
            return match (o.load(Ordering::SeqCst), f.lock().get(&key)) {
                (0, Some(&(w, h))) => match req.header("Range") {
                    Some("bytes=0-31") => {
                        let mut r = Resp::image(png_of(w, h, 32)).with_header("Content-Range", "bytes 0-31/314351");
                        r.status = 206;
                        r
                    }
                    _ => Resp::image(png_of(w, h, 3000)),
                },
                (0, None) => Resp::status(404),
                (status, _) => Resp::status(status),
            };
        }
        match req.path() {
            "/lutris/api/games" => {
                let q = req.query("search").unwrap_or_default();
                Resp::json(json!({"count": 1, "results": [{"id": 1, "name": q, "slug": q.to_lowercase().replace(' ', "-"), "year": 1991}]}))
            }
            p if p.starts_with("/lutris/api/games/") => {
                let slug = p.trim_start_matches("/lutris/api/games/");
                let name = if slug.starts_with("sonic") { "Sonic The Hedgehog" } else { "Super Metroid" };
                Resp::json(json!({"name": name, "slug": slug, "year": 1991, "description": "What Lutris says.",
                                  "genres": [{"name": "Platform"}], "platforms": [], "coverart": "https://lutris.net/c.jpg"}))
            }
            _ => Resp::status(404),
        }
    });
    Mock { server, files, outage, throttled }
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

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    svc: MetadataService,
    mock: Mock,
}

fn write(path: &std::path::Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn entry(app: &str, title: &str, platform: &str, rom: &str) -> Value {
    json!({"appName": app, "runner": "sideload", "title": title, "playniteId": "", "source": "emulator", "launchKind": "emulator",
           "platformId": platform, "roms": [{"path": rom}]})
}

/// Sonic on a Mega Drive, Super Metroid on a SNES, a PlayStation 3 game
/// and a PC game; `keep`: `keepCoverFamilies`.
fn setup(files: &[(&str, u32, u32)], keep: &[&str]) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    write(
        &paths.local_meta(),
        json!({"games": {
            "sonic": entry("sonic", "Sonic The Hedgehog", "sega_genesis", "/roms/md/Sonic The Hedgehog (USA, Europe).md"),
            "renamed": entry("renamed", "Sonic The Hedgehog", "sega_genesis", "/roms/md/sonic.md"),
            "sm": entry("sm", "Super Metroid", "nintendo_super_nes", "/roms/snes/Super Metroid (Japan, USA) (En,Ja).sfc"),
            "ps3": entry("ps3", "Demon's Souls", "sony_playstation3", "/roms/ps3/Demon's Souls (USA)/PS3_GAME/USRDIR/EBOOT.BIN"),
            "hk": {"appName": "hk", "runner": "sideload", "title": "Hollow Knight", "playniteId": "", "source": "manual", "launchKind": "executable"}
        }}),
    );
    write(&paths.collection_settings(), json!({"metadata": {"keepCoverFamilies": keep}}));
    let mock = mock(files);
    let svc = MetadataService::with_endpoints(paths.clone(), mock.server.endpoints());
    Setup { _dir: dir, paths, svc, mock }
}

fn args(app: &str, title: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: title.into(), ..Default::default() }
}

fn lookup(s: &Setup, game: &SourceGame) -> Lookup<MetadataCandidate> {
    let sources = s.svc.sources();
    let source = sources.lookups().find(|l| l.descriptor().id == "libretro").unwrap();
    source.lookup(&SourceContext::new(&s.paths, &s.svc.http), game)
}

fn url(s: &Setup, system: &str, name: &str) -> String {
    let escape = |text: &str| text.replace(' ', "%20").replace(',', "%2C");
    format!("{}/libretro/{}/Named_Boxarts/{}.png", s.mock.server.base, escape(system), escape(name))
}

#[test]
fn the_dumps_own_name_is_one_request_and_a_sure_match() {
    let s = setup(&[("Nintendo - Super Nintendo Entertainment System/Super Metroid (Japan, USA) (En,Ja)", 512, 357)], &[]);
    let c =
        lookup(&s, &emulated("nintendo_super_nes", Some("Super Metroid (Japan, USA) (En,Ja).sfc"), "Super Metroid")).into_option().unwrap();
    assert_eq!((c.matched_by, c.match_name.as_deref(), c.match_id), (Some(MatchedBy::ExactTitle), Some("Super Metroid"), None));
    assert!(c.matched_by.unwrap().is_sure() && c.adult_pictures.is_empty(), "no rating, nothing flagged adult");
    assert_eq!(s.mock.requests(), [format!("{SNES}/Super Metroid (Japan, USA) (En,Ja)")]);
    let req = s.mock.server.log.lock()[0].clone();
    assert_eq!(req.method, "GET");
    assert_eq!(req.header("Range"), Some("bytes=0-31"), "the head of the picture only");
    assert_eq!(req.header("User-Agent").unwrap_or_default(), format!("Heronite/{}", env!("CARGO_PKG_VERSION")));
    // 512 x 357: a landscape box is the wide cover, and only that.
    let md = &c.metadata;
    assert_eq!(md.cover_wide_url.as_deref(), Some(url(&s, SNES, "Super Metroid (Japan, USA) (En,Ja)").as_str()));
    assert!(md.cover_url.is_none() && md.cover_square_url.is_none() && md.hero_url.is_none() && md.title.is_none());
}

#[test]
fn the_characters_the_collection_scrubs_are_replaced_in_the_request() {
    let s = setup(&[("Sega - Mega Drive - Genesis/Sonic _ Knuckles (World)", 512, 731)], &[]);
    let c = lookup(&s, &emulated("sega_genesis", Some("Sonic & Knuckles (World).md"), "Sonic & Knuckles")).into_option().unwrap();
    assert_eq!(c.matched_by, Some(MatchedBy::ExactTitle));
    assert_eq!(s.mock.requests(), [format!("{GENESIS}/Sonic _ Knuckles (World)")]);
    assert_eq!(c.metadata.cover_url.as_deref(), Some(url(&s, GENESIS, "Sonic _ Knuckles (World)").as_str()));
    assert_eq!(c.match_name.as_deref(), Some("Sonic & Knuckles"), "the title as people write it, not the scrubbed file name");
}

#[test]
fn a_shortened_name_is_a_fuzzy_match_and_the_chain_is_ordered() {
    // The revision is not in the collection; the language group is.
    let s = setup(&[("Nintendo - Super Nintendo Entertainment System/Super Metroid (Europe) (En,Fr,De)", 512, 357)], &["wide"]);
    let c = lookup(&s, &emulated("nintendo_super_nes", Some("Super Metroid (Europe) (En,Fr,De) (Rev 1).sfc"), "Super Metroid"))
        .into_option()
        .unwrap();
    assert_eq!(c.matched_by, Some(MatchedBy::FuzzyTitle));
    assert!(!c.matched_by.unwrap().is_sure());
    assert_eq!(c.match_name.as_deref(), Some("Super Metroid"));
    assert_eq!(
        s.mock.requests(),
        [format!("{SNES}/Super Metroid (Europe) (En,Fr,De) (Rev 1)"), format!("{SNES}/Super Metroid (Europe) (En,Fr,De)"),]
    );

    // A renamed ROM: the title with the regions of the interface language.
    let s = setup(&[("Sega - Mega Drive - Genesis/Sonic The Hedgehog (Europe)", 512, 731)], &[]);
    let found = lookup(&s, &emulated("sega_genesis", Some("sonic.md"), "Sonic The Hedgehog")).into_option().unwrap();
    assert_eq!(found.matched_by, Some(MatchedBy::FuzzyTitle));
    let regions = heronite_metadata::libretro::names::regions_for(heronite_i18n::current());
    assert_eq!(s.mock.requests()[0], format!("{GENESIS}/sonic"), "its own name first");
    assert_eq!(s.mock.requests().len(), 1 + regions.iter().position(|r| *r == "Europe").unwrap() + 1, "then each region until one exists");

    // Without a ROM, the game's title: written as No-Intro writes it.
    let s =
        setup(&[("Nintendo - Super Nintendo Entertainment System/Legend of Zelda, The - A Link to the Past (USA)", 512, 357)], &["wide"]);
    let zelda = emulated("nintendo_super_nes", None, "The Legend of Zelda: A Link to the Past");
    assert!(lookup(&s, &zelda).into_option().is_some_and(|c| c.matched_by == Some(MatchedBy::FuzzyTitle)));
    assert_eq!(s.mock.requests()[0], format!("{SNES}/The Legend of Zelda_ A Link to the Past"));
    assert_eq!(s.mock.requests()[1], format!("{SNES}/Legend of Zelda, The - A Link to the Past (USA)"));
}

#[test]
fn the_regions_follow_the_interface_language() {
    use heronite_i18n::Lang;
    // The server has only the German box.
    for (lang, expected_first) in [(Lang::De, "Germany"), (Lang::PtBr, "Brazil"), (Lang::Es, "Spain")] {
        let regions = heronite_metadata::libretro::names::regions_for(lang);
        assert_eq!(regions[0], expected_first);
    }
    let s = setup(&[], &[]);
    let found = heronite_metadata::libretro::find(
        &s.svc.http,
        &emulated("sega_genesis", Some("sonic.md"), "Sonic"),
        Lang::De,
        &[heronite_data::cover_family::CoverFamily::Tall],
    );
    assert!(matches!(found, Lookup::NotFound));
    let asked = s.mock.requests();
    assert_eq!(asked[..3], [format!("{GENESIS}/sonic"), format!("{GENESIS}/Sonic (Germany)"), format!("{GENESIS}/Sonic (Europe)")]);
}

#[test]
fn a_game_costs_at_most_eight_requests() {
    let find = |s: &Setup, game: &SourceGame, lang| {
        heronite_metadata::libretro::find(&s.svc.http, game, lang, &heronite_data::cover_family::CoverFamily::ALL)
    };
    // Nothing exists: a dump name asks itself, its shorter forms, then the
    // regions of the language (German has six: one name too many).
    let s = setup(&[], &[]);
    let game = emulated("sega_genesis", Some("Sonic The Hedgehog (Europe) (En,Fr,De) (Rev A).md"), "Sonic The Hedgehog");
    assert!(matches!(find(&s, &game, heronite_i18n::Lang::De), Lookup::NotFound));
    let asked = s.mock.requests();
    assert_eq!(asked.len(), heronite_metadata::libretro::MAX_REQUESTS);
    assert_eq!(heronite_metadata::libretro::MAX_REQUESTS, 8);
    assert_eq!(asked[0], format!("{GENESIS}/Sonic The Hedgehog (Europe) (En,Fr,De) (Rev A)"));
    assert_eq!(asked[1], format!("{GENESIS}/Sonic The Hedgehog (Europe) (En,Fr,De)"));
    assert_eq!(asked[2], format!("{GENESIS}/Sonic The Hedgehog (Europe)"));
    assert_eq!(asked[3], format!("{GENESIS}/Sonic The Hedgehog (Germany)"));
    assert_eq!(asked[7], format!("{GENESIS}/Sonic The Hedgehog (Japan)"), "the title alone is not asked");
    assert!(asked.iter().all(|a| a.starts_with(GENESIS)), "one system");
    assert_eq!(asked.iter().collect::<std::collections::BTreeSet<_>>().len(), 8, "no name twice");
    // English: the same file fits (seven names, then the title alone).
    let s = setup(&[], &[]);
    assert!(matches!(find(&s, &game, heronite_i18n::Lang::En), Lookup::NotFound));
    assert_eq!(s.mock.requests().len(), 8);
    assert_eq!(s.mock.requests()[7], format!("{GENESIS}/Sonic The Hedgehog"));

    // A platform with three folders asks the dump's own name in each.
    let s = setup(&[], &[]);
    let game = emulated("nec_turbografx_16", Some("Gate of Thunder (USA).chd"), "Gate of Thunder");
    assert!(matches!(find(&s, &game, heronite_i18n::Lang::De), Lookup::NotFound));
    let asked = s.mock.requests();
    assert_eq!(asked.len(), 8);
    assert_eq!(
        asked[..3],
        [
            "NEC - PC Engine - TurboGrafx 16/Gate of Thunder (USA)",
            "NEC - PC Engine CD - TurboGrafx-CD/Gate of Thunder (USA)",
            "NEC - PC Engine SuperGrafx/Gate of Thunder (USA)"
        ]
    );
    assert!(asked[3..].iter().all(|a| a.starts_with("NEC - PC Engine - TurboGrafx 16/")), "the others, on the first");

    // A disc image of the second folder is found there, in three requests.
    let s = setup(&[("NEC - PC Engine CD - TurboGrafx-CD/Gate of Thunder (USA)", 512, 512)], &["square"]);
    let found = lookup(&s, &emulated("nec_turbografx_16", Some("Gate of Thunder (USA).chd"), "Gate of Thunder")).into_option().unwrap();
    assert_eq!(found.matched_by, Some(MatchedBy::ExactTitle));
    assert_eq!(s.mock.requests().len(), 2);
    // A Game Boy Color game goes to its own folder first.
    let s = setup(&[("Nintendo - Game Boy Color/Pokemon - Crystal Version (USA, Europe)", 512, 512)], &["square"]);
    let found =
        lookup(&s, &emulated("nintendo_gameboy", Some("Pokemon - Crystal Version (USA, Europe).gbc"), "Pokemon")).into_option().unwrap();
    assert_eq!(found.matched_by, Some(MatchedBy::ExactTitle));
    assert_eq!(s.mock.requests().len(), 1);
}

#[test]
fn the_shape_of_the_picture_picks_the_cover_family() {
    let files = [
        ("Sega - Mega Drive - Genesis/Tall (USA)", 512, 731),
        ("Sony - PlayStation/Round (USA)", 512, 512),
        ("Nintendo - Nintendo 64/Wide (USA)", 512, 357),
    ];
    let s = setup(&files, &["square", "wide"]);
    let fields = |c: &MetadataCandidate| {
        let m = &c.metadata;
        (m.cover_url.is_some(), m.cover_square_url.is_some(), m.cover_wide_url.is_some())
    };
    let tall = lookup(&s, &emulated("sega_genesis", Some("Tall (USA).md"), "Tall")).into_option().unwrap();
    let square = lookup(&s, &emulated("sony_playstation", Some("Round (USA).chd"), "Round")).into_option().unwrap();
    let wide = lookup(&s, &emulated("nintendo_64", Some("Wide (USA).z64"), "Wide")).into_option().unwrap();
    assert_eq!((fields(&tall), fields(&square), fields(&wide)), ((true, false, false), (false, true, false), (false, false, true)));

    // Only the families asked: a picture of another shape is not a find,
    // and the lookup stops there (no more requests for it).
    let only = |families: &[&str]| SourceGame {
        fields: Some(families.iter().map(|f| f.to_string()).collect()),
        ..emulated("sony_playstation", Some("Round (USA).chd"), "Round")
    };
    let before = s.mock.requests().len();
    assert!(matches!(lookup(&s, &only(&["cover", "coverWide"])), Lookup::NotFound));
    assert_eq!(s.mock.requests().len(), before + 1);
    assert!(lookup(&s, &only(&["coverSquare"])).into_option().is_some());
    // Nothing asked of it (it has no background): not asked at all.
    let before = s.mock.requests().len();
    assert!(matches!(lookup(&s, &only(&["hero", "description"])), Lookup::Skipped));
    assert_eq!(s.mock.requests().len(), before);
}

#[test]
fn it_is_not_asked_for_pc_games_nor_platforms_without_a_folder() {
    let s = setup(&[], &[]);
    for game in [
        SourceGame { emulated: false, platform: None, ..emulated("sega_genesis", Some("x.md"), "x") },
        emulated("sony_playstation3", Some("EBOOT.BIN"), "Demon's Souls"),
        emulated("arcade", Some("sf2.zip"), "Street Fighter II"),
        emulated("pc_dos", Some("DOOM.EXE"), "Doom"),
        SourceGame { platform: None, ..emulated("sega_genesis", Some("x.md"), "x") },
    ] {
        assert!(matches!(lookup(&s, &game), Lookup::Skipped), "{game:?}");
    }
    assert!(s.mock.requests().is_empty());

    // Through the service, which asks only the sources that apply.
    for app in ["hk", "ps3"] {
        let preview = s.svc.preview_game_metadata(&args(app, "Game")).unwrap();
        assert!(preview.candidates.iter().all(|c| c.source != "libretro"), "{app}");
    }
    assert!(s.mock.requests().is_empty(), "no request to the server for a PC game or a PlayStation 3 game");
    let preview = s.svc.preview_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap();
    assert!(preview.candidates.iter().any(|c| c.source == "lutris"));
    let asked = s.mock.requests();
    assert!(asked.len() >= 2 && asked.len() <= 8, "a game of a platform with a folder is asked (nothing exists here): {asked:?}");
    assert_eq!(asked[0], format!("{GENESIS}/Sonic The Hedgehog (USA, Europe)"));
}

#[test]
fn a_server_that_cannot_answer_is_unreachable_and_a_page_is_not_a_picture() {
    let s = setup(&[("Sega - Mega Drive - Genesis/Sonic The Hedgehog (USA, Europe)", 512, 731)], &[]);
    let game = emulated("sega_genesis", Some("Sonic The Hedgehog (USA, Europe).md"), "Sonic The Hedgehog");
    // Any status but 404 / 410 is no answer for the game, a block (403) too.
    for status in [429, 500, 503, 400, 401, 403] {
        s.mock.outage.store(status, Ordering::SeqCst);
        let before = s.mock.requests().len();
        assert!(matches!(lookup(&s, &game), Lookup::Unreachable), "{status}");
        assert_eq!(s.mock.requests().len(), before + 1, "{status}: it stops at the first answer it cannot use");
    }
    // The server answering 200 with a page (a captive portal) has no picture.
    s.mock.outage.store(0, Ordering::SeqCst);
    s.mock.files.lock().clear();
    let portal = Server::start(|_: &Req| Resp::html("<html>sign in</html>"));
    let http = Http::new(Endpoints { libretro_thumbnails: portal.base.clone(), ..s.mock.server.endpoints() });
    let found = heronite_metadata::libretro::find(&http, &game, heronite_i18n::Lang::En, &[heronite_data::cover_family::CoverFamily::Tall]);
    assert!(matches!(found, Lookup::NotFound));

    // No network at all.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            drop(stream);
        }
    });
    let svc = MetadataService::with_endpoints(
        s.paths.clone(),
        Endpoints { libretro_thumbnails: dead.clone(), lutris: format!("{dead}/lutris"), ..s.mock.server.endpoints() },
    );
    let preview = svc.preview_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap();
    assert!(preview.offline, "every network source asked was unreachable");
    // Only libretro down: the others still answer, and the fetch is not offline.
    let svc = MetadataService::with_endpoints(s.paths.clone(), Endpoints { libretro_thumbnails: dead, ..s.mock.server.endpoints() });
    let preview = svc.preview_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap();
    assert!(!preview.offline && preview.candidates.iter().any(|c| c.source == "lutris"));
}

/// Libretro's box art for a game found by its dump goes ahead of Lutris
/// (found by title); the user's order, a fuzzy match and a shape nobody
/// asked for change that.
#[test]
fn its_place_in_the_order() {
    let files = [
        ("Sega - Mega Drive - Genesis/Sonic The Hedgehog (USA, Europe)", 512, 731),
        ("Sega - Mega Drive - Genesis/Sonic The Hedgehog (World)", 512, 731),
        ("Nintendo - Super Nintendo Entertainment System/Super Metroid (Japan, USA) (En,Ja)", 512, 357),
    ];
    let s = setup(&files, &[]);
    let saved = s.svc.apply_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap();
    assert_eq!(saved.field_sources["cover"], "libretro", "found by the dump's name: ahead of Lutris's title match");
    assert_eq!(saved.cover_url.as_deref(), Some(url(&s, GENESIS, "Sonic The Hedgehog (USA, Europe)").as_str()));
    assert_eq!(saved.field_sources["description"], "lutris", "it has no description");
    assert!(saved.hero_url.is_none() && saved.field_sources.get("hero").is_none(), "and no background");

    // The user's order of the field wins.
    s.svc.set_field_order("cover", &["lutris".to_string(), "libretro".to_string()]).unwrap();
    let mut again = args("sonic", "Sonic The Hedgehog");
    again.overwrite_existing = Some(true);
    assert_eq!(s.svc.apply_game_metadata(&again).unwrap().field_sources["cover"], "lutris");

    // A renamed ROM: only a guess at the region. Lutris, sure by title, is
    // first by the default order; the guess is no leader.
    let s = setup(&files, &[]);
    let saved = s.svc.apply_game_metadata(&args("renamed", "Sonic The Hedgehog")).unwrap();
    assert_eq!(saved.field_sources["cover"], "lutris");

    // A picture of a shape no view keeps is not saved: Super Metroid's box is landscape.
    let s = setup(&files, &[]);
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert!(saved.cover_wide_url.is_none());
    assert_eq!(saved.field_sources["cover"], "lutris");
    // With the wide cover kept, it is the wide cover.
    let s = setup(&files, &["wide"]);
    let saved = s.svc.apply_game_metadata(&args("sm", "Super Metroid")).unwrap();
    assert_eq!(saved.field_sources["coverWide"], "libretro");
    assert_eq!(saved.cover_wide_url.as_deref(), Some(url(&s, SNES, "Super Metroid (Japan, USA) (En,Ja)").as_str()));
    assert_eq!(saved.field_sources["cover"], "lutris", "the tall cover stays Lutris's");

    // Off: not asked.
    s.svc.set_source_enabled("libretro", false).unwrap();
    let before = s.mock.requests().len();
    let mut again = args("sm", "Super Metroid");
    again.overwrite_existing = Some(true);
    s.svc.apply_game_metadata(&again).unwrap();
    assert_eq!(s.mock.requests().len(), before);
}

#[test]
fn the_settings_row_is_a_pictures_source_without_options() {
    let s = setup(&[], &[]);
    let settings = s.svc.metadata_settings();
    assert_eq!(s.svc.source_status(&settings, "libretro"), Some(sources::SourceStatus::On));
    assert_eq!(settings.field_priority["cover"].last().map(String::as_str), Some("libretro"));
    assert_eq!(settings.field_priority["coverWide"].last().map(String::as_str), Some("libretro"));
    assert!(!settings.field_priority["hero"].iter().any(|s| s == "libretro"));
    let row = s.svc.source_rows(&settings).into_iter().find(|r| r.descriptor.id == "libretro").unwrap();
    assert!(row.options.is_empty() && row.download.is_none() && !row.can_test);
    // A search by hand does not offer it: there is nothing to search.
    let found = s.svc.search_sources("Super Metroid", Some(&["libretro".to_string()]));
    assert!(found.asked.is_empty() && found.choices.is_empty());
    let found = s.svc.search_sources("Super Metroid", None);
    assert!(!found.asked.contains(&"libretro".to_string()));
    assert!(s.mock.requests().is_empty());
}

/// A source of emulated games alone must not promise a download of square
/// covers to a Collection that has none, or to the games that are not for it.
#[test]
fn a_family_download_is_offered_only_where_the_source_applies() {
    let s = setup(&[], &[]);
    let item =
        |app: &str, title: &str| BulkItem { app_name: app.into(), runner: "sideload".into(), title: title.into(), steam_app_id: None };
    // No SteamGridDB key and no LaunchBox download: libretro is the only source of square covers that is ready.
    assert!(s.svc.can_download_family(heronite_data::cover_family::CoverFamily::Square), "Sonic is emulated");
    let square = heronite_data::cover_family::CoverFamily::Square;
    assert!(s.svc.family_lack(square, Some(vec![item("sonic", "Sonic The Hedgehog")])).unwrap().can_download);
    assert!(!s.svc.family_lack(square, Some(vec![item("hk", "Hollow Knight")])).unwrap().can_download, "a PC game");
    assert!(!s.svc.family_lack(square, Some(vec![item("ps3", "Demon's Souls")])).unwrap().can_download, "a platform without a folder");

    write(
        &s.paths.local_meta(),
        json!({"games": {"hk": {"appName": "hk", "runner": "sideload", "title": "Hollow Knight", "playniteId": "", "source": "manual",
                                  "launchKind": "executable"}}}),
    );
    assert!(!s.svc.can_download_family(square), "no emulated game in the Collection");
    assert!(s.svc.cover_family_job(square, None).is_none());
    assert!(s.mock.requests().is_empty(), "deciding asks no source");
}

#[test]
fn only_404_and_410_say_a_name_is_missing() {
    let s = setup(&[], &[]);
    let game = emulated("sega_genesis", Some("Sonic The Hedgehog (Europe) (En,Fr,De) (Rev A).md"), "Sonic The Hedgehog");
    for status in [404, 410] {
        s.mock.outage.store(status, Ordering::SeqCst);
        let client = heronite_metadata::libretro::Libretro::new();
        let before = s.mock.requests().len();
        assert!(matches!(client.find(&s.svc.http, &game, heronite_i18n::Lang::En, &CoverFamily::ALL), Lookup::NotFound), "{status}");
        assert_eq!(s.mock.requests().len(), before + heronite_metadata::libretro::MAX_REQUESTS, "{status}: the next names are asked");
    }
}

/// How much earlier than asked a wait may end as `Instant` measures it:
/// none on Unix; on Windows the timer a sleep waits on and `Instant` (the
/// performance counter) are different clocks.
const EARLY: std::time::Duration = std::time::Duration::from_millis(if cfg!(windows) { 50 } else { 0 });

#[test]
fn a_short_retry_after_is_waited_out_once() {
    let s = setup(&[("Sega - Mega Drive - Genesis/Sonic The Hedgehog (USA, Europe)", 512, 731)], &[]);
    let game = emulated("sega_genesis", Some("Sonic The Hedgehog (USA, Europe).md"), "Sonic The Hedgehog");
    s.mock.throttled.store(1, Ordering::SeqCst);
    let started = std::time::Instant::now();
    assert!(lookup(&s, &game).into_option().is_some_and(|c| c.matched_by == Some(MatchedBy::ExactTitle)));
    assert!(started.elapsed() + EARLY >= std::time::Duration::from_secs(1), "Retry-After: 1 was honoured: {:?}", started.elapsed());
    assert_eq!(s.mock.requests().len(), 2, "refused once, then answered");
    // Refused again after the wait: no answer for this game, no more names asked.
    s.mock.throttled.store(2, Ordering::SeqCst);
    assert!(matches!(lookup(&s, &game), Lookup::Unreachable));
    assert_eq!(s.mock.requests().len(), 4);
}

#[test]
fn requests_are_spaced_and_missing_names_are_not_asked_twice() {
    let s = setup(&[], &[]);
    let game = emulated("sega_genesis", Some("Sonic The Hedgehog (Europe) (En,Fr,De) (Rev A).md"), "Sonic The Hedgehog");
    let started = std::time::Instant::now();
    assert!(matches!(lookup(&s, &game), Lookup::NotFound));
    assert_eq!(s.mock.requests().len(), heronite_metadata::libretro::MAX_REQUESTS);
    assert!(started.elapsed() + EARLY >= std::time::Duration::from_millis(700), "about 100 ms apart: {:?}", started.elapsed());
    // A second pass (another family's download): the names are known missing.
    let only_square = SourceGame { fields: Some(vec!["coverSquare".into()]), ..game.clone() };
    assert!(matches!(lookup(&s, &only_square), Lookup::NotFound));
    assert_eq!(s.mock.requests().len(), heronite_metadata::libretro::MAX_REQUESTS, "no request again");
    // A picture is asked again each time: only missing names are remembered.
    s.mock.files.lock().insert(format!("{GENESIS}/Sonic The Hedgehog (Europe) (En,Fr,De) (Rev A)"), (512, 731));
    assert!(matches!(lookup(&s, &game), Lookup::NotFound), "still remembered as missing");
}

#[test]
fn a_title_without_a_rom_is_a_guess() {
    // The server has a picture named after the title itself.
    let s = setup(&[("Nintendo - Super Nintendo Entertainment System/Super Metroid", 512, 357)], &["wide"]);
    let c = lookup(&s, &emulated("nintendo_super_nes", None, "Super Metroid")).into_option().unwrap();
    assert_eq!(c.matched_by, Some(MatchedBy::FuzzyTitle));
    assert_eq!(s.mock.requests(), [format!("{SNES}/Super Metroid")]);
    // The same name as a ROM's file is the dump's own: sure.
    let c = lookup(&s, &emulated("nintendo_super_nes", Some("Super Metroid.sfc"), "Super Metroid")).into_option().unwrap();
    assert_eq!(c.matched_by, Some(MatchedBy::ExactTitle));
}

/// An emulated game no details source has (Lutris off, no IGDB keys, no
/// LaunchBox download): it is found in no source and its sure box art is
/// saved with that mark, by the idle fill and by a job.
#[test]
fn the_box_art_of_a_game_no_details_source_has_is_saved_with_the_mark() {
    let files = [("Sega - Mega Drive - Genesis/Sonic The Hedgehog (USA, Europe)", 512, 731)];
    let s = setup(&files, &[]);
    s.svc.set_source_enabled("lutris", false).unwrap();
    let saved = s.svc.apply_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap();
    assert_eq!(saved.fetch_error.as_deref(), Some("No metadata sources returned data"), "tried again in a week");
    assert_eq!(saved.field_sources["cover"], "libretro");
    assert_eq!(saved.cover_url.as_deref(), Some(url(&s, GENESIS, "Sonic The Hedgehog (USA, Europe)").as_str()));
    assert!(saved.description.is_none());
    // A renamed ROM: only a guess, not saved.
    let saved = s.svc.apply_game_metadata(&args("renamed", "Sonic The Hedgehog")).unwrap();
    assert!(saved.fetch_error.is_some() && saved.cover_url.is_none());

    let s = setup(&files, &[]);
    s.svc.set_source_enabled("lutris", false).unwrap();
    let item = BulkItem { app_name: "sonic".into(), runner: "sideload".into(), title: "Sonic The Hedgehog".into(), steam_app_id: None };
    let done = s.svc.run_bulk_metadata(&BulkArgs::fill_missing(vec![item]), &|_| {});
    assert_eq!(done.results[0].outcome, GameOutcome::Updated);
    let md = s.svc.get_game_metadata("sideload", "sonic").unwrap().unwrap();
    assert_eq!((md.field_sources["cover"].as_str(), md.fetch_error.is_some()), ("libretro", true));

    // libretro down, the only source asked: offline, nothing written.
    let s = setup(&files, &[]);
    s.svc.set_source_enabled("lutris", false).unwrap();
    s.mock.outage.store(503, Ordering::SeqCst);
    assert!(service::is_offline(&s.svc.apply_game_metadata(&args("sonic", "Sonic The Hedgehog")).unwrap_err()));
    assert!(s.svc.get_game_metadata("sideload", "sonic").unwrap().is_none());
}
