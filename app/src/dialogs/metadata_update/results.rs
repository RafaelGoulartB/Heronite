//! Step 3, the update: its progress, the results filtered by outcome and the
//! list of games (a `uniform_list`: only the rows in view are built, so a job
//! of 2,000 games stays cheap), each with what changed and from where, or
//! why nothing did; a game sent to review has "Review". Stop and Hide while
//! it runs; Undo, "Review N…", "Update again" and Done once it ended. A job
//! waiting behind another says so.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkItem, BulkProgress, BulkState, GameOutcome, GameResult, JobTally};

use super::hub::ReviewRequest;
use super::words::{self, Bucket};
use super::{footer_note, MetadataUpdate, Step};
use crate::icons::Icon;
use crate::services::widgets::{cover_box, progress_bar};
use crate::theme;
use crate::ui::controls::{button, button_sm, icon_button_sm, ButtonKind};
use crate::ui::icon;

const ROW_H: f32 = 48.;
/// Rows in view before the list scrolls.
const ROWS: f32 = 7.;
/// Changed fields shown on a row before "+N".
const CHIPS: usize = 3;

/// The rows of the filter shown, worked out again only when the job, its
/// number of results or the filter changes (not every frame).
#[derive(Default)]
pub(super) struct Cache {
    key: (u64, usize, Option<Bucket>, BulkState),
    rows: Vec<usize>,
    tally: JobTally,
}

impl MetadataUpdate {
    /// The job the step shows is waiting behind another.
    fn waiting(&self, cx: &App) -> bool {
        let p = &self.hub.read(cx).progress;
        self.started.is_some_and(|id| p.job != id && p.queued.iter().any(|q| q.id == id))
    }

