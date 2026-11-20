//! A source that is asked from a database it downloads first
//! ([`Needs::Download`](super::Needs::Download)): what its descriptor says
//! about the download ([`SourceDownload`]), where it stands
//! ([`DownloadState`]) and the progress of one running
//! ([`DownloadProgress`]). Settings draws the same row for any such source:
//! "Download the database (about 105 MB)", the progress, then when it was
//! refreshed, "Refresh now" and "Remove the database".
//!
//! Nothing is downloaded until the user asks (the button), and then again
//! only by "Refresh now" or the idle maintenance, while the source is on.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// What a descriptor says about its download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDownload {
    /// About how much is downloaded, in MB: shown on the button before
    /// anything is asked of the source's server.
    pub size_mb: u32,
    /// Days after which the idle maintenance refreshes it while the source
    /// is on (0: only by hand).
    #[serde(default)]
    pub refresh_days: u32,
}

impl SourceDownload {
    /// Whether a copy refreshed at `refreshed` is due again at `now`.
    pub fn due(&self, refreshed: Option<SystemTime>, now: SystemTime) -> bool {
        if self.refresh_days == 0 {
            return false;
        }
        let every = Duration::from_secs(u64::from(self.refresh_days) * 86_400);
        refreshed.is_none_or(|at| now.duration_since(at).is_ok_and(|age| age >= every))
    }
}

/// Where a source's download stands on this PC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadState {
    /// Downloaded and usable: the source can be asked.
    pub ready: bool,
    /// When the copy in use was last downloaded or found unchanged.
    pub refreshed_at: Option<SystemTime>,
    /// Bytes it takes on disk.
    pub size_on_disk: u64,
    /// Set after a refresh failed, or when the copy no longer opens: the
    /// idle maintenance tries again from then on (whatever the refresh
    /// days say), not before. `None`: on the usual schedule.
    pub retry_at: Option<SystemTime>,
}

impl DownloadState {
    /// Whether the idle maintenance refreshes a copy in this state at `now`.
    pub fn refresh_due(&self, download: &SourceDownload, now: SystemTime) -> bool {
        match self.retry_at {
            Some(at) => download.refresh_days > 0 && now >= at,
            None => self.ready && download.due(self.refreshed_at, now),
        }
    }
}

/// The step a download is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Phase {
    #[default]
    Starting,
    Downloading,
    /// Turning what came into what the source reads (an index).
    Preparing,
}

/// The progress of a running download, shared between the thread that
/// runs it and whoever shows it. Cancelling is asked here; the download
/// stops at its next step.
#[derive(Debug, Default)]
pub struct DownloadProgress {
    phase: AtomicU8,
    done: AtomicU64,
    /// 0: not known yet.
    total: AtomicU64,
    cancel: AtomicBool,
}

/// A copy of [`DownloadProgress`] to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressSnapshot {
    pub phase: Phase,
    /// Bytes downloaded, or entries prepared.
    pub done: u64,
    pub total: Option<u64>,
}

impl ProgressSnapshot {
    /// 0.0..=1.0 when the total is known.
    pub fn fraction(&self) -> Option<f32> {
        self.total.filter(|t| *t > 0).map(|t| (self.done as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }
}

impl DownloadProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    pub fn set(&self, done: u64, total: Option<u64>) {
        self.done.store(done, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let phase = match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Downloading,
            2 => Phase::Preparing,
            _ => Phase::Starting,
        };
        let total = self.total.load(Ordering::Relaxed);
        ProgressSnapshot { phase, done: self.done.load(Ordering::Relaxed), total: (total > 0).then_some(total) }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A download stopped because the user cancelled it: nothing changed.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the download was cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Whether `err` is [`Cancelled`].
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Cancelled>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_is_due_after_its_days() {
        let d = SourceDownload { size_mb: 105, refresh_days: 7 };
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
        let days_ago = |n: u64| Some(now - Duration::from_secs(n * 86_400));
        assert!(!d.due(days_ago(6), now));
        assert!(d.due(days_ago(7), now));
        assert!(d.due(None, now), "never refreshed");
        assert!(!d.due(Some(now + Duration::from_secs(60)), now), "a clock that went back does not refresh");
        assert!(!SourceDownload { refresh_days: 0, ..d }.due(None, now));
    }

    #[test]
    fn a_failed_or_broken_copy_is_tried_again_from_its_retry_time() {
        let d = SourceDownload { size_mb: 105, refresh_days: 7 };
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
        let fresh = DownloadState { ready: true, refreshed_at: Some(now - Duration::from_secs(86_400)), ..Default::default() };
        assert!(!fresh.refresh_due(&d, now));
        let failed = DownloadState { retry_at: Some(now + Duration::from_secs(60)), ..fresh.clone() };
        assert!(!failed.refresh_due(&d, now), "waits for its retry time");
        assert!(failed.refresh_due(&d, now + Duration::from_secs(60)));
        let broken = DownloadState { ready: false, retry_at: Some(SystemTime::UNIX_EPOCH), ..fresh };
        assert!(broken.refresh_due(&d, now), "a copy that no longer opens is downloaded again");
        assert!(!broken.refresh_due(&SourceDownload { refresh_days: 0, ..d }, now), "only by hand");
        assert!(!DownloadState::default().refresh_due(&d, now), "never downloaded");
    }

    #[test]
    fn progress() {
        let p = DownloadProgress::new();
        assert_eq!(p.snapshot(), ProgressSnapshot::default());
        p.set_phase(Phase::Downloading);
        p.set(25, Some(100));
        assert_eq!(p.snapshot().fraction(), Some(0.25));
        p.set_phase(Phase::Preparing);
        assert_eq!(p.snapshot(), ProgressSnapshot { phase: Phase::Preparing, done: 0, total: None });
        assert!(!p.cancelled());
        p.cancel();
        assert!(p.cancelled());
    }
}
