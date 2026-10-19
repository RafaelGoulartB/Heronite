//! Backups of a copy of a real setup. Opt-in: `HERONITE_BACKUP_FIXTURE` names
//! a folder holding `data/` (a data folder) and `config/` (an app state
//! folder); the test copies nothing back into it.

use std::path::PathBuf;

use heronite_backup::{restore, snapshot, usage, Part, Reason};
use heronite_data::{AppPaths, DataPaths};

#[test]
fn backup_twice_and_restore_a_copy_of_a_real_setup() {
    let Some(root) = std::env::var_os("HERONITE_BACKUP_FIXTURE").map(PathBuf::from) else {
        eprintln!("HERONITE_BACKUP_FIXTURE not set; skipping");
        return;
    };
    let data = DataPaths::at(root.join("data"));
    let app = AppPaths { config: root.join("config"), cache: root.join("cache") };
    let u = usage::measure(&data, &app);
    println!("parts: {:?}\nleft out: {:?}", u.parts, u.left);

    let out = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    let first = snapshot::create(&data, &app, out.path(), &Part::ALL, Reason::Manual, chrono::Local::now()).unwrap();
    println!("first: {:?} in {:?}", first.summary, t.elapsed());
    let t = std::time::Instant::now();
    let second = snapshot::create(&data, &app, out.path(), &Part::ALL, Reason::Manual, chrono::Local::now()).unwrap();
    println!("second: {:?} in {:?}", second.summary, t.elapsed());
    assert_eq!(second.summary.new_bytes, 0, "nothing changed in between");
    println!("folder on disk: {} bytes for two backups", snapshot::folder_usage(out.path()));

    let target = tempfile::tempdir().unwrap();
    let to = DataPaths::at(target.path().join("data"));
    let to_app = AppPaths { config: target.path().join("config"), cache: target.path().join("cache") };
    let t = std::time::Instant::now();
    let done = restore::restore(&to, &to_app, &second.path, &Part::ALL).unwrap();
    println!("restore: {} files, {} rewritten, in {:?}", done.written, done.rewritten, t.elapsed());
    assert_eq!(done.written, second.summary.files);
    assert!(to.local_meta().is_file());
}
