//! The metadata pill of the Collection's header, modelled on the downloads
//! pill. While a bulk update runs: a ring, "Metadata" and its percent.
//! Once it ended, "Metadata · N not found" in amber when some games were not
//! found or failed, else a quiet "Metadata · N updated", until its popover
//! (or the dialog's results) showed it; the next update replaces it. Nothing
//! when idle. Its popover: what runs, Stop, the last results, Undo and "See
//! all" (the dialog on the results).

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkProgress, BulkState, GameResult};

use super::downloads_pill::{pill_base, ring};
use super::header::Menu;
use super::CollectionView;
use crate::dialogs::metadata_update::hub::{hub, HubGlobal};
use crate::dialogs::metadata_update::words::{self, Bucket};
use crate::icons::Icon;
use crate::pages::icon_action;
use crate::services::widgets::{cover_box, progress_bar};
use crate::theme;
use crate::ui::controls::{icon_button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::glass;

/// Results the popover lists.
const POPOVER_RESULTS: usize = 3;

/// What the popover shows, read before its body is built.
struct JobView {
    p: BulkProgress,
    can_undo: bool,
    undoing: bool,
    undone: Option<usize>,
    results: Vec<(Arc<GameResult>, Option<ImageSource>)>,
    now: Option<String>,
}

impl CollectionView {
    /// The Collection key of a result's game (`runner_appName`; a game of
    /// the `zoom` runner is recorded as `sideload`).
    fn result_key(&self, r: &GameResult) -> SharedString {
        let key = format!("{}_{}", r.runner, r.app_name);
        let zoom = format!("zoom_{}", r.app_name);
        match self.lib.as_ref() {
            Some(lib) if lib.get(&key).is_none() && r.runner == "sideload" && lib.get(&zoom).is_some() => zoom.into(),
            _ => key.into(),
        }
    }

    pub(super) fn metadata_pill(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !cx.has_global::<HubGlobal>() {
            return None;
        }
        let hub_entity = hub(cx);
        let open = self.open_menu == Some(Menu::Metadata);
        let (running, done, job, seen, percent, queued) = {
            let p = &hub_entity.read(cx).progress;
            (p.is_running(), p.state == BulkState::Done && p.job != 0, p.job, hub_entity.read(cx).seen, p.percent, p.queued.len())
        };
        if !(running || (done && (job != seen || open))) {
            return None;
        }
        if open && done && job != seen {
            hub_entity.update(cx, |h, _| h.seen = job);
        }
        let label =
            div().flex_none().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("Metadata"));
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
            let (colour, glyph, detail, text) = if problems > 0 {
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
        let view = open.then(|| self.job_view(window, cx));
        Some(self.hover_dropdown(Menu::Metadata, pill, Align::Right, move |_, cx| job_popover(view, cx), cx))
    }

    /// The popover's data: the last results (the ones that need a look,
    /// when the update ended with some) and their covers.
    fn job_view(&mut self, window: &Window, cx: &mut Context<Self>) -> JobView {
        let h = hub(cx).read(cx);
        let p = h.progress.clone();
        let (can_undo, undoing, undone) = (h.can_undo(), h.undoing, h.undone_of(p.job).map(|u| u.games));
        let needs_look = |r: &&Arc<GameResult>| matches!(Bucket::of(&r.outcome), Bucket::NotFound | Bucket::Failed);
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
        JobView { p, can_undo, undoing, undone, results, now }
    }
}

fn result_row(r: &GameResult, cover: Option<ImageSource>) -> Div {
    let line = if r.changes.is_empty() { words::reason(&r.outcome).to_string() } else { format!("+ {}", words::changes_line(&r.changes)) };
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
                .child(div().truncate().text_size(px(13.)).font_weight(theme::W_BOLD).child(r.title.clone()))
                .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(line)),
        )
        .child(div().flex_none().size(px(7.)).rounded_full().bg(Bucket::of(&r.outcome).colour()))
}

/// The pill's popover.
fn job_popover(view: Option<JobView>, cx: &mut Context<CollectionView>) -> AnyElement {
    let Some(v) = view else { return div().into_any_element() };
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
    let queued = (!p.queued.is_empty()).then(|| {
        div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(trn!(
            p.queued.len(),
            "{n} more update waiting",
            "{n} more updates waiting"
        ))
    });
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
                .children(queued)
                .when(p.queued.is_empty(), |d| d.child(div().flex_1()))
                .children(undo)
                .child(see_all),
        )
        .into_any_element()
}
