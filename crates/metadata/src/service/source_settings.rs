//! Settings → Metadata's entry points: the rows of the sources and the
//! changes to them. Every change is one read-modify-write of
//! `local_library/settings.json` (the SteamGridDB key: of the global
//! settings) and returns the settings as saved, like the other setters.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{anyhow, bail, Result};
use heronite_data::models::MetadataSettings;
use heronite_data::settings::{self, EffectiveSettings};
use heronite_data::source_prefs;
use parking_lot::Mutex;

use super::MetadataService;
use crate::sources::options::{option_values, set_option_value};
use crate::sources::{
    rows, DownloadProgress, DownloadState, MetadataSource, OptionValue, ProgressSnapshot, SourceContext, SourceDescriptor, SourceRow,
    SourceStatus, SourceTest,
};

/// The downloads running, by source id.
pub(super) type Downloads = Mutex<HashMap<String, Arc<DownloadProgress>>>;

impl MetadataService {
    fn descriptor(&self, id: &str) -> Result<SourceDescriptor> {
        self.sources().get(id).cloned().ok_or_else(|| anyhow!("unknown metadata source {id}"))
    }

    /// Adds a source (an extension) to the registry, or replaces the
    /// extension with its id; asked from the next fetch on. A built-in id
    /// is refused.
    pub fn add_source(&self, source: Arc<dyn MetadataSource>) -> Result<()> {
        let mut sources = self.sources.write();
        *sources = Arc::new(sources.with_source(source)?);
        Ok(())
    }

    /// Takes extension `id` out of the registry; `false` when it was not
    /// there. What the user set for it stays in the settings.
    pub fn remove_source(&self, id: &str) -> bool {
        let mut sources = self.sources.write();
        match sources.without_source(id) {
            Some(next) => {
                *sources = Arc::new(next);
                true
            }
            None => false,
        }
    }

    /// Whether source `id` is on in the saved settings.
    pub fn source_on(&self, id: &str) -> bool {
        source_prefs::is_on(&self.metadata_settings(), id)
    }

    /// The rows of Settings → Metadata (see [`SourceRow`]). Reads the
    /// global settings: call it after a change, not per frame.
    pub fn source_rows(&self, settings: &MetadataSettings) -> Vec<SourceRow> {
        self.sources().rows(&self.paths, settings, &self.tests)
    }

    /// The status of one source (`None`: not registered).
    pub fn source_status(&self, settings: &MetadataSettings, id: &str) -> Option<SourceStatus> {
        let sources = self.sources();
        let d = sources.get(id)?;
        let values = option_values(&self.paths, settings, d);
        let download = sources.download_state(&self.paths, id);
        let health = sources.health(id);
        Some(rows::status_with(d, source_prefs::is_on(settings, id), &values, self.tests.get(id, &values), download.as_ref(), &health))
    }

    /// Runs the test of `id` with the saved options and remembers the
    /// outcome for its status. `None` when it has no test. Blocking.
    pub fn test_source(&self, id: &str) -> Option<SourceTest> {
        let sources = self.sources();
        let d = sources.get(id)?;
        let ctx = SourceContext::new(&self.paths, &self.http);
        let result = sources.test(&ctx, id)?;
        self.tests.record(id, &option_values(&self.paths, ctx.settings(), d), result.ok);
        Some(result)
    }

    /// Turns a source on or off (any id: one this build does not know is
    /// kept for the build that does). Switching it on clears the failures
    /// that stopped it ([`SourceStatus::Failing`]).
    pub fn set_source_enabled(&self, id: &str, on: bool) -> Result<EffectiveSettings> {
        if on {
            self.sources().reset_health(id);
        }
        settings::update_metadata(&self.paths, |m| source_prefs::set_on(m, id, on))
    }

    /// Saves option `key` of source `id` where it is kept.
    pub fn set_source_option(&self, id: &str, key: &str, value: OptionValue) -> Result<EffectiveSettings> {
        set_option_value(&self.paths, &self.descriptor(id)?, key, value)
    }

    /// Lets the merge take `field` from source `id`, or not.
    pub fn set_source_field_enabled(&self, id: &str, field: &str, on: bool) -> Result<EffectiveSettings> {
        settings::update_metadata(&self.paths, |m| source_prefs::set_field_on(m, id, field, on))
    }

