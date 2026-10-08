//! Read-only checks against the user's real data: a copy of `~/.config/heroic`
//! (`HERONITE_FIXTURE`, default `/tmp/heronite-fixture`), their ROM folders and
//! emulator config files. Writes only go to temp directories. Run with
//! `cargo test -p heronite-emulation --test real_data -- --nocapture`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use heronite_data::models::LocalMetaFile;
use heronite_data::{json_store, HeroicPaths};
use heronite_emulation::detect::{detect_emulators, SystemEnv};
use heronite_emulation::disc_id::{read_playstation_serial, serial_from_game_list_cache};
use heronite_emulation::import::emu_app_name;
use heronite_emulation::pathutil::normalize_key;
use heronite_emulation::{
    list_emulator_save_states, preview_rom_scan, store, HostEnv, RomScanArgs, RomScanImportArgs,
};

fn fixture() -> Option<PathBuf> {
    let p = std::env::var_os("HERONITE_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/heronite-fixture"));
    if p.join("local_library/emulators.json").exists() {
        Some(p)
    } else {
        eprintln!("fixture not found at {}; skipping", p.display());
        None
    }
}

/// Copies only the JSON the tests touch into a temp dir (never writes the fixture).
fn temp_copy(fixture: &Path) -> (tempfile::TempDir, HeroicPaths) {
    let dir = tempfile::tempdir().unwrap();
    for rel in [
        "local_library/emulators.json",
        "local_library/rom-scanners.json",
        "local_library/library.json",
        "local_library/metadata.json",
        "local_library/statuses.json",
        "local_library/settings.json",
        "sideload_apps/library.json",
    ] {
        let from = fixture.join(rel);
        if from.exists() {
            let to = dir.path().join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(from, to).unwrap();
        }
    }
    let paths = HeroicPaths::at(dir.path());
    (dir, paths)
}

#[test]
fn scan_preview_of_the_real_rom_folders() {
    let Some(fx) = fixture() else { return };
    let (_d, paths) = temp_copy(&fx);
    let scanners = store::list_rom_scanners(&paths).unwrap();
    println!("scanners: {}", scanners.len());
    for s in scanners {
        if !Path::new(&s.directory).exists() {
            println!("  skip {} (folder missing)", s.directory);
            continue;
        }
        let preview = preview_rom_scan(
            &paths,
            &RomScanArgs {
                emulator_id: s.emulator_id.clone(),
                profile_id: s.profile_id.clone(),
                directory: s.directory.clone(),
                scan_subfolders: Some(s.scan_subfolders),
                merge_related_files: Some(s.merge_related_files),
            },
        )
        .unwrap();
        println!(
            "  {} / {} / {:?}: {} games, errors {:?}",
            preview.emulator_name,
            preview.profile_name,
            preview.platform_name,
            preview.games.len(),
            preview.errors
        );
        for g in &preview.games {
            println!(
                "    {:<62} roms={} imported={}",
                g.title,
                g.roms.len(),
                g.already_imported
            );
        }
        assert!(preview.errors.is_empty());
        assert!(!preview.games.is_empty());
        // Everything the fixture already imported is flagged as such.
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        let known: HashSet<String> = metas
            .games
            .values()
            .filter(|m| m.emulator_id.as_deref() == Some(&s.emulator_id))
            .flat_map(|m| m.roms.iter().flatten().map(|r| normalize_key(&r.path)))
            .collect();
        for g in &preview.games {
            let all_known = g.roms.iter().all(|r| known.contains(&normalize_key(&r.path)));
            assert_eq!(g.already_imported, all_known, "{}", g.title);
        }

        // App names Heronite generated are reproduced exactly.
        for g in &preview.games {
            let mut identity: Vec<String> = g.roms.iter().map(|r| normalize_key(&r.path)).collect();
            identity.sort();
            let name = emu_app_name(&s.emulator_id, &s.profile_id, &identity.join("|"));
            let found = metas.games.contains_key(&name);
            println!("    app name {name} {}", if found { "matches Heronite" } else { "(not in fixture)" });
            assert!(found, "{} should have app name {name} in the fixture", g.title);
        }

        // A forced re-import on a temp copy updates in place, same names, no new games.
        let before = metas.games.len();
        let mut forced = preview.games.clone();
        for g in &mut forced {
            g.import = true;
        }
        let r = heronite_emulation::import_scanned_roms(
            &paths,
            &RomScanImportArgs {
                emulator_id: s.emulator_id.clone(),
                profile_id: s.profile_id.clone(),
                directory: s.directory.clone(),
                scanner_id: Some(s.id.clone()),
                games: forced,
                ..Default::default()
            },
        )
        .unwrap();
        println!("  re-import: imported {} updated {} errors {:?}", r.imported, r.updated, r.errors);
        assert_eq!(r.imported, 0);
        assert_eq!(r.updated, preview.games.len());
        let after: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(after.games.len(), before);
        // User data survives (selectedRomPath of the fixture entries).
        let selected = |m: &LocalMetaFile| m.games.values().filter(|g| g.selected_rom_path.is_some()).count();
        assert_eq!(selected(&metas), selected(&after));
        // Sideload art survives the update.
        let side = json_store::read_value(&paths.sideload_library()).unwrap();
        let with_art = side["games"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["app_name"].as_str().is_some_and(|a| a.starts_with("emu_")))
            .filter(|g| g["art_cover"].as_str().is_some_and(|a| !a.is_empty()))
            .count();
        println!("  sideload emu games with art after re-import: {with_art}");
        assert_eq!(with_art, preview.games.len());
        // Idempotent preview afterwards.
        let again = preview_rom_scan(
            &paths,
            &RomScanArgs { emulator_id: s.emulator_id, profile_id: s.profile_id, directory: s.directory, ..Default::default() },
        )
        .unwrap();
        assert!(again.games.iter().all(|g| g.already_imported));
    }
}

