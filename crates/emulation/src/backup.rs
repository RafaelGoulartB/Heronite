//! Scheduled backup of the data folder (port of Heronite's
//! `local-library/backup.ts`): a filtered copy to `heronite-YYYY-MM-DD`
//! inside the configured folder when due (daily/weekly/monthly).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Local};
use heronite_data::models::BackupSettings;
use heronite_data::{settings, DataPaths};
use serde::{Deserialize, Serialize};

use crate::iso::iso_millis;
use crate::pathutil::{join, resolve};
use crate::settings_ext::{record_collection_backup, BackupRecord};

/// File/folder names never copied (caches, tools, logs).
pub const SKIP_NAMES: &[&str] =
    &["images-cache", "tools", "store_cache", "screenshot-thumbnails", "steam-details", "logs"];

/// `CollectionBackupResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionBackupResult {
    pub ran: bool,
    /// 'disabled' | 'not-due' | 'already-running'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static RUNNING: AtomicBool = AtomicBool::new(false);

struct Running;

impl Running {
    fn acquire() -> Option<Self> {
        RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Running)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// `localDateKey`: `YYYY-MM-DD` in local time.
pub fn local_date_key(date: DateTime<Local>) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// `isBackupDue`.
pub fn is_backup_due(backup: &BackupSettings, now: DateTime<Local>) -> bool {
    if backup.folder.trim().is_empty() {
        return false;
    }
    let Some(last_raw) = backup.last_backup_at.as_deref().filter(|l| !l.is_empty()) else {
        return true;
    };
    let Ok(last) = DateTime::parse_from_rfc3339(last_raw) else {
        return true;
    };
    let last = last.with_timezone(&Local);
    match backup.interval.as_str() {
        "daily" => local_date_key(now) != local_date_key(last),
        "weekly" => (now - last) >= chrono::Duration::days(7),
        _ => now.year() != last.year() || now.month() != last.month(),
    }
}

/// `shouldCopy`: false when any component of the path below the root is in
/// [`SKIP_NAMES`].
pub fn should_copy(source_root: &Path, src: &Path) -> bool {
    let Ok(rel) = src.strip_prefix(source_root) else {
        return true;
    };
    !rel.components().any(|c| SKIP_NAMES.contains(&c.as_os_str().to_string_lossy().as_ref()))
}

/// `assertSafeDestination`: the backup can never land inside the folder it
/// copies (it would recurse into itself).
pub fn assert_safe_destination(source_root: &str, destination: &str) -> Result<()> {
    let source = resolve(source_root);
    let dest = resolve(destination);
    if dest == source || dest.starts_with(&format!("{source}/")) {
        return Err(anyhow!("Backup folder cannot be inside the data folder"));
    }
    Ok(())
}

fn copy_tree(root: &Path, src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        if !should_copy(root, &path) {
            continue;
        }
        let target = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            #[cfg(unix)]
            {
                let link = fs::read_link(&path)?;
                let _ = fs::remove_file(&target);
                std::os::unix::fs::symlink(link, &target)?;
            }
        } else if ft.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree(root, &path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Copies `source` (the data folder) to `destination`, skipping
/// [`SKIP_NAMES`].
pub fn copy_config(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Err(anyhow!("Data folder was not found"));
    }
    assert_safe_destination(&source.to_string_lossy(), &destination.to_string_lossy())?;
    fs::create_dir_all(destination)?;
    copy_tree(source, source, destination)
}

/// `runCollectionBackup`: copies `paths.root` (the data folder) to
/// `<folder>/heronite-YYYY-MM-DD` when `force` or due, and records the
/// outcome in `settings.json`.
pub fn run_collection_backup(paths: &DataPaths, force: bool, now: DateTime<Local>) -> CollectionBackupResult {
    let backup = match settings::load(paths) {
        Ok(s) => s.backup,
        Err(e) => return CollectionBackupResult { error: Some(format!("{e:#}")), ..Default::default() },
    };
    let folder = backup.folder.trim().to_string();
    let skipped = |why: &str| CollectionBackupResult { skipped_reason: Some(why.into()), ..Default::default() };
    if folder.is_empty() {
        return skipped("disabled");
    }
    if !force && !is_backup_due(&backup, now) {
        return skipped("not-due");
    }
    let Some(_guard) = Running::acquire() else {
        return skipped("already-running");
    };

    let at = iso_millis(now);
    let destination = join(&folder, &format!("heronite-{}", local_date_key(now)));
    let outcome = (|| -> Result<()> {
        // Checked first (Heronite removes the old copy before checking).
        assert_safe_destination(&paths.root.to_string_lossy(), &folder)?;
        if Path::new(&folder).exists() && !Path::new(&folder).is_dir() {
            return Err(anyhow!("Backup path is not a folder"));
        }
        if Path::new(&destination).exists() {
            fs::remove_dir_all(&destination)?;
        }
        fs::create_dir_all(&folder)?;
        copy_config(&paths.root, Path::new(&destination))
    })();
    match outcome {
        Ok(()) => {
            let _ = record_collection_backup(
                paths,
                &BackupRecord { at, path: Some(destination.clone()), error: None },
            );
            log::info!("Collection backup saved to {destination}");
            CollectionBackupResult { ran: true, path: Some(destination), ..Default::default() }
        }
        Err(e) => {
            let message = format!("{e:#}");
            let _ = record_collection_backup(
                paths,
                &BackupRecord { at, path: None, error: Some(message.clone()) },
            );
            log::error!("Collection backup failed: {message}");
            CollectionBackupResult { error: Some(message), ..Default::default() }
        }
    }
}

/// `maybeRunScheduledBackup`.
pub fn maybe_run_scheduled_backup(paths: &DataPaths) -> CollectionBackupResult {
    let r = run_collection_backup(paths, false, Local::now());
    if let Some(e) = &r.error {
        log::error!("Scheduled Collection backup failed: {e}");
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn settings_with(folder: &str, interval: &str, last: Option<DateTime<Local>>) -> BackupSettings {
        BackupSettings {
            folder: folder.into(),
            interval: interval.into(),
            last_backup_at: last.map(iso_millis),
            ..Default::default()
        }
    }

    #[test]
    fn due_rules() {
        let now = at(2026, 10, 7, 12);
        assert!(!is_backup_due(&settings_with("", "daily", None), now));
        assert!(!is_backup_due(&settings_with("  ", "daily", None), now));
        assert!(is_backup_due(&settings_with("/b", "weekly", None), now));
        // Daily: another calendar day, even a few hours later.
        assert!(is_backup_due(&settings_with("/b", "daily", Some(at(2026, 10, 6, 23))), at(2026, 10, 7, 1)));
        assert!(!is_backup_due(&settings_with("/b", "daily", Some(at(2026, 10, 7, 1))), at(2026, 10, 7, 23)));
        // Weekly: 7 x 24h.
        assert!(!is_backup_due(&settings_with("/b", "weekly", Some(at(2026, 10, 1, 13))), now));
        assert!(is_backup_due(&settings_with("/b", "weekly", Some(at(2026, 9, 30, 12))), now));
        // Monthly: calendar month.
        assert!(!is_backup_due(&settings_with("/b", "monthly", Some(at(2026, 10, 1, 0))), at(2026, 10, 31, 23)));
        assert!(is_backup_due(&settings_with("/b", "monthly", Some(at(2026, 9, 30, 23))), at(2026, 10, 1, 0)));
        // Garbage timestamp counts as due.
        let mut b = settings_with("/b", "daily", None);
        b.last_backup_at = Some("nope".into());
        assert!(is_backup_due(&b, now));
    }

    #[test]
    fn skip_rules() {
        let root = Path::new("/h");
        assert!(should_copy(root, Path::new("/h")));
        assert!(should_copy(root, Path::new("/h/store/config.json")));
        assert!(!should_copy(root, Path::new("/h/tools/proton")));
        assert!(!should_copy(root, Path::new("/h/store_cache/x.json")));
        assert!(!should_copy(root, Path::new("/h/local_library/screenshot-thumbnails/a.jpg")));
        assert!(!should_copy(root, Path::new("/h/logs/x.log")));
        assert!(should_copy(root, Path::new("/h/local_library/art/logs-ok.png")));
        // Only components below the root count.
        assert!(should_copy(Path::new("/logs/h"), Path::new("/logs/h/a")));
    }

    #[test]
    fn destination_safety() {
        assert!(assert_safe_destination("/h", "/h").is_err());
        assert!(assert_safe_destination("/h", "/h/sub/").is_err());
        assert!(assert_safe_destination("/h", "/h2").is_ok());
        assert!(assert_safe_destination("/h", "/other").is_ok());
    }

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fake_data_paths(root: &Path) {
        for (p, c) in [
            ("store/config.json", "{}"),
            ("local_library/library.json", "{\"games\":{}}"),
            ("local_library/screenshot-thumbnails/t.jpg", "x"),
            ("store_cache/legendary_library.json", "{}"),
            ("tools/wine/bin", "x"),
            ("logs/a.log", "x"),
            ("GamesConfig/g.json", "{}"),
        ] {
            let f = root.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, c).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink("store/config.json", root.join("link")).unwrap();
    }

    #[test]
    fn copies_filtered_tree_and_records_status() {
        let _s = serial();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let dest_root = dir.path().join("backups");
        fake_data_paths(&data_dir);
        let paths = DataPaths::at(&data_dir);
        settings::save_backup(&paths, dest_root.to_str().unwrap(), "daily").unwrap();
        let now = at(2026, 10, 7, 12);

        let r = run_collection_backup(&paths, false, now);
        assert!(r.ran, "{r:?}");
        let expected = dest_root.join("heronite-2026-10-07");
        assert_eq!(r.path.as_deref(), expected.to_str());
        assert!(expected.join("store/config.json").exists());
        assert!(expected.join("local_library/library.json").exists());
        assert!(expected.join("GamesConfig/g.json").exists());
        assert!(!expected.join("logs").exists());
        assert!(!expected.join("tools").exists());
        assert!(!expected.join("store_cache").exists());
        assert!(!expected.join("local_library/screenshot-thumbnails").exists());
        #[cfg(unix)]
        assert_eq!(fs::read_link(expected.join("link")).unwrap().to_str(), Some("store/config.json"));

        let b = settings::load(&paths).unwrap().backup;
        assert_eq!(b.last_backup_path.as_deref(), expected.to_str());
        assert!(b.last_backup_at.is_some());
        assert_eq!(b.last_error, None);

        // Not due again the same day; forced reruns replace the copy.
        let again = run_collection_backup(&paths, false, now);
        assert_eq!(again.skipped_reason.as_deref(), Some("not-due"));
        fs::write(expected.join("stale"), "x").unwrap();
        let forced = run_collection_backup(&paths, true, now);
        assert!(forced.ran);
        assert!(!expected.join("stale").exists());
    }

    #[test]
    fn errors_are_recorded_and_unsafe_folders_refused() {
        let _s = serial();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fake_data_paths(&data_dir);
        let paths = DataPaths::at(&data_dir);
        // Disabled.
        assert_eq!(run_collection_backup(&paths, true, Local::now()).skipped_reason.as_deref(), Some("disabled"));
        // Folder inside the data folder.
        settings::save_backup(&paths, data_dir.join("inside").to_str().unwrap(), "daily").unwrap();
        let r = run_collection_backup(&paths, true, Local::now());
        assert_eq!(r.error.as_deref(), Some("Backup folder cannot be inside the data folder"));
        assert!(!data_dir.join("inside").exists());
        let b = settings::load(&paths).unwrap().backup;
        assert_eq!(b.last_error.as_deref(), r.error.as_deref());
        // A file instead of a folder.
        let file = dir.path().join("afile");
        fs::write(&file, "x").unwrap();
        settings::save_backup(&paths, file.to_str().unwrap(), "daily").unwrap();
        let r = run_collection_backup(&paths, true, Local::now());
        assert_eq!(r.error.as_deref(), Some("Backup path is not a folder"));
        // Missing data folder.
        let err = copy_config(&dir.path().join("nothing"), &dir.path().join("out")).unwrap_err();
        assert_eq!(err.to_string(), "Data folder was not found");
    }
}
