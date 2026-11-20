//! Read-only requests to the real services, to check the parsers against
//! today's responses. Opt in with `HERONITE_LIVE=1`; IGDB additionally needs
//! `HERONITE_FIXTURE` (credentials are read from its `settings.json` and never
//! printed). Writes go to a tempdir.

use heronite_data::paths::DataPaths;
use heronite_metadata::*;

fn enabled() -> bool {
    std::env::var_os("HERONITE_LIVE").is_some()
}

fn service() -> (tempfile::TempDir, MetadataService) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    if let Some(root) = std::env::var_os("HERONITE_FIXTURE") {
        let src = std::path::Path::new(&root).join("local_library/settings.json");
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::copy(src, paths.collection_settings()).unwrap();
    }
    let svc = MetadataService::new(paths);
    (dir, svc)
}

#[test]
fn live_steam_lutris_hltb() {
    if !enabled() {
        return;
    }
    let (_d, svc) = service();
    let d = svc.steam_details("400", Some("en")).expect("steam appdetails");
    assert_eq!(d.name, "Portal");
    assert!(svc.paths.steam_details("400", "english").exists());

    let l = heronite_metadata::lutris::fetch_lutris_metadata(&svc.http, "a", "sideload", "Portal", None).into_option().expect("lutris");
    assert!(l.lutris_slug.as_deref().is_some_and(|s| s.starts_with("portal")));
    assert_eq!(l.title.as_deref(), Some("Portal"));

    let h = svc.how_long_to_beat("Portal 2", None).unwrap();
    let e = h.entry.expect("hltb");
    assert_eq!(e.game_id, Some(7231));
    assert!(e.main_story > 5.0);
    assert!(svc.paths.wiki_game_info().exists());

    let id = heronite_metadata::covers::steam_app_id_from_title(&svc.http, "Portal 2");
    assert_eq!(id.as_deref(), Some("620"));
}

#[test]
fn live_igdb() {
    if !enabled() || std::env::var_os("HERONITE_FIXTURE").is_none() {
        return;
    }
    let (_d, svc) = service();
    assert!(svc.igdb_configured());
    assert!(svc.test_igdb_credentials().ok);
    let hits = svc.search_game_metadata("Portal 2");
    assert!(hits.iter().any(|h| h.id == "72"));
    let c = svc.gather_candidates(&MetadataApplyArgs {
        app_name: "steam_620".into(),
        runner: "sideload".into(),
        title: "Portal 2".into(),
        steam_app_id: Some("620".into()),
        ..Default::default()
    });
    let igdb = c.iter().find(|c| c.source == "igdb").expect("igdb candidate by steam id");
    assert_eq!(igdb.metadata.igdb_id.map(|n| n.get()), Some(72.0));
    assert_eq!(igdb.metadata.steam_app_id.as_deref(), Some("620"));
}

#[test]
fn live_web_images() {
    if !enabled() {
        return;
    }
    let (_d, svc) = service();
    let images = svc.search_web_images("\"Portal 2\" cover", false).expect("some engine answers");
    assert!(!images.is_empty());
    assert!(images.iter().all(|i| i.image_url.starts_with("http") && i.thumb_url.starts_with("http")));
    eprintln!("web images: {} (first: {})", images.len(), images[0].image_url);
}