#[test]
fn disc_ids_and_save_states_of_the_real_library() {
    let Some(fx) = fixture() else { return };
    let paths = HeroicPaths::at(&fx); // read-only use
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
    let env = HostEnv::system();
    let cache = env.home.join(".config/PCSX2/cache/gamelist.cache");
    let mut checked = 0;
    let mut mismatches = 0;
    for meta in metas.games.values().filter(|m| m.app_name.starts_with("emu_")) {
        let Some(rom) = meta.roms.as_ref().and_then(|r| r.first()).map(|r| r.path.clone()) else { continue };
        if !Path::new(&rom).exists() {
            continue;
        }
        let from_disc = read_playstation_serial(Path::new(&rom));
        let from_cache = serial_from_game_list_cache(&cache, &rom);
        println!("{:<55} disc={:?} cache={:?}", meta.title, from_disc, from_cache);
        if let (Some(a), Some(b)) = (&from_disc, &from_cache) {
            if a != b {
                mismatches += 1;
            }
        }
        assert!(from_disc.is_some(), "{} has no readable serial", meta.title);
        checked += 1;

        let list = list_emulator_save_states(&paths, &env, &meta.app_name, None, true).unwrap();
        println!(
            "   supported={} states={} {:?}",
            list.supported,
            list.states.len(),
            list.states
                .iter()
                .map(|s| (format!("{:?}", s.kind), s.slot, s.thumbnail.as_ref().map(|t| (t.mime, t.bytes.len()))))
                .collect::<Vec<_>>()
        );
        assert!(list.supported);
        for s in &list.states {
            if let Some(t) = &s.thumbnail {
                assert_eq!(t.mime, "image/png");
                assert!(t.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
            }
        }
    }
    println!("checked {checked} PS2 images, {mismatches} disc/cache mismatches");
    assert_eq!(mismatches, 0);
}

#[test]
fn real_machine_detection_and_ludusavi() {
    let Some(fx) = fixture() else { return };
    let (_d, paths) = temp_copy(&fx);
    let existing = store::list_user_emulators(&paths).unwrap();
    let detected = detect_emulators(&SystemEnv, &existing);
    println!("detected emulators: {}", detected.len());
    for d in &detected {
        println!("  {} -> {} ({}) profiles={}", d.definition_id, d.executable, d.launch_kind, d.profiles.len());
    }

    use heronite_emulation::ludusavi::{detect_ludusavi, SystemRunner};
    let det = detect_ludusavi(&SystemRunner::default(), &heronite_launch::SystemProbe, None);
    println!("ludusavi: {det:#?}");
    if let Some(det) = det {
        assert!(Path::new(&det.binary).exists());
        assert!(det.version.is_some());
    }
}

#[test]
fn config_backup_of_a_copy_of_the_fixture() {
    let Some(fx) = fixture() else { return };
    let src = tempfile::tempdir().unwrap();
    // Plain recursive copy of the fixture into a temp "heroic folder".
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let t = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &t);
            } else {
                std::fs::copy(e.path(), t).unwrap();
            }
        }
    }
    copy(&fx, src.path());
    let paths = HeroicPaths::at(src.path());
    let out = tempfile::tempdir().unwrap();
    heronite_data::settings::save_backup(&paths, out.path().to_str().unwrap(), "weekly").unwrap();
    // The fixture's last backup is recent, so a weekly run is not due...
    let due = heronite_emulation::backup::run_collection_backup(&paths, false, chrono::Local::now());
    println!("scheduled run: {due:?}");
    assert_eq!(due.skipped_reason.as_deref(), Some("not-due"));
    // ...but a forced one copies the (temp) folder.
    let r = heronite_emulation::backup::run_collection_backup(&paths, true, chrono::Local::now());
    println!("backup: {r:?}");
    assert!(r.ran, "{r:?}");
    let dest = PathBuf::from(r.path.unwrap());
    assert!(dest.join("local_library/library.json").exists());
    assert!(!dest.join("store_cache").exists());
    assert!(!dest.join("local_library/steam-details").exists());
    let size: u64 = walk(&dest);
    println!("backup size: {} KiB", size / 1024);
    fn walk(p: &Path) -> u64 {
        std::fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() { walk(&e.path()) } else { e.metadata().unwrap().len() }
            })
            .sum()
    }
}

