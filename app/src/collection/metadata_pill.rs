//! The metadata pill of the Collection's header, modelled on the downloads
//! pill. While a bulk update runs: a ring, "Metadata" and its percent.
//! Once it ended, "Metadata · N to review" in amber when it sent games to
//! review, "· N not found" when some games were not found or failed, else a
//! quiet "Metadata · N updated", until its popover (or the dialog's results,
//! or the review) showed it; the next update replaces it. When the idle fill
//! queued games, "Metadata · N to review" too, until the review or the
//! popover opens. Nothing otherwise. Its popover: what runs, Stop, the last
//! results, Undo, "See all" (the dialog on the results) and "Review".

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkProgress, BulkState, GameResult};

use super::downloads_pill::{pill_base, ring};
use super::header::Menu;
use super::CollectionView;
use crate::dialogs::metadata_update::hub::{hub, HubGlobal, ReviewRequest};
use crate::dialogs::metadata_update::words::{self, Bucket};
use crate::icons::Icon;
use crate::pages::icon_action;
use crate::services::widgets::{cover_box, progress_bar};
use crate::theme;
use crate::ui::controls::{button_sm, icon_button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::glass;

/// Results the popover lists.
const POPOVER_RESULTS: usize = 3;

/// What the popover shows, read before its body is built.
enum PillView {
    Job(Box<JobView>),
    /// The idle fill queued games: the first ones, their covers, how many.
    Idle(Vec<(String, &'static str, Option<ImageSource>)>, usize),
}

struct JobView {
    p: BulkProgress,
    can_undo: bool,
    undoing: bool,
    undone: Option<usize>,
    results: Vec<(Arc<GameResult>, Option<ImageSource>)>,
    now: Option<String>,
    /// The games the job sent to review.
    review: Vec<heronite_metadata::BulkItem>,
}

impl CollectionView {
    /// The Collection key of a result's game (`runner_appName`; a game of
    /// the `zoom` runner is recorded as `sideload`).
    fn result_key(&self, r: &GameResult) -> SharedString {
        self.collection_key(&r.runner, &r.app_name)
    }

    /// The Collection key of a game a metadata record names
    /// (`runner_appName`; a game of the `zoom` runner is recorded as
    /// `sideload`).
    pub(super) fn collection_key(&self, runner: &str, app_name: &str) -> SharedString {
        let key = format!("{runner}_{app_name}");
        let zoom = format!("zoom_{app_name}");
        match self.lib.as_ref() {
            Some(lib) if lib.get(&key).is_none() && runner == "sideload" && lib.get(&zoom).is_some() => zoom.into(),
            _ => key.into(),
        }
    }

    pub(super) fn metadata_pill(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !cx.has_global::<HubGlobal>() {
            return None;
        }
        let hub_entity = hub(cx);
        let open = self.open_menu == Some(Menu::Metadata);
        let (running, done, job, seen, percent, queued, idle, waiting) = {
            let h = hub_entity.read(cx);
            let p = &h.progress;
            let done = p.state == BulkState::Done && p.job != 0;
            (p.is_running(), done, p.job, h.seen, p.percent, p.queued.len(), h.idle_review, h.queue.len())
        };
        let job_shown = running || (done && (job != seen || open));
        if !job_shown && idle {
            return Some(self.idle_pill(open, waiting, window, cx));
        }
        if !job_shown {
            return None;
        }
        if open && done && job != seen {
            hub_entity.update(cx, |h, _| h.seen = job);
        }
        let label = pill_label();
        let pill = if running {
            pill_base("hdr-metadata", open)
                .child(ring(percent as f32, theme::accent().into(), None))
                .child(label)
                .child(div().flex_none().min_w(px(30.)).text_size(px(12.)).text_color(theme::text_dim()).child(format!("{percent}%")))
                .when(queued > 0, |d| {
                    d.child(div().flex_none().pr(px(4.)).text_size(px(12.)).text_color(theme::text_dim()).child(format!("+{queued}")))
                })
        } else {
            let t = hub_entity.read(cx).progress.tally();
            let problems = words::problems(&t);
            let (colour, glyph, detail, text) = if t.to_review > 0 {
                (theme::warning(), Icon::Warning, trn!(t.to_review, "{n} to review", "{n} to review"), theme::caution_text())
            } else if problems > 0 {
                let detail = if t.not_found == problems {
                    trn!(problems, "{n} not found", "{n} not found")
                } else if t.not_found == 0 {
                    trn!(problems, "{n} failed", "{n} failed")
                } else {
                    trn!(problems, "{n} needs a look", "{n} need a look")
                };
                (theme::warning(), Icon::Warning, detail, theme::caution_text())
            } else if t.updated > 0 {
                (theme::success(), Icon::Check, trn!(t.updated, "{n} updated", "{n} updated"), theme::text_dim())
            } else {
                (theme::success(), Icon::Check, tr!("nothing new").to_string(), theme::text_dim())
            };
            pill_base("hdr-metadata", open)
                .when(problems > 0, |d| d.border_color(Hsla { a: 0.4, ..colour.into() }))
                .child(ring(100., colour.into(), Some(glyph)))
                .child(label)
                .child(div().flex_none().pr(px(4.)).text_size(px(12.)).text_color(text).child(format!("\u{b7} {detail}")))
        };
        let view = open.then(|| PillView::Job(Box::new(self.job_view(window, cx))));
        Some(self.hover_dropdown(Menu::Metadata, pill, Align::Right, move |_, cx| pill_popover(view, cx), cx))
    }

    /// "Metadata · N to review" after the idle fill queued games. Opening
    /// its popover counts as seen: the pill goes once it closes.
    fn idle_pill(&mut self, open: bool, waiting: usize, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let hub_entity = hub(cx);
        if open {
            self.metadata_idle_opened = true;
        } else if self.metadata_idle_opened {
            self.metadata_idle_opened = false;
            hub_entity.update(cx, |h, cx| h.review_seen(cx));
        }
        let colour = theme::warning();
        let pill = pill_base("hdr-metadata", open)
            .border_color(Hsla { a: 0.4, ..colour.into() })
            .child(ring(100., colour.into(), Some(Icon::Warning)))
            .child(pill_label())
            .child(
                div()
                    .flex_none()
                    .pr(px(4.))
                    .text_size(px(12.))
                    .text_color(theme::caution_text())
                    .child(format!("\u{b7} {}", trn!(waiting, "{n} to review", "{n} to review"))),
            );
        let view = open.then(|| {
            let scale = window.scale_factor();
            let items: Vec<_> = hub_entity.read(cx).queue.iter().take(POPOVER_RESULTS).cloned().collect();
            let rows = items
                .into_iter()
                .map(|q| {
                    let key: SharedString = self.collection_key(&q.runner, &q.app_name);
                    let cover = self.cover(&key, 26., 35., scale, 40);
                    (q.title, words::review_short(q.reason), cover)
                })
                .collect();
            PillView::Idle(rows, waiting)
        });
        self.hover_dropdown(Menu::Metadata, pill, Align::Right, move |_, cx| pill_popover(view, cx), cx)
    }

    /// The popover's data: the last results (the ones that need a look,
    /// when the update ended with some) and their covers.
    fn job_view(&mut self, window: &Window, cx: &mut Context<Self>) -> JobView {
        let h = hub(cx).read(cx);
        let p = h.progress.clone();
        let (can_undo, undoing, undone) = (h.can_undo(), h.undoing, h.undone_of(p.job).map(|u| u.games));
        let needs_look = |r: &&Arc<GameResult>| matches!(Bucket::of(&r.outcome), Bucket::ToReview | Bucket::NotFound | Bucket::Failed);
        let picked: Vec<Arc<GameResult>> = if !p.is_running() && p.results.iter().any(|r| needs_look(&r)) {
            p.results.iter().filter(needs_look).take(POPOVER_RESULTS).cloned().collect()
        } else {
            p.results.iter().rev().filter(|r| Bucket::of(&r.outcome) != Bucket::NotDone).take(POPOVER_RESULTS).cloned().collect()
        };
        let scale = window.scale_factor();
        let results = picked
            .into_iter()
            .map(|r| {
                let key = self.result_key(&r);
                let cover = self.cover(&key, 26., 35., scale, 40);
                (r, cover)
            })
            .collect();
        let now = p.current_title.clone().filter(|_| p.is_running());
        let review = self.review_items_of(&p);
        JobView { p, can_undo, undoing, undone, results, now, review }
    }
}

fn result_row(r: &GameResult, cover: Option<ImageSource>) -> Div {
    let line = if r.changes.is_empty() { words::reason(&r.outcome).to_string() } else { format!("+ {}", words::changes_line(&r.changes)) };
    game_row(r.title.clone(), line, cover, Bucket::of(&r.outcome).colour())
}

fn game_row(title: String, line: String, cover: Option<ImageSource>, dot: Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(5.))
        .child(cover_box(cover, 26., 35.).rounded(px(4.)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(13.)).font_weight(theme::W_BOLD).child(title))
                .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(line)),
        )
        .child(div().flex_none().size(px(7.)).rounded_full().bg(dot))
}

