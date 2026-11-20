//! Issue #318: Steam games on a second drive showed "Install" while Steam ran
//! them. The machine: Steam runs from its Snap (Ubuntu's App Center), a
//! stale `~/.steam/steam` is left from the native package, and the Snap's
//! `libraryfolders.vdf` lists a library on an HDD. Real files on disk, read
//! through the same refresh the app runs to set Play / Install.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::steam_owned::SteamOwnedFile;
use heronite_data::DataPaths;
use heronite_launch::install_state::refresh_local_install_states;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::{platform_roots, snap_steam_root, SteamLocator};
use heronite_launch::{Probe, SystemProbe};

/// The real filesystem, with a home folder of the test's own.
struct HomeProbe(PathBuf);

impl Probe for HomeProbe {
    fn exists(&self, path: &str) -> bool {
        SystemProbe.exists(path)
    }
    fn read_head(&self, path: &str, n: usize) -> Option<Vec<u8>> {
        SystemProbe.read_head(path, n)
    }
    fn which(&self, _name: &str) -> Option<String> {
        None
    }
    fn env_var(&self, _key: &str) -> Option<String> {
        None
    }
    fn home(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn steam_roots(&self) -> Vec<PathBuf> {
        platform_roots(&self.0)
    }
    fn steam_system_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn library_folders(paths: &[&Path]) -> String {
    let mut out = String::from("\"libraryfolders\"\n{\n");
    for (i, p) in paths.iter().enumerate() {
        out +=
            &format!("\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"label\"\t\t\"\"\n\t\t\"apps\"\n\t\t{{\n\t\t}}\n\t}}\n", p.display());
    }
    out + "}\n"
}

fn manifest(steamapps: &Path, id: &str, name: &str, dir: &str) {
    write(
        &steamapps.join(format!("appmanifest_{id}.acf")),
        &format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"{dir}\"\n\t\"SizeOnDisk\"\t\t\"1024\"\n}}\n"),
    );
    fs::create_dir_all(steamapps.join("common").join(dir)).unwrap();
}

#[test]
fn games_on_a_second_drive_listed_by_the_running_snap_steam_are_installed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home/ragnar");
    let stale = home.join(".steam/steam");
    let snap = snap_steam_root(&home);
    let hdd = dir.path().join("media/ragnar/HDD/SteamLibrary");

    // The old native Steam: its folder exists, its list knows only itself.
    fs::create_dir_all(stale.join("config")).unwrap();
    write(&stale.join("steamapps/libraryfolders.vdf"), &library_folders(&[&stale]));
    // The Steam that runs: its own library and the HDD.
    fs::create_dir_all(snap.join("config")).unwrap();
    write(&snap.join("steamapps/libraryfolders.vdf"), &library_folders(&[&snap, &hdd]));
    manifest(&snap.join("steamapps"), "400", "Portal", "Portal");
    write(&hdd.join("libraryfolder.vdf"), "\"libraryfolder\"\n{\n\t\"contentid\"\t\t\"1\"\n\t\"label\"\t\t\"\"\n}\n");
    manifest(&hdd.join("steamapps"), "1245620", "ELDEN RING", "ELDEN RING");

    // The account's games, read from Steam before anything was installed.
    let paths = DataPaths::at(dir.path().join("heronite"));
    let game = |id: &str, name: &str| serde_json::json!({ "appId": id, "name": name, "installed": false });
    write(
        &paths.steam_owned(),
        &serde_json::json!({
            "games": [game("400", "Portal"), game("1245620", "ELDEN RING"), game("620", "Portal 2")],
            "updatedAt": "2026-11-12T00:00:00Z"
        })
        .to_string(),
    );

    let probe = HomeProbe(home.clone());
    let locator = SteamLocator::detect(&GlobalSettings::default(), &probe);
    assert_eq!(locator.steam_path, stale, "the stale folder is the one picked, as on the reporter's machine");

    refresh_local_install_states(&paths, &probe).unwrap();
    let owned = SteamOwnedFile::read(&paths);
    let installed = |id: &str| owned.get(id).unwrap().installed;
    assert!(installed("400"), "game in the running Steam's own library");
    assert!(installed("1245620"), "game on the second drive");
    assert!(!installed("620"), "game installed nowhere");

    assert_eq!(locator.install_dir("1245620"), Some(hdd.join("steamapps/common/ELDEN RING")));
    assert_eq!(locator.size_on_disk("1245620"), Some(1024));
}
