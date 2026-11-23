//! The Store media of a game: one source, the first in the order of the
//! "Store media" field that has anything to show, against a local mock of
//! the remote services.

use std::fs;
use std::path::Path;

use crate::common::{Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_data::settings::STORE_MEDIA;
use heronite_metadata::store_media::shots::ShotsGame;
use heronite_metadata::store_media::{MediaGame, SteamTarget};
use heronite_metadata::*;
use serde_json::{json, Value};

fn make_root(dir: &Path, igdb_keys: bool) -> DataPaths {
    let paths = DataPaths::at(dir);
    let settings = if igdb_keys { json!({"metadata": {"igdbClientId": "id", "igdbClientSecret": "secret"}}) } else { json!({}) };
    let p = paths.collection_settings();
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    json_store::write(&p, &settings).unwrap();
    paths
}

fn gog_shot(id: &str) -> Value {
    json!({"image_id": id, "formatted_images": [
        {"formatter_name": "ggvgm_2x", "image_url": format!("https://gog.test/{id}_ggvgm_2x.jpg")},
        {"formatter_name": "ggvgl_2x", "image_url": format!("https://gog.test/{id}_ggvgl_2x.jpg")}
    ]})
}

const SNES: &str = "Nintendo%20-%20Super%20Nintendo%20Entertainment%20System";

fn mock() -> Server {
    Server::start(|req| match req.path() {
        "/steam/api/appdetails" => Resp::json(json!({"620": {"success": true, "data": {
            "movies": [{"id": 1, "name": "Trailer", "thumbnail": "https://steam.test/t.jpg", "hls_h264": "https://steam.test/hls.m3u8"}],
            "screenshots": [{"path_thumbnail": "https://steam.test/s.600.jpg", "path_full": "https://steam.test/s.1920.jpg"}]
        }}})),
        "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
        "/igdb/v4/games" => {
            Resp::json(json!([{"id": 7, "url": "https://www.igdb.com/games/x", "screenshots": [{"image_id": "sc1"}, {"image_id": "sc2"}]}]))
        }
        "/gog/products/1" => Resp::json(json!({"links": {"product_card": "https://www.gog.com/game/x"}, "screenshots": [gog_shot("a")]})),
        // A GOG game without screenshots.
        "/gog/products/2" => Resp::json(json!({"screenshots": []})),
        p if p == format!("/libretro/{SNES}/Named_Snaps/Zelda.png") => Resp::image(vec![0x89, b'P', b'N', b'G']),
        _ => Resp::status(404),
    })
}

fn game(server: &Server) -> MediaGame {
    MediaGame {
        steam: Some(SteamTarget::Id("620".into())),
        others: ShotsGame {
            igdb_id: Some(7),
            gog_id: Some("1".into()),
            libretro_boxart: Some(format!("{}/libretro/{SNES}/Named_Boxarts/Zelda.png", server.base)),
            ..Default::default()
        },
        videos: true,
        screenshots: true,
    }
}

fn source(svc: &MetadataService, dir: &Path, game: &MediaGame) -> Option<String> {
    svc.store_media(dir, game).into_option().map(|(_, m)| m.source)
}

#[test]
fn only_the_first_source_of_the_fields_order_with_media_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock();
    let svc = MetadataService::with_endpoints(make_root(&tmp.path().join("root"), true), server.endpoints());
    let dir = tmp.path().join("store-media");
    let game = game(&server);

    // Steam first by default, with its app id.
    let (id, m) = svc.store_media(&dir, &game).into_option().unwrap();
    assert_eq!((id.as_deref(), m.source.as_str(), m.videos.len(), m.screenshots.len()), (Some("620"), "steam", 1, 1));

    // Switched off for the Store media only: IGDB, ahead of the store by default.
    svc.set_source_field_enabled("steam", STORE_MEDIA, false).unwrap();
    let (id, m) = svc.store_media(&dir, &game).into_option().unwrap();
    assert_eq!((id, m.source.as_str(), m.page.as_str()), (None, "igdb", "https://www.igdb.com/games/x"));
    assert_eq!(m.screenshots[0].full, "https://images.igdb.com/igdb/image/upload/t_1080p/sc1.jpg");

    // The field's own order: the store first, then Steam (on again).
    svc.set_source_field_enabled("steam", STORE_MEDIA, true).unwrap();
    svc.set_field_order(STORE_MEDIA, &["store".into(), "steam".into()]).unwrap();
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("gog"));
    let none_on_gog = MediaGame { others: ShotsGame { gog_id: Some("2".into()), ..game.others.clone() }, ..game.clone() };
    assert_eq!(source(&svc, &dir, &none_on_gog).as_deref(), Some("steam"), "a source with none gives way to the next");

    // Videos only: the others have none, Steam's are taken wherever it is.
    let videos_only = MediaGame { screenshots: false, ..game.clone() };
    assert_eq!(source(&svc, &dir, &videos_only).as_deref(), Some("steam"));
    svc.reset_field_order(STORE_MEDIA).unwrap();

    // A source turned off is not asked; the last one with screenshots answers.
    for id in ["steam", "igdb", "store"] {
        svc.set_source_enabled(id, false).unwrap();
    }
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("libretro"));
    svc.set_source_field_enabled("libretro", STORE_MEDIA, false).unwrap();
    assert_eq!(source(&svc, &dir, &game), None);
}

#[test]
fn igdb_without_keys_is_left_out_and_a_list_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock();
    let svc = MetadataService::with_endpoints(make_root(&tmp.path().join("root"), false), server.endpoints());
    let dir = tmp.path().join("store-media");
    let game = MediaGame { steam: None, ..game(&server) };
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("gog"));
    assert_eq!(server.count(|r| r.path().starts_with("/igdb") || r.path().starts_with("/twitch")), 0);

    // Asked again: the kept list answers.
    let asked = server.count(|r| r.path().starts_with("/gog"));
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("gog"));
    assert_eq!(server.count(|r| r.path().starts_with("/gog")), asked);

    // Amazon's own screenshots need no request; libretro's snap and title screen, the ones that exist.
    let amazon = MediaGame { others: ShotsGame { store_shots: vec!["https://m.test/1.jpg".into()], ..Default::default() }, ..game.clone() };
    assert_eq!(source(&svc, &dir, &amazon).as_deref(), Some("amazon"));
    let retro =
        MediaGame { others: ShotsGame { libretro_boxart: game.others.libretro_boxart.clone(), ..Default::default() }, ..game.clone() };
    let (_, m) = svc.store_media(&dir, &retro).into_option().unwrap();
    assert_eq!(m.screenshots.len(), 1, "no title screen on the server");
    assert!(m.screenshots[0].full.ends_with("/Named_Snaps/Zelda.png"));

    // Nothing matched: nothing asked, nothing found.
    assert!(matches!(svc.store_media(&dir, &MediaGame { screenshots: true, ..Default::default() }), Lookup::NotFound));
}
