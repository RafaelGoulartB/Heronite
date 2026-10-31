//! Windows: backs up, lists, restores and undoes a save with the real
//! `ludusavi.exe`, through the code the app runs, then restores a backup
//! made on Linux (in a Proton prefix) into this user's folder.
//!
//! Needs `HERONITE_LUDUSAVI_EXE` (the program of Ludusavi's `-win64.zip`);
//! skipped otherwise. The program is copied into a temp folder as a portable
//! copy (`ludusavi.portable` next to it), so its config, the "game" (a custom
//! game whose save is a temp file), the backups and Heronite's data folder
//! are all temporary: `%APPDATA%\ludusavi` and real saves are never touched.
//! Run with `cargo test -p heronite-emulation --test real_windows -- --nocapture`.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use heronite_emulation::ludusavi::restore::{
    apply_restore, list_backups, plan_restore, undo, FileKind, RestoreRequest, RestoreTarget, Tool,
};
use heronite_emulation::ludusavi::{
    backup_after_play, backup_for_game, detect_ludusavi, last_backup, AfterPlay, BackupReason, LudusaviEnv, LudusaviGame, SystemRunner,
};
use heronite_launch::SystemProbe;

const GAME: &str = "Heronite Test Game";

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// A portable Ludusavi in `dir` whose only game is [`GAME`], saved at `save`.
fn portable_ludusavi(exe: &Path, dir: &Path, save: &Path, backups: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let copy = dir.join("ludusavi.exe");
    fs::copy(exe, &copy).unwrap();
    fs::write(dir.join("ludusavi.portable"), "").unwrap();
    // JSON is YAML. No manifest (no download), no update check, no roots:
    // the custom game is all it knows.
    let config = serde_json::json!({
        "manifest": {"enable": false},
        "release": {"check": false},
        "roots": [],
        "backup": {"path": slashes(backups)},
        "restore": {"path": slashes(backups)},
        "customGames": [{"name": GAME, "integration": "override", "files": [slashes(save)], "registry": []}],
    });
    fs::write(dir.join("config.yaml"), config.to_string()).unwrap();
    copy
}

