//! What a bulk metadata job is asked to do and what it reports: how it
//! treats what a game has ([`UpdateMode`]), the presets the one-click
//! actions use (constructors of [`BulkArgs`]), each game's result
//! ([`GameResult`]), the counts shown before a job runs ([`GameCounts`])
//! and what an Undo did ([`UndoReport`]). The service runs the jobs
//! (`MetadataService::start_bulk_metadata`).

use std::sync::Arc;

use heronite_data::cover_family::CoverFamily;
use heronite_data::settings::ALL_FIELDS;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::review::ReviewReason;
use crate::sources::extra_fields;
use crate::types::{BulkArgs, BulkItem, BulkProgress, BulkState};

/// How a job treats the fields a game already has. Each maps onto the
/// merge's options ([`UpdateMode::merge_flags`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateMode {
    /// Only empty fields are filled: nothing the game has changes, and a
    /// field the user emptied by hand stays empty.
    Fill,
    /// Fields a source filled are fetched again; fields edited by hand stay.
    Refresh,
    /// Every chosen field is fetched again, hand edits too.
    Replace,
}

impl UpdateMode {
    /// The merge's `(overwrite_existing, preserve_manual)`.
    pub fn merge_flags(self) -> (bool, bool) {
        match self {
            Self::Fill => (false, true),
            Self::Refresh => (true, true),
            Self::Replace => (true, false),
        }
    }
}

/// What asked for a job; the header pill and the results name it. Later
/// presets ("fix matches") add theirs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// Chosen field by field (the dialog, or the older one).
    #[default]
    Custom,
    /// "Fill what is missing".
    FillMissing,
    /// "Download the <family> covers" ([`BulkArgs::download_covers`]); also
    /// what a view that starts using a family queues.
    Covers(CoverFamily),
}

/// The fields a job can update besides `METADATA_FIELDS`: the extras with a
/// bulk path. Steam tags are asked 50 games per request, a second apart.
/// Time to beat has none on purpose: HowLongToBeat has no API, its lookup is
/// one search of the site per game (as a browser, with a token), and a
/// bulk run of hundreds of those would hammer it. ProtonDB is asked by the
/// app per game, with its own cache.
pub const BULK_EXTRA_FIELDS: [&str; 1] = [extra_fields::STEAM_TAGS];

/// Every field a job can update: `METADATA_FIELDS`, the covers of the
/// other families (`coverSquare`, `coverWide`), then [`BULK_EXTRA_FIELDS`].
/// A job fills a family's covers only while the family is kept
/// (`heronite_data::cover_family`), unless it is that family's download.
pub fn bulk_fields() -> Vec<String> {
    ALL_FIELDS.iter().chain(BULK_EXTRA_FIELDS.iter()).map(|f| f.to_string()).collect()
}

impl BulkArgs {
    /// "Fill what is missing": every empty field of `games`, extras too,
    /// from every source that is on.
    pub fn fill_missing(games: Vec<BulkItem>) -> Self {
        Self { kind: JobKind::FillMissing, ..Self::update(games, UpdateMode::Fill, None, None) }
    }

    /// A job chosen field by field. `fields` (ids of [`bulk_fields`]) and
    /// `sources` (source ids) `None`: all of them.
    pub fn update(games: Vec<BulkItem>, mode: UpdateMode, fields: Option<Vec<String>>, sources: Option<Vec<String>>) -> Self {
        Self { games, mode: Some(mode), fields: Some(fields.unwrap_or_else(bulk_fields)), sources, ..Default::default() }
    }

    /// "Download the <family> covers": the covers of `family` for the games
    /// of `games` that have no picture of it (fetched or set by hand), from
    /// the sources that give that family only. Leaves every other field,
    /// the tall cover and its source included, as it is.
    pub fn download_covers(games: Vec<BulkItem>, family: CoverFamily) -> Self {
        Self { kind: JobKind::Covers(family), ..Self::update(games, UpdateMode::Fill, Some(vec![family.field().to_owned()]), None) }
    }

    /// Every field `source` gives, from that source only.
    pub fn only_source(games: Vec<BulkItem>, mode: UpdateMode, source: &str) -> Self {
        Self::update(games, mode, None, Some(vec![source.to_owned()]))
    }

    /// The older dialog's options: "Skip games that already have metadata",
    /// "Keep fields edited by hand", "Copy covers locally".
    pub fn legacy(games: Vec<BulkItem>, skip_existing: bool, keep_manual: bool, copy_covers: bool) -> Self {
        Self {
            games,
            overwrite_existing: Some(!skip_existing),
            preserve_manual: Some(keep_manual),
            download_images: Some(copy_covers),
            ..Default::default()
        }
    }
}

/// A field a job changed and the source its new value came from (for Steam
/// tags: `steamtags`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    /// A [`bulk_fields`] id.
    pub field: String,
    pub source: String,
}

