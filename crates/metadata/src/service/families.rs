//! Cover families as the service sees them: which ones this installation
//! keeps (`heronite_data::cover_family`), and the job a view that starts
//! using a family queues ([`MetadataService::cover_family_job`]).

use std::collections::HashSet;

use heronite_data::cover_family::{CoverFamilies, CoverFamily};
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{ArtFile, LocalMetaFile};
use heronite_data::status::STATUS_ONLY_KEY;

use super::counts::missing_details;
use super::MetadataService;
use crate::sources::SourceStatus;
use crate::store::{as_runner, get_all_game_metadata};
use crate::types::{ArtKind, BulkArgs, BulkItem};

impl MetadataService {
    /// Which cover families this installation keeps and which some view
    /// uses (the settings and the console theme of its data folder).
    pub fn cover_families(&self) -> CoverFamilies {
        CoverFamilies::load(&self.paths)
    }

    /// The job to queue when a view starts drawing `family` (the desktop's
    /// aspect or the console theme changed): the download of its covers
    /// ([`BulkArgs::download_covers`]) for the games of `games` (`None`:
    /// every game of the Collection) that have no picture of it, fetched or
    /// set by hand, and whose field is not locked or emptied by hand.
    /// `None` when there is nothing to do: the tall family, the option
    /// `fillCoverFamilies` off, no source that gives the family can be
    /// asked (off, or its key not set), or no game lacks it.
    ///
    /// Reads the records and `art.json`, asks no source: run it off the UI
    /// thread and hand the job to the queue
    /// ([`start_bulk_job`](Self::start_bulk_job)), which never blocks.
    pub fn cover_family_job(&self, family: CoverFamily, games: Option<Vec<BulkItem>>) -> Option<BulkArgs> {
        let settings = self.metadata_settings();
        if family == CoverFamily::Tall || !settings.fill_cover_families {
            return None;
        }
        let field = family.field();
        // A source of the family's order that can be asked now (its key set).
        let ready = |id: &String| {
            self.sources.has_lookup(id)
                && self.source_status(&settings, id).is_some_and(|s| matches!(s, SourceStatus::On | SourceStatus::Connected))
        };
        if !self.sources.field_order(&settings, field).iter().any(ready) {
            return None;
        }
        let games = match games {
            Some(games) => games,
            None => collection_games(&json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok()?),
        };
        let records = get_all_game_metadata(&self.paths).ok()?;
        let art: ArtFile = json_store::read(&self.paths.art()).unwrap_or_default();
        let kind = ArtKind::cover(family);
        let custom: HashSet<&String> =
            art.games.iter().filter(|(_, a)| kind.of_art(a).is_some_and(|u| !u.is_empty())).map(|(k, _)| k).collect();
        let lacking: Vec<BulkItem> = games
            .into_iter()
            .filter(|g| {
                let key = metadata_key(as_runner(&g.runner), &g.app_name);
                !custom.contains(&key) && missing_details(records.get(&key), &[field]).next().is_some()
            })
            .collect();
        (!lacking.is_empty()).then(|| BulkArgs::download_covers(lacking, family))
    }
}

/// Every game of the Collection's overlay, as the idle fill lists them.
fn collection_games(overlay: &LocalMetaFile) -> Vec<BulkItem> {
    overlay
        .games
        .values()
        .filter(|m| m.extra.get(STATUS_ONLY_KEY) != Some(&serde_json::Value::Bool(true)))
        .map(|m| BulkItem {
            app_name: m.app_name.clone(),
            runner: m.runner.clone(),
            title: m.title.clone(),
            steam_app_id: m.steam_app_id.clone(),
        })
        .collect()
}
