//! Bulk metadata jobs and the idle auto-fill.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::models::{CollectionGameMetadata, LocalMetaFile};

use super::MetadataService;
use crate::store::{as_runner, get_game_metadata};
use crate::types::*;
use crate::util::now_ms;

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Games filled per idle pass.
pub const AUTO_FILL_BATCH: usize = 40;

impl MetadataService {
    /// `getBulkMetadataProgress`.
    pub fn bulk_progress(&self) -> BulkProgress {
        self.bulk.lock().clone()
    }

    fn publish(&self, next: BulkProgress, on_progress: &dyn Fn(&BulkProgress)) {
        *self.bulk.lock() = next.clone();
        on_progress(&next);
    }

    /// `cancelBulkMetadata`: the job stops before the next game.
    pub fn cancel_bulk_metadata(&self, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        if self.bulk_running.load(Ordering::SeqCst) {
            self.bulk_cancel.store(true, Ordering::SeqCst);
            let mut p = self.bulk_progress();
            p.state = BulkState::Cancelling;
            self.publish(p, on_progress);
        }
        self.bulk_progress()
    }

    /// `startBulkMetadata`: runs the job on its own thread and returns the
    /// initial progress; `on_progress` is called after every step (and for
    /// `cancel_bulk_metadata`) from that thread. Ignored while a job runs.
    pub fn start_bulk_metadata(
        self: &Arc<Self>,
        args: BulkArgs,
        on_progress: impl Fn(&BulkProgress) + Send + Sync + 'static,
    ) -> BulkProgress {
        if self.bulk_running.swap(true, Ordering::SeqCst) {
            return self.bulk_progress();
        }
        self.bulk_cancel.store(false, Ordering::SeqCst);
        let initial = BulkProgress {
            state: BulkState::Running,
            total: args.games.len(),
            percent: if args.games.is_empty() { 100 } else { 0 },
            ..Default::default()
        };
        self.publish(initial.clone(), &on_progress);
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("metadata-bulk".into())
            .spawn(move || this.run_bulk(&args, &on_progress))
            .expect("spawn bulk thread");
        initial
    }

    /// Blocking variant of [`start_bulk_metadata`](Self::start_bulk_metadata)
    /// for callers that already have a worker thread (and for tests).
    pub fn run_bulk_metadata(&self, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        if self.bulk_running.swap(true, Ordering::SeqCst) {
            return self.bulk_progress();
        }
        self.bulk_cancel.store(false, Ordering::SeqCst);
        self.run_bulk(args, on_progress)
    }

    /// `runBulkMetadata` (expects `bulk_running` already set).
    fn run_bulk(&self, args: &BulkArgs, on_progress: &dyn Fn(&BulkProgress)) -> BulkProgress {
        struct Done<'a>(&'a MetadataService);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.bulk_running.store(false, Ordering::SeqCst);
                self.0.bulk_cancel.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(self);

        let total = args.games.len();
        let overwrite = args.overwrite_existing == Some(true);
        let download = args.download_images.unwrap_or(self.metadata_settings().download_images);
        let mut errors: Vec<String> = Vec::new();
        let (mut updated, mut skipped, mut failed) = (0usize, 0usize, 0usize);

        if total == 0 {
            let done = BulkProgress { state: BulkState::Done, percent: 100, ..Default::default() };
            self.publish(done.clone(), on_progress);
            return done;
        }

