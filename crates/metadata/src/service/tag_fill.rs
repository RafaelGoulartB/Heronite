//! Steam store tags for every game of the Collection with a Steam app id
//! (see [`crate::steam_tags`]), asked in the idle pass and when the setting
//! is turned on.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use heronite_data::focus::steam_language;
use heronite_data::library::Library;

use super::MetadataService;
use crate::steam_tags::{self, Refreshed};

/// One refresh at a time (the idle pass and the Settings toggle can meet).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Holds [`RUNNING`] while a refresh runs and lets it go when dropped, even
/// when the refresh panics: a stuck flag would stop every later refresh
/// until the app restarts.
pub(super) struct Running;

/// Whether a refresh of the Steam tags runs.
pub(super) fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

impl Running {
    /// `None` when another refresh holds it.
    pub(super) fn take() -> Option<Self> {
        (!RUNNING.swap(true, Ordering::SeqCst)).then_some(Running)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

impl MetadataService {
    /// Fetches the Steam tags the Collection lacks (when "Steam tags" is on),
    /// checking `should_stop` before each request.
    /// Gives way to a bulk job: does nothing while one runs or waits, and
    /// stops before its next request when one starts.
    pub fn refresh_steam_tags_while(&self, should_stop: &dyn Fn() -> bool) -> Result<Refreshed> {
        if !self.metadata_settings().steam_tags {
            return Ok(Refreshed::default());
        }
        let Some(_running) = Running::take() else { return Ok(Refreshed::default()) };
        if self.job_pending() {
            return Ok(Refreshed::default());
        }
        let should_stop = &|| should_stop() || self.job_pending();
        let result = (|| {
            let lib = Library::load(&self.paths)?;
            let ids: Vec<String> = lib.games.iter().filter_map(|g| g.tag_steam_id().map(str::to_owned)).collect();
            if ids.is_empty() {
                return Ok(Refreshed::default());
            }
            let language = steam_language(heronite_i18n::current().code());
            steam_tags::refresh(&self.paths, &self.http, &ids, language, steam_tags::GAP, should_stop)
        })();
        if let Ok(r) = &result {
            if r.changed() {
                log::info!("Steam tags: {} game(s), {} name list(s) updated", r.apps, r.names);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::Running;

    #[test]
    fn a_refresh_that_panics_lets_the_next_one_run() {
        let panicked = std::panic::catch_unwind(|| {
            let _running = Running::take().expect("free at first");
            panic!("the refresh failed midway");
        });
        assert!(panicked.is_err());
        let running = Running::take();
        assert!(running.is_some(), "the flag was let go by the panic");
        assert!(Running::take().is_none(), "one refresh at a time");
        drop(running);
        assert!(Running::take().is_some());
    }
}
