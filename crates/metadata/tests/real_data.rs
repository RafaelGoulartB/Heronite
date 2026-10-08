//! Writes against a tempdir copy of real Heroic data. Run with
//! `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test -p heronite-metadata`
//! (skipped when unset). The real `~/.config/heroic` is never touched.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::json_store;
use heronite_data::library::Library;
use heronite_data::models::{ArtFile, MetadataFile};
use heronite_data::paths::HeroicPaths;
use heronite_metadata::*;
use serde_json::Value;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else if ty.is_file() {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// A private copy of the fixture, or `None` when `HERONITE_FIXTURE` is unset.
fn copy_fixture() -> Option<(tempfile::TempDir, HeroicPaths)> {
    let root = PathBuf::from(std::env::var_os("HERONITE_FIXTURE")?);
    let dir = tempfile::tempdir().unwrap();
    // Everything but the (large) image folders; art files are recreated by the tests.
    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if entry.file_type().unwrap().is_dir() && name == "local_library" {
            let dest = dir.path().join(&name);
            fs::create_dir_all(&dest).unwrap();
            for inner in fs::read_dir(entry.path()).unwrap() {
                let inner = inner.unwrap();
                if inner.file_type().unwrap().is_file() {
                    fs::copy(inner.path(), dest.join(inner.file_name())).unwrap();
                }
            }
        } else if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dir.path().join(&name));
        }
    }
    let paths = HeroicPaths::at(dir.path());
    Some((dir, paths))
}

fn offline_service(paths: &HeroicPaths) -> MetadataService {
    // Nothing listens on port 9: every request fails fast.
    let mut e = Endpoints::default();
    for url in [&mut e.twitch_token, &mut e.igdb, &mut e.steam_store, &mut e.steam_community, &mut e.lutris] {
        *url = "http://127.0.0.1:9".into();
    }
    MetadataService::with_endpoints(paths.clone(), e)
}

#[test]
fn details_edit_touches_only_one_record() {
    let Some((_dir, paths)) = copy_fixture() else { return };
    let before: Value = json_store::read_value(&paths.metadata()).unwrap();
    let lib = Library::load(&paths).unwrap();
    let game = lib.games.iter().find(|g| g.metadata.is_some()).expect("a game with metadata");
    let svc = offline_service(&paths);

    let r = svc
        .update_game_details(&DetailsPatch {
            app_name: game.app_name.clone(),
            runner: game.overlay_runner().into(),
            description: Some("Edited by the Rust backend".into()),
            notes: Some("note".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.metadata.field_sources["description"], "manual");

    let after: Value = json_store::read_value(&paths.metadata()).unwrap();
    let key = game.metadata_key();
    let (b, a) = (before["games"].as_object().unwrap(), after["games"].as_object().unwrap());
    assert_eq!(b.len(), a.len());
    assert_eq!(b.keys().collect::<Vec<_>>(), a.keys().collect::<Vec<_>>(), "record order unchanged");
    for (k, v) in b {
        if *k != key {
            assert_eq!(&a[k], v, "record {k} changed");
        }
    }
    // Same record, only the edited fields differ.
    let (old, new) = (&b[&key], &a[&key]);
    for (field, value) in old.as_object().unwrap() {
        if !["description", "notes", "fieldSources"].contains(&field.as_str()) {
            assert_eq!(&new[field], value, "{field} changed");
        }
    }
    assert_eq!(new["description"], "Edited by the Rust backend");

    // Heronite's reader still loads everything and sees the edit.
    let lib = Library::load(&paths).unwrap();
    let again = lib.get(&game.key).unwrap();
    assert_eq!(again.metadata.as_ref().unwrap().description.as_deref(), Some("Edited by the Rust backend"));
    let typed: MetadataFile = json_store::read(&paths.metadata()).unwrap();
    assert_eq!(typed.games.len(), b.len());
}

#[test]
fn art_set_and_reset_on_a_real_library() {
    let Some((dir, paths)) = copy_fixture() else { return };
    let before: Value = json_store::read_value(&paths.art()).unwrap();
    let lib = Library::load(&paths).unwrap();
    let game = lib.games.iter().find(|g| g.art.is_none()).expect("a game without custom art");
    let svc = offline_service(&paths);

    let img = dir.path().join("cover.png");
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend([1u8; 200]);
    fs::write(&img, &bytes).unwrap();
    let art = svc.set_art_from_file(&game.runner, &game.app_name, ArtKind::Cover, img.to_str().unwrap()).unwrap();
    let url = art.cover_url.unwrap();
    assert!(url.starts_with(&format!("localart://art/{}_{}/cover-", game.runner, game.app_name)) || url.contains("/cover-"));

    let typed: ArtFile = json_store::read(&paths.art()).unwrap();
    assert_eq!(typed.games.len(), before["games"].as_object().unwrap().len() + 1);
    let lib = Library::load(&paths).unwrap();
    assert_eq!(lib.get(&game.key).unwrap().art.as_ref().unwrap().cover_url.as_deref(), Some(url.as_str()));

    svc.reset_art(&game.runner, &game.app_name, ArtKind::Cover).unwrap();
    let after: Value = json_store::read_value(&paths.art()).unwrap();
    assert_eq!(after, before, "art.json is back to the original");
}

#[test]
fn real_settings_drive_the_merge_priority() {
    let Some((_dir, paths)) = copy_fixture() else { return };
    let svc = offline_service(&paths);
    let m = svc.metadata_settings();
    assert_eq!(m.field_priority["description"].first().map(String::as_str), Some("igdb"));
    // Offline: the Store candidate still works from the real libraries, other sources fail quietly.
    let lib = Library::load(&paths).unwrap();
    let game = lib.games.iter().find(|g| g.runner == "legendary").expect("an Epic game");
    let preview = svc
        .preview_game_metadata(&MetadataApplyArgs {
            app_name: game.app_name.clone(),
            runner: game.runner.clone(),
            title: game.title.clone(),
            ..Default::default()
        })
        .unwrap();
    assert!(preview.candidates.iter().any(|c| c.source == "store"), "store candidate from the real Epic library");
    assert!(preview.candidates.iter().all(|c| c.source == "store"), "network sources are down");
}
