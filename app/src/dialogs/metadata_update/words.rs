//! How a bulk job and its results read on screen, shared by the dialog's
//! results, the header pill and "Review matches": each game's outcome as a
//! group, a word, a colour and a reason; why a game waits for review; a
//! changed field as a chip with its source's mark.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::{
    BulkProgress, FieldChange, GameOutcome, JobTally, MatchedBy, MetadataService, NoChangeReason, ReviewReason, SkipReason, UpdateMode,
};

use crate::pages::settings::metadata_labels as labels;
use crate::services::widgets::duration_text;
use crate::theme;

/// The groups the results are filtered by, in the order of the pills.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bucket {
    Updated,
    /// Sent to the review queue.
    ToReview,
    NoChange,
    NotFound,
    Failed,
    /// Left by a Stop.
    NotDone,
}

impl Bucket {
    pub const ALL: [Bucket; 6] = [Self::Updated, Self::ToReview, Self::NoChange, Self::NotFound, Self::Failed, Self::NotDone];

    pub fn of(o: &GameOutcome) -> Self {
        match o {
            GameOutcome::Updated => Self::Updated,
            GameOutcome::NoChange(_)
            | GameOutcome::Skipped(SkipReason::AlreadyFetched | SkipReason::NoSource | SkipReason::InNoSource | SkipReason::NoDetails) => {
                Self::NoChange
            }
            GameOutcome::ToReview(_) => Self::ToReview,
            GameOutcome::NotFound => Self::NotFound,
            GameOutcome::Offline | GameOutcome::Failed(_) => Self::Failed,
            GameOutcome::Skipped(SkipReason::Stopped) => Self::NotDone,
        }
    }

    pub fn count(self, t: &JobTally) -> usize {
        match self {
            Self::Updated => t.updated,
            Self::NoChange => t.no_change + t.skipped,
            Self::ToReview => t.to_review,
            Self::NotFound => t.not_found,
            Self::Failed => t.failed + t.offline,
            Self::NotDone => t.stopped,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Updated => tr!("Updated"),
            Self::ToReview => tr!("To review"),
            Self::NoChange => tr!("No change"),
            Self::NotFound => tr!("Not found"),
            Self::Failed => tr!("Failed"),
            Self::NotDone => tr!("Not done"),
        }
    }

    /// The dot before the word.
    pub fn colour(self) -> Hsla {
        match self {
            Self::Updated => theme::success().into(),
            Self::ToReview => theme::warning().into(),
            Self::NoChange => theme::ink_a(0.35),
            Self::NotFound => theme::warning().into(),
            Self::Failed => theme::danger().into(),
            Self::NotDone => theme::ink_a(0.22),
        }
    }
}

/// Why a game did not change, in a few words.
pub(crate) fn reason(o: &GameOutcome) -> &'static str {
    match o {
        GameOutcome::Updated => "",
        GameOutcome::NoChange(NoChangeReason::NothingMissing) => tr!("Nothing was missing"),
        GameOutcome::NoChange(NoChangeReason::NothingNew) => tr!("The sources had nothing new"),
        GameOutcome::NoChange(NoChangeReason::Locked) => tr!("Its fields are locked"),
        GameOutcome::ToReview(r) => review_short(*r),
        GameOutcome::NotFound => tr!("No source found this game"),
        GameOutcome::Offline => tr!("A source did not answer: try again later"),
        GameOutcome::Failed(_) => tr!("Could not be saved"),
        GameOutcome::Skipped(SkipReason::AlreadyFetched) => tr!("Already had details"),
        GameOutcome::Skipped(SkipReason::NoSource) => tr!("No source is for this game"),
        GameOutcome::Skipped(SkipReason::Stopped) => tr!("The update stopped before it"),
        GameOutcome::Skipped(SkipReason::InNoSource) => tr!("You said no source has this game"),
        GameOutcome::Skipped(SkipReason::NoDetails) => tr!("Its details are not fetched yet"),
    }
}