    fn refresh_cache(&mut self, p: &BulkProgress) {
        let key = (p.job, p.results.len(), self.filter, p.state);
        if self.cache.key == key && !(p.job == 0 && self.cache.rows.is_empty() && p.results.is_empty()) {
            return;
        }
        if self.cache.key.0 != p.job || self.cache.key.2 != self.filter {
            self.list.scroll_to_item(0, ScrollStrategy::Top);
        }
        self.cache.key = key;
        self.cache.tally = p.tally();
        self.cache.rows =
            p.results.iter().enumerate().filter(|(_, r)| self.filter.is_none_or(|f| Bucket::of(&r.outcome) == f)).map(|(i, _)| i).collect();
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> Div {
        let t = &self.cache.tally;
        let total: usize = Bucket::ALL.iter().map(|b| b.count(t)).sum();
        let mut pills: Vec<(Option<Bucket>, String)> = vec![(None, tr!("All {n}", n = total))];
        pills.extend(Bucket::ALL.into_iter().filter(|b| b.count(t) > 0).map(|b| (Some(b), format!("{} {}", b.label(), b.count(t)))));
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, (bucket, label)) in pills.into_iter().enumerate() {
            let on = self.filter == bucket;
            row = row.child(
                div()
                    .id(("mu-filter", i))
                    .px(px(11.))
                    .py(px(5.))
                    .rounded_full()
                    .text_size(px(12.5))
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::seg_active()).text_color(theme::text_bright()))
                    .when(!on, |d| d.text_color(theme::text_muted()).hover(|s| s.bg(theme::ink_a(0.06)).text_color(theme::text_primary())))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = bucket;
                        cx.notify();
                    })),
            );
        }
        row
    }

    fn cover_of(&mut self, key: &str, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let chain = self.games.covers.get(key)?;
        self.covers.get_chain(&format!("mu:{key}"), chain, w, h, scale)
    }

    /// The games the job sent to review, in its order.
    fn review_items(&self, cx: &App) -> Vec<BulkItem> {
        let p = &self.hub.read(cx).progress;
        p.results
            .iter()
            .filter(|r| matches!(r.outcome, GameOutcome::ToReview(_)))
            .map(|r| {
                let steam =
                    self.games.all.iter().find(|g| g.app_name == r.app_name && heronite_metadata::store::as_runner(&g.runner) == r.runner);
                BulkItem {
                    app_name: r.app_name.clone(),
                    runner: r.runner.clone(),
                    title: r.title.clone(),
                    steam_app_id: steam.and_then(|g| g.steam_app_id.clone()),
                }
            })
            .collect()
    }

    /// "Review matches" on the job's games sent to review, from `start`.
    fn open_review(&mut self, start: usize, cx: &mut Context<Self>) {
        let games = self.review_items(cx);
        if games.is_empty() {
            return;
        }
        let request = ReviewRequest { start: start.min(games.len() - 1), games: Some(games) };
        self.hub.update(cx, |h, cx| h.request_review(request, cx));
    }

    fn render_row(&mut self, i: usize, r: &GameResult, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let cover = self.cover_of(&r.key, 26., 35., scale);
        let service = self.hub.read(cx).service.clone();
        let middle = if r.changes.is_empty() {
            let reason = div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(words::reason(&r.outcome));
            match &r.outcome {
                GameOutcome::Failed(err) => {
                    div().id(("mu-why", i)).min_w_0().child(reason).tooltip(crate::ui::tooltip(err.clone())).into_any_element()
                }
                _ => reason.into_any_element(),
            }
        } else {
            let more = r.changes.len().saturating_sub(CHIPS);
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .min_w_0()
                .overflow_hidden()
                .children(r.changes.iter().take(CHIPS).map(|c| words::change_chip(c, &service)))
                .when(more > 0, |d| d.child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(format!("+{more}"))))
                .into_any_element()
        };
        div()
            .h(px(ROW_H))
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .border_b_1()
            .border_color(theme::row_separator())
            .child(cover_box(cover, 26., 35.).rounded(px(4.)))
            .child(div().w(px(210.)).flex_none().truncate().text_size(px(13.5)).text_color(theme::text_primary()).child(r.title.clone()))
            .child(div().flex().flex_1().min_w_0().child(middle))
            .child(match r.outcome {
                GameOutcome::ToReview(_) => {
                    let key = r.key.clone();
                    button_sm(("mu-review", i), ButtonKind::Glass, tr!("Review"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let at = this.review_index(&key, cx);
                            this.open_review(at, cx);
                        }))
                        .into_any_element()
                }
                _ => words::status(&r.outcome).into_any_element(),
            })
            .into_any_element()
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let results: Vec<(usize, Arc<GameResult>)> = {
            let p = &self.hub.read(cx).progress;
            range.filter_map(|row| self.cache.rows.get(row).and_then(|i| p.results.get(*i).map(|r| (row, r.clone())))).collect()
        };
        let scale = window.scale_factor();
        results.into_iter().map(|(row, r)| self.render_row(row, &r, scale, cx)).collect()
    }

    /// Where the game of `key` is among the job's games sent to review.
    fn review_index(&self, key: &str, cx: &App) -> usize {
        let p = &self.hub.read(cx).progress;
        p.results.iter().filter(|r| matches!(r.outcome, GameOutcome::ToReview(_))).position(|r| r.key == key).unwrap_or(0)
    }

    /// The game being fetched, above the list while the job runs.
    fn render_now(&mut self, p: &BulkProgress, window: &Window) -> Option<Div> {
        let title = p.current_title.clone().filter(|_| p.is_running())?;
        let cover = self.now_key(p).and_then(|k| self.cover_of(&k, 26., 35., window.scale_factor()));
        let word = if p.state == BulkState::Cancelling { tr!("Stopping after this game\u{2026}") } else { tr!("Fetching\u{2026}") };
        Some(
            div()
                .h(px(ROW_H))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .rounded(px(10.))
                .bg(theme::accent_wash())
                .child(cover_box(cover, 26., 35.).rounded(px(4.)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.5)).text_color(theme::text_primary()).child(title))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(7.))
                        .child(icon(Icon::FaSpinner, px(12.), theme::accent()))
                        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(word)),
                ),
        )
    }

    /// The metadata key of the game being fetched (the progress names it by
    /// its title only).
    fn now_key(&self, p: &BulkProgress) -> Option<String> {
        let title = p.current_title.as_deref()?;
        let g = self.games.all.iter().find(|g| g.title == title)?;
        Some(heronite_data::library::metadata_key(&g.runner, &g.app_name))
    }

    pub(super) fn render_results(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.hub.read(cx).progress.clone();
        if p.state == BulkState::Done && self.hub.read(cx).seen != p.job {
            self.hub.update(cx, |h, _| h.seen = p.job);
        }
        if self.waiting(cx) {
            let left = p.left();
            return div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(24.))
                .items_center()
                .child(icon(Icon::AccessTime, px(28.), theme::text_muted()))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Waiting for the update running now")))
                .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(trn!(
                    left,
                    "It starts after {n} more game.",
                    "It starts after {n} more games."
                )));
        }
        if p.job == 0 {
            return div().py(px(24.)).text_size(px(13.)).text_color(theme::text_dim()).child(tr!("No update has run yet."));
        }
        self.refresh_cache(&p);
        let running = p.is_running();
        let t = self.cache.tally;
        let colour = if running {
            theme::accent()
        } else if words::problems(&t) > 0 {
            theme::warning()
        } else {
            theme::success()
        };
        let head = if running {
            let done = p.results.len();
            trn!(p.total, "{done} of {n} game", "{done} of {n} games", done)
        } else {
            trn!(p.total, "Updated {updated} of {n} game", "Updated {updated} of {n} games", updated = t.updated)
        };
        let queued = p.queued.len();
        let head_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_muted()).child(head))
            .when(queued > 0, |d| {
                d.child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(trn!(
                    queued,
                    "{n} more update waiting",
                    "{n} more updates waiting"
                )))
                .child(
                    div()
                        .id("mu-clear-queue")
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::text_bright()))
                        .child(tr!("Clear the queue"))
                        .on_click(cx.listener(|this, _, _, cx| this.hub.update(cx, |h, cx| h.clear_queue(cx)))),
                )
            });
        let rows = self.cache.rows.len();
        let list_h = (rows as f32).clamp(1., ROWS) * ROW_H;
        let list = if rows == 0 {
            div().h(px(ROW_H)).flex().items_center().px(px(12.)).text_size(px(12.5)).text_color(theme::text_dim()).child(if running {
                tr!("The first results appear here.")
            } else {
                tr!("No games here.")
            })
        } else {
            div().h(px(list_h)).child(
                uniform_list("mu-results", rows, cx.processor(|this, range: Range<usize>, window, cx| this.render_rows(range, window, cx)))
                    .size_full()
                    .track_scroll(&self.list),
            )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(head_row)
            .child(progress_bar(Some(if running { p.percent as f32 } else { 100. }), 5., colour))
            .child(self.render_filters(cx))
            .children(self.render_now(&p, window))
            .child(
                div()
                    .rounded(px(theme::R_CARD))
                    .bg(theme::card_fill())
                    .border_1()
                    .border_color(theme::card_border())
                    .overflow_hidden()
                    .child(list),
            )
    }

    pub(super) fn results_footer(&self, cx: &mut Context<Self>) -> Div {
        let row = div().flex().flex_1().items_center().gap(px(10.));
        let hide = button("mu-hide", ButtonKind::Primary, tr!("Hide")).on_click(cx.listener(|this, _, _, cx| this.hide(cx)));
        if self.waiting(cx) {
            return row
                .child(footer_note(tr!("The update keeps going when this is hidden")))
                .child(button("mu-unqueue", ButtonKind::Glass, tr!("Clear the queue")).on_click(cx.listener(|this, _, _, cx| {
                    this.hub.update(cx, |h, cx| h.clear_queue(cx));
                    this.started = None;
                    this.go(Step::What, cx);
                })))
                .child(hide);
        }
        let hub = self.hub.read(cx);
        let p = &hub.progress;
        if p.is_running() {
            let stopping = p.state == BulkState::Cancelling;
            return row
                .child(footer_note(tr!("The update keeps going when this is hidden")))
                .child(
                    icon_button_sm(
                        "mu-stop",
                        ButtonKind::Glass,
                        Icon::Stop,
                        if stopping { tr!("Stopping\u{2026}") } else { tr!("Stop") },
                        !stopping,
                        cx.listener(|this, _, _, cx| this.hub.update(cx, |h, cx| h.cancel(cx))),
                    )
                    .h(px(theme::BUTTON_H)),
                )
                .child(hide);
        }
        let undone = hub.undone_of(p.job);
        let updated = self.cache.tally.updated;
        if self.confirm_undo && hub.can_undo() {
            return row
                .child(div().flex_1().min_w_0().text_size(px(12.5)).text_color(theme::caution_text()).child(trn!(
                    updated,
                    "Put back what this update changed in {n} game? Changes made since stay.",
                    "Put back what this update changed in {n} games? Changes made since stay."
                )))
                .child(button("mu-undo-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_undo = false;
                    cx.notify();
                })))
                .child(button("mu-undo-yes", ButtonKind::Danger, tr!("Undo")).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_undo = false;
                    this.hub.update(cx, |h, cx| h.undo(cx));
                })));
        }
        let note = if hub.undoing {
            tr!("Undoing\u{2026}").to_string()
        } else if let Some(u) = undone {
            trn!(u.games, "Undone: {n} game put back", "Undone: {n} games put back")
        } else if p.job != 0 {
            words::done_line(p)
        } else {
            String::new()
        };
        let can_undo = hub.can_undo() && undone.is_none() && updated > 0;
        let to_review = self.cache.tally.to_review;
        row.child(footer_note(note))
            .when(can_undo, |d| {
                d.child(
                    icon_button_sm(
                        "mu-undo",
                        ButtonKind::Ghost,
                        Icon::Undo,
                        tr!("Undo this update"),
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.confirm_undo = true;
                            cx.notify();
                        }),
                    )
                    .h(px(theme::BUTTON_H)),
                )
            })
            .when(to_review > 0, |d| {
                d.child(
                    button("mu-review-all", ButtonKind::Glass, trn!(to_review, "Review {n}\u{2026}", "Review {n}\u{2026}"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_review(0, cx))),
                )
            })
            .child(button("mu-again", ButtonKind::Glass, tr!("Update again")).on_click(cx.listener(|this, _, _, cx| {
                this.started = None;
                this.go(Step::What, cx);
            })))
            .child(button("mu-done", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|this, _, _, cx| this.hide(cx))))
    }
}
