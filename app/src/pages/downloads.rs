//! Downloads page: the running install/update with its progress, the queue
//! (reorder / remove) and the history, backed by the download manager shared
//! by Epic, GOG and Amazon Games.

mod console;
mod palette;

use chrono::{Local, TimeZone};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::{DmState, Snapshot};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind};
use heronite_i18n::{tr, trc};

use super::{caption_with, columns, content_width, empty_row, icon_action, list_item, page_close, sheet, status_pill, PageCtx, PageEvent};
use crate::icons::Icon;
use crate::services::widgets::{cover_box, fmt_mib, fmt_percent, progress_bar, Covers};
use crate::services::{self, Downloads};
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, card_box, ButtonKind};

pub struct DownloadsPage {
    ctx: PageCtx,
    svc: Entity<Downloads>,
    covers: Covers,
    /// "Stop installation" question open for the running element.
    confirm_stop: bool,
    /// Console mode: focused row and the open sheet.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<PageEvent> for DownloadsPage {}

impl DownloadsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let covers = Covers::new(&ctx.data_paths, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        Self { ctx, svc, covers, confirm_stop: false, console_list: Default::default(), layer: Default::default() }
    }
}

fn meta_line(el: &DmElement) -> String {
    let kind = if el.kind == ElementKind::Install { trc!("download kind", "Install") } else { trc!("download kind", "Update") };
    let store = match el.params.runner.as_str() {
        "gog" => "GOG",
        "nile" => "Amazon",
        _ => "Epic",
    };
    // GOG names its platforms in lower case.
    let platform = match el.params.platform_to_install.as_str() {
        "" | "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let size = el.params.size.as_deref().map(heronite_epic::util::localize_size).unwrap_or_else(|| "?? MB".into());
    format!("{kind} \u{b7} {store} \u{b7} {platform} \u{b7} {size}")
}

/// A failure as the history shows it: the tool's message without its log
/// prefixes, lines joined.
fn failure_reason(error: &str) -> String {
    let strip = |line: &str| {
        // legendary: `[DLManager] ERROR: text`.
        let line = line.strip_prefix('[').and_then(|l| l.split_once("] ")).map_or(line, |(_, rest)| rest);
        ["ERROR: ", "CRITICAL: "].iter().find_map(|p| line.strip_prefix(p)).unwrap_or(line).trim().to_string()
    };
    let lines: Vec<String> = error.lines().map(strip).filter(|l| !l.is_empty()).collect();
    lines.join(" \u{b7} ")
}

fn date_text(ms: u64) -> String {
    let format = match heronite_i18n::current() {
        heronite_i18n::Lang::PtBr => "%d/%m/%Y %H:%M",
        heronite_i18n::Lang::En => "%Y-%m-%d %H:%M",
    };
    Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format(format).to_string()).unwrap_or_default()
}

fn progress_text(p: &Progress) -> String {
    tr!(
        "{percent} - {downloaded} downloaded - {down} MiB/s download - {disk} MiB/s disk - ETA {eta}",
        percent = fmt_percent(p.percent),
        downloaded = fmt_mib(p.downloaded_mib),
        down = heronite_i18n::decimal(p.down_speed, 2),
        disk = heronite_i18n::decimal(p.disk_speed, 2),
        eta = if p.eta.is_empty() { "--:--:--" } else { p.eta.as_str() }
    )
}

