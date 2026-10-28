//! The schedule and "Back up now": the folder checks, the backup, the
//! retention and the status fields of `settings.json` (`backup`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use chrono::{DateTime, Datelike, Local, SecondsFormat, Utc};
use heronite_data::models::{BackupSettings, CollectionSettingsFile};
use heronite_data::settings::{self, DEFAULT_BACKUP_KEEP};
use heronite_data::{json_store, AppPaths, DataPaths};

use crate::catalog::Part;
use crate::manifest::{Reason, Summary};
use crate::snapshot;
use crate::Error;

/// What [`run`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub ran: bool,
    pub skipped: Option<Skipped>,
    pub path: Option<PathBuf>,
    pub summary: Option<Summary>,
    /// Old backups the retention removed.
    pub pruned: usize,
    /// Already in the user's language.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// No folder set.
    Disabled,
    NotDue,
    AlreadyRunning,
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Held while a backup or a restore reads or writes a backup folder.
pub(crate) struct Lock;

impl Lock {
    pub(crate) fn acquire() -> Option<Self> {
        RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| Lock)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Whether a backup is being made now.
pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

fn local_date_key(date: DateTime<Local>) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Daily = another calendar day, weekly = 7 x 24 h, monthly = another
/// calendar month; never when no folder is set or the interval is `never`.
pub fn is_due(backup: &BackupSettings, now: DateTime<Local>) -> bool {
    if backup.folder.trim().is_empty() || backup.interval == "never" {
        return false;
    }
    let Some(last) = backup.last_backup_at.as_deref().and_then(|l| DateTime::parse_from_rfc3339(l).ok()) else {
        return true;
    };
    let last = last.with_timezone(&Local);
    match backup.interval.as_str() {
        "daily" => local_date_key(now) != local_date_key(last),
        "weekly" => (now - last) >= chrono::Duration::days(7),
        _ => now.year() != last.year() || now.month() != last.month(),
    }
}

/// The backup folder, checked. A folder on a drive that is not mounted
/// would otherwise be made on the system disk and filled there: scheduled
/// backups never create it, and `create_last` only makes its last level.
pub fn ready_folder(folder: &str, create_last: bool) -> Result<PathBuf, Error> {
    let path = PathBuf::from(folder.trim());
    if path.exists() && !path.is_dir() {
        return Err(Error::NotAFolder);
    }
    if !path.is_dir() {
        if !create_last || !path.parent().is_some_and(Path::is_dir) {
            return Err(Error::FolderMissing);
        }
        std::fs::create_dir(&path).map_err(|_| Error::FolderMissing)?;
    }
    Ok(path)
}

/// Backs up into the configured folder when `force` or due, then removes
/// the backups beyond the configured number. Records the outcome.
pub fn run(data: &DataPaths, app: &AppPaths, force: bool, now: DateTime<Local>) -> Outcome {
    let backup = match settings::load(data) {
        Ok(s) => s.backup,
        Err(e) => return Outcome { error: Some(format!("{e:#}")), ..Default::default() },
    };
    let skipped = |why| Outcome { skipped: Some(why), ..Default::default() };
    if backup.folder.trim().is_empty() {
        return skipped(Skipped::Disabled);
    }
    if !force && !is_due(&backup, now) {
        return skipped(Skipped::NotDue);
    }
    let Some(_lock) = Lock::acquire() else {
        return skipped(Skipped::AlreadyRunning);
    };
    let reason = if force { Reason::Manual } else { Reason::Scheduled };
    let parts = Part::included(&backup.exclude);
    let made = ready_folder(&backup.folder, force)
        .map_err(anyhow::Error::from)
        .and_then(|folder| snapshot::create(data, app, &folder, &parts, reason, now).map(|c| (folder, c)));
    let at = now.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true);
    match made {
        Ok((folder, created)) => {
            let pruned = snapshot::prune(&folder, backup.keep.unwrap_or(DEFAULT_BACKUP_KEEP)).len();
            let path = created.path.to_string_lossy().into_owned();
            let _ = record(data, &at, Some(&path), None);
            log::info!(
                "backup saved to {path}: {} files, {} new bytes, {pruned} old backups removed",
                created.summary.files,
                created.summary.new_bytes
            );
            Outcome { ran: true, path: Some(created.path), summary: Some(created.summary), pruned, ..Default::default() }
        }
        Err(e) => {
            let english = format!("{e:#}");
            let _ = record(data, &at, None, Some(&english));
            log::error!("backup failed: {english}");
            Outcome { error: Some(crate::describe(&e)), ..Default::default() }
        }
    }
}