/// What a job did with one game. The UI words each one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum GameOutcome {
    /// At least one chosen field changed ([`GameResult::changes`]).
    Updated,
    NoChange(NoChangeReason),
    /// No source asked had the game. When the job asked every source for
    /// every field, the record says so (`fetchError`: "failed last time").
    NotFound,
    /// No source found the game and one that could have did not answer
    /// (offline, or it asked to wait); nothing was saved or marked.
    Offline,
    /// The sources were not sure which game it is: nothing was saved and the
    /// game waits in the review queue.
    ToReview(ReviewReason),
    /// Reading or writing the game's record failed: the error, in English
    /// (it is a diagnostic, not a sentence of the interface).
    Failed(String),
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoChangeReason {
    /// "Fill": every chosen field already had a value; no source was asked.
    NothingMissing,
    /// The sources answered with what the game already had.
    NothingNew,
    /// Every chosen field is locked; no source was asked.
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// The older "Skip games that already have metadata".
    AlreadyFetched,
    /// None of the chosen sources is for this game (Steam tags of a game
    /// that is not on Steam).
    NoSource,
    /// The job was stopped before this game.
    Stopped,
    /// The user said no source has this game ("In no source").
    InNoSource,
    /// The job asked only for pictures and the game's details were never
    /// fetched: the idle fill fetches them, pictures included.
    NoDetails,
}

/// One game of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameResult {
    /// `runner:appName`, the key of `metadata.json`.
    pub key: String,
    pub app_name: String,
    pub runner: String,
    pub title: String,
    pub outcome: GameOutcome,
    /// What changed, in `bulk_fields` order; empty unless `Updated`.
    pub changes: Vec<FieldChange>,
}

/// A job waiting for the running one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedJob {
    pub id: u64,
    pub kind: JobKind,
    pub mode: Option<UpdateMode>,
    pub games: usize,
}

/// [`BulkProgress::results`] counted by outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobTally {
    pub updated: usize,
    pub no_change: usize,
    pub not_found: usize,
    pub offline: usize,
    pub failed: usize,
    /// Sent to the review queue.
    pub to_review: usize,
    /// Skipped for being fetched, having no source or being "in no source",
    /// not for a Stop.
    pub skipped: usize,
    pub stopped: usize,
    /// Fields changed over every game.
    pub fields: usize,
}

impl BulkProgress {
    /// A job is running or stopping.
    pub fn is_running(&self) -> bool {
        matches!(self.state, BulkState::Running | BulkState::Cancelling)
    }

    /// Games of the running job not done yet.
    pub fn left(&self) -> usize {
        if self.is_running() {
            self.total.saturating_sub(self.results.len())
        } else {
            0
        }
    }

    pub fn tally(&self) -> JobTally {
        let mut t = JobTally::default();
        for r in &self.results {
            t.fields += r.changes.len();
            match &r.outcome {
                GameOutcome::Updated => t.updated += 1,
                GameOutcome::NoChange(_) => t.no_change += 1,
                GameOutcome::NotFound => t.not_found += 1,
                GameOutcome::Offline => t.offline += 1,
                GameOutcome::Failed(_) => t.failed += 1,
                GameOutcome::ToReview(_) => t.to_review += 1,
                GameOutcome::Skipped(SkipReason::Stopped) => t.stopped += 1,
                GameOutcome::Skipped(_) => t.skipped += 1,
            }
        }
        t
    }

    /// The result of `key` in this job, if done.
    pub fn result_for(&self, key: &str) -> Option<&Arc<GameResult>> {
        self.results.iter().find(|r| r.key == key)
    }
}

/// What a set of games has, read from `metadata.json` (and the Steam tags
/// cache) without asking any source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameCounts {
    pub games: usize,
    /// No record, or one without a value in any field (games marked "in no
    /// source" left out, as in the next two).
    pub no_details: usize,
    /// The last fetch found the game in no source (`fetchError`).
    pub failed_last_time: usize,
    /// Waiting in the review queue.
    pub to_review: usize,
    /// The three above, each game once.
    pub without_match: usize,
    /// Details but no description.
    pub no_description: usize,
    /// Details but no cover.
    pub no_cover: usize,
    /// Per cover family in use beside the tall one: the games with details
    /// and no picture of that family, shown cropped from another one
    /// ([`crate::service::Attention::cropped`]). Not part of
    /// `needs_attention`.
    pub cropped: IndexMap<CoverFamily, usize>,
    /// Marked "in no source" by the user.
    pub in_no_source: usize,
    /// Games with any reason of [`crate::service::Attention`]; the others
    /// are complete.
    pub needs_attention: usize,
    /// Games where "Fill what is missing" has a field to look at.
    pub games_missing: usize,
    /// The fields "Fill what is missing" would look at, over every game.
    pub fields_missing: usize,
    /// Per [`bulk_fields`] id: the games that lack it.
    pub missing_by_field: IndexMap<String, usize>,
    /// Per source a job may ask (`MetadataService::bulk_sources`, in that
    /// order): the games it is for (none while it waits for its download).
    pub reach: IndexMap<String, usize>,
    /// The sources of `reach` that cannot be asked until their database is
    /// downloaded.
    pub needs_download: Vec<String>,
}

/// What [`MetadataService::undo_bulk_metadata`](crate::MetadataService::undo_bulk_metadata) did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UndoReport {
    pub job: u64,
    /// Records put back as they were (or removed, when the job created them).
    pub games: usize,
    /// Parts of records left as they are because they changed after the job.
    pub kept: usize,
    /// Pictures the job had copied that nothing uses any more.
    pub files_removed: usize,
    /// Pictures the job had replaced, put back.
    pub files_restored: usize,
    /// Entries of the review queue put back as they were (the job's added
    /// ones removed).
    pub reviews: usize,
}
