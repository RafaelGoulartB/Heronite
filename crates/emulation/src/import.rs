//! Importing scanned ROMs (port of `emulation/import-roms.ts`) and the ROM
//! auto-scan on refresh.
//!
//! Imports write, in this order and each in one read-modify-write:
//! `sideload_apps/library.json` (Heroic `GameInfo`, merged by `app_name`),
//! `local_library/library.json` (overlay meta), `local_library/metadata.json`
//! (seeds `platforms`) -- the same files and shapes Heronite writes.

use anyhow::Result;
use heronite_data::models::{CollectionGameMetadata, LocalGameMeta, LocalGameRom, LocalMetaFile, MetadataFile};
use heronite_data::status::{inferred_status_id, load_statuses};
use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::pathutil::{dirname, normalize_key};
use crate::scan::{imported_with, live_emulator_ids, orphaned, preview_rom_scan};
use crate::sha1::sha1_hex;
use crate::store::{self, resolve_template};
use crate::types::{RomScanArgs, RomScanImportArgs, RomScanImportResult, RomScanItem, UpsertRomScannerArgs};

/// `emuAppName`: `emu_` + first 16 hex chars of sha1(`emulatorId|profileId|key`).
pub fn emu_app_name(emulator_id: &str, profile_id: &str, key: &str) -> String {
    let hash = sha1_hex(format!("{emulator_id}|{profile_id}|{key}").as_bytes());
    format!("emu_{}", &hash[..16])
}

/// `findExisting`: the imported game that already owns one of these ROMs,
/// else one whose emulator was removed ([`orphaned`]), which is re-linked.
fn find_existing<'a>(
    metas: &'a LocalMetaFile,
    emulator_id: &str,
    profile_id: &str,
    live: &std::collections::HashSet<String>,
    item: &RomScanItem,
) -> Option<&'a LocalGameMeta> {
    let keys: std::collections::HashSet<String> = item.roms.iter().map(|r| normalize_key(&r.path)).collect();
    let owns = |meta: &&LocalGameMeta| meta.roms.iter().flatten().any(|r| keys.contains(&normalize_key(&r.path)));
    metas
        .games
        .values()
        .filter(owns)
        .find(|meta| imported_with(meta, emulator_id, profile_id))
        .or_else(|| metas.games.values().filter(owns).find(|meta| meta.source == "emulator" && orphaned(meta, live)))
}

/// Shallow merge into `sideload_apps/library.json` `games` like
/// `{ ...current[index], ...game }` -- except that empty art strings never
/// replace art the game already has (Heronite wipes it on re-import).
fn upsert_sideload_game(games: &mut Vec<Value>, game: Value) {
    let app = game["app_name"].as_str().unwrap_or_default().to_owned();
    let existing = games.iter_mut().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app.as_str()));
    match existing {
        Some(current) => {
            let cur = current.as_object_mut().expect("game object");
            for (k, v) in game.as_object().expect("game object") {
                let keep_art = matches!(k.as_str(), "art_cover" | "art_square")
                    && v.as_str() == Some("")
                    && cur.get(k).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
                if !keep_art {
                    cur.insert(k.clone(), v.clone());
                }
            }
        }
        None => games.push(game),
    }
}

/// `seedPlatforms` on the in-memory metadata file.
fn seed_platforms(file: &mut MetadataFile, app_name: &str, platform_name: Option<&str>) {
    let Some(platform) = platform_name.filter(|p| !p.is_empty()) else {
        return;
    };
    let key = format!("sideload:{app_name}");
    let entry = file.games.entry(key).or_insert_with(|| CollectionGameMetadata {
        app_name: app_name.into(),
        runner: "sideload".into(),
        ..Default::default()
    });
    if entry.platforms.iter().any(|p| p.eq_ignore_ascii_case(platform)) {
        return;
    }
    entry.platforms.push(platform.into());
    entry.field_sources.insert("platforms".into(), "manual".into());
}

