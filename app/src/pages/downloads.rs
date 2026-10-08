//! Downloads page: the running install/update with its progress, the queue
//! (reorder / remove) and the history, backed by the Epic download manager.

use chrono::{Local, TimeZone};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::{DmState, Snapshot};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind};

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::icons::Icon;
use crate::services::widgets::{cover_box, fmt_mib, fmt_percent, progress_bar, Covers};
use crate::services::{self, Downloads};
use crate::theme;
use crate::ui::controls::{button, section_title, ButtonKind};
use crate::ui::icon;

pub struct DownloadsPage {
    ctx: PageCtx,
    svc: Entity<Downloads>,
    covers: Covers,
    /// "Stop installation" question open for the running element.
    confirm_stop: bool,
}

impl EventEmitter<PageEvent> for DownloadsPage {}

impl DownloadsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let covers = Covers::new(&ctx.heroic, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        Self { ctx, svc, covers, confirm_stop: false }
    }
}

fn card() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .p(px(12.))
        .rounded(px(8.))
        .bg(theme::white_a(0.05))
        .border_1()
        .border_color(theme::white_a(0.08))
}

fn small_button(id: impl Into<ElementId>, ic: Icon, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.12))))
        .when(!enabled, |d| d.opacity(0.3))
        .child(icon(ic, px(18.), rgb(0xcaf3fd)))
}

fn meta_line(el: &DmElement) -> String {
    let kind = if el.kind == ElementKind::Install { "Install" } else { "Update" };
    let platform = if el.params.platform_to_install.is_empty() { "Windows" } else { el.params.platform_to_install.as_str() };
    let size = el.params.size.clone().unwrap_or_else(|| "?? MB".into());
    format!("{kind} | {platform} | {size}")
}

