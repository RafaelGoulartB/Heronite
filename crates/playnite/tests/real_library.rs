//! End-to-end against the user's real Playnite library, read-only, importing
//! into a temp copy of the Heronite fixture. Skips when either is missing.
//!
//! `HERONITE_FIXTURE` (default `/tmp/heronite-fixture`) is a copy of
//! `~/.config/heroic`; the library path comes from its `statuses.json`
//! (`lastPlayniteLibraryPath`).

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::models::*;
use heronite_data::{json_store, HeroicPaths};
use heronite_playnite::*;
use serde::{de::DeserializeOwned, Serialize};

fn fixture() -> Option<PathBuf> {
    let root = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from).unwrap_or_else(|| "/tmp/heronite-fixture".into());
    root.join("local_library/statuses.json").exists().then_some(root)
}

fn library_path(root: &Path) -> Option<String> {
    let v = json_store::read_value(&root.join("local_library/statuses.json")).ok()?;
    let p = v.get("lastPlayniteLibraryPath")?.as_str()?.to_owned();
    Path::new(&p).join("games.db").exists().then_some(p)
}

fn copy_json_tree(from: &Path, to: &Path, rels: &[&str]) {
    for rel in rels {
        let src = from.join(rel);
        if src.is_dir() {
            for e in fs::read_dir(&src).unwrap().flatten() {
                if e.path().is_file() && e.path().extension().is_some_and(|x| x == "json") {
                    let dst = to.join(rel).join(e.file_name());
                    fs::create_dir_all(dst.parent().unwrap()).unwrap();
                    fs::copy(e.path(), dst).unwrap();
                }
            }
        } else if src.is_file() {
            let dst = to.join(rel);
            fs::create_dir_all(dst.parent().unwrap()).unwrap();
            fs::copy(src, dst).unwrap();
        }
    }
}

fn temp_copy(root: &Path) -> (tempfile::TempDir, HeroicPaths) {
    let dir = tempfile::tempdir().unwrap();
    copy_json_tree(root, dir.path(), &["local_library", "sideload_apps", "store", "store_cache", "GamesConfig"]);
    let paths = HeroicPaths::at(dir.path());
    (dir, paths)
}

fn round_trips<T: DeserializeOwned + Serialize + Default>(path: &Path) {
    if !path.exists() {
        return;
    }
    let original = json_store::read_value(path).unwrap();
    let typed: T = json_store::read(path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
    assert!(original == serde_json::to_value(&typed).unwrap(), "{} did not round-trip", path.display());
}

fn assert_all_round_trip(paths: &HeroicPaths) {
    round_trips::<LocalMetaFile>(&paths.local_meta());
    round_trips::<SessionsFile>(&paths.sessions());
    round_trips::<StatusesFile>(&paths.statuses());
    round_trips::<MetadataFile>(&paths.metadata());
    round_trips::<TimestampFile>(&paths.timestamps());
    round_trips::<SideloadLibraryFile>(&paths.sideload_library());
    round_trips::<heronite_launch::emulators::EmulatorsFile>(&paths.emulators());
}

#[test]
fn real_library_preview_import_merge_export() {
    let Some(root) = fixture() else {
        eprintln!("no fixture; skipping");
        return;
    };
    let Some(lib) = library_path(&root) else {
        eprintln!("Playnite library from the fixture is not on disk; skipping");
        return;
    };
    let (_dir, paths) = temp_copy(&root);
    let before_meta = json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap();
    let playnite = Playnite::new(paths.clone());

    // preview
    let preview = playnite.preview_import(&lib).unwrap();
    eprintln!(
        "PREVIEW games={} sessionFiles={} drives={:?} counts={:?} errors={:?}",
        preview.game_count, preview.session_file_count, preview.detected_drives, preview.counts, preview.errors
    );
    assert!(preview.game_count > 0);
    assert_eq!(preview.games.len(), preview.game_count);
    let already = preview.games.iter().filter(|g| g.already_imported).count();
    eprintln!("alreadyImported={already} existingOverlayGames={}", before_meta.games.len());
    for g in preview.games.iter().take(5) {
        eprintln!("  {:?}", g);
    }

    // import with the saved drive map (none in this library) into the copy
    let drive_map = playnite.last_drive_map().unwrap();
    let args = PlayniteImportArgs { library_path: lib.clone(), drive_map, fetch_covers: Some(true), merge_existing: false };
    let result = playnite.import(&args).unwrap();
    eprintln!("IMPORT {result:?}");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_all_round_trip(&paths);

    if let Some(out) = std::env::var_os("PLAYNITE_E2E_OUT") {
        // Keep the first import's result for manual diffing against the fixture.
        copy_json_tree(&paths.root, Path::new(&out), &["local_library", "sideload_apps", "store", "GamesConfig"]);
    }

    // second run is idempotent: nothing new, same game keys
    let after = json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap();
    let again = playnite.import(&args).unwrap();
    eprintln!("IMPORT again {again:?}");
    assert_eq!(again.imported, 0);
    let after2 = json_store::read::<LocalMetaFile>(&paths.local_meta()).unwrap();
    assert_eq!(after.games.len(), after2.games.len());
    assert_all_round_trip(&paths);

    // merge preview / merge
    let mp = playnite.preview_merge().unwrap();
    eprintln!(
        "MERGE PREVIEW new={} playtime={} statusFromPlaynite={} conflicts={} newSessions={} unchanged={} skipped={}",
        mp.new_games.len(), mp.playtime_updates.len(), mp.status_from_playnite.len(), mp.status_conflicts.len(), mp.new_sessions, mp.unchanged, mp.skipped
    );
    assert!(mp.errors.is_empty());
    assert!(mp.new_games.is_empty());
    let merged = playnite.merge().unwrap();
    eprintln!("MERGE {merged:?}");
    assert!(merged.errors.is_empty());
    assert_all_round_trip(&paths);

    // export
    let out = paths.root.join("exports");
    let res = export_playnite_library(&paths, &out).unwrap();
    let text = fs::read_to_string(&res.path).unwrap();
    assert!(text.ends_with("}\n"));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["format"], "heroic-playnite-export");
    assert_eq!(v["version"], 1);
    assert_eq!(v["games"].as_array().unwrap().len(), res.game_count);
    eprintln!("EXPORT {} games -> {}", res.game_count, res.path);
}