/// `importScannedRoms`.
pub fn import_scanned_roms(paths: &DataPaths, args: &RomScanImportArgs) -> Result<RomScanImportResult> {
    let mut result = RomScanImportResult { scanner_id: args.scanner_id.clone().unwrap_or_default(), ..Default::default() };
    let Some(emulator) = store::get_user_emulator(paths, &args.emulator_id)? else {
        result.errors.push(tr!("Emulator not found.").into());
        return Ok(result);
    };
    let Some(profile) = emulator.profile(&args.profile_id).cloned() else {
        result.errors.push(tr!("Emulator profile not found.").into());
        return Ok(result);
    };
    let template = resolve_template(&emulator, &profile);
    let native = emulator.launch_kind != "wine";
    let installed_exe = if emulator.launch_kind == "flatpak" { "flatpak".to_string() } else { emulator.executable.clone() };

    let scanner = store::upsert_rom_scanner(
        paths,
        UpsertRomScannerArgs {
            id: args.scanner_id.clone(),
            name: None,
            emulator_id: emulator.id.clone(),
            profile_id: profile.id.clone(),
            directory: args.directory.clone(),
            scan_subfolders: args.scan_subfolders,
            merge_related_files: args.merge_related_files,
            auto_scan_on_refresh: args.auto_scan_on_refresh,
        },
    )?;
    result.scanner_id = scanner.id;

    let selected: Vec<&RomScanItem> = args.games.iter().filter(|g| g.import && !g.roms.is_empty()).collect();
    result.skipped = args.games.len() - selected.len();
    if selected.is_empty() {
        return Ok(result);
    }

    let statuses = load_statuses(paths)?;
    let not_played = inferred_status_id(0.0, &statuses);

    let live = live_emulator_ids(paths)?;
    let mut metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let mut sideload_games: Vec<Value> = vec![];
    let mut seeded: Vec<String> = vec![];
    for item in selected {
        let existing = find_existing(&metas, &emulator.id, &profile.id, &live, item).cloned();
        let mut identity: Vec<String> = item.roms.iter().map(|r| normalize_key(&r.path)).collect();
        identity.sort();
        let app_name =
            existing.as_ref().map(|m| m.app_name.clone()).unwrap_or_else(|| emu_app_name(&emulator.id, &profile.id, &identity.join("|")));
        let roms: Vec<LocalGameRom> = match &existing {
            Some(ex) => {
                let incoming: Vec<String> = item.roms.iter().map(|r| normalize_key(&r.path)).collect();
                ex.roms
                    .iter()
                    .flatten()
                    .filter(|r| !incoming.contains(&normalize_key(&r.path)))
                    .cloned()
                    .chain(item.roms.iter().cloned())
                    .collect()
            }
            None => item.roms.clone(),
        };
        let first_rom = roms.first().map(|r| r.path.clone());
        let title = if !item.title.is_empty() {
            item.title.clone()
        } else {
            existing.as_ref().map(|m| m.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| app_name.clone())
        };
        // Start from the existing entry so selectedRomPath, saveStateKeys,
        // notes, ... survive a re-import.
        let mut meta = existing.clone().unwrap_or_default();
        meta.app_name = app_name.clone();
        meta.runner = "sideload".into();
        meta.playnite_id = existing.as_ref().map(|m| m.playnite_id.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| app_name.clone());
        meta.source = "emulator".into();
        meta.launch_kind = "emulator".into();
        meta.title = title.clone();
        meta.remapped_executable = Some(installed_exe.clone());
        meta.launcher_args = Some(template.arguments.clone());
        meta.roms = Some(roms);
        meta.emulator_name = Some(emulator.name.clone());
        meta.emulator_id = Some(emulator.id.clone());
        meta.emulator_profile_id = Some(profile.id.clone());
        meta.platform_id = template.platform_id.clone();
        meta.completion_status_id = existing.as_ref().and_then(|m| m.completion_status_id.clone()).or_else(|| Some(not_played.clone()));

        let folder = match &first_rom {
            Some(r) => dirname(r),
            None => dirname(&installed_exe),
        };
        let game = json!({
            "runner": "sideload",
            "app_name": app_name,
            "title": title,
            "art_cover": "",
            "art_square": "",
            "is_installed": !installed_exe.is_empty() && first_rom.is_some(),
            "canRunOffline": true,
            "is_linux_native": native,
            "folder_name": folder,
            "install": {
                "executable": installed_exe,
                "platform": if native { "linux" } else { "Windows" },
                "is_dlc": false,
            },
        });
        upsert_sideload_game(&mut sideload_games, game);
        metas.games.insert(app_name.clone(), meta);
        seeded.push(app_name);
        if existing.is_some() {
            result.updated += 1;
        } else {
            result.imported += 1;
        }
    }

    json_store::update_value(&paths.sideload_library(), |doc| {
        let obj = doc.as_object_mut().expect("object");
        let games = obj.entry("games").or_insert_with(|| Value::Array(vec![]));
        if !games.is_array() {
            *games = Value::Array(vec![]);
        }
        let list = games.as_array_mut().expect("array");
        for g in sideload_games.drain(..) {
            upsert_sideload_game(list, g);
        }
        Ok(())
    })?;
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for name in &seeded {
            if let Some(m) = metas.games.get(name) {
                file.games.insert(name.clone(), m.clone());
            }
        }
        Ok(())
    })?;
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        for name in &seeded {
            seed_platforms(file, name, template.platform_name.as_deref());
        }
        Ok(())
    })?;
    log::info!("ROM scan imported {}, updated {}", result.imported, result.updated);
    Ok(result)
}