fn date_text(ms: u64) -> String {
    Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

fn progress_text(p: &Progress) -> String {
    format!(
        "{} - {} downloaded - {:.2} MiB/s download - {:.2} MiB/s disk - ETA {}",
        fmt_percent(p.percent),
        fmt_mib(p.downloaded_mib),
        p.down_speed,
        p.disk_speed,
        if p.eta.is_empty() { "--:--:--" } else { p.eta.as_str() }
    )
}

impl Render for DownloadsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.covers.drop_evicted(window);
        let scale = window.scale_factor();
        let svc = self.svc.read(cx);
        let snap: Snapshot = svc.snapshot().clone();
        let progress = snap.current.as_ref().and_then(|(app, _)| svc.progress_of(app).cloned());
        let running = snap.state == DmState::Running;
        let paused = !running;

        let mut column = page_column().gap(px(14.));

        // Running element.
        if let Some(cur) = snap.queue.first() {
            let title = cur.params.title();
            let cover = self.covers.get(cur.params.cover().as_deref(), 60., 80., scale);
            let percent = progress.as_ref().map(|p| p.percent as f32);
            let status = match snap.state {
                DmState::Running => "Downloading",
                DmState::Paused => "Paused",
                DmState::Idle => "Waiting",
            };
            let mut info = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(10.))
                        .child(div().text_size(px(17.)).font_weight(theme::W_BOLD).text_color(white()).child(title.clone()))
                        .child(div().text_size(px(12.5)).text_color(if paused { theme::accent() } else { theme::success() }).child(status)),
                )
                .child(div().text_size(px(12.5)).text_color(theme::white_a(0.7)).child(meta_line(cur)))
                .child(progress_bar(percent, 8., if paused { theme::accent() } else { theme::success() }))
                .child(div().text_size(px(12.5)).text_color(theme::white_a(0.82)).child(match &progress {
                    Some(p) => progress_text(p),
                    None if snap.state == DmState::Paused => "Paused - resume to continue where it stopped".to_string(),
                    None if paused => "Press play to start the queue".to_string(),
                    None => "Starting...".to_string(),
                }));
            if self.confirm_stop {
                info = info.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .mt(px(4.))
                        .child(div().text_size(px(13.33)).text_color(rgb(0xcaf3fd)).child("Stop the installation? Keep the downloaded files?"))
                        .child(button("dm-keep-running", ButtonKind::Outline, "Keep installing").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_stop = false;
                            cx.notify();
                        })))
                        .child(button("dm-stop-keep", ButtonKind::Secondary, "Stop, keep files").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_stop = false;
                            this.svc.update(cx, |s, _| s.cancel_current(false));
                            cx.notify();
                        })))
                        .child(button("dm-stop-delete", ButtonKind::Tertiary, "Stop, delete files").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_stop = false;
                            this.svc.update(cx, |s, _| s.cancel_current(true));
                            cx.notify();
                        }))),
                );
            }
            column = column.child(section_title("Downloading").font_family(theme::FONT_HEADING)).child(
                card()
                    .child(cover_box(cover, 60., 80.))
                    .child(info)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(4.))
                            .child(
                                small_button("dm-pause", if paused { Icon::PlayArrow } else { Icon::Pause }, true).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(small_button("dm-stop", Icon::Close, true).on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_stop = !this.confirm_stop;
                                cx.notify();
                            }))),
                    ),
            );
        } else {
            self.confirm_stop = false;
        }

        // Waiting elements.
        column = column.child(section_title("Queued").font_family(theme::FONT_HEADING));
        let waiting: Vec<DmElement> = snap.queue.iter().skip(1).cloned().collect();
        if waiting.is_empty() {
            column = column.child(div().text_size(px(14.)).text_color(theme::white_a(0.6)).child("Nothing to download"));
        }
        let last = waiting.len().saturating_sub(1);
        for (i, el) in waiting.iter().enumerate() {
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let app = el.params.app_name.clone();
            let (a_up, a_down, a_rm) = (app.clone(), app.clone(), app.clone());
            column = column.child(
                card()
                    .child(cover_box(cover, 36., 48.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::accent()).child(el.params.title()))
                            .child(div().text_size(px(12.5)).text_color(theme::white_a(0.7)).child(meta_line(el))),
                    )
                    .child(div().text_size(px(12.)).text_color(theme::white_a(0.6)).child(date_text(el.add_to_queue_time)))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(small_button(SharedString::from(format!("dm-up-{app}")), Icon::FaChevronUp, i > 0).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.svc.update(cx, |s, _| s.move_in_queue(&a_up, true));
                                },
                            )))
                            .child(small_button(SharedString::from(format!("dm-down-{app}")), Icon::FaChevronDown, i < last).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.svc.update(cx, |s, _| s.move_in_queue(&a_down, false));
                                }),
                            ))
                            .child(small_button(SharedString::from(format!("dm-rm-{app}")), Icon::Close, true).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.svc.update(cx, |s, _| s.remove_from_queue(&a_rm));
                                },
                            ))),
                    ),
            );
        }

        // History.
        let mut finished: Vec<DmElement> = snap.finished.clone();
        finished.sort_by(|a, b| b.end_time.cmp(&a.end_time));
        if !finished.is_empty() {
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt(px(8.))
                    .child(section_title("Completed").font_family(theme::FONT_HEADING))
                    .child(button("dm-clear", ButtonKind::Text, "Clear list").on_click(cx.listener(|this, _, _, cx| {
                        this.svc.update(cx, |s, _| s.clear_finished());
                    }))),
            );
        }
        for el in finished {
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let app = el.params.app_name.clone();
            let (color, note) = match el.status {
                Some(DmStatus::Done) => (theme::success(), ""),
                Some(DmStatus::Error) => (theme::danger(), " (Failed)"),
                Some(DmStatus::Abort) => (theme::danger(), " (Canceled)"),
                _ => (rgb(0xcaf3fd), ""),
            };
            column = column.child(
                card()
                    .child(cover_box(cover, 36., 48.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(color).child(format!("{}{note}", el.params.title())))
                            .child(div().text_size(px(12.5)).text_color(theme::white_a(0.7)).child(meta_line(&el))),
                    )
                    .child(div().text_size(px(12.)).text_color(theme::white_a(0.6)).child(date_text(el.end_time)))
                    .child(small_button(SharedString::from(format!("dm-clear-{app}")), Icon::Close, true).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.svc.update(cx, |s, _| s.clear_finished_item(&app));
                        },
                    ))),
            );
        }

        let _ = &self.ctx;
        page_body("page-downloads").child(column)
    }
}
