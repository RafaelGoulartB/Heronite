//! The databases metadata sources download (`Needs::Download`: the
//! LaunchBox Games Database): the hub starts, cancels and removes them off
//! the UI thread and follows a running one, so Settings → Metadata draws
//! its progress whatever source it is and whether the page was open when
//! it started. The idle maintenance's weekly refresh shows the same way.
//!
//! Debug: `HERONITE_DEBUG_SOURCE_DOWNLOAD=<source id>:<state>` shows a
//! state without the network: `none` (not downloaded), `downloading[:<percent>]`,
//! `preparing[:<percent>]`, `ready`, `failed`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use gpui_kit::*;
use heronite_metadata::sources::{DownloadState, Phase, ProgressSnapshot};

use super::hub::{HubEvent, MetadataHub};

/// How often a running download's progress is read.
const POLL: Duration = Duration::from_millis(250);

/// What Settings shows of one source's download.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceDownloadUi {
    /// Running: its progress.
    pub progress: Option<ProgressSnapshot>,
    /// The last attempt failed (not cancelled): why.
    pub error: Option<String>,
}

/// The state `HERONITE_DEBUG_SOURCE_DOWNLOAD` asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct FakeDownload {
    pub source: String,
    /// `Some`: the database is (or is not) there, whatever is on disk.
    pub state: Option<DownloadState>,
    pub ui: SourceDownloadUi,
}

impl FakeDownload {
    pub fn from_env() -> Option<Self> {
        let raw = crate::env::value("HERONITE_DEBUG_SOURCE_DOWNLOAD")?;
        let mut parts = raw.split(':');
        let source = parts.next()?.trim().to_string();
        let kind = parts.next().unwrap_or("none");
        let percent: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(42);
        let missing = Some(DownloadState::default());
        let snapshot = |phase, total: u64| ProgressSnapshot { phase, done: total * percent / 100, total: Some(total) };
        let (state, ui) = match kind {
            "downloading" => (missing, SourceDownloadUi { progress: Some(snapshot(Phase::Downloading, 109_521_005)), error: None }),
            "preparing" => (missing, SourceDownloadUi { progress: Some(snapshot(Phase::Preparing, 580_584_091)), error: None }),
            "ready" => {
                let refreshed = SystemTime::now() - Duration::from_secs(2 * 86_400);
                (
                    Some(DownloadState { ready: true, refreshed_at: Some(refreshed), size_on_disk: 79_822_848, retry_at: None }),
                    SourceDownloadUi::default(),
                )
            }
            "failed" => (missing, SourceDownloadUi { progress: None, error: Some("Could not reach gamesdb.launchbox-app.com".into()) }),
            _ => (missing, SourceDownloadUi::default()),
        };
        Some(Self { source, state, ui })
    }
}

impl MetadataHub {
    /// The download of `id` as Settings shows it.
    pub fn source_download(&self, id: &str) -> SourceDownloadUi {
        match &self.fake_download {
            Some(f) if f.source == id => f.ui.clone(),
            _ => self.source_downloads.get(id).cloned().unwrap_or_default(),
        }
    }

    /// Whether the database of `id` is downloading.
    pub fn source_downloading(&self, id: &str) -> bool {
        self.source_download(id).progress.is_some()
    }

    /// The made-up download state of `id`, if `HERONITE_DEBUG_SOURCE_DOWNLOAD` names it.
    pub fn fake_download_state(&self, id: &str) -> Option<DownloadState> {
        self.fake_download.as_ref().filter(|f| f.source == id).and_then(|f| f.state.clone())
    }

    /// Downloads (or refreshes) the database of source `id` off the UI
    /// thread; the user asked for it.
    pub fn download_source(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.fake_download.is_some() || self.source_downloading(id) {
            return;
        }
        let service = self.service.clone();
        let source = id.to_string();
        self.source_downloads.insert(source.clone(), SourceDownloadUi { progress: Some(ProgressSnapshot::default()), error: None });
        cx.spawn(async move |this, cx| {
            let id = source.clone();
            let done = cx.background_spawn(async move { service.download_source(&id) }).await;
            this.update(cx, |this, cx| {
                let error = match done {
                    Ok(()) => None,
                    Err(e) if heronite_metadata::sources::download::is_cancelled(&e) => None,
                    Err(e) => {
                        log::warn!("downloading the database of {source}: {e:#}");
                        Some(format!("{e:#}"))
                    }
                };
                this.source_downloads.insert(source.clone(), SourceDownloadUi { progress: None, error });
                cx.emit(HubEvent::SourceDownloaded);
                cx.notify();
            })
            .ok();
        })
        .detach();
        self.follow_downloads(cx);
        cx.notify();
    }

    /// Asks the download of `id` to stop; what was in use stays.
    pub fn cancel_source_download(&mut self, id: &str) {
        self.service.cancel_source_download(id);
    }

    /// Removes the database of `id` off the UI thread.
    pub fn remove_source_download(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.fake_download.is_some() || self.source_downloading(id) {
            return;
        }
        let service = self.service.clone();
        let source = id.to_string();
        cx.spawn(async move |this, cx| {
            let id = source.clone();
            let done = cx.background_spawn(async move { service.remove_source_download(&id) }).await;
            this.update(cx, |this, cx| {
                let error = done.err().map(|e| format!("{e:#}"));
                this.source_downloads.insert(source.clone(), SourceDownloadUi { progress: None, error });
                cx.emit(HubEvent::SourceDownloaded);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the progress of the running downloads (the user's and the idle
    /// refresh's) until none runs. One loop at a time.
    pub(super) fn follow_downloads(&mut self, cx: &mut Context<Self>) {
        if self.following_downloads {
            return;
        }
        self.following_downloads = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let alive = this.update(cx, |this, cx| {
                let running = this.service.downloading_sources();
                let mut changed = false;
                for id in &running {
                    let progress = this.service.source_download_progress(id);
                    let entry = this.source_downloads.entry(id.clone()).or_default();
                    if entry.progress != progress {
                        entry.progress = progress;
                        changed = true;
                    }
                }
                if changed {
                    cx.notify();
                }
                // Keep going while the service or a task started here still runs one.
                let busy = !running.is_empty() || this.source_downloads.values().any(|d| d.progress.is_some());
                if !busy {
                    this.following_downloads = false;
                }
                busy
            });
            if !matches!(alive, Ok(true)) {
                break;
            }
        })
        .detach();
    }
}

/// The downloads the hub follows, by source id.
pub type SourceDownloads = HashMap<String, SourceDownloadUi>;
