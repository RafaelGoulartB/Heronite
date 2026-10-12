//! Downloads in console mode: the running install large (cover, progress),
//! then rows for its controls, the queue and the history. A on a queued
//! game opens a sheet (move up / down, remove); stopping asks first.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::manager::{DmState, Snapshot};
use heronite_epic::queue::{DmElement, DmStatus};
use heronite_i18n::tr;

use super::{date_text, meta_line, progress_text, DownloadsPage};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::widgets::progress_bar;
use crate::theme;

fn status_text(el: &DmElement) -> String {
    let what = match el.status {
        Some(DmStatus::Done) => tr!("Done"),
        Some(DmStatus::Error) => tr!("Failed"),
        Some(DmStatus::Abort) => tr!("Stopped"),
        Some(DmStatus::Paused) => tr!("Paused"),
        None => tr!("Finished"),
    };
    format!("{what} \u{b7} {}", date_text(el.end_time))
}

impl DownloadsPage {
    fn console_rows(&self, snap: &Snapshot) -> Vec<Row> {
        let mut rows = Vec::new();
        if !snap.queue.is_empty() {
            let running = snap.state == DmState::Running;
            rows.push(Row::action("pause", if running { tr!("Pause") } else { tr!("Resume") }).icon(if running {
                Icon::Pause
            } else {
                Icon::PlayArrow
            }));
            rows.push(Row::action("stop", tr!("Stop installing\u{2026}")).icon(Icon::Stop).danger());
        }
        let waiting: Vec<&DmElement> = snap.queue.iter().skip(1).collect();
        if !waiting.is_empty() {
            rows.push(Row::header(tr!("Queued \u{b7} {count}", count = waiting.len())));
            for el in waiting {
                rows.push(
                    Row::action(SharedString::from(format!("q:{}", el.params.app_name)), el.params.title())
                        .desc(meta_line(el))
                        .icon(Icon::Download),
                );
            }
        }
        if !snap.finished.is_empty() {
            rows.push(Row::header(tr!("Completed \u{b7} {count}", count = snap.finished.len())));
            for el in snap.finished.iter().rev() {
                rows.push(Row::info(SharedString::from(format!("f:{}", el.params.app_name)), el.params.title(), status_text(el)));
            }
            rows.push(Row::action("clear", tr!("Clear the list")).icon(Icon::DeleteOutline));
        }
        rows
    }