fn pill_label() -> Div {
    div().flex_none().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("Metadata"))
}

/// "Review": "Review matches" on the queue (`None`) or on the job's games.
fn review_button(games: Option<Vec<heronite_metadata::BulkItem>>, cx: &mut Context<CollectionView>) -> Stateful<Div> {
    button_sm("hdr-md-review", ButtonKind::Primary, tr!("Review")).on_click(cx.listener(move |this, _, _, cx| {
        this.open_menu = None;
        let request = ReviewRequest { games: games.clone(), start: 0 };
        hub(cx).update(cx, |h, cx| h.request_review(request, cx));
    }))
}

/// "N to review" with its amber dot, at the start of the popover's last row.
fn to_review_note(n: usize) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(7.))
        .child(div().flex_none().size(px(7.)).rounded_full().bg(theme::warning()))
        .child(div().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(trn!(n, "{n} to review", "{n} to review")))
}

/// The pill's popover.
fn pill_popover(view: Option<PillView>, cx: &mut Context<CollectionView>) -> AnyElement {
    match view {
        Some(PillView::Job(v)) => job_popover(*v, cx),
        Some(PillView::Idle(rows, n)) => idle_popover(rows, n, cx),
        None => div().into_any_element(),
    }
}

/// The idle fill's games to review.
fn idle_popover(rows: Vec<(String, &'static str, Option<ImageSource>)>, n: usize, cx: &mut Context<CollectionView>) -> AnyElement {
    let head = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().truncate().text_size(px(14.)).font_weight(theme::W_BOLD).child(trn!(n, "{n} game to review", "{n} games to review")))
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("The sources were not sure which game these are.")));
    let mut list = div().flex().flex_col();
    for (title, why, cover) in rows {
        list = list.child(game_row(title, why.to_string(), cover, theme::warning().into()));
    }
    glass::glass_panel(theme::R_POPOVER)
        .w(px(360.))
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(head)
        .child(list)
        .child(div().flex().items_center().gap(px(8.)).mt(px(2.)).child(to_review_note(n)).child(review_button(None, cx)))
        .into_any_element()
}

