//! `HERONITE_DEBUG_METADATA_JOB=running|done|clean|review|idle-review[+queued]`:
//! every bulk job (from the dock, the dialog or the Collection at start) is
//! made up: nothing is fetched or written, no network. `running` holds at
//! about 60% with the next game "being fetched"; `done` finishes with some
//! games not found and one failed; `clean` finishes with every game updated
//! or unchanged; `review` finishes with some games sent to review;
//! `idle-review` starts no job but fills the review queue the hub shows with
//! the first games on screen, as the idle fill would; `+queued` adds a job
//! waiting behind it. Stop and Undo act on the made-up job, and "Review
//! matches" makes up its entries while jobs are made up. For captures of the
//! dialog's results and the header pill.

use std::sync::Arc;

use gpui_kit::*;
use heronite_data::library::metadata_key;
use heronite_metadata::{
    BulkArgs, BulkItem, BulkProgress, BulkState, FieldChange, GameOutcome, GameResult, JobKind, NoChangeReason, QueuedJob, ReviewItem,
    ReviewReason, SkipReason, UndoReport, UpdateMode,
};

use super::hub::{HubEvent, MetadataHub};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ends {
    Running,
    Done,
    Clean,
    Review,
    /// No job: the idle fill queued games.
    IdleReview,
}

pub(super) struct Fake {
    ends: Ends,
    queued: bool,
    next_id: u64,
    /// The games of the made-up job running (Stop marks the rest).
    games: Vec<BulkItem>,
}

impl Fake {
    pub(super) fn from_env() -> Option<Self> {
        let value = crate::env::value("HERONITE_DEBUG_METADATA_JOB")?;
        let (ends, rest) = value.split_once('+').unwrap_or((value.as_str(), ""));
        let ends = match ends {
            "running" => Ends::Running,
            "clean" => Ends::Clean,
            "review" => Ends::Review,
            "idle-review" => Ends::IdleReview,
            _ => Ends::Done,
        };
        Some(Self { ends, queued: rest == "queued", next_id: 0, games: Vec::new() })
    }
}

/// The fields a made-up update changes, with their source.
const CHANGES: [(&str, &str); 7] = [
    ("cover", "steamgriddb"),
    ("description", "igdb"),
    ("genres", "igdb"),
    ("releaseDate", "steam"),
    ("criticScore", "igdb"),
    ("hero", "steamgriddb"),
    ("steamTags", "steamtags"),
];

/// What the made-up job did with game `i`.
fn outcome(i: usize, ends: Ends) -> (GameOutcome, Vec<FieldChange>) {
    let clean = ends == Ends::Clean;
    match i % 9 {
        0 if ends == Ends::Review => (GameOutcome::ToReview(ReviewReason::SourcesDisagree), Vec::new()),
        4 if ends == Ends::Review => (GameOutcome::ToReview(ReviewReason::LoneGuess), Vec::new()),
        3 if ends == Ends::Review => (GameOutcome::NoChange(NoChangeReason::NothingNew), Vec::new()),
        3 if !clean => (GameOutcome::NotFound, Vec::new()),
        7 if !clean && i % 2 == 1 => (GameOutcome::Failed("metadata.json: permission denied".into()), Vec::new()),
        5 => (GameOutcome::NoChange(NoChangeReason::NothingMissing), Vec::new()),
        8 => (GameOutcome::NoChange(NoChangeReason::NothingNew), Vec::new()),
        _ => {
            let n = 1 + i % 3;
            let changes = (0..n)
                .map(|k| CHANGES[(i + k * 2) % CHANGES.len()])
                .map(|(field, source)| FieldChange { field: field.into(), source: source.into() })
                .collect();
            (GameOutcome::Updated, changes)
        }
    }
}

