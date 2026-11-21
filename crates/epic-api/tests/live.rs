//! Checks against Epic's real services with the sign-in of this PC's data
//! folder (`HERONITE_DATA_DIR`, else `~/.local/share/heronite`). Ignored by
//! default: `cargo test -p heronite-epic-api --test live -- --ignored
//! --test-threads 1`.
//!
//! Nothing here changes the installs: the library is refreshed into a copy
//! of the config folder, manifests and chunks are only read. A token that
//! has to be renewed is written back to the real `user.json` at once (Epic
//! may refuse the old refresh token afterwards), as the app does.

use std::path::{Path, PathBuf};

use heronite_epic_api::{chunk, info, installed, launch, legendary_config, library, Client, Manifest};

fn config_dir() -> PathBuf {
    let data = std::env::var_os("HERONITE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".local/share/heronite"));
    data.join("legendaryConfig/legendary")
}

fn client() -> Client {
    legendary_config::client_for(&config_dir())
}

fn session() -> heronite_epic_api::Session {
    client().ensure_session(&config_dir()).expect("sign-in").expect("signed in")
}

/// A copy of the config folder's library files (no sign-in, no tokens).
fn copy_config(to: &Path) {
    let from = config_dir();
    std::fs::create_dir_all(to.join("metadata")).unwrap();
    for f in ["assets.json", "installed.json", "version.json"] {
        let _ = std::fs::copy(from.join(f), to.join(f));
    }
    for e in std::fs::read_dir(from.join("metadata")).unwrap().flatten() {
        std::fs::copy(e.path(), to.join("metadata").join(e.file_name())).unwrap();
    }
}

#[test]
#[ignore]
fn stored_manifests_parse() {
    let dir = installed::manifests_dir(&config_dir());
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let m = Manifest::parse(&std::fs::read(e.path()).unwrap()).unwrap();
        let first = m.chunks.first().map(|c| m.chunk_path(c)).unwrap_or_default();
        println!(
            "{} {} {} {} {} {} {} {} {} {}",
            e.file_name().to_string_lossy(),
            m.meta.app_name,
            m.meta.build_version,
            m.meta.build_id(),
            m.files.len(),
            m.chunks.len(),
            m.disk_size(),
            m.download_size(),
            m.meta.launch_exe,
            first
        );
    }
}

#[test]
#[ignore]
fn refresh_the_library_into_a_copy() {
    let s = session();
    let tmp = tempfile::tempdir().unwrap();
    copy_config(tmp.path());
    let before = library::stored_apps(tmp.path()).len();
    let report = library::refresh(&client(), tmp.path(), &s, &library::RefreshOptions::default()).unwrap();
    println!("assets {} fetched {} pruned {} failures {:?}", report.assets, report.fetched, report.pruned, report.failures);
    println!("metadata files: {before} before, {} after", library::stored_apps(tmp.path()).len());
    // Every metadata file the app reads has its shape.
    for app in library::stored_apps(tmp.path()) {
        let g = library::GameMeta::load(tmp.path(), &app).unwrap();
        assert!(!g.app_title.is_empty(), "{app}");
    }
    // A forced refresh of one game against what legendary wrote.
    let real = library::GameMeta::load(&config_dir(), "Calluna").unwrap();
    let ours = library::GameMeta::load(tmp.path(), "Calluna").unwrap();
    assert_eq!(ours.app_version("Windows"), real.app_version("Windows"));
    assert_eq!(ours.namespace(), real.namespace());
}