/// The popover of a bulk update.
fn job_popover(v: JobView, cx: &mut Context<CollectionView>) -> AnyElement {
    let p = &v.p;
    let running = p.is_running();
    let t = p.tally();
    let (title, line) = if running {
        let done = trn!(p.results.len(), "{n} done", "{n} done");
        let line = match (&v.now, p.state) {
            (_, BulkState::Cancelling) => tr!("Stopping after this game\u{2026}").to_string(),
            (Some(now), _) => format!("{done} \u{b7} {}", tr!("now: {title}", title = now.as_str())),
            (None, _) => done,
        };
        (trn!(p.total, "Updating {n} game", "Updating {n} games"), line)
    } else {
        let title = trn!(p.total, "Updated {updated} of {n} game", "Updated {updated} of {n} games", updated = t.updated);
        let line = match v.undone {
            Some(n) => trn!(n, "Undone: {n} game put back", "Undone: {n} games put back"),
            None => words::done_line(p),
        };
        (title, line)
    };
    let colour = if running {
        theme::accent()
    } else if words::problems(&t) > 0 {
        theme::warning()
    } else {
        theme::success()
    };
    let stop = (running && p.state != BulkState::Cancelling).then(|| {
        icon_action("hdr-md-stop", Icon::Stop, true).tooltip(crate::ui::tooltip(tr!("Stop"))).on_click(cx.listener(|_, _, _, cx| {
            hub(cx).update(cx, |h, cx| h.cancel(cx));
        }))
    });
    let head = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().truncate().text_size(px(14.)).font_weight(theme::W_BOLD).child(title))
                .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(line)),
        )
        .children(stop);
    let mut panel =
        glass::glass_panel(theme::R_POPOVER).w(px(360.)).p(px(14.)).flex().flex_col().gap(px(8.)).child(head).child(progress_bar(
            Some(if running { p.percent as f32 } else { 100. }),
            4.,
            colour,
        ));
    let mut list = div().flex().flex_col();
    for (r, cover) in &v.results {
        list = list.child(result_row(r, cover.clone()));
    }
    panel = panel.child(list);
    let to_review = (!running && t.to_review > 0).then(|| v.review.clone());
    let queued = (!p.queued.is_empty()).then(|| {
        div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(trn!(
            p.queued.len(),
            "{n} more update waiting",
            "{n} more updates waiting"
        ))
    });
    let note = match (&to_review, queued) {
        (Some(_), _) => Some(to_review_note(t.to_review)),
        (None, queued) => queued,
    };
    let undo = (v.can_undo && v.undone.is_none() && !v.undoing && t.updated > 0).then(|| {
        icon_button_sm("hdr-md-undo", ButtonKind::Ghost, Icon::Undo, tr!("Undo"), true, |_, _, cx| {
            hub(cx).update(cx, |h, cx| h.undo(cx));
        })
    });
    let see_all = icon_button_sm("hdr-md-all", ButtonKind::Glass, Icon::Checklist, tr!("See all"), true, {
        let view = cx.entity().downgrade();
        move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.open_menu = None;
                this.open_metadata_results(window, cx);
            })
            .ok();
        }
    });
    panel
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(2.))
                .when(note.is_none(), |d| d.child(div().flex_1()))
                .children(note)
                .children(undo)
                .child(see_all)
                .children(to_review.map(|games| review_button(Some(games), cx))),
        )
        .into_any_element()
}
