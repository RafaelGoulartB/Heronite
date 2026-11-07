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

impl MetadataService {
    /// Fetches the Steam tags the Collection lacks (when "Steam tags" is on),
    /// checking `should_stop` before each request.
    pub fn refresh_steam_tags_while(&self, should_stop: &dyn Fn() -> bool) -> Result<Refreshed> {
        if !self.metadata_settings().steam_tags || RUNNING.swap(true, Ordering::SeqCst) {
            return Ok(Refreshed::default());
        }
        let result = (|| {
            let lib = Library::load(&self.paths)?;
            let ids: Vec<String> = lib.games.iter().filter_map(|g| g.tag_steam_id().map(str::to_owned)).collect();
            if ids.is_empty() {
                return Ok(Refreshed::default());
            }
            let language = steam_language(heronite_i18n::current().code());
            steam_tags::refresh(&self.paths, &self.http, &ids, language, steam_tags::GAP, should_stop)
        })();
        RUNNING.store(false, Ordering::SeqCst);
        if let Ok(r) = &result {
            if r.changed() {
                log::info!("Steam tags: {} game(s), {} name list(s) updated", r.apps, r.names);
            }
        }
        result
    }
}