/// Why a game waits for review, in a few words (a row of the results).
pub(crate) fn review_short(r: ReviewReason) -> &'static str {
    match r {
        ReviewReason::LoneGuess => tr!("Found by a similar name only"),
        ReviewReason::SourcesDisagree => tr!("The sources found different games"),
        ReviewReason::Requested => tr!("You asked to check it"),
        ReviewReason::Other => tr!("The sources were not sure: waiting for review"),
    }
}

/// Why a game is asked about, as "Review matches" says it.
pub(crate) fn review_why(r: ReviewReason) -> &'static str {
    match r {
        ReviewReason::LoneGuess => tr!("Only one source found a game for this name, and its name is not quite the same."),
        ReviewReason::SourcesDisagree => tr!("The sources found different games for this name."),
        ReviewReason::Requested => tr!("You asked to check which game this is."),
        ReviewReason::Other => tr!("The sources were not sure which game this is."),
    }
}

/// How a source found a game, after its name ("by its Steam id"); `None`
/// where it says nothing the user needs.
pub(crate) fn matched_by(m: MatchedBy) -> Option<&'static str> {
    Some(match m {
        MatchedBy::Game => return None,
        MatchedBy::SourceId => tr!("by its id"),
        MatchedBy::SteamId => tr!("by its Steam id"),
        MatchedBy::RomHash => tr!("by ROM hash"),
        MatchedBy::ExactTitle => tr!("by name"),
        MatchedBy::FuzzyTitle => tr!("by a similar name"),
    })
}

/// The pill of a sure match in "Review matches" ("Hash matches"); `None`
/// for a match by name.
pub(crate) fn sure_pill(m: MatchedBy) -> Option<&'static str> {
    Some(match m {
        MatchedBy::RomHash => tr!("Hash matches"),
        MatchedBy::SteamId => tr!("Steam id matches"),
        MatchedBy::SourceId | MatchedBy::Game => tr!("Id matches"),
        MatchedBy::ExactTitle | MatchedBy::FuzzyTitle => return None,
    })
}

/// The dot and the word of an outcome.
pub(crate) fn status(o: &GameOutcome) -> Div {
    let b = Bucket::of(o);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .child(div().flex_none().size(px(7.)).rounded_full().bg(b.colour()))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(b.label()))
}

/// A source's name as shown, from the registry (its id when unknown).
pub(crate) fn source_name(service: &MetadataService, id: &str) -> String {
    service.sources().get(id).map(|d| labels::name(&d.name).to_string()).unwrap_or_else(|| id.to_string())
}

/// A field's name as shown.
pub(crate) fn field_name(field: &str) -> &str {
    labels::field_name(field)
}

/// A changed field: its source's mark and the field's name.
pub(crate) fn change_chip(change: &FieldChange, service: &MetadataService) -> Div {
    let name = source_name(service, &change.source);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(4.))
        .pr(px(9.))
        .rounded_full()
        .bg(theme::chip_fill())
        .text_size(px(12.))
        .text_color(theme::text_primary())
        .child(labels::mark(&change.source, &name, 16.))
        .child(field_name(&change.field).to_string())
}

/// The fields a game got, as one line ("Cover, Genres, Release date").
pub(crate) fn changes_line(changes: &[FieldChange]) -> String {
    changes.iter().map(|c| field_name(&c.field)).collect::<Vec<_>>().join(", ")
}

/// The line under a finished job: how long it took and how many fields
/// changed, or that it was stopped.
pub(crate) fn done_line(p: &BulkProgress) -> String {
    let t = p.tally();
    let fields = match p.mode {
        Some(UpdateMode::Fill) => trn!(t.fields, "{n} field filled", "{n} fields filled"),
        _ => trn!(t.fields, "{n} field updated", "{n} fields updated"),
    };
    if t.stopped > 0 {
        let done = p.total.saturating_sub(t.stopped);
        format!("{} \u{b7} {fields}", tr!("Stopped after {done} of {total} games", done, total = p.total))
    } else {
        format!(
            "{} \u{b7} {fields}",
            tr!("Done in {time}", time = duration_text(((p.finished_at_ms - p.started_at_ms).max(1000) / 1000) as u64))
        )
    }
}

/// Games of a finished job that need a look (not found, sent to review or
/// failed).
pub(crate) fn problems(t: &JobTally) -> usize {
    t.not_found + t.to_review + t.failed + t.offline
}
