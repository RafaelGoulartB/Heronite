//! Epic Games Store through the `legendary` CLI (port of Heroic's
//! `backend/storeManagers/legendary/**` and the download queue).
//!
//! STUB: the public items below are the contract other code builds against;
//! the implementation is in progress.

pub mod saves {
    use heronite_data::HeroicPaths;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SyncMode {
        /// Upload and download as needed (`legendary sync-saves`).
        Both,
        ForceDownload,
        ForceUpload,
    }

    /// Runs `legendary sync-saves` for `app_name` with `saves_path`; returns
    /// legendary's output summary. Blocking: call off the UI thread.
    pub fn sync(_heroic: &HeroicPaths, _app_name: &str, _saves_path: &str, _mode: SyncMode) -> anyhow::Result<String> {
        anyhow::bail!("not implemented yet")
    }

    /// Default save location legendary computes for the game in its Wine
    /// prefix (Heroic's "Cloud saves" tab auto-detection), if any.
    pub fn detect_path(_heroic: &HeroicPaths, _app_name: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}