        for (index, game) in args.games.iter().enumerate() {
            if self.bulk_cancel.load(Ordering::SeqCst) {
                break;
            }
            self.publish(
                BulkProgress {
                    state: BulkState::Running,
                    total,
                    processed: index,
                    updated,
                    skipped,
                    failed,
                    percent: (index as f64 / total as f64 * 100.0).round() as u32,
                    current_title: Some(game.title.clone()),
                    errors: errors.iter().take(8).cloned().collect(),
                },
                on_progress,
            );
            let step = || -> Result<Option<CollectionGameMetadata>> {
                let current = get_game_metadata(&self.paths, as_runner(&game.runner), &game.app_name)?;
                if !overwrite
                    && current.as_ref().is_some_and(|c| c.fetched_at.as_deref().is_some_and(|f| !f.is_empty()) && c.fetch_error.is_none())
                {
                    return Ok(None);
                }
                self.apply_game_metadata(&MetadataApplyArgs {
                    app_name: game.app_name.clone(),
                    runner: game.runner.clone(),
                    title: game.title.clone(),
                    steam_app_id: game.steam_app_id.clone(),
                    source: Some("auto".into()),
                    overwrite_existing: Some(overwrite),
                    preserve_manual: Some(args.preserve_manual != Some(false)),
                    download_images: Some(download),
                    ..Default::default()
                })
                .map(Some)
            };
            match step() {
                Ok(None) => skipped += 1,
                Ok(Some(saved)) => match saved.fetch_error {
                    Some(err) => {
                        failed += 1;
                        errors.push(format!("{}: {err}", game.title));
                    }
                    None => updated += 1,
                },
                Err(e) => {
                    failed += 1;
                    errors.push(format!("{}: {e}", game.title));
                }
            }
        }

        let processed = updated + skipped + failed;
        let done = BulkProgress {
            state: BulkState::Done,
            total,
            processed,
            updated,
            skipped,
            failed,
            percent: (processed as f64 / total as f64 * 100.0).round() as u32,
            current_title: None,
            errors: errors.into_iter().take(8).collect(),
        };
        self.publish(done.clone(), on_progress);
        done
    }

    /// `refreshMissingCollectionMetadata`: when `autoFillMissing` is on,
    /// fetches metadata for up to 40 overlay games that have none (or whose
    /// last attempt failed more than a week ago) and returns how many got
    /// data. Call it when the app is idle.
    pub fn refresh_missing_collection_metadata(&self) -> Result<usize> {
        self.refresh_missing_while(&|| false)
    }

    /// Same, checking `should_stop` before each game.
    pub fn refresh_missing_while(&self, should_stop: &dyn Fn() -> bool) -> Result<usize> {
        if self.filling.swap(true, Ordering::SeqCst) {
            return Ok(0);
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&self.filling);

        let settings = self.metadata_settings();
        if !settings.auto_fill_missing {
            return Ok(0);
        }
        let overlay: LocalMetaFile = json_store::read(&self.paths.local_meta())?;
        let mut missing = Vec::new();
        for meta in overlay.games.values() {
            let current = get_game_metadata(&self.paths, &meta.runner, &meta.app_name)?;
            let needs = match current {
                None => true,
                Some(c) if c.fetch_error.is_some() => match c.fetched_at.as_deref() {
                    None | Some("") => true,
                    Some(at) => {
                        chrono::DateTime::parse_from_rfc3339(at).map(|t| now_ms() - t.timestamp_millis() > WEEK_MS).unwrap_or(false)
                    }
                },
                Some(_) => false,
            };
            if needs {
                missing.push(meta);
            }
        }
        if missing.is_empty() {
            return Ok(0);
        }
        log::info!("Filling Collection metadata for {} game(s)", missing.len());
        let mut filled = 0;
        for meta in missing.into_iter().take(AUTO_FILL_BATCH) {
            if should_stop() {
                break;
            }
            let saved = self.apply_game_metadata(&MetadataApplyArgs {
                app_name: meta.app_name.clone(),
                runner: meta.runner.clone(),
                title: meta.title.clone(),
                steam_app_id: meta.steam_app_id.clone(),
                source: Some("auto".into()),
                overwrite_existing: Some(false),
                preserve_manual: Some(true),
                download_images: Some(settings.download_images),
                ..Default::default()
            })?;
            if saved.fetch_error.is_none() {
                filled += 1;
            }
        }
        Ok(filled)
    }
}
