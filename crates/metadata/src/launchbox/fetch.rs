//! The files of the LaunchBox source under the data folder's cache area
//! (`store_cache/sources/launchbox/`) and the download of the archive.
//!
//! - `index.sqlite`: the index in use; `state.json`: when it was
//!   downloaded, the archive's ETag (a refresh asks for the archive only if
//!   it changed), how many games it has, and the refreshes that failed
//!   since the last one that worked.
//! - `Metadata.zip.part` (+ `.json`, its ETag): the archive while it comes.
//!   An interrupted download resumes from where it stopped; a whole one
//!   whose index could not be built is kept for the next try (asked for
//!   again only from its end: the server answers 416 while it is current).
//!   It is removed once the index is built, or when it turns out not to be
//!   one that reads ([`super::build::BadArchive`]).
//! - `index.sqlite.part`: an index being built, renamed over `index.sqlite`
//!   only once it is complete and opens. A failed or cancelled refresh
//!   leaves the old index as it was.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::http::{Fetched, Http};
use crate::sources::download::{Cancelled, DownloadProgress, DownloadState, Phase};

pub(super) struct Files {
    dir: PathBuf,
}

impl Files {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn index(&self) -> PathBuf {
        self.dir.join("index.sqlite")
    }

    pub fn index_part(&self) -> PathBuf {
        self.dir.join("index.sqlite.part")
    }

    fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    pub fn archive_part(&self) -> PathBuf {
        self.dir.join("Metadata.zip.part")
    }

    fn archive_part_meta(&self) -> PathBuf {
        self.dir.join("Metadata.zip.part.json")
    }

    /// What `state.json` says; `None` without one.
    pub fn stored(&self) -> Option<Stored> {
        serde_json::from_slice(&std::fs::read(self.state()).ok()?).ok()
    }

    pub fn store(&self, stored: &Stored) -> Result<()> {
        Ok(heronite_data::temps::write_atomic(&self.state(), &serde_json::to_vec_pretty(stored)?, None)?)
    }

    /// Where the download stands: ready once an index is in place (whether
    /// it opens is the caller's to check).
    pub fn download_state(&self) -> DownloadState {
        let Ok(meta) = std::fs::metadata(self.index()) else { return DownloadState::default() };
        let stored = self.stored();
        let refreshed_at = stored.as_ref().map(|s| UNIX_EPOCH + Duration::from_secs(s.refreshed_at)).or_else(|| meta.modified().ok());
        DownloadState { ready: true, refreshed_at, size_on_disk: meta.len(), retry_at: stored.and_then(|s| s.retry_at()) }
    }

    /// Records a refresh that failed: the next automatic one waits
    /// ([`Stored::retry_at`]).
    pub fn note_failure(&self) {
        let mut stored = self.stored().unwrap_or_default();
        stored.failures += 1;
        stored.failed_at = Some(now_secs());
        if let Err(e) = std::fs::create_dir_all(self.dir()).map_err(anyhow::Error::from).and_then(|()| self.store(&stored)) {
            log::warn!("LaunchBox: cannot record the failed refresh: {e:#}");
        }
    }

    /// Everything but the folder.
    pub fn remove_all(&self) -> Result<()> {
        for path in [self.index(), self.index_part(), self.state(), self.archive_part(), self.archive_part_meta()] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

/// `state.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Stored {
    /// Seconds since 1970.
    pub refreshed_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub games: u64,
    /// Refreshes that failed in a row since the last one that worked.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub failures: u32,
    /// When the last of them failed, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<u64>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Stored {
    /// When the idle maintenance may try again after [`Self::failures`]
    /// failed refreshes: a day after the first, then two, four, and a week
    /// after each from the fourth on. `None` without failures.
    pub fn retry_at(&self) -> Option<SystemTime> {
        let at = self.failed_at.filter(|_| self.failures > 0)?;
        let days = (1u64 << (self.failures - 1).min(3)).min(7);
        Some(UNIX_EPOCH + Duration::from_secs(at + days * 86_400))
    }
}

pub(super) fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The ETag of the archive being downloaded.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PartMeta {
    #[serde(default)]
    etag: Option<String>,
}

/// What [`fetch_archive`] got.
pub(super) enum Archive {
    /// The archive is at [`Files::archive_part`], whole.
    Whole { etag: Option<String>, last_modified: Option<String> },
    /// The copy the index was built from is still current.
    Unchanged,
}

/// Downloads the archive at `url` into [`Files::archive_part`], resuming a
/// part left by an interrupted download. `current_etag`: the ETag of the
/// archive the index in use came from (asked only when it changed).
pub(super) fn fetch_archive(
    files: &Files,
    http: &Http,
    url: &str,
    current_etag: Option<&str>,
    progress: &DownloadProgress,
) -> Result<Archive> {
    std::fs::create_dir_all(files.dir())?;
    progress.set_phase(Phase::Downloading);
    let part = files.archive_part();
    let part_etag: Option<String> = std::fs::read(files.archive_part_meta())
        .ok()
        .and_then(|b| serde_json::from_slice::<PartMeta>(&b).ok())
        .and_then(|m| m.etag)
        .filter(|_| part.exists());
    // Resume only a part whose archive is known; without one, ask whether the index's copy changed.
    let resume = part_etag.as_deref();
    let not_modified_if = if resume.is_none() { current_etag } else { None };
    // The part's ETag is written as soon as the answer starts, so a
    // download cut short resumes only while the archive is the same.
    let mut remember = |etag: Option<&str>| {
        let meta = PartMeta { etag: etag.map(str::to_owned) };
        if let Ok(bytes) = serde_json::to_vec(&meta) {
            let _ = heronite_data::temps::write_atomic(&files.archive_part_meta(), &bytes, None);
        }
    };
    let fetched = http.download_to(url, &part, resume, not_modified_if, &mut remember, &mut |done, total| {
        progress.set(done, total);
        !progress.cancelled()
    })?;
    match fetched {
        Fetched::NotModified => Ok(Archive::Unchanged),
        Fetched::Stopped => Err(Cancelled.into()),
        Fetched::Complete { etag, last_modified } => {
            if std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) == 0 {
                bail!("the LaunchBox archive came empty");
            }
            Ok(Archive::Whole { etag, last_modified })
        }
    }
}

/// Drops the archive (whole and used, or not a zip).
pub(super) fn drop_archive(files: &Files) {
    let _ = std::fs::remove_file(files.archive_part());
    let _ = std::fs::remove_file(files.archive_part_meta());
}
