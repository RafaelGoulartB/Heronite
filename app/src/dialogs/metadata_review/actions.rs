//! What the review does: reading a game's proposals (queued: from the
//! queue; not queued: asking the sources), searching by another name, and
//! the answers. Everything that reads or writes runs off the UI thread.

use heronite_i18n::tr;
use heronite_metadata::service::{is_offline, MatchUnavailable, Resolution, SourceSearch};
use heronite_metadata::{MatchChoice, ReviewReason};

use gpui_kit::*;

use super::{Answer, Busy, MetadataReview};
use crate::dialogs::metadata_update::words;

/// The choices most sources agree on first (a stable sort: the sources'
/// order otherwise).
fn by_agreement(mut choices: Vec<MatchChoice>) -> Vec<MatchChoice> {
    choices.sort_by_key(|c| std::cmp::Reverse(c.hits.len()));
    choices
}

impl MetadataReview {
    /// Reads the proposals of the game shown.
    pub(super) fn load(&mut self, cx: &mut Context<Self>) {
        let Some(game) = self.game().cloned() else { return };
        self.reason = None;
        self.choices.clear();
        self.picked = None;
        self.search_note = None;
        self.load_error = None;
        self.error = None;
        self.search_task = None;
        self.searching = false;
        self.loading = true;
        if let Some(fake) = &self.fake {
            let (reason, choices) = fake.entry(self.at, &game);
            self.loaded(Ok((reason, choices)));
            return;
        }
        let service = self.hub.read(cx).service.clone();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async move {
                    let queued = game.queued.then(|| service.review_entry(&game.key)).flatten();
                    match queued {
                        Some(entry) => Ok(entry),
                        None => service.review_entry_for(&game.item),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.loaded(read.map(|e| (e.reason, e.choices())));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn loaded(&mut self, read: anyhow::Result<(ReviewReason, Vec<MatchChoice>)>) {
        self.loading = false;
        match read {
            Ok((reason, choices)) => {
                self.reason = Some(reason);
                self.choices = by_agreement(choices);
                self.picked = (!self.choices.is_empty()).then_some(0);
            }
            Err(err) => self.load_error = Some(error_text(&err)),
        }
    }

    /// Searches every source for what the field holds (the game's title
    /// when empty) and shows what they found instead of the proposals.
    pub(super) fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(game) = self.game().cloned() else { return };
        if self.searching || self.busy.is_some() {
            return;
        }
        let typed = self.search.read(cx).value().trim().to_string();
        let query = if typed.is_empty() { game.item.title.clone() } else { typed };
        self.searching = true;
        self.search_note = None;
        self.error = None;
        if let Some(fake) = &self.fake {
            let found = fake.search(&query);
            self.searched(&query, found, cx);
            return;
        }
        let service = self.hub.read(cx).service.clone();
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let q = query.clone();
            let found = cx.background_spawn(async move { service.search_sources(&q, None) }).await;
            this.update(cx, |this, cx| this.searched(&query, found, cx)).ok();
        }));
        cx.notify();
    }

    fn searched(&mut self, query: &str, found: SourceSearch, cx: &mut Context<Self>) {
        self.searching = false;
        let service = self.hub.read(cx).service.clone();
        let names = |ids: &[String]| ids.iter().map(|id| words::source_name(&service, id)).collect::<Vec<_>>().join(", ");
        let mut notes = Vec::new();
        if found.asked.is_empty() {
            notes.push(tr!("No source that can search by name is on.").to_string());
        } else if found.choices.is_empty() && found.unreachable.len() < found.asked.len() {
            notes.push(tr!("Nothing found for \u{201c}{query}\u{201d}.", query));
        }
        if !found.unreachable.is_empty() {
            notes.push(tr!("Could not reach {sources}.", sources = names(&found.unreachable)));
        }
        if !found.needs_key.is_empty() {
            notes.push(tr!("Needs a key: {sources}.", sources = names(&found.needs_key)));
        }
        self.search_note = (!notes.is_empty()).then(|| notes.join(" "));
        self.choices = by_agreement(found.choices);
        self.picked = (!self.choices.is_empty()).then_some(0);
        cx.notify();
    }

    /// "Use this match": looks the picked game up in its sources and saves it.
    pub(super) fn use_match(&mut self, cx: &mut Context<Self>) {
        let Some(picks) = self.picked.and_then(|i| self.choices.get(i)).map(MatchChoice::picks).filter(|p| !p.is_empty()) else {
            return;
        };
        self.resolve(Busy::Use, Resolution::UseMatch(picks), cx);
    }

    /// "In no source": automatic updates stop asking about the game.
    pub(super) fn in_no_source(&mut self, cx: &mut Context<Self>) {
        self.resolve(Busy::InNoSource, Resolution::InNoSource, cx);
    }

    fn resolve(&mut self, busy: Busy, resolution: Resolution, cx: &mut Context<Self>) {
        let Some(game) = self.game().cloned() else { return };
        if self.busy.is_some() || self.loading {
            return;
        }
        let answer = if busy == Busy::Use { Answer::Matched } else { Answer::InNoSource };
        self.error = None;
        if self.fake.is_some() {
            self.answered(answer, cx);
            return;
        }
        self.busy = Some(busy);
        let task = self.hub.update(cx, |h, cx| h.resolve_review(game.item.clone(), resolution, cx));
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match done {
                    Ok(_) => this.answered(answer, cx),
                    Err(err) => this.error = Some(error_text(&err)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Skip": the game stays in the queue.
    pub(super) fn skip(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_none() && !self.finished {
            self.answered(Answer::Skipped, cx);
        }
    }

    /// Records the answer for the game shown and goes to the next game not
    /// answered yet, or to the end.
    fn answered(&mut self, answer: Answer, cx: &mut Context<Self>) {
        if let Some(a) = self.answers.get_mut(self.at) {
            *a = Some(answer);
        }
        let n = self.games.len();
        match (1..=n).map(|k| (self.at + k) % n).find(|&i| self.answers[i].is_none()) {
            Some(next) => self.go_to(next, cx),
            None => {
                self.finished = true;
                self.load_task = None;
                cx.notify();
            }
        }
    }

    /// Shows game `i` of the review.
    pub(super) fn go_to(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.games.len() || self.busy.is_some() {
            return;
        }
        self.at = i;
        self.clear_search = true;
        self.load(cx);
        cx.notify();
    }
}

/// An error of the review as a sentence: offline and a chosen game that
/// could not be read said plainly.
fn error_text(err: &anyhow::Error) -> String {
    if is_offline(err) {
        return heronite_metadata::service::Offline.to_string();
    }
    if let Some(MatchUnavailable(sources)) = err.downcast_ref::<MatchUnavailable>() {
        return tr!("The chosen game could not be read from {sources}. Try again later, or pick another.", sources = sources.join(", "));
    }
    format!("{err:#}")
}

#[cfg(test)]
mod tests {
    use super::by_agreement;
    use heronite_metadata::{MatchChoice, SourceHit};

    fn choice(title: &str, sources: &[&str]) -> MatchChoice {
        let hits = sources
            .iter()
            .map(|s| SourceHit {
                source: s.to_string(),
                id: Some("1".into()),
                title: Some(title.into()),
                year: None,
                platform: None,
                thumb_url: None,
                matched_by: None,
                alt_title: None,
                thumb_adult: false,
            })
            .collect();
        MatchChoice { title: Some(title.into()), year: None, platform: None, alt_title: None, thumb_url: None, hits }
    }

    #[test]
    fn the_most_agreed_choice_comes_first_and_ties_keep_their_order() {
        let sorted = by_agreement(vec![choice("a", &["igdb"]), choice("b", &["igdb", "steam"]), choice("c", &["lutris"])]);
        let titles: Vec<_> = sorted.iter().map(|c| c.title.clone().unwrap()).collect();
        assert_eq!(titles, ["b", "a", "c"]);
    }
}