fn result(game: &BulkItem, outcome: GameOutcome, changes: Vec<FieldChange>) -> Arc<GameResult> {
    let runner = if game.runner == "zoom" { "sideload" } else { game.runner.as_str() };
    Arc::new(GameResult {
        key: metadata_key(runner, &game.app_name),
        app_name: game.app_name.clone(),
        runner: runner.to_owned(),
        title: game.title.clone(),
        outcome,
        changes,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl MetadataHub {
    /// Whether made-up jobs stand for the idle fill (no job is started).
    pub fn fake_idle(&self) -> bool {
        self.fake_ref().is_some_and(|f| f.ends == Ends::IdleReview)
    }

    /// `idle-review`: the first games of `games` wait for review.
    pub fn fake_idle_queue(&mut self, games: &[BulkItem], cx: &mut Context<Self>) {
        let queue = games
            .iter()
            .take(4)
            .enumerate()
            .map(|(i, g)| {
                let runner = if g.runner == "zoom" { "sideload" } else { g.runner.as_str() };
                ReviewItem {
                    key: metadata_key(runner, &g.app_name),
                    app_name: g.app_name.clone(),
                    runner: runner.to_owned(),
                    title: g.title.clone(),
                    reason: if i % 2 == 0 { ReviewReason::SourcesDisagree } else { ReviewReason::LoneGuess },
                    choices: 3 - i % 2,
                }
            })
            .collect();
        self.set_fake_queue(queue, cx);
    }

    pub(super) fn fake_start(&mut self, args: BulkArgs, cx: &mut Context<Self>) -> u64 {
        let Some(fake) = self.fake_mut() else { return 0 };
        fake.next_id += 1;
        fake.games = args.games.clone();
        let (id, ends, queued) = (fake.next_id, fake.ends, fake.queued);
        let total = args.games.len();
        let done = if ends == Ends::Running { total * 3 / 5 } else { total };
        let results: Vec<_> = args
            .games
            .iter()
            .take(done)
            .enumerate()
            .map(|(i, g)| {
                let (outcome, changes) = outcome(i, ends);
                result(g, outcome, changes)
            })
            .collect();
        let now = now_ms();
        let mut p = BulkProgress {
            state: if ends == Ends::Running { BulkState::Running } else { BulkState::Done },
            total,
            processed: done,
            percent: (done * 100).checked_div(total).unwrap_or(100) as u32,
            current_title: args.games.get(done).filter(|_| ends == Ends::Running).map(|g| g.title.clone()),
            job: id,
            kind: args.kind,
            mode: args.mode,
            started_at_ms: now - 14_000,
            finished_at_ms: if ends == Ends::Running { 0 } else { now },
            results,
            queued: if queued {
                vec![QueuedJob { id: id + 1000, kind: JobKind::Custom, mode: Some(UpdateMode::Fill), games: 12 }]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let tally = p.tally();
        p.updated = tally.updated + tally.no_change;
        p.failed = tally.not_found + tally.failed;
        p.can_undo = ends != Ends::Running && tally.updated > 0;
        self.on_progress(p, cx);
        id
    }

    pub(super) fn fake_stop(&mut self, cx: &mut Context<Self>) {
        if !self.progress.is_running() {
            return;
        }
        let games = self.fake_mut().map(|f| std::mem::take(&mut f.games)).unwrap_or_default();
        let mut p = self.progress.clone();
        for game in games.iter().skip(p.results.len()) {
            p.results.push(result(game, GameOutcome::Skipped(SkipReason::Stopped), Vec::new()));
        }
        p.state = BulkState::Done;
        p.current_title = None;
        p.finished_at_ms = now_ms();
        p.can_undo = p.tally().updated > 0;
        self.on_progress(p, cx);
    }

    pub(super) fn fake_undo(&mut self, cx: &mut Context<Self>) {
        let report = UndoReport { job: self.progress.job, games: self.progress.tally().updated, ..Default::default() };
        self.undoing = false;
        self.progress.can_undo = false;
        self.undone = Some(report);
        cx.emit(HubEvent::Undone(report));
        cx.notify();
    }
}