/// The real LaunchBox archive (about 105 MB) downloaded into a temporary
/// data folder and built into an index; three games looked up by the file
/// names their dumps have. Prints the sizes and times.
#[test]
fn live_launchbox() {
    if !enabled() {
        return;
    }
    let (_d, svc) = service();
    let start = std::time::Instant::now();
    svc.download_source("launchbox").expect("LaunchBox download");
    let state = svc.source_download_state("launchbox").unwrap();
    assert!(state.ready);
    eprintln!("LaunchBox: downloaded and built in {:?}, index {} MB", start.elapsed(), state.size_on_disk / 1_000_000);
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        eprintln!("{}", status.lines().filter(|l| l.starts_with("VmHWM")).collect::<String>());
    }
    // The square and wide families kept: the box art's shape is read.
    std::fs::create_dir_all(svc.paths.local_library()).unwrap();
    std::fs::write(svc.paths.collection_settings(), r#"{"metadata":{"keepCoverFamilies":["square","wide"]}}"#).unwrap();
    let ctx = SourceContext::new(&svc.paths, &svc.http);
    let sources = svc.sources();
    let lb = sources.lookups().find(|s| s.descriptor().id == "launchbox").unwrap();
    // The North American SNES box is landscape, the PlayStation's CD case
    // square, the Genesis box tall.
    use heronite_data::cover_family::CoverFamily::{Square, Tall, Wide};
    let cases = [
        ("nintendo_super_nes", "Legend of Zelda, The - A Link to the Past (USA).sfc", "The Legend of Zelda: A Link to the Past", Wide),
        ("sony_playstation", "Castlevania - Symphony of the Night (USA) (Track 1).bin", "Castlevania: Symphony of the Night", Square),
        ("sega_genesis", "Sonic The Hedgehog 2 (World) (Rev A).md", "Sonic the Hedgehog 2", Tall),
    ];
    for (platform, rom, name, family) in cases {
        let game = SourceGame {
            app_name: "a".into(),
            runner: "sideload".into(),
            title: "?".into(),
            emulated: true,
            platform: Some(platform.into()),
            rom_file: Some(rom.into()),
            ..Default::default()
        };
        let _ = lb.lookup(&ctx, &game);
        let t = std::time::Instant::now();
        let c = lb.lookup(&ctx, &game).into_option().expect(rom);
        eprintln!("{rom}: {:?} by {:?} in {:?}", c.match_name, c.matched_by, t.elapsed());
        assert_eq!((c.match_name.as_deref(), c.matched_by), (Some(name), Some(MatchedBy::ExactTitle)));
        let md = &c.metadata;
        assert!(md.cover_url.as_deref().is_some_and(|u| u.starts_with("https://images.launchbox-app.com/")));
        eprintln!("  box {:?}, square {:?}, wide {:?}", md.cover_url, md.cover_square_url, md.cover_wide_url);
        assert_eq!(md.cover_wide_url.is_some(), family == Wide, "{rom}");
        assert_eq!(md.cover_wide_url.as_ref().or(md.cover_square_url.as_ref()) == md.cover_url.as_ref(), family != Tall, "{rom}");
    }
}

/// libretro-thumbnails' real server: three well-known games on three
/// platforms, by the file names their dumps typically have (one with a
/// revision the collection lacks). Prints the requests each cost.
#[test]
fn live_libretro() {
    if !enabled() {
        return;
    }
    use heronite_data::cover_family::CoverFamily::{Square, Tall, Wide};
    let (_d, svc) = service();
    let cases = [
        ("nintendo_super_nes", "Super Metroid (Japan, USA) (En,Ja).sfc", MatchedBy::ExactTitle, Wide),
        ("sega_genesis", "Sonic The Hedgehog 2 (World) (Rev C).md", MatchedBy::FuzzyTitle, Tall),
        ("sony_playstation", "Castlevania - Symphony of the Night (USA).cue", MatchedBy::ExactTitle, Square),
    ];
    for (platform, rom, by, family) in cases {
        let game = SourceGame {
            app_name: "a".into(),
            runner: "sideload".into(),
            title: "?".into(),
            emulated: true,
            platform: Some(platform.into()),
            rom_file: Some(rom.into()),
            ..Default::default()
        };
        let found =
            heronite_metadata::libretro::find(&svc.http, &game, heronite_i18n::Lang::En, &[Tall, Square, Wide]).into_option().expect(rom);
        eprintln!("{rom}: {} by {:?} as {:?} in {} request(s): {}", found.name, found.matched_by, found.family, found.requests, found.url);
        assert_eq!((found.matched_by, found.family), (by, family), "{rom}");
        let picture = svc.http.get(&found.url).send().expect("the picture");
        assert!(picture.ok() && picture.content_type.starts_with("image/png"));
        assert_eq!(
            heronite_metadata::libretro::png_size(&picture.body).map(|(w, h)| heronite_metadata::libretro::family_of(w, h)),
            Some(family)
        );
    }
}

/// LaunchBox's server ignores `Range`: the size of its box art is read from
/// the first bytes of the whole answer (a JPEG's frame header comes after
/// its metadata; the 12 MB PNG says it at once).
#[test]
fn live_launchbox_picture_sizes() {
    if !enabled() {
        return;
    }
    let (_d, svc) = service();
    let lb = heronite_metadata::launchbox::LaunchBox::new();
    let base = &svc.http.endpoints.launchbox_images;
    for (file, size) in [
        ("r2_4f09af04-78b8-43e9-97a9-a8d9c4bb0b85.jpg", (4209, 2937)),
        ("cc0a933e-52b2-498b-bc3b-3e1d520d70f9.jpg", (1280, 2302)),
        ("00fb4aca-c22f-4ca9-84d0-90cf8c22d4af.png", (2460, 1716)),
    ] {
        let started = std::time::Instant::now();
        assert_eq!(lb.picture_size(&svc.http, base, file), Some(size), "{file}");
        assert!(started.elapsed() < std::time::Duration::from_secs(8), "{file}: only its start is read");
    }
}