/// The status fields of `settings.json` `backup`: a success sets the time and
/// path, any run sets or clears the error.
fn record(data: &DataPaths, at: &str, path: Option<&str>, error: Option<&str>) -> Result<()> {
    json_store::update::<CollectionSettingsFile, _>(&data.collection_settings(), |f| {
        let mut next = f.backup.clone().unwrap_or_default();
        if error.is_none() {
            next.last_backup_at = Some(at.to_owned());
            next.last_backup_path = path.map(str::to_owned);
        }
        next.last_error = error.map(str::to_owned);
        f.backup = Some(next);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use heronite_data::settings::BackupPatch;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn with(folder: &str, interval: &str, last: Option<DateTime<Local>>) -> BackupSettings {
        BackupSettings {
            folder: folder.into(),
            interval: interval.into(),
            last_backup_at: last.map(|l| l.with_timezone(&Utc).to_rfc3339()),
            ..Default::default()
        }
    }

    #[test]
    fn due_rules() {
        let now = at(2026, 10, 7, 12);
        assert!(!is_due(&with("", "daily", None), now));
        assert!(!is_due(&with("/b", "never", None), now));
        assert!(is_due(&with("/b", "weekly", None), now));
        assert!(is_due(&with("/b", "daily", Some(at(2026, 10, 6, 23))), at(2026, 10, 7, 1)));
        assert!(!is_due(&with("/b", "daily", Some(at(2026, 10, 7, 1))), at(2026, 10, 7, 23)));
        assert!(!is_due(&with("/b", "weekly", Some(at(2026, 10, 1, 13))), now));
        assert!(is_due(&with("/b", "weekly", Some(at(2026, 9, 30, 12))), now));
        assert!(!is_due(&with("/b", "monthly", Some(at(2026, 10, 1, 0))), at(2026, 10, 31, 23)));
        assert!(is_due(&with("/b", "monthly", Some(at(2026, 9, 30, 23))), at(2026, 10, 1, 0)));
        let mut b = with("/b", "daily", None);
        b.last_backup_at = Some("nope".into());
        assert!(is_due(&b, now));
    }

    #[test]
    fn runs_records_and_keeps_the_newest() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = crate::snapshot::tests::fixture(d.path());
        let folder = d.path().join("backups");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(run(&data, &app, true, Local::now()).skipped, Some(Skipped::Disabled));
        let patch = BackupPatch {
            folder: Some(folder.to_string_lossy().into()),
            interval: Some("daily".into()),
            keep: Some(2),
            exclude: Some(vec!["art".into(), "logins".into()]),
        };
        settings::save_backup_settings(&data, patch).unwrap();

        let first = run(&data, &app, false, at(2026, 10, 7, 12));
        assert!(first.ran, "{first:?}");
        let path = first.path.unwrap();
        assert!(!path.join("data/local_library/art").exists());
        assert!(!path.join("data/legendaryConfig/legendary/user.json").exists());
        assert!(path.join("config/ui-state.json").exists());
        let b = settings::load(&data).unwrap().backup;
        assert_eq!(b.last_backup_path.as_deref(), path.to_str());
        assert_eq!(b.last_error, None);

        assert_eq!(run(&data, &app, false, at(2026, 10, 7, 13)).skipped, Some(Skipped::NotDue));
        assert!(run(&data, &app, false, at(2026, 10, 8, 12)).ran);
        let third = run(&data, &app, true, at(2026, 10, 8, 13));
        assert_eq!(third.pruned, 1);
        assert_eq!(crate::snapshot::list(&folder).len(), 2);
    }

    #[test]
    fn a_folder_on_a_missing_drive_is_never_created() {
        let _s = crate::tests::serial();
        let d = tempfile::tempdir().unwrap();
        let (data, app) = crate::snapshot::tests::fixture(d.path());
        let unplugged = d.path().join("media/usb/backups");
        settings::save_backup(&data, unplugged.to_str().unwrap(), "daily").unwrap();
        let r = run(&data, &app, true, Local::now());
        assert_eq!(r.error.as_deref(), Some("Backup folder was not found. Is its drive connected?"));
        assert!(!d.path().join("media").exists());
        assert_eq!(settings::load(&data).unwrap().backup.last_error.as_deref(), Some("Backup folder was not found"));
        // "Back up now" makes a new folder in an existing one; the schedule does not.
        let new = d.path().join("backups");
        settings::save_backup(&data, new.to_str().unwrap(), "daily").unwrap();
        assert!(run(&data, &app, false, Local::now()).error.is_some());
        assert!(!new.exists());
        assert!(run(&data, &app, true, Local::now()).ran);
        // A file instead of a folder.
        let file = d.path().join("afile");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(ready_folder(file.to_str().unwrap(), true), Err(Error::NotAFolder));
    }
}