impl Render for DownloadsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.covers.drop_evicted(window);
        let scale = window.scale_factor();
        let svc = self.svc.read(cx);
        let snap: Snapshot = svc.snapshot().clone();
        let progress = snap.current.as_ref().and_then(|(app, _)| svc.progress_of(app).cloned());
        let offline = svc.paused_offline();
        let running = snap.state == DmState::Running;
        let paused = !running;
        // Wide: the running download on top, then the queue and the history
        // side by side. Narrow: all three stacked.
        let wide = content_width(window) >= 1080.;

        let mut column = div().flex().flex_col().w_full();

        if let Some(cur) = snap.queue.first() {
            let title = cur.params.title();
            let cover = self.covers.get(cur.params.cover().as_deref(), 60., 80., scale);
            let percent = progress.as_ref().map(|p| p.percent as f32);
            let (status, status_color) = match snap.state {
                DmState::Running => (tr!("Downloading"), theme::success()),
                DmState::Paused if offline => (tr!("Waiting for the connection"), theme::accent()),
                DmState::Paused => (tr!("Paused"), theme::accent()),
                DmState::Idle => (tr!("Waiting"), theme::accent()),
            };
            let bar_color = if paused { theme::accent() } else { theme::success() };
            let info = div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().text_size(px(17.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title.clone()))
                        .child(status_pill(status, status_color)),
                )
                .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(meta_line(cur)))
                .child(progress_bar(percent, 8., bar_color))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(match &progress {
                    Some(p) => progress_text(p),
                    None if snap.state == DmState::Paused => tr!("Paused - resume to continue where it stopped").to_string(),
                    None if paused => tr!("Press play to start the queue").to_string(),
                    None => tr!("Starting...").to_string(),
                }));
            let controls = div()
                .flex()
                .gap(px(6.))
                .child(icon_action("dm-pause", if paused { Icon::PlayArrow } else { Icon::Pause }, true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() });
                        cx.notify();
                    },
                )))
                .child(icon_action("dm-stop", Icon::Close, true).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_stop = !this.confirm_stop;
                    cx.notify();
                })));
            let mut now = card_box().child(list_item(Some(cover_box(cover, 60., 80.).into_any_element()), info, controls));
            if self.confirm_stop {
                now = now.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .py(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(240.))
                                .text_size(px(13.))
                                .text_color(theme::text_primary())
                                .child(tr!("Stop the installation? Keep the downloaded files?")),
                        )
                        .child(button_sm("dm-keep-running", ButtonKind::Ghost, tr!("Keep installing")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.confirm_stop = false;
                                cx.notify();
                            },
                        )))
                        .child(button_sm("dm-stop-keep", ButtonKind::Glass, tr!("Stop, keep files")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.confirm_stop = false;
                                this.svc.update(cx, |s, _| s.cancel_current(false));
                                cx.notify();
                            },
                        )))
                        .child(button_sm("dm-stop-delete", ButtonKind::Danger, tr!("Stop, delete files")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.confirm_stop = false;
                                this.svc.update(cx, |s, _| s.cancel_current(true));
                                cx.notify();
                            },
                        ))),
                );
            }
            column = column.child(caption(tr!("Downloading"))).child(now);
        } else {
            self.confirm_stop = false;
        }

        let waiting: Vec<DmElement> = snap.queue.iter().skip(1).cloned().collect();
        let mut queued = div().child(caption(if waiting.is_empty() {
            tr!("Queued").to_string()
        } else {
            tr!("Queued \u{b7} {count}", count = waiting.len())
        }));
        let last = waiting.len().saturating_sub(1);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, el) in waiting.iter().enumerate() {
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let app = el.params.app_name.clone();
            let (a_up, a_down, a_rm) = (app.clone(), app.clone(), app.clone());
            let content = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .truncate()
                        .child(el.params.title()),
                )
                .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(meta_line(el)));
            let trailing = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().mr(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(date_text(el.add_to_queue_time)))
                .child(icon_action(SharedString::from(format!("dm-up-{app}")), Icon::FaChevronUp, i > 0).when(i > 0, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.svc.update(cx, |s, _| s.move_in_queue(&a_up, true));
                    }))
                }))
                .child(icon_action(SharedString::from(format!("dm-down-{app}")), Icon::FaChevronDown, i < last).when(i < last, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.svc.update(cx, |s, _| s.move_in_queue(&a_down, false));
                    }))
                }))
                .child(icon_action(SharedString::from(format!("dm-rm-{app}")), Icon::Close, true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.svc.update(cx, |s, _| s.remove_from_queue(&a_rm));
                    },
                )));
            rows.push(list_item(Some(cover_box(cover, 36., 48.).into_any_element()), content, trailing).into_any_element());
        }
        if rows.is_empty() {
            rows.push(empty_row(tr!("Nothing to download")).into_any_element());
        }
        queued = queued.child(card(rows));

        let mut done = div();
        let mut finished: Vec<DmElement> = snap.finished.clone();
        finished.sort_by_key(|e| std::cmp::Reverse(e.end_time));
        if !finished.is_empty() {
            done = done.child(caption_with(
                tr!("Completed \u{b7} {count}", count = finished.len()),
                button_sm("dm-clear", ButtonKind::Ghost, tr!("Clear list")).on_click(cx.listener(|this, _, _, cx| {
                    this.svc.update(cx, |s, _| s.clear_finished());
                })),
            ));
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for el in finished {
            let cover = self.covers.get(el.params.cover().as_deref(), 36., 48., scale);
            let app = el.params.app_name.clone();
            let failed = el.status == Some(DmStatus::Error);
            // Queued again already: nothing to retry.
            let retry = failed && !snap.queue.iter().any(|q| q.params.app_name == app);
            let reason = failed.then(|| el.error.as_deref().map(failure_reason)).flatten();
            let pill = match el.status {
                Some(DmStatus::Done) => Some(status_pill(tr!("Done"), theme::success())),
                Some(DmStatus::Error) => Some(status_pill(tr!("Failed"), theme::danger_text())),
                Some(DmStatus::Abort) => Some(status_pill(tr!("Canceled"), theme::danger_text())),
                _ => None,
            };
            let content = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(theme::W_BOLD)
                                .text_color(theme::text_primary())
                                .truncate()
                                .child(el.params.title()),
                        )
                        .children(pill),
                )
                .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(meta_line(&el)))
                .children(
                    reason.map(|r| div().line_clamp(2).text_ellipsis().text_size(px(12.)).text_color(theme::danger_text()).child(r)),
                );
            let retry_app = app.clone();
            let trailing = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().mr(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(date_text(el.end_time)))
                .when(retry, |d| {
                    d.child(button_sm(SharedString::from(format!("dm-retry-{app}")), ButtonKind::Glass, tr!("Retry")).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.svc.update(cx, |s, cx| s.retry(&retry_app, cx));
                        }),
                    ))
                })
                .child(icon_action(SharedString::from(format!("dm-clear-{app}")), Icon::Close, true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.svc.update(cx, |s, _| s.clear_finished_item(&app));
                    },
                )));
            rows.push(list_item(Some(cover_box(cover, 36., 48.).into_any_element()), content, trailing).into_any_element());
        }
        if !rows.is_empty() {
            done = done.child(card(rows));
        } else if wide {
            done = done
                .child(caption(trc!("downloads", "Completed")))
                .child(card([empty_row(tr!("Finished installs and updates show up here")).into_any_element()]));
        }
        column = column.child(columns(wide, [queued, done]));

        let _ = &self.ctx;
        sheet(
            "page-downloads",
            tr!("Downloads"),
            Some(tr!("Installs and updates from Epic Games, GOG and Amazon Games run one at a time, in this order.").into()),
            None,
            1560.,
            window,
            page_close(cx),
            column,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn failure_reasons_lose_the_log_prefixes() {
        let why = "[DLManager] ERROR: Not enough available disk space!\n\n[cli] ERROR: Installation failed";
        assert_eq!(super::failure_reason(why), "Not enough available disk space! \u{b7} Installation failed");
        assert_eq!(super::failure_reason("nile exited with Some(1)"), "nile exited with Some(1)");
    }
}
