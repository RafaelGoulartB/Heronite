//! Restores a real Ludusavi backup into a temporary prefix with the real
//! binary. Needs Ludusavi and a backup folder (`HERONITE_LUDUSAVI_BACKUPS`,
//! `HERONITE_LUDUSAVI_GAME`); skipped otherwise. Writes only to temp folders.
//! Run with `cargo test -p heronite-emulation --test real_restore -- --nocapture`.

use std::path::PathBuf;

use heronite_data::models::LudusaviSettings;
use heronite_data::DataPaths;
use heronite_emulation::ludusavi::restore::{
    apply_restore, parse_backups, plan_restore, undo, FileKind, RestoreRequest, RestoreTarget, Tool,
};
use heronite_emulation::ludusavi::{parse_json, CommandRunner, LudusaviEnv, SystemRunner};
use heronite_launch::SystemProbe;

#[test]
fn restores_into_a_new_prefix_and_undoes_it() {
    let (Some(root), Some(game)) = (std::env::var_os("HERONITE_LUDUSAVI_BACKUPS"), std::env::var("HERONITE_LUDUSAVI_GAME").ok()) else {
        eprintln!("HERONITE_LUDUSAVI_BACKUPS / HERONITE_LUDUSAVI_GAME not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let Some(binary) = ["/usr/bin/ludusavi", "/usr/local/bin/ludusavi"].into_iter().find(|b| std::path::Path::new(b).exists()) else {
        eprintln!("no Ludusavi binary; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(tmp.path().join("data"));
    let runner = SystemRunner::default();
    let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &SystemProbe };
    let tool = Tool { binary: binary.into(), detected: None, settings: LudusaviSettings::default() };

    let listed = runner
        .run(
            binary,
            &["--no-manifest-update".into(), "backups".into(), "--api".into(), "--path".into(), root.display().to_string(), game.clone()],
        )
        .unwrap();
    let backups = parse_backups(&parse_json(&listed.stdout), &game);
    eprintln!("backups: {backups:#?}");
    let backup = backups.first().expect("a backup of the game");

    let prefix = tmp.path().join("prefix/pfx");
    std::fs::create_dir_all(prefix.join("drive_c/users/steamuser")).unwrap();
    let target = RestoreTarget { prefix: Some(prefix.clone()), user: Some("steamuser".into()), needs_prefix: true, ..Default::default() };
    let request = RestoreRequest { source_root: root.clone(), game_name: game.clone(), backup_id: backup.id.clone(), target };
    let plan = plan_restore(&env, &tool, request).unwrap();
    for f in &plan.files {
        eprintln!("{:?} {:?} {} <- {}", f.kind, f.change, f.target, f.original);
    }
    eprintln!("redirects: {:#?}", plan.redirects);
    let saves: Vec<&String> = plan.files.iter().filter(|f| f.kind == FileKind::Save).map(|f| &f.target).collect();
    assert!(!saves.is_empty(), "no file lands in the prefix");
    assert!(saves.iter().all(|t| t.starts_with(&prefix.to_string_lossy().into_owned())));

    let outcome = apply_restore(&env, &tool, &plan, &paths.save_restore_undo(), "real-test").unwrap();
    eprintln!("written {}, unchanged {}, failed {:?}", outcome.written, outcome.unchanged, outcome.failed);
    assert!(outcome.failed.is_empty());
    for t in &saves {
        assert!(std::path::Path::new(t).is_file(), "{t} was not restored");
    }
    let record = outcome.undo.expect("an undo copy");
    let removed = undo::undo(&record).unwrap();
    assert_eq!(removed, saves.len());
    assert!(saves.iter().all(|t| !std::path::Path::new(t).exists()));
}
