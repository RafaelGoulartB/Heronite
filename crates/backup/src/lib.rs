//! Backups of this app's data (Settings → Backups).
//!
//! * [`catalog`]: what each file is: a [`Part`] a backup holds (Collection,
//!   settings, art, sign-ins) or something left out because it is downloaded
//!   or rebuilt again (Wine builds, image caches, logs).
//! * [`snapshot`]: incremental backups (hard links to the previous one),
//!   their list and the retention.
//! * [`run`]: the schedule and "Back up now", recorded in `settings.json`.
//! * [`restore`]: putting a backup back. The files are checked first, what is
//!   there now is backed up, and the copy happens at the next start, before
//!   anything reads the data.
//! * [`usage`]: the size of each part, for the settings page.
//!
//! Game saves are not here: Ludusavi backs them up (Settings → Saves).
//! All functions block; the app runs them off the UI thread.

pub mod catalog;
pub mod manifest;
pub mod restore;
pub mod run;
pub mod snapshot;
pub mod usage;

pub use catalog::{Left, Part, Side};
pub use manifest::{Manifest, Reason, Summary};
pub use snapshot::Info;

use heronite_i18n::tr;

/// Failures the settings page explains in words of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InsideData,
    NoData,
    NotAFolder,
    FolderMissing,
    /// A backup file is missing or not what the manifest says.
    Damaged(String),
    /// Made by a newer version of the app.
    TooNew,
}

impl std::fmt::Display for Error {
    /// English, as stored in `settings.json`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InsideData => write!(f, "Backup folder cannot be inside the data folder"),
            Error::NoData => write!(f, "Data folder was not found"),
            Error::NotAFolder => write!(f, "Backup path is not a folder"),
            Error::FolderMissing => write!(f, "Backup folder was not found"),
            Error::Damaged(file) => write!(f, "This backup is damaged ({file})"),
            Error::TooNew => write!(f, "This backup was made by a newer version of Heronite"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn localized(&self) -> String {
        match self {
            Error::InsideData => tr!("Backup folder cannot be inside the data folder").into(),
            Error::NoData => tr!("Data folder was not found").into(),
            Error::NotAFolder => tr!("Backup path is not a folder").into(),
            Error::FolderMissing => tr!("Backup folder was not found. Is its drive connected?").into(),
            Error::Damaged(file) => tr!("This backup is damaged ({file})", file),
            Error::TooNew => tr!("This backup was made by a newer version of Heronite").into(),
        }
    }
}

/// The text to show for an error of this crate.
pub fn describe(e: &anyhow::Error) -> String {
    match e.downcast_ref::<Error>() {
        Some(own) => own.localized(),
        None => format!("{e:#}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    /// Backups and restores share one process-wide lock: tests that take it run one at a time.
    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }
}