/// Outcome of scanning ROM folders in the background.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutoScanReport {
    /// Folders that were scanned.
    pub scanned: usize,
    /// Newly imported games.
    pub imported: usize,
    /// One message per folder that could not be scanned (missing folder,
    /// unknown emulator, ...). The other folders are still scanned.
    pub errors: Vec<String>,
}

/// Scans one saved folder and imports the games not imported yet.
pub fn scan_saved_folder(paths: &DataPaths, scanner: &crate::types::RomScanner) -> Result<(usize, Vec<String>)> {
    let preview = preview_rom_scan(
        paths,
        &RomScanArgs {
            emulator_id: scanner.emulator_id.clone(),
            profile_id: scanner.profile_id.clone(),
            directory: scanner.directory.clone(),
            scan_subfolders: Some(scanner.scan_subfolders),
            merge_related_files: Some(scanner.merge_related_files),
        },
    )?;
    let mut errors = preview.errors.clone();
    let newcomers: Vec<RomScanItem> = preview
        .games
        .into_iter()
        .filter(|g| !g.already_imported)
        .map(|mut g| {
            g.import = true;
            g
        })
        .collect();
    if newcomers.is_empty() {
        return Ok((0, errors));
    }
    let result = import_scanned_roms(
        paths,
        &RomScanImportArgs {
            emulator_id: scanner.emulator_id.clone(),
            profile_id: scanner.profile_id.clone(),
            directory: scanner.directory.clone(),
            scanner_id: Some(scanner.id.clone()),
            scan_subfolders: Some(scanner.scan_subfolders),
            merge_related_files: Some(scanner.merge_related_files),
            auto_scan_on_refresh: Some(scanner.auto_scan_on_refresh),
            games: newcomers,
        },
    )?;
    errors.extend(result.errors);
    Ok((result.imported, errors))
}

