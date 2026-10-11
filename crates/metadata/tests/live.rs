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

    let l = heronite_metadata::lutris::fetch_lutris_metadata(&svc.http, "a", "sideload", "Portal", None).expect("lutris");
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
    let c = svc
        .gather_candidates(&MetadataApplyArgs {
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
