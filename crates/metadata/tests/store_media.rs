//! The Store media's screenshots of a game Steam has nothing for: one
//! source, the first in the user's order that has any, against a local mock
//! of the remote services.

mod common;

use std::fs;
use std::path::Path;

use common::{Resp, Server};
use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::store_media::shots::ShotsGame;
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

fn mock() -> Server {
    Server::start(|req| match req.path() {
        "/twitch/oauth2/token" => Resp::json(json!({"access_token": "tok", "expires_in": 3600, "token_type": "bearer"})),
        "/igdb/v4/games" => {
            Resp::json(json!([{"id": 7, "url": "https://www.igdb.com/games/x", "screenshots": [{"image_id": "sc1"}, {"image_id": "sc2"}]}]))
        }
        "/gog/products/1" => Resp::json(json!({"links": {"product_card": "https://www.gog.com/game/x"}, "screenshots": [gog_shot("a")]})),
        // A GOG game without screenshots.
        "/gog/products/2" => Resp::json(json!({"screenshots": []})),
        "/libretro/Nintendo%20-%20Super%20Nintendo%20Entertainment%20System/Named_Snaps/Zelda.png" => {
            Resp::image(vec![0x89, b'P', b'N', b'G'])
        }
        _ => Resp::status(404),
    })
}

fn game(server: &Server) -> ShotsGame {
    ShotsGame {
        igdb_id: Some(7),
        gog_id: Some("1".into()),
        libretro_boxart: Some(format!(
            "{}/libretro/Nintendo%20-%20Super%20Nintendo%20Entertainment%20System/Named_Boxarts/Zelda.png",
            server.base
        )),
        ..Default::default()
    }
}

fn source(svc: &MetadataService, dir: &Path, game: &ShotsGame) -> Option<String> {
    svc.other_screenshots(dir, game).into_option().map(|m| m.source)
}

#[test]
fn only_the_first_source_in_the_users_order_with_screenshots_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock();
    let paths = make_root(&tmp.path().join("root"), true);
    let svc = MetadataService::with_endpoints(paths, server.endpoints());
    let game = game(&server);

    // IGDB is ahead of the store by default.
    let m = svc.other_screenshots(&tmp.path().join("a"), &game).into_option().unwrap();
    assert_eq!((m.source.as_str(), m.page.as_str()), ("igdb", "https://www.igdb.com/games/x"));
    assert_eq!(m.screenshots.len(), 2);
    assert_eq!(m.screenshots[0].full, "https://images.igdb.com/igdb/image/upload/t_1080p/sc1.jpg");
    assert!(m.videos.is_empty());

    // The store moved first: its GOG product's.
    svc.move_source("store", 0).unwrap();
    let m = svc.other_screenshots(&tmp.path().join("b"), &game).into_option().unwrap();
    assert_eq!((m.source.as_str(), m.screenshots[0].full.as_str()), ("gog", "https://gog.test/a_ggvgl_2x.jpg"));

    // A source with none, or off, gives way to the next one.
    let none_on_gog = ShotsGame { gog_id: Some("2".into()), ..game.clone() };
    assert_eq!(source(&svc, &tmp.path().join("c"), &none_on_gog).as_deref(), Some("igdb"));
    svc.set_source_enabled("igdb", false).unwrap();
    assert_eq!(source(&svc, &tmp.path().join("d"), &none_on_gog).as_deref(), Some("libretro"));
    svc.set_source_enabled("libretro", false).unwrap();
    assert_eq!(source(&svc, &tmp.path().join("e"), &none_on_gog), None);
}

#[test]
fn igdb_without_keys_is_left_out_and_a_list_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock();
    let svc = MetadataService::with_endpoints(make_root(&tmp.path().join("root"), false), server.endpoints());
    let dir = tmp.path().join("store-media");
    let game = game(&server);
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("gog"));
    assert_eq!(server.count(|r| r.path().starts_with("/igdb") || r.path().starts_with("/twitch")), 0);

    // Asked again: the kept list answers.
    let asked = server.count(|r| r.path().starts_with("/gog"));
    assert_eq!(source(&svc, &dir, &game).as_deref(), Some("gog"));
    assert_eq!(server.count(|r| r.path().starts_with("/gog")), asked);

    // Amazon's own screenshots need no request; libretro's snap and title screen, the ones that exist.
    let amazon = ShotsGame { store_shots: vec!["https://m.test/1.jpg".into()], ..Default::default() };
    assert_eq!(source(&svc, &dir, &amazon).as_deref(), Some("amazon"));
    let retro = ShotsGame { libretro_boxart: game.libretro_boxart.clone(), ..Default::default() };
    let m = svc.other_screenshots(&dir, &retro).into_option().unwrap();
    assert_eq!(m.screenshots.len(), 1, "no title screen on the server");
    assert!(m.screenshots[0].full.ends_with("/Named_Snaps/Zelda.png"));

    // Nothing matched: nothing asked, nothing found.
    assert!(matches!(svc.other_screenshots(&dir, &ShotsGame::default()), Lookup::NotFound));
}