#[test]
fn backs_up_restores_and_undoes_with_the_real_program() {
    let Some(exe) = std::env::var_os("HERONITE_LUDUSAVI_EXE").map(PathBuf::from) else {
        eprintln!("HERONITE_LUDUSAVI_EXE not set; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join(r"saves\HeroniteTest\slot1.sav");
    fs::create_dir_all(save.parent().unwrap()).unwrap();
    fs::write(&save, "first").unwrap();
    let backups = tmp.path().join("backups");
    let binary = portable_ludusavi(&exe, &tmp.path().join("ludusavi"), &save, &backups);
    let binary_s = binary.to_string_lossy().into_owned();

    let runner = SystemRunner::default();
    let detected = detect_ludusavi(&runner, &SystemProbe, Some(&binary_s)).expect("Ludusavi found");
    eprintln!("detected: {detected:#?}");
    assert!(detected.version.is_some());
    assert_eq!(detected.config_path.as_deref().map(|p| p.replace('\\', "/")), Some(slashes(&tmp.path().join(r"ludusavi\config.yaml"))));
    assert_eq!(detected.backup_path.as_deref(), Some(slashes(&backups).as_str()));

    // Heronite's settings: Ludusavi's own config (the portable one).
    let paths = DataPaths::at(tmp.path().join("data"));
    fs::create_dir_all(paths.local_library()).unwrap();
    let settings = serde_json::json!({"ludusavi": {
        "enabled": true, "useInstalledConfig": true, "binaryPath": binary_s, "backupPath": "", "format": "zip", "compression": "deflate"
    }});
    fs::write(paths.collection_settings(), settings.to_string()).unwrap();
    let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &SystemProbe };
    let game = LudusaviGame { app_name: "htest".into(), title: GAME.into(), runner: "sideload".into(), ..Default::default() };

    // By hand.
    let r = backup_for_game(&env, &game, BackupReason::Manual);
    eprintln!("manual backup: {r:#?}");
    assert!(r.ran, "{r:?}");
    assert_eq!(r.game_name.as_deref(), Some(GAME));
    assert_eq!(r.backup_path.as_deref().map(|p| p.replace('\\', "/")), Some(format!("{}/{GAME}", slashes(&backups))));
    let last = last_backup(&paths, GAME, None).expect("the last backup is found in the folder");
    assert_eq!(last.game_name, GAME);

    // After play (a session long enough).
    fs::write(&save, "second").unwrap();
    let play = AfterPlay { app_name: "htest".into(), title: GAME.into(), runner: "sideload".into(), elapsed_seconds: 60 };
    let r = backup_after_play(&env, &play).expect("a backup after play");
    assert!(r.ran, "{r:?}");

    // List, then restore the newest ("second") over a changed save.
    fs::write(&save, "changed").unwrap();
    let tool = Tool::resolve(&env).unwrap();
    let listed = list_backups(&env, &tool, &game, None);
    eprintln!("listed: {listed:#?}");
    let source = listed.iter().find(|s| !s.backups.is_empty()).expect("backups listed");
    let newest = &source.backups[0];
    assert_eq!(newest.os.as_deref(), Some("windows"));
    // This user's folder: the save (in the temp folder) is in it.
    let target = RestoreTarget { user_home: heronite_platform::dirs::home(), ..Default::default() };
    let request = RestoreRequest {
        source_root: source.source.root.clone(),
        game_name: source.game_name.clone().unwrap(),
        backup_id: newest.id.clone(),
        target,
    };
    let plan = plan_restore(&env, &tool, request).unwrap();
    eprintln!("plan: {:#?} redirects {:?}", plan.files, plan.redirects);
    assert!(plan.redirects.is_empty(), "a backup of this PC goes back where it was");
    assert_eq!(plan.files.len(), 1);
    assert_eq!((plan.files[0].kind, plan.files[0].selected), (FileKind::Save, true));
    assert_eq!(plan.overwrites(), 1);
    let undo_root = paths.save_restore_undo();
    let outcome = apply_restore(&env, &tool, &plan, &undo_root, "htest").unwrap();
    assert_eq!((outcome.written, outcome.failed.len()), (1, 0), "{outcome:?}");
    assert_eq!(fs::read_to_string(&save).unwrap(), "second");
    let record = undo::latest(&undo_root, "htest").expect("an undo copy");
    assert_eq!(undo::undo(&record).unwrap(), 1);
    assert_eq!(fs::read_to_string(&save).unwrap(), "changed");

    // A backup made on Linux, in a Proton prefix: lands in this user's folder.
    let linux_root = tmp.path().join("linux-backups");
    let pfx = "/home/u/.local/share/Steam/steamapps/compatdata/42/pfx";
    let stored = format!("{pfx}/drive_c/users/steamuser/AppData/Roaming/HT/slot1.sav");
    let game_dir = linux_root.join(GAME);
    let file = game_dir.join("drive-0").join(stored.trim_start_matches('/').replace('/', "\\"));
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "from linux").unwrap();
    let mapping = format!(
        "---\nname: {GAME}\ndrives:\n  drive-0: \"\"\nbackups:\n  - name: \".\"\n    when: \"2026-01-01T00:00:00Z\"\n    os: linux\n    files:\n      \"{stored}\":\n        hash: 0000000000000000000000000000000000000000\n        size: 10\n    registry:\n      hash: ~\n    children: []\n"
    );
    fs::write(game_dir.join("mapping.yaml"), mapping).unwrap();
    let listed = list_backups(&env, &tool, &game, Some(&linux_root));
    let source = listed.iter().find(|s| s.source.root == linux_root).unwrap();
    assert_eq!(source.backups[0].os.as_deref(), Some("linux"));
    // A stand-in for this user's folder, so nothing lands in the real one.
    let home = tmp.path().join(r"Users\me");
    let target = RestoreTarget { user_home: Some(home.clone()), ..Default::default() };
    let request = RestoreRequest { source_root: linux_root.clone(), game_name: GAME.into(), backup_id: ".".into(), target };
    let plan = plan_restore(&env, &tool, request).unwrap();
    eprintln!("linux plan: {:#?} redirects {:?}", plan.files, plan.redirects);
    let landed = home.join(r"AppData\Roaming\HT\slot1.sav");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].target, slashes(&landed));
    assert_eq!(plan.files[0].kind, FileKind::Save);
    let outcome = apply_restore(&env, &tool, &plan, &undo_root, "htest").unwrap();
    assert_eq!(outcome.written, 1, "{outcome:?}");
    assert_eq!(fs::read_to_string(&landed).unwrap(), "from linux");
    // Undo removes what the restore created.
    assert_eq!(undo::undo(&outcome.undo.unwrap()).unwrap(), 1);
    assert!(!landed.exists());
}
