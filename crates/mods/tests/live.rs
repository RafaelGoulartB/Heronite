//! The whole chain against the real Thunderstore and a real fuse-overlayfs:
//! resolve a package with its dependencies, download, install with the
//! BepInEx layout, mount over a fake game folder, unmount. Needs the network
//! and FUSE, so it only runs when asked:
//! `cargo test -p heronite-mods --test live -- --ignored --nocapture`.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use heronite_mods::apply::{apply, ApplyRequest};
use heronite_mods::games::{self, Profile};
use heronite_mods::install::{commit, stage, InstallCtx, Plan, Source};
use heronite_mods::{download, store, thunderstore, ModEntry, ModSource, ModsRoot};

#[test]
#[ignore]
fn thunderstore_package_with_dependencies_mounts_over_the_game() {
    let tmp = tempfile::tempdir().unwrap();
    let game_dir = tmp.path().join("Lethal Company");
    std::fs::create_dir_all(game_dir.join("Lethal Company_Data")).unwrap();
    std::fs::write(game_dir.join("Lethal Company.exe"), b"exe").unwrap();
    let root = ModsRoot::at(tmp.path().join("mods"));
    let key = "sideload_lc";
    let dirs = root.game(key);
    let mut list = store::load_or_new(&root, key, "Lethal Company");
    let known = games::find(Some("1966720"), "Lethal Company");
    list.links.thunderstore = Some("lethal-company".into());
    let profile = Profile::of(&list.links, known);
    let cancel = AtomicBool::new(false);

    let planned = thunderstore::resolve(&list, "notnotnotswipez", "MoreCompany", None, &mut |ns, name| thunderstore::package(ns, name))
        .expect("resolve");
    assert!(planned.len() >= 2, "MoreCompany needs BepInExPack: {planned:?}");
    assert!(thunderstore::is_bepinex_pack(&planned[0].namespace, &planned[0].name), "dependencies first");

    for p in &planned {
        let archive = download::download(&p.version.download_url, &dirs.downloads(), None, &[], &mut |_| {}, &cancel).expect("download");
        let staged = stage(&dirs, &Source::Archive(archive), &cancel).expect("stage");
        let ctx = InstallCtx { dirs: &dirs, game_dir: Some(&game_dir), profile: &profile };
        let entry = ModEntry {
            id: format!("ts-{}-{}", p.namespace, p.name),
            name: p.name.clone(),
            version: Some(p.version.version_number.clone()),
            enabled: true,
            source: ModSource::Thunderstore { community: "lethal-company".into(), namespace: p.namespace.clone(), name: p.name.clone() },
            ..Default::default()
        };
        commit(&ctx, &mut list, &staged, Plan::Auto, entry, None, &cancel).expect("commit");
    }
    let pack = list.mods.iter().find(|m| m.name.starts_with("BepInExPack")).unwrap();
    assert_eq!(pack.dll_overrides, ["winhttp"]);
    assert!(dirs.mod_dir(&pack.id).join("BepInEx/core").is_dir(), "the pack goes to the game root");
    let more = list.mods.iter().find(|m| m.name == "MoreCompany").unwrap();
    // The package ships its own `BepInEx/plugins/`, which merges at the root.
    assert!(dirs.mod_dir(&more.id).join("BepInEx/plugins/MoreCompany.dll").is_file());

    if heronite_mods::tool::find(Path::new("/nonexistent")).is_none() || !Path::new("/dev/fuse").exists() {
        eprintln!("no fuse-overlayfs (set HERONITE_FUSE_OVERLAYFS): mount not checked");
        return;
    }
    let req = ApplyRequest { root: &root, tools_bin: Path::new("/nonexistent"), game_key: key, game_dir: &game_dir, prefix: None, known };
    let applied = apply(&req, &mut |_| {}, &cancel).expect("apply").expect("mods to apply");
    assert_eq!(applied.env, [("WINEDLLOVERRIDES".to_string(), "winhttp=n,b".to_string())]);
    assert!(game_dir.join("winhttp.dll").is_file(), "the loader shows in the game folder");
    assert!(game_dir.join("BepInEx/plugins/MoreCompany.dll").is_file());
    println!("{}", applied.summary);
    drop(applied);
    assert!(!game_dir.join("winhttp.dll").exists(), "the game folder is untouched after");
    assert!(game_dir.join("Lethal Company.exe").is_file());
}

#[test]
#[ignore]
fn fuse_overlayfs_is_downloaded_and_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    let cancel = AtomicBool::new(false);
    let mut last = heronite_mods::Progress::default();
    let path = heronite_mods::tool::ensure(&bin, &mut |p| last = p, &cancel).expect("download");
    // `ensure` prefers a copy on $PATH; only a fresh download lands in `bin`.
    if path.starts_with(&bin) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(last.done > 0);
    }
    let out = std::process::Command::new(&path).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("fuse-overlayfs"));
}