/// Merge straight onto the fixture as Heronite left it: user edits survive and
/// the new games / playtime from Playnite arrive.
#[test]
fn real_library_merge_onto_fixture() {
    let Some(root) = fixture() else { return };
    if library_path(&root).is_none() {
        return;
    }
    let (_dir, paths) = temp_copy(&root);
    let before: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
    let before_ts: TimestampFile = json_store::read(&paths.timestamps()).unwrap();
    let playnite = Playnite::new(paths.clone());
    let mp = playnite.preview_merge().unwrap();
    eprintln!(
        "MERGE PREVIEW (fixture) new={} playtime={} statusFromPlaynite={} conflicts={} newSessions={} unchanged={} skipped={}",
        mp.new_games.len(), mp.playtime_updates.len(), mp.status_from_playnite.len(), mp.status_conflicts.len(), mp.new_sessions, mp.unchanged, mp.skipped
    );
    for g in mp.new_games.iter().take(10) {
        eprintln!("  new: {} ({:?} min, {:?})", g.title, g.playnite_minutes, g.playnite_status);
    }
    for g in mp.playtime_updates.iter().take(5) {
        eprintln!("  playtime: {} {:?} -> {:?}", g.title, g.heroic_minutes, g.playnite_minutes);
    }
    for g in mp.status_conflicts.iter().take(5) {
        eprintln!("  conflict: {} {:?} vs {:?}", g.title, g.heroic_status, g.playnite_status);
    }
    let r = playnite.merge().unwrap();
    eprintln!("MERGE (fixture) {r:?}");
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_all_round_trip(&paths);

    let after: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
    // merge keeps identity and the fields the mapper never sets
    for (k, v) in &before.games {
        let w = &after.games[k];
        assert_eq!(v.playnite_id, w.playnite_id, "{k}");
        assert_eq!(v.runner, w.runner, "{k}");
        assert_eq!(v.launch_kind, w.launch_kind, "{k}");
        assert_eq!(v.platform_id, w.platform_id, "{k}");
        assert_eq!(v.selected_rom_path, w.selected_rom_path, "{k}");
    }
    assert_eq!(after.games.len(), before.games.len() + mp.new_games.len());
    let after_ts: TimestampFile = json_store::read(&paths.timestamps()).unwrap();
    for (k, v) in &before_ts {
        assert!(after_ts[k].total_played.get() >= v.total_played.get(), "{k}");
    }
    assert_eq!(after_ts.len() >= before_ts.len(), true);
}