    /// Moves `id` to place `to` among the rows of its kind (0 = first).
    pub fn move_source(&self, id: &str, to: usize) -> Result<EffectiveSettings> {
        let current = self.metadata_settings();
        let order = self.sources().order_after_move(&current, id, to).ok_or_else(|| anyhow!("{id} cannot be moved"))?;
        settings::update_metadata(&self.paths, |m| m.source_order = order)
    }

    /// The sources `field` is taken from, first to last, as the merge uses them.
    pub fn field_order(&self, settings: &MetadataSettings, field: &str) -> Vec<String> {
        self.sources().field_order(settings, field)
    }

    /// Whether the user set an order for `field`.
    pub fn is_field_customised(&self, settings: &MetadataSettings, field: &str) -> bool {
        source_prefs::is_field_customised(settings, field)
    }

    /// Saves `order` as the user's order for `field`; the sources it leaves
    /// out keep their place after it.
    pub fn set_field_order(&self, field: &str, order: &[String]) -> Result<EffectiveSettings> {
        let list = self.sources().field_list_to_save(&self.metadata_settings(), field, order);
        settings::update_metadata(&self.paths, |m| {
            m.field_priority.insert(field.to_owned(), list);
        })
    }

    /// Back to the order every source move changes.
    pub fn reset_field_order(&self, field: &str) -> Result<EffectiveSettings> {
        settings::update_metadata(&self.paths, |m| source_prefs::reset_field(m, field))
    }
}

impl MetadataService {
    /// Where the database of source `id` stands (`None`: it downloads none).
    pub fn source_download_state(&self, id: &str) -> Option<DownloadState> {
        self.sources().download_state(&self.paths, id)
    }

    /// Downloads (or refreshes) the database of source `id` now, whether the
    /// source is on or not: the user asked. Blocking; one download per
    /// source at a time. Its progress: [`Self::source_download_progress`].
    pub fn download_source(&self, id: &str) -> Result<()> {
        let progress = Arc::new(DownloadProgress::new());
        {
            let mut running = self.downloads.lock();
            if running.contains_key(id) {
                bail!("{id} is downloading already");
            }
            running.insert(id.to_owned(), progress.clone());
        }
        let ctx = SourceContext::new(&self.paths, &self.http);
        let result = self.sources().download(&ctx, id, &progress);
        self.downloads.lock().remove(id);
        result
    }

    /// The progress of the download of `id`, while one runs.
    pub fn source_download_progress(&self, id: &str) -> Option<ProgressSnapshot> {
        self.downloads.lock().get(id).map(|p| p.snapshot())
    }

    /// The sources downloading now.
    pub fn downloading_sources(&self) -> Vec<String> {
        self.downloads.lock().keys().cloned().collect()
    }

    /// Asks the download of `id` to stop; what is in use stays.
    pub fn cancel_source_download(&self, id: &str) {
        if let Some(p) = self.downloads.lock().get(id) {
            p.cancel();
        }
    }

    /// Frees the space the database of `id` takes; the source waits for a
    /// download again. Refused while it downloads.
    pub fn remove_source_download(&self, id: &str) -> Result<()> {
        if self.downloads.lock().contains_key(id) {
            bail!("{id} is downloading");
        }
        self.sources().remove_download(&SourceContext::new(&self.paths, &self.http), id)
    }

    /// The idle maintenance's part: refreshes the database of every source
    /// that is on, has one, and whose copy is older than its refresh days,
    /// or whose last refresh failed or copy no longer opens once its retry
    /// time came ([`DownloadState::refresh_due`]). A source never downloaded
    /// is left alone (only the user starts the first download). Returns how
    /// many were refreshed. Blocking.
    pub fn refresh_due_source_downloads(&self) -> usize {
        let settings = self.metadata_settings();
        let now = SystemTime::now();
        let sources = self.sources();
        let due: Vec<String> = sources
            .lookups()
            .map(|s| s.descriptor())
            .filter(|d| source_prefs::is_on(&settings, &d.id))
            .filter(|d| {
                let (Some(download), Some(state)) = (&d.download, sources.download_state(&self.paths, &d.id)) else { return false };
                state.refresh_due(download, now)
            })
            .map(|d| d.id.clone())
            .collect();
        due.iter()
            .filter(|id| match self.download_source(id) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("refreshing the database of {id}: {e:#}");
                    false
                }
            })
            .count()
    }
}