#[test]
fn real_save_state_dirs_per_engine() {
    use heronite_emulation::save_states::{Engine, EngineContext};
    let env = HostEnv::system();
    let ctx = EngineContext {
        executable: Some("/data/Games/Emulation/PS2/emulator/pcsx2-v2.8.2.AppImage".into()),
        ..Default::default()
    };
    for (name, engine) in [
        ("pcsx2", Engine::Pcsx2),
        ("duckstation", Engine::DuckStation),
        ("ppsspp", Engine::Ppsspp),
        ("dolphin", Engine::Dolphin),
        ("retroarch", Engine::RetroArch),
        ("rpcs3", Engine::Rpcs3),
    ] {
        for dir in engine.state_dirs(&env, &ctx, "/nonexistent/rom.bin", &[]) {
            let count = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| engine.parse(&e.file_name().to_string_lossy()).is_some())
                        .count()
                })
                .ok();
            println!("{name:<12} {dir}  exists={} state files={:?}", count.is_some(), count);
        }
    }
}

/// Read-only: the real machine's emulator discovery finds the AppImages in the
/// folders of the configured emulator and in a user-chosen folder. Nothing is
/// executed.
#[test]
fn discovery_finds_real_appimages() {
    let Some(fx) = fixture() else { return };
    let paths = HeroicPaths::at(&fx);
    let existing = store::list_user_emulators(&paths).unwrap();
    let Some(cfg) = existing.iter().find(|e| e.executable.starts_with("/data/Games/Emulation/")) else {
        eprintln!("no configured emulator under /data/Games/Emulation; skipping");
        return;
    };
    let root = "/data/Games/Emulation";
    if !Path::new(root).is_dir() {
        return;
    }
    let found = heronite_emulation::discover::discover_in_folder(&heronite_emulation::SystemEnv, root, &existing);
    for d in &found {
        eprintln!("{:<12} {} added={}", d.name, d.executable, d.already_added);
    }
    assert!(found.iter().any(|d| d.executable == cfg.executable && d.already_added));
    assert!(found.iter().any(|d| d.name == "DuckStation" && !d.already_added));
    // No game folder was entered.
    assert!(found.iter().all(|d| !d.executable.contains("/games/")));
}

/// Read-only: the automatic discovery of this machine finishes quickly.
#[test]
fn discovery_on_this_pc_is_quick() {
    let Some(fx) = fixture() else { return };
    let existing = store::list_user_emulators(&HeroicPaths::at(&fx)).unwrap();
    let t = std::time::Instant::now();
    let found = heronite_emulation::discover::discover_emulators(&heronite_emulation::SystemEnv, &existing);
    eprintln!("discovery took {:?}", t.elapsed());
    for d in &found {
        eprintln!("{:<12} {} ({}) added={}", d.name, d.executable, d.source, d.already_added);
    }
    assert!(t.elapsed() < std::time::Duration::from_secs(15));
}
