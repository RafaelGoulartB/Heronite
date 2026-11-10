//! Cover families as the service sees them: which ones this installation
//! keeps (`heronite_data::cover_family`), and the job a view that starts
//! using a family queues ([`MetadataService::cover_family_job`]).

use std::collections::HashSet;

use heronite_data::cover_family::{CoverFamilies, CoverFamily};
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{ArtFile, LocalMetaFile};
use heronite_data::status::STATUS_ONLY_KEY;

use super::counts::{missing_details, waits_for_details};
use super::MetadataService;
use crate::job::JobKind;
use crate::sources::{AppliesTo, SourceDescriptor, SourceGame, SourceStatus};
use crate::store::{as_runner, get_all_game_metadata};
use crate::types::{ArtKind, BulkArgs, BulkItem, MetadataApplyArgs};

impl MetadataService {
    /// Which cover families this installation keeps and which some view
    /// uses (the settings and the console theme of its data folder).
    pub fn cover_families(&self) -> CoverFamilies {
        CoverFamilies::load(&self.paths)
    }

    /// The job to queue when a view starts drawing `family` (the desktop's
    /// aspect or the console theme changed): the download of its covers
    /// ([`BulkArgs::download_covers`]) for the games [`family_lack`](Self::family_lack)
    /// finds. `None` when there is nothing to do: the tall family, the
    /// option `fillCoverFamilies` off, the family's download already running
    /// or waiting (switching the aspect back and forth queues it once), no
    /// source that gives the family can be asked (off, or its key not set),
    /// or no game lacks it.
    ///
    /// Reads the records and `art.json`, asks no source: run it off the UI
    /// thread and hand the job to the queue
    /// ([`start_bulk_job`](Self::start_bulk_job)), which never blocks.
    pub fn cover_family_job(&self, family: CoverFamily, games: Option<Vec<BulkItem>>) -> Option<BulkArgs> {
        if family == CoverFamily::Tall || !self.metadata_settings().fill_cover_families || self.job_of_kind(JobKind::Covers(family)) {
            return None;
        }
        let lack = self.family_lack(family, games)?;
        (lack.can_download && !lack.games.is_empty()).then(|| BulkArgs::download_covers(lack.games, family))
    }

    /// Whether a source of `family`'s order can be asked now (on, its key
    /// set) and is for some game of the Collection (a source of emulated
    /// games asks nothing of a Collection without any): a download of its
    /// covers can find something.
    pub fn can_download_family(&self, family: CoverFamily) -> bool {
        self.family_askable(family, None)
    }

    /// [`can_download_family`](Self::can_download_family) for `games` (`None`:
    /// every game of the Collection).
    fn family_askable(&self, family: CoverFamily, games: Option<&[BulkItem]>) -> bool {
        let settings = self.metadata_settings();
        // One registry throughout, whatever is installed meanwhile.
        let sources = self.sources();
        let ready: Vec<&SourceDescriptor> = sources
            .field_order(&settings, family.field())
            .iter()
            .filter(|id| {
                sources.has_lookup(id)
                    && self.source_status(&settings, id).is_some_and(|s| matches!(s, SourceStatus::On | SourceStatus::Connected))
            })
            .filter_map(|id| sources.get(id))
            .collect();
        if ready.iter().any(|d| d.applies_to == AppliesTo::All) {
            return true;
        }
        if ready.is_empty() {
            return false;
        }
        let Ok(overlay) = json_store::read::<LocalMetaFile>(&self.paths.local_meta()) else { return false };
        let views: Vec<SourceGame> = overlay
            .games
            .values()
            .filter(|m| games.is_none_or(|list| list.iter().any(|g| g.app_name == m.app_name && g.runner == m.runner)))
            .map(|m| {
                let args = MetadataApplyArgs {
                    app_name: m.app_name.clone(),
                    runner: m.runner.clone(),
                    title: m.title.clone(),
                    steam_app_id: m.steam_app_id.clone(),
                    ..Default::default()
                };
                SourceGame::new(&args, Some(m), &[])
            })
            .collect();
        ready.iter().any(|d| views.iter().any(|g| sources.applies(&self.paths, &settings, d, g)))
    }

    /// The games of `games` (`None`: every game of the Collection) with
    /// details, or found in no source, that have no picture of `family`,
    /// fetched or set by hand, and whose field is not locked or emptied by
    /// hand: what a download of the family's covers looks for, whatever
    /// `fillCoverFamilies` says (a game whose details were never fetched is
    /// left to the idle fill, as the download does: [`waits_for_details`]).
    /// Empty for the tall family. `None` when the files cannot be read.
    /// Reads the records and `art.json`, asks no source.
    pub fn family_lack(&self, family: CoverFamily, games: Option<Vec<BulkItem>>) -> Option<FamilyLack> {
        let can_download = family != CoverFamily::Tall && self.family_askable(family, games.as_deref());
        if family == CoverFamily::Tall {
            return Some(FamilyLack { games: Vec::new(), can_download });
        }
        let field = family.field();
        let games = match games {
            Some(games) => games,
            None => collection_games(&json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok()?),
        };
        let records = get_all_game_metadata(&self.paths).ok()?;
        let art: ArtFile = json_store::read(&self.paths.art()).unwrap_or_default();
        let kind = ArtKind::cover(family);
        let custom: HashSet<&String> =
            art.games.iter().filter(|(_, a)| kind.of_art(a).is_some_and(|u| !u.is_empty())).map(|(k, _)| k).collect();
        let games = games
            .into_iter()
            .filter(|g| {
                let key = metadata_key(as_runner(&g.runner), &g.app_name);
                let md = records.get(&key);
                !custom.contains(&key) && !waits_for_details(md) && missing_details(md, &[field]).next().is_some()
            })
            .collect();
        Some(FamilyLack { games, can_download })
    }
}

/// What [`MetadataService::family_lack`] found.
#[derive(Debug, Clone, Default)]
pub struct FamilyLack {
    /// The games lacking a picture of the family, in the order given.
    pub games: Vec<BulkItem>,
    /// A source that gives the family can be asked now.
    pub can_download: bool,
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