    fn open_sheet(
        &mut self,
        title: String,
        message: Option<String>,
        rows: Vec<Row>,
        cx: &mut Context<Self>,
        on: impl Fn(&mut Self, &str, &mut Context<Self>) + 'static,
    ) {
        let sheet = cx.new(|_| Sheet::new(title, message, rows));
        let sub = cx.subscribe(&sheet, move |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.layer.close();
            on(this, id, cx);
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn console_activate(&mut self, id: &str, snap: &Snapshot, cx: &mut Context<Self>) {
        match id {
            "pause" => self.svc.update(cx, |s, _| if s.is_running() { s.pause() } else { s.resume() }),
            "stop" => {
                let title = snap.queue.first().map(|e| e.params.title()).unwrap_or_default();
                let rows = vec![
                    Row::action("keep-going", tr!("Keep installing")),
                    Row::action("stop-keep", tr!("Stop, keep the downloaded files")),
                    Row::action("stop-delete", tr!("Stop and delete the files")).danger(),
                ];
                self.open_sheet(tr!("Stop installing {title}?", title), None, rows, cx, |this, id, cx| match id {
                    "stop-keep" => this.svc.update(cx, |s, _| s.cancel_current(false)),
                    "stop-delete" => this.svc.update(cx, |s, _| s.cancel_current(true)),
                    _ => {}
                });
            }
            "clear" => self.svc.update(cx, |s, _| s.clear_finished()),
            id if id.starts_with("q:") => {
                let app = id[2..].to_string();
                let title = snap.queue.iter().find(|e| e.params.app_name == app).map(|e| e.params.title()).unwrap_or_default();
                let rows = vec![
                    Row::action("up", tr!("Move up")).icon(crate::icons::Icon::FaChevronUp),
                    Row::action("down", tr!("Move down")).icon(crate::icons::Icon::FaChevronDown),
                    Row::action("remove", tr!("Remove from the queue")).icon(Icon::Close).danger(),
                ];
                self.open_sheet(title, None, rows, cx, move |this, id, cx| match id {
                    "up" => this.svc.update(cx, |s, _| s.move_in_queue(&app, true)),
                    "down" => this.svc.update(cx, |s, _| s.move_in_queue(&app, false)),
                    "remove" => this.svc.update(cx, |s, _| s.remove_from_queue(&app)),
                    _ => {}
                });
            }
            id if id.starts_with("f:") => {
                let app = id[2..].to_string();
                let rows = vec![Row::action("forget", tr!("Remove from the list")).icon(Icon::Close)];
                let title = snap.finished.iter().find(|e| e.params.app_name == app).map(|e| e.params.title()).unwrap_or_default();
                self.open_sheet(title, None, rows, cx, move |this, id, cx| {
                    if id == "forget" {
                        this.svc.update(cx, |s, _| s.clear_finished_item(&app));
                    }
                });
            }
            _ => {}
        }
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.covers.drop_evicted(window);
        let scale = window.scale_factor();
        let s = kit::scale();
        let svc = self.svc.read(cx);
        let snap: Snapshot = svc.snapshot().clone();
        let progress = snap.current.as_ref().and_then(|(app, _)| svc.progress_of(app).cloned());
        let rows = self.console_rows(&snap);

        let now = snap.queue.first().map(|cur| {
            let cover = self.covers.get(cur.params.cover().as_deref(), 96. * s, 128. * s, scale);
            let running = snap.state == DmState::Running;
            let percent = progress.as_ref().map(|p| p.percent as f32);
            let status = match snap.state {
                DmState::Running => tr!("Downloading"),
                DmState::Paused => tr!("Paused"),
                DmState::Idle => tr!("Waiting"),
            };
            div()
                .flex()
                .items_center()
                .gap(u(24.))
                .p(u(22.))
                .rounded(u(18.))
                .bg(theme::black_a(0.3))
                .border_1()
                .border_color(theme::white_a(0.08))
                .child(crate::services::widgets::cover_box(cover, f32::from(u(96.)), f32::from(u(128.))))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(u(10.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(u(14.))
                                .child(kit::title(cur.params.title(), 30.))
                                .child(kit::badge(status, if running { theme::console_btn_a() } else { theme::accent() })),
                        )
                        .child(div().text_size(u(15.)).text_color(theme::text_muted()).child(meta_line(cur)))
                        .child(progress_bar(percent, f32::from(u(10.)), if running { theme::success() } else { theme::accent() }))
                        .child(div().text_size(u(15.)).text_color(theme::text_muted()).child(match &progress {
                            Some(p) => progress_text(p),
                            None if running => tr!("Starting\u{2026}").to_string(),
                            None => tr!("Resume (A on \u{201c}Resume\u{201d}) to continue").to_string(),
                        })),
                )
        });

        let body: AnyElement = if rows.is_empty() {
            kit::empty(Icon::Download, tr!("Nothing to download"), tr!("Install a game from its page and it shows up here."))
                .into_any_element()
        } else {
            self.console_list.render("console-downloads", &rows, !self.layer.is_open()).flex_1().min_h_0().into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark(tr!("DOWNLOADS")))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap(u(18.))
                    .px(u(46.))
                    .pb(u(16.))
                    .children(now)
                    .child(div().w(u(860.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(body)),
            )
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for DownloadsPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let snap = self.svc.read(cx).snapshot().clone();
        let rows = self.console_rows(&snap);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.console_activate(&id, &snap, cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, tr!("Select"))]
    }
}
