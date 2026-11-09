//! The two lists under the stage: what waits (drag a row to reorder it) and
//! what finished (with Play, Retry and a filter for the failures).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::DmState;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind};
use heronite_i18n::tr;

use super::timing::{clock_in, relative_date, size_bytes, size_text};
use super::{failure_reason, meta_line, play_key, poster, DownloadsPage, History, Overview};
use crate::icons::Icon;
use crate::pages::{caption_with, empty_row, icon_action, status_pill, PageEvent};
use crate::services::widgets::duration_text;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, icon_button_sm, ButtonKind};
use crate::ui::{icon, tooltip};

/// A waiting row being dragged onto another one.
#[derive(Clone)]
struct QueueDrag {
    app: String,
    title: SharedString,
}

/// What follows the pointer while a row is dragged.
struct QueueGhost(SharedString);

impl Render for QueueGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::glass::glass_menu_surface(theme::R_ITEM)
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(13.))
            .text_color(theme::text_primary())
            .child(icon(Icon::DragIndicator, px(16.), theme::text_dim()))
            .child(self.0.clone())
    }
}

impl DownloadsPage {
    pub(super) fn render_queue(&mut self, o: &Overview, window: &Window, cx: &mut Context<Self>) -> Div {
        let scale = window.scale_factor();
        let waiting = &o.snap.queue[o.snap.queue.len().min(1)..];
        let total: f64 = waiting.iter().filter_map(size_bytes).sum();
        let mut col = div();
        if waiting.is_empty() {
            col = col.child(caption(tr!("Up next")));
            let text = if o.snap.queue.is_empty() {
                tr!("The queue is empty. Install or update a game and it shows up here.")
            } else {
                tr!("Nothing after this one. Games you install next line up here, in order.")
            };
            return col.child(card([empty_row(text).into_any_element()]));
        }
        let mut meta = vec![size_text(total)];
        if let Some(last) = o.starts.last().copied().flatten().zip(waiting.last().and_then(size_bytes)) {
            let all = last.0 + (last.1 / (o.speed.max(0.01) * 1024. * 1024.)) as u64;
            meta.push(tr!("all done around {time}", time = clock_in(all)));
        }
        col = col.child(caption_with(
            tr!("Up next \u{b7} {count}", count = waiting.len()),
            div().mx(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(meta.join(" \u{b7} ")),
        ));
        let running = o.snap.state == DmState::Running;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, el) in waiting.iter().enumerate() {
            // Its place in the whole queue (0 runs).
            let index = i + 1;
            let app = el.params.app_name.clone();
            let title: SharedString = el.params.title().into();
            let group: SharedString = format!("dm-q-{app}").into();
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let when = match o.starts.get(i).copied().flatten() {
                Some(secs) if running => div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(div().text_size(px(12.5)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(clock_in(secs)))
                    .child(
                        div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("starts in {time}", time = duration_text(secs))),
                    ),
                _ => div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Waiting")),
            };
            let (a_top, a_rm, a_drop) = (app.clone(), app.clone(), app.clone());
            let actions = div()
                .flex()
                .items_center()
                .gap(px(4.))
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .when(index > 1, |d| {
                    d.child(
                        icon_action(SharedString::from(format!("dm-top-{app}")), Icon::ArrowUpward, true)
                            .tooltip(tooltip(tr!("Download next")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.svc.update(cx, |s, _| s.move_in_queue_to(&a_top, 1));
                            })),
                    )
                })
                .child(
                    icon_action(SharedString::from(format!("dm-rm-{app}")), Icon::Close, true)
                        .tooltip(tooltip(tr!("Remove from the queue")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.svc.update(cx, |s, _| s.remove_from_queue(&a_rm));
                        })),
                );
            let drag = QueueDrag { app: app.clone(), title: title.clone() };
            let row = div()
                .id(SharedString::from(format!("dm-q-{app}")))
                .group(group)
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .rounded(px(theme::R_ITEM))
                .cursor(CursorStyle::OpenHand)
                .on_drag(drag, |d, _, _, cx| cx.new(|_| QueueGhost(d.title.clone())))
                .drag_over::<QueueDrag>(|s, _, _, _| s.bg(theme::accent_alpha(0.1)))
                .on_drop(cx.listener(move |this, d: &QueueDrag, _, cx| {
                    if d.app != a_drop {
                        this.svc.update(cx, |s, _| s.move_in_queue_to(&d.app, index));
                    }
                }))
                .child(icon(Icon::DragIndicator, px(18.), theme::ink_a(0.3)))
                .child(
                    div()
                        .w(px(18.))
                        .flex_none()
                        .text_center()
                        .font_family(theme::font_heading())
                        .text_size(px(12.))
                        .text_color(theme::text_dim())
                        .child(index.to_string()),
                )
                .child(poster(cover, 36., 48., 5.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div().flex().items_center().gap(px(8.)).min_w_0().child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(theme::text_primary())
                                    .child(title),
                            ),
                        )
                        .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(meta_line(el))),
                )
                .child(actions)
                .child(div().flex_none().min_w(px(92.)).flex().justify_end().child(when));
            rows.push(row.into_any_element());
        }
        col.child(card(rows))
    }

    pub(super) fn render_history(&mut self, o: &Overview, window: &Window, cx: &mut Context<Self>) -> Div {
        let scale = window.scale_factor();
        let mut finished: Vec<&DmElement> = o.snap.finished.iter().collect();
        finished.sort_by_key(|e| std::cmp::Reverse(e.end_time));
        if finished.is_empty() {
            return div()
                .child(caption(tr!("History")))
                .child(card([empty_row(tr!("Finished installs and updates show up here, with a way to play them.")).into_any_element()]));
        }
        let failed = finished.iter().filter(|e| e.status != Some(DmStatus::Done)).count();
        if failed == 0 && self.history == History::Failed {
            self.history = History::All;
        }
        let chip = |id: &'static str, label: String, which: History, cx: &mut Context<Self>| {
            let on = self.history == which;
            div()
                .id(id)
                .px(px(9.))
                .py(px(3.))
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(if which == History::Failed { theme::danger_fill() } else { theme::ink_a(0.1) })
                        .font_weight(theme::W_BOLD)
                        .text_color(if which == History::Failed { theme::danger_text() } else { theme::text_primary() })
                })
                .when(!on, |d| d.text_color(theme::text_dim()).hover(|s| s.bg(theme::ink_a(0.06))))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.history = which;
                    cx.notify();
                }))
        };
        let chips = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(chip("dm-h-all", tr!("All \u{b7} {count}", count = finished.len()), History::All, cx))
            .child(chip("dm-h-done", tr!("Done").to_string(), History::Done, cx))
            .when(failed > 0, |d| d.child(chip("dm-h-failed", tr!("Did not finish \u{b7} {count}", count = failed), History::Failed, cx)));
        let head = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(crate::ui::controls::CAPTION_H))
            .mt(px(14.))
            .mb(px(6.))
            .child(div().mx(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).child(tr!("History")))
            .child(chips)
            .child(div().flex_1())
            .child(button_sm("dm-clear", ButtonKind::Ghost, tr!("Clear list")).on_click(cx.listener(|this, _, _, cx| {
                this.svc.update(cx, |s, _| s.clear_finished());
            })));

        let mut rows: Vec<AnyElement> = Vec::new();
        for el in finished.into_iter().filter(|e| match self.history {
            History::All => true,
            History::Done => e.status == Some(DmStatus::Done),
            History::Failed => e.status != Some(DmStatus::Done),
        }) {
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let app = el.params.app_name.clone();
            let queued = o.snap.queue.iter().any(|q| q.params.app_name == app);
            let pill = match el.status {
                Some(DmStatus::Done) => Some(status_pill(tr!("Done"), theme::success())),
                Some(DmStatus::Error) => Some(status_pill(tr!("Failed"), theme::danger_text())),
                Some(DmStatus::Abort) => Some(status_pill(tr!("Canceled"), theme::text_dim())),
                _ => None,
            };
            let reason = (el.status == Some(DmStatus::Error)).then(|| el.error.as_deref().map(failure_reason)).flatten();
            let no_space = el.error.as_deref().is_some_and(|e| e.to_ascii_lowercase().contains("disk space"));
            let action = match el.status {
                Some(DmStatus::Done) if el.kind == ElementKind::Install => {
                    let key = play_key(el);
                    Some(
                        icon_button_sm(
                            SharedString::from(format!("dm-play-{app}")),
                            ButtonKind::Glass,
                            Icon::PlayArrow,
                            tr!("Play"),
                            true,
                            {
                                let page = cx.entity().downgrade();
                                move |_, _, cx| {
                                    page.update(cx, |_, cx| cx.emit(PageEvent::Play(key.clone()))).ok();
                                }
                            },
                        )
                        .into_any_element(),
                    )
                }
                // Queued again already: nothing to retry.
                Some(DmStatus::Error | DmStatus::Abort) if !queued => {
                    let label = if el.status == Some(DmStatus::Error) { tr!("Retry") } else { tr!("Download again") };
                    let retry = app.clone();
                    Some(
                        icon_button_sm(SharedString::from(format!("dm-retry-{app}")), ButtonKind::Glass, Icon::Refresh, label, true, {
                            let svc = self.svc.downgrade();
                            move |_, _, cx| {
                                svc.update(cx, |s, cx| s.retry(&retry, cx)).ok();
                            }
                        })
                        .into_any_element(),
                    )
                }
                _ => None,
            };
            let group: SharedString = format!("dm-h-{app}").into();
            let clear = icon_action(SharedString::from(format!("dm-clear-{app}")), Icon::Close, true)
                .tooltip(tooltip(tr!("Remove from the list")))
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.svc.update(cx, |s, _| s.clear_finished_item(&app));
                }));
            let row = div()
                .group(group)
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(11.))
                .child(poster(cover, 36., 48., 5.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(14.))
                                        .font_weight(theme::W_BOLD)
                                        .text_color(theme::text_primary())
                                        .child(el.params.title()),
                                )
                                .children(pill),
                        )
                        .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(format!(
                            "{} \u{b7} {}",
                            meta_line(el),
                            relative_date(el.end_time)
                        )))
                        .children(
                            reason
                                .map(|r| div().line_clamp(2).text_ellipsis().text_size(px(12.)).text_color(theme::danger_text()).child(r)),
                        )
                        .when(no_space, |d| {
                            d.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::text_muted())
                                    .child(tr!("Free some space on that drive, or install it in another folder.")),
                            )
                        }),
                )
                .child(div().flex().flex_none().items_center().gap(px(6.)).children(action).child(clear));
            rows.push(row.into_any_element());
        }
        if rows.is_empty() {
            rows.push(empty_row(tr!("Nothing with this filter.")).into_any_element());
        }
        div().child(head).child(card(rows))
    }
}