/// `runAutoRomScanners` with a report: every scanner with `autoScanOnRefresh`
/// (or, with `only`, exactly that scanner whatever its flag) imports the games
/// not imported yet. A folder that fails does not stop the others.
pub fn run_rom_scanners(paths: &DataPaths, only: Option<&str>) -> Result<AutoScanReport> {
    let mut report = AutoScanReport::default();
    for scanner in store::list_rom_scanners(paths)? {
        match only {
            Some(id) if scanner.id != id => continue,
            None if !scanner.auto_scan_on_refresh => continue,
            _ => {}
        }
        report.scanned += 1;
        match scan_saved_folder(paths, &scanner) {
            Ok((n, errors)) => {
                report.imported += n;
                report.errors.extend(errors.into_iter().map(|e| format!("{}: {e}", scanner.name)));
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", scanner.name)),
        }
    }
    Ok(report)
}

/// `runAutoRomScanners`: for every scanner with `autoScanOnRefresh`, imports
/// the games not imported yet. Returns the number of newly imported games.
pub fn run_auto_rom_scanners(paths: &DataPaths) -> Result<usize> {
    Ok(run_rom_scanners(paths, None)?.imported)
}

/// `setEmulatorLaunchRom`: remembers which ROM (disc) to launch.
pub fn set_emulator_launch_rom(paths: &DataPaths, app_name: &str, rom_path: &str) -> Result<Option<LocalGameMeta>> {
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        Ok(file.games.get_mut(app_name).map(|m| {
            m.selected_rom_path = Some(rom_path.to_string());
            m.clone()
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UpsertUserEmulatorArgs;
    use std::fs::{create_dir_all, write};

    fn setup() -> (tempfile::TempDir, DataPaths, tempfile::TempDir, String, String) {
        let data_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(data_dir.path());
        let roms = tempfile::tempdir().unwrap();
        create_dir_all(roms.path().join("Sub")).unwrap();
        write(roms.path().join("Alpha (USA).iso"), "x").unwrap();
        write(roms.path().join("Sub/Beta (EU).iso"), "x").unwrap();
        let e = store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "pcsx2".into(),
                name: "PCSX2".into(),
                executable: "/opt/pcsx2".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        (data_dir, paths, roms, e.id, e.profiles[0].id.clone())
    }

    fn scan(paths: &DataPaths, e: &str, p: &str, dir: &str) -> Vec<RomScanItem> {
        preview_rom_scan(paths, &RomScanArgs { emulator_id: e.into(), profile_id: p.into(), directory: dir.into(), ..Default::default() })
            .unwrap()
            .games
    }

    #[test]
    fn app_names_are_stable_sha1_prefixes() {
        let n = emu_app_name("e", "p", "/a|/b");
        assert!(n.starts_with("emu_") && n.len() == 20);
        assert_eq!(n, emu_app_name("e", "p", "/a|/b"));
        // sha1("c6933e76-...|custom|/data/...") reproduces Heronite's name
        // (checked against the real fixture in the real-data test).
        assert_ne!(n, emu_app_name("e", "p", "/a"));
    }

    #[test]
    fn import_writes_all_files_and_reimport_updates() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        let games = scan(&paths, &e, &p, dir);
        assert_eq!(games.len(), 2);
        let r = import_scanned_roms(
            &paths,
            &RomScanImportArgs {
                emulator_id: e.clone(),
                profile_id: p.clone(),
                directory: dir.into(),
                games: games.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((r.imported, r.updated, r.skipped), (2, 0, 0));
        assert!(r.errors.is_empty());
        assert!(!r.scanner_id.is_empty());

        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games.len(), 2);
        let alpha = metas.games.values().find(|m| m.title == "Alpha").unwrap();
        assert!(alpha.app_name.starts_with("emu_"));
        assert_eq!(alpha.launch_kind, "emulator");
        assert_eq!(alpha.source, "emulator");
        assert_eq!(alpha.remapped_executable.as_deref(), Some("/opt/pcsx2"));
        assert_eq!(alpha.launcher_args.as_deref(), Some("-fullscreen -slowboot -- {ImagePath}"));
        assert_eq!(alpha.platform_id.as_deref(), Some("sony_playstation2"));
        assert_eq!(alpha.completion_status_id.as_deref(), Some("not-played"));
        assert_eq!(alpha.playnite_id, alpha.app_name);

        let side = json_store::read_value(&paths.sideload_library()).unwrap();
        let list = side["games"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        let g = list.iter().find(|g| g["title"] == "Alpha").unwrap();
        assert_eq!(g["runner"], "sideload");
        assert_eq!(g["is_installed"], true);
        assert_eq!(g["canRunOffline"], true);
        assert_eq!(g["install"]["executable"], "/opt/pcsx2");
        assert_eq!(g["install"]["platform"], "linux");
        assert_eq!(g["folder_name"].as_str().unwrap(), dir);

        let md: MetadataFile = json_store::read(&paths.metadata()).unwrap();
        let m = &md.games[&format!("sideload:{}", alpha.app_name)];
        assert_eq!(m.platforms, ["PlayStation 2"]);
        assert_eq!(m.field_sources["platforms"], "manual");

        // Scanner persisted.
        assert_eq!(store::list_rom_scanners(&paths).unwrap().len(), 1);

        // Already-imported items are flagged and re-import updates in place,
        // keeping user data and art.
        let again = scan(&paths, &e, &p, dir);
        assert!(again.iter().all(|g| g.already_imported && !g.import));
        json_store::update_value(&paths.sideload_library(), |doc| {
            doc["games"][0]["art_cover"] = json!("https://art/1.png");
            Ok(())
        })
        .unwrap();
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            let k = f.games.keys().next().unwrap().clone();
            f.games.get_mut(&k).unwrap().selected_rom_path = Some("/x/sel.iso".into());
            Ok(())
        })
        .unwrap();
        let mut forced = again.clone();
        for g in &mut forced {
            g.import = true;
        }
        let r2 = import_scanned_roms(
            &paths,
            &RomScanImportArgs { emulator_id: e, profile_id: p, directory: dir.into(), games: forced, ..Default::default() },
        )
        .unwrap();
        assert_eq!((r2.imported, r2.updated), (0, 2));
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        assert_eq!(metas.games.len(), 2);
        assert!(metas.games.values().any(|m| m.selected_rom_path.as_deref() == Some("/x/sel.iso")));
        let side = json_store::read_value(&paths.sideload_library()).unwrap();
        assert_eq!(side["games"].as_array().unwrap().len(), 2);
        assert_eq!(side["games"][0]["art_cover"], "https://art/1.png");
    }

    #[test]
    fn an_emulator_removed_and_added_again_finds_its_games() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        let import = |e: &str, p: &str| {
            let games = scan(&paths, e, p, dir);
            import_scanned_roms(
                &paths,
                &RomScanImportArgs { emulator_id: e.into(), profile_id: p.into(), directory: dir.into(), games, ..Default::default() },
            )
            .unwrap()
        };
        assert_eq!(import(&e, &p).imported, 2);
        let before: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();

        store::remove_user_emulator(&paths, &e).unwrap();
        let again = store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "pcsx2".into(),
                name: "PCSX2".into(),
                executable: "/opt/pcsx2".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(again.id, e);
        let r = import(&again.id, &again.profiles[0].id);
        assert_eq!((r.imported, r.updated), (0, 2), "re-linked, not added twice");
        let after: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        let names = |m: &LocalMetaFile| m.games.keys().cloned().collect::<Vec<_>>();
        assert_eq!(names(&after), names(&before));
        assert!(after.games.values().all(|m| m.emulator_id.as_deref() == Some(again.id.as_str())));
        assert_eq!(json_store::read_value(&paths.sideload_library()).unwrap()["games"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn skipped_and_missing_emulator() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        let mut games = scan(&paths, &e, &p, dir);
        games[0].import = false;
        let r = import_scanned_roms(
            &paths,
            &RomScanImportArgs { emulator_id: e, profile_id: p, directory: dir.into(), games, ..Default::default() },
        )
        .unwrap();
        assert_eq!((r.imported, r.skipped), (1, 1));
        let r = import_scanned_roms(
            &paths,
            &RomScanImportArgs {
                emulator_id: "nope".into(),
                profile_id: "x".into(),
                directory: dir.into(),
                games: vec![],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.errors, ["Emulator not found."]);
    }

    #[test]
    fn auto_scan_imports_only_newcomers() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs { emulator_id: e.clone(), profile_id: p.clone(), directory: dir.into(), ..Default::default() },
        )
        .unwrap();
        assert_eq!(run_auto_rom_scanners(&paths).unwrap(), 2);
        assert_eq!(run_auto_rom_scanners(&paths).unwrap(), 0);
        write(roms.path().join("Gamma.iso"), "x").unwrap();
        assert_eq!(run_auto_rom_scanners(&paths).unwrap(), 1);
        // Disabled scanners are skipped.
        store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                emulator_id: e,
                profile_id: p,
                directory: dir.into(),
                auto_scan_on_refresh: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        write(roms.path().join("Delta.iso"), "x").unwrap();
        assert_eq!(run_auto_rom_scanners(&paths).unwrap(), 0);
    }

    #[test]
    fn rom_scanners_report_errors_and_continue() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        // A broken folder first: it must not stop the good one.
        store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                name: Some("Gone".into()),
                emulator_id: e.clone(),
                profile_id: p.clone(),
                directory: "/no/such/folder".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let good = store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs { emulator_id: e.clone(), profile_id: p.clone(), directory: dir.into(), ..Default::default() },
        )
        .unwrap();
        let off = store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                emulator_id: e,
                profile_id: p,
                directory: "/off".into(),
                auto_scan_on_refresh: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        let r = run_rom_scanners(&paths, None).unwrap();
        assert_eq!((r.scanned, r.imported), (2, 2));
        assert_eq!(r.errors, ["Gone: Folder does not exist: /no/such/folder"]);
        // A single folder is scanned whatever its auto-scan flag.
        write(roms.path().join("Gamma.iso"), "x").unwrap();
        let r = run_rom_scanners(&paths, Some(&good.id)).unwrap();
        assert_eq!((r.scanned, r.imported, r.errors.len()), (1, 1, 0));
        let r = run_rom_scanners(&paths, Some(&off.id)).unwrap();
        assert_eq!((r.scanned, r.imported, r.errors.len()), (1, 0, 1));
    }

    #[test]
    fn launch_rom_selection() {
        let (_h, paths, roms, e, p) = setup();
        let dir = roms.path().to_str().unwrap();
        let games = scan(&paths, &e, &p, dir);
        import_scanned_roms(
            &paths,
            &RomScanImportArgs { emulator_id: e, profile_id: p, directory: dir.into(), games, ..Default::default() },
        )
        .unwrap();
        let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
        let name = metas.games.keys().next().unwrap().clone();
        let m = set_emulator_launch_rom(&paths, &name, "/r/x.iso").unwrap().unwrap();
        assert_eq!(m.selected_rom_path.as_deref(), Some("/r/x.iso"));
        assert!(set_emulator_launch_rom(&paths, "nope", "/r").unwrap().is_none());
    }
}