#[test]
#[ignore]
fn the_manifest_of_an_installed_game_matches_legendarys_copy() {
    let s = session();
    let cfg = config_dir();
    let ig = installed::get(&cfg, "Calluna").expect("Control installed");
    let game = library::GameMeta::load(&cfg, "Calluna").unwrap();
    let c = client();
    let fetched = info::fetch_manifest(&c, s.access_token(), &game, "Windows").unwrap();
    let stored = installed::installed_manifest(&cfg, &ig).unwrap();
    println!("latest {} installed {}", fetched.manifest.meta.build_version, stored.meta.build_version);
    if fetched.manifest.meta.build_version == stored.meta.build_version {
        assert_eq!(fetched.manifest.files.len(), stored.files.len());
        assert_eq!(fetched.manifest.disk_size(), stored.disk_size());
        assert_eq!(fetched.manifest.download_size(), stored.download_size());
        assert_eq!(fetched.manifest.files, stored.files);
    }
    // A few chunks download, decode and match the manifest.
    let m = &fetched.manifest;
    for info in m.chunks.iter().take(3) {
        let data = c.fetch_chunk(&fetched.info.base_urls, &m.chunk_path(info), info, &fetched.info.secrets).unwrap();
        assert_eq!(data.len(), info.window_size as usize);
        assert_eq!(chunk::rolling_hash(&data), info.hash, "rolling hash of {}", info.guid.text());
    }
    println!("base urls: {:?}", fetched.info.base_urls.iter().map(|u| heronite_epic_api::http::host(u)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn install_info_has_the_cached_keys() {
    let s = session();
    let cfg = config_dir();
    let doc = info::fetch(&client(), &cfg, Some(&s), "Calluna", "Windows", false).unwrap();
    let cached: serde_json::Value = serde_json::from_slice(
        &std::fs::read(cfg.parent().unwrap().parent().unwrap().join("store_cache/legendary_install_info.json")).unwrap(),
    )
    .unwrap();
    let old = &cached["Calluna"];
    for part in ["game", "install", "manifest"] {
        let keys = |v: &serde_json::Value| {
            v[part].as_object().map(|o| o.keys().cloned().collect::<std::collections::BTreeSet<_>>()).unwrap_or_default()
        };
        let missing: Vec<_> = keys(old).difference(&keys(&doc)).cloned().collect();
        assert!(missing.is_empty(), "{part}: missing {missing:?}");
    }
    assert_eq!(doc["manifest"]["download_size"], old["manifest"]["download_size"]);
    assert_eq!(doc["game"]["grant_date"], old["game"]["grant_date"]);
    println!("{}", serde_json::to_string(&doc["game"]).unwrap());
}

#[test]
#[ignore]
fn launch_parameters_with_a_sign_in_code() {
    let s = session();
    let p = launch::launch_params(&client(), &config_dir(), Some(&s), "Calluna", &launch::LaunchOptions::default()).unwrap();
    let code = p.egl_parameters.iter().find_map(|a| a.strip_prefix("-AUTH_PASSWORD=")).unwrap();
    assert!(code.len() > 20, "a sign-in code");
    println!("exe {} in {} with {} game arguments", p.game_executable, p.working_directory, p.game_parameters.len());
    // The update check rewrites `assets.json`: on a copy.
    let tmp = tempfile::tempdir().unwrap();
    copy_config(tmp.path());
    println!("update available: {}", launch::update_available(&client(), tmp.path(), &s, "Calluna").unwrap());
}

#[test]
#[ignore]
fn cloud_saves_are_listed() {
    let s = session();
    let saves = client().cloud_saves(&s, "Calluna").unwrap();
    println!("{} save(s) of Control: {:?}", saves.len(), saves.iter().map(|s| &s.manifest_name).collect::<Vec<_>>());
    let all = client().cloud_saves(&s, "").unwrap();
    println!("{} save(s) in the account", all.len());
}

#[test]
#[ignore]
fn the_eos_overlay_build_is_public() {
    let c = client();
    let tmp = tempfile::tempdir().unwrap();
    let version = heronite_epic_api::eos::latest_version(&c, tmp.path()).unwrap();
    let plan = heronite_epic_api::eos::prepare_install(&c, tmp.path(), &tmp.path().join("overlay")).unwrap();
    println!("overlay {version}: {} files, {} bytes to download", plan.plan.files.len(), plan.plan.download_size);
    assert_eq!(plan.record.version, version);
}

#[test]
#[ignore]
fn a_forced_refresh_writes_what_legendary_wrote() {
    let s = session();
    let tmp = tempfile::tempdir().unwrap();
    copy_config(tmp.path());
    let opts = library::RefreshOptions { force: true, ..Default::default() };
    let report = library::refresh(&client(), tmp.path(), &s, &opts).unwrap();
    println!("fetched {} failures {:?}", report.fetched, report.failures);
    let mut differ = 0;
    for app in library::stored_apps(&config_dir()) {
        let (Some(real), Some(ours)) = (library::GameMeta::load(&config_dir(), &app), library::GameMeta::load(tmp.path(), &app)) else {
            println!("{app}: missing in the copy");
            continue;
        };
        let keys = |v: &serde_json::Value| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
        if real.app_title != ours.app_title || real.asset_infos != ours.asset_infos || keys(&real.metadata) != keys(&ours.metadata) {
            differ += 1;
            println!(
                "{app}: title {:?}/{:?} assets same {} keys {:?} vs {:?}",
                real.app_title,
                ours.app_title,
                real.asset_infos == ours.asset_infos,
                keys(&real.metadata),
                keys(&ours.metadata)
            );
        }
    }
    println!("{differ} game(s) differ");
}

#[test]
#[ignore]
fn the_newest_cloud_save_downloads() {
    let s = session();
    let c = client();
    let saves = c.cloud_saves(&s, "Calluna").unwrap();
    let Some(save) = saves.last() else { return };
    let game = library::GameMeta::load(&config_dir(), "Calluna").unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let n = c.download_save(&s, &game, save, tmp.path()).unwrap();
    println!("{} file(s) from {}", n, save.manifest_name);
    assert!(n > 0);
}
