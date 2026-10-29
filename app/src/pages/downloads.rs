//! Downloads page: the running install/update on a stage over its art
//! (progress, speeds, when it ends, a speed chart), the queue (drag to
//! reorder, remove) and the history (Play, Retry), backed by the download
//! manager shared by Epic, GOG and Amazon Games.

mod console;
mod lists;
mod palette;
mod stage;
mod timing;

use chrono::{Local, TimeZone};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::disk::{self, DiskSpace};
use heronite_epic::manager::{DmState, Snapshot};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, ElementKind};
use heronite_i18n::{tr, trc, trn};

use super::{columns, content_width, page_close, sheet, PageCtx, PageEvent};
use crate::icons::Icon;
use crate::services::widgets::{fmt_mib, fmt_percent, progress_bar, Covers};
use crate::services::{self, Downloads};
use crate::theme;
use crate::ui::controls::{icon_button_sm, ButtonKind};
use crate::ui::icon;
use timing::{current_eta, left_bytes, size_bytes, size_text, starts_in};

/// Widest the page's column gets.
const MAX_W: f32 = 1560.;

/// Which part of the history shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum History {
    #[default]
    All,
    Done,
    /// Failed or canceled.
    Failed,
}

pub struct DownloadsPage {
    ctx: PageCtx,
    svc: Entity<Downloads>,
    covers: Covers,
    /// "Stop installation" question open for this app name (shown while
    /// it is the running element).
    confirm_stop: Option<String>,
    history: History,
    /// Free space where the first element of the queue installs, and how
    /// much of it the queue still needs: `(install path, queue length)` it
    /// was read for, then the answer.
    disk: Option<((String, usize), Option<(DiskSpace, u64)>)>,
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
        Self {
            ctx,
            svc,
            covers,
            // `HERONITE_DEBUG_DOWNLOADS_STOP=<appName>`: the "Stop?" question open for that game.
            confirm_stop: crate::env::value("HERONITE_DEBUG_DOWNLOADS_STOP"),
            history: History::All,
            disk: None,
            console_list: Default::default(),
            layer: Default::default(),
        }
    }

    /// The Collection hid the page: frees the cover textures (a hidden page
    /// never renders to do it); they load again from the disk cache when it comes back.
    pub fn hidden(&mut self, window: &mut Window) {
        self.covers.release(window);
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
        heronite_i18n::Lang::PtBr | heronite_i18n::Lang::Es => "%d/%m/%Y %H:%M",
        heronite_i18n::Lang::De => "%d.%m.%Y %H:%M",
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

/// What the page shows of the queue, read once per frame.
struct Overview {
    snap: Snapshot,
    /// The running element's last report.
    progress: Option<Progress>,
    /// Paused because the connection dropped.
    offline: bool,
    /// MiB/s the queue is expected to go on at (recent reports' average).
    speed: f64,
    /// Seconds until the running element ends.
    current_eta: Option<u64>,
    /// Seconds until each waiting element starts (while running).
    starts: Vec<Option<u64>>,
    /// Updates of installed games not in the queue yet.
    updates: usize,
}

impl Overview {
    fn read(svc: &Downloads) -> Self {
        let snap = svc.snapshot().clone();
        let cur = snap.queue.first();
        let progress = cur.and_then(|c| svc.progress_of(&c.params.app_name).cloned());
        let samples = cur.map(|c| svc.speed_samples(&c.params.app_name));
        let recent: Vec<f32> = samples.map(|s| s.iter().rev().take(10).copied().filter(|v| *v > 0.).collect()).unwrap_or_default();
        let speed = if recent.is_empty() {
            progress.as_ref().map_or(0., |p| p.down_speed)
        } else {
            recent.iter().map(|v| *v as f64).sum::<f64>() / recent.len() as f64
        };
        let running = snap.state == DmState::Running;
        let current_eta = cur.filter(|_| running).and_then(|c| current_eta(c, progress.as_ref(), speed));
        let starts = match current_eta {
            Some(eta) => starts_in(eta, &snap.queue[1..], speed),
            None => Vec::new(),
        };
        let updates = svc.update_apps().iter().filter(|a| svc.queued(a).is_none()).count();
        Self { offline: svc.paused_offline(), snap, progress, speed, current_eta, starts, updates }
    }

    /// Bytes the whole queue still has to download (elements of unknown size
    /// count as nothing).
    fn left(&self) -> f64 {
        let mut q = self.snap.queue.iter();
        let first = q.next().and_then(|c| left_bytes(c, self.progress.as_ref())).unwrap_or(0.);
        first + q.filter_map(size_bytes).sum::<f64>()
    }

    /// The line under the page's top: what runs and when it all ends.
    fn summary(&self) -> String {
        let Some(cur) = self.snap.queue.first() else {
            if self.snap.finished.is_empty() && self.updates == 0 {
                return tr!("Installs and updates from Epic Games, GOG and Amazon Games show up here, one at a time.").into();
            }
            let failed = self.snap.finished.iter().filter(|e| e.status == Some(heronite_epic::queue::DmStatus::Error)).count();
            let mut parts = vec![tr!("Nothing is downloading").to_string()];
            if self.updates > 0 {
                parts.push(trn!(self.updates, "{n} update available", "{n} updates available"));
            }
            if failed > 0 {
                parts.push(trn!(failed, "{n} failed", "{n} failed"));
            }
            return parts.join(" \u{b7} ");
        };
        let waiting = self.snap.queue.len() - 1;
        let mut parts = vec![match self.snap.state {
            DmState::Running => tr!("Downloading {title}", title = cur.params.title()),
            DmState::Paused if self.offline => tr!("Waiting for the connection: the queue goes on by itself").to_string(),
            DmState::Paused => tr!("Paused").to_string(),
            DmState::Idle => tr!("Waiting").to_string(),
        }];
        if waiting > 0 {
            parts.push(trn!(waiting, "{n} more in the queue", "{n} more in the queue"));
        }
        let left = self.left();
        if left > 0. {
            parts.push(tr!("{size} left", size = size_text(left)));
        }
        let end = self.current_eta.zip(self.starts.last().copied().unwrap_or(self.current_eta)).map(|(eta, last)| {
            let tail = self.snap.queue.last().filter(|_| waiting > 0).and_then(size_bytes).unwrap_or(0.);
            if waiting == 0 {
                eta
            } else {
                last + (tail / (self.speed.max(0.01) * 1024. * 1024.)) as u64
            }
        });
        if let Some(secs) = end {
            parts.push(tr!("all done around {time}", time = timing::clock_in(secs)));
        }
        parts.join(" \u{b7} ")
    }
}

/// Cover of `w`x`h` with corners of `radius` (the image is clipped with them).
fn poster(src: Option<ImageSource>, w: f32, h: f32, radius: f32) -> Div {
    div()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius))
        .overflow_hidden()
        .bg(theme::ink_a(0.05))
        .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover).rounded(px(radius))))
}

/// The Collection's key of an element's game (`runner_appName`).
fn play_key(el: &DmElement) -> String {
    let runner = if el.params.runner.is_empty() { "legendary" } else { el.params.runner.as_str() };
    format!("{runner}_{}", el.params.app_name)
}

/// The folder an element installs into, when it is a real one on this PC.
fn folder_of(el: &DmElement) -> Option<String> {
    let base = el.params.path.trim_matches('\'');
    if !std::path::Path::new(base).is_absolute() {
        return None;
    }
    match el.params.folder_name() {
        Some(folder) if el.kind == ElementKind::Install && !el.params.is_dlc() => {
            Some(std::path::Path::new(base).join(folder).to_string_lossy().into_owned())
        }
        _ => Some(base.to_string()),
    }
}

impl DownloadsPage {
    /// Updates waiting for installed games, with "Update all".
    fn updates_banner(&self, count: usize) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .mt(px(16.))
            .pl(px(16.))
            .pr(px(12.))
            .py(px(12.))
            .rounded(px(theme::R_TILE))
            .bg(theme::accent_alpha(0.08))
            .border_1()
            .border_color(theme::accent_alpha(0.22))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(34.))
                    .rounded(px(10.))
                    .bg(theme::accent_alpha(0.15))
                    .child(icon(Icon::Update, px(20.), theme::accent())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).child(trn!(
                        count,
                        "{n} update available",
                        "{n} updates available"
                    )))
                    .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(trn!(
                        count,
                        "Put it in the queue.",
                        "Put them all in the queue, one after the other."
                    ))),
            )
            .child(icon_button_sm("dm-update-all", ButtonKind::Primary, Icon::Download, tr!("Update all"), true, {
                let svc = self.svc.downgrade();
                move |_, _, cx| {
                    svc.update(cx, |s, cx| s.update_all(cx)).ok();
                }
            }))
    }

    /// Reads the free space of the queue's install folder again when the
    /// first element or the queue's length changes (off the UI thread).
    fn watch_disk(&mut self, o: &Overview, cx: &mut Context<Self>) {
        let Some(first) = o.snap.queue.first() else {
            self.disk = None;
            return;
        };
        let wanted = (first.params.path.trim_matches('\'').to_string(), o.snap.queue.len());
        if self.disk.as_ref().is_some_and(|(k, _)| *k == wanted) {
            return;
        }
        let path = wanted.0.clone();
        let needs = self.svc.read(cx).queued_needs("");
        self.disk = Some((wanted.clone(), None));
        cx.spawn(async move |this, cx| {
            let answer =
                cx.background_spawn(async move { disk::check(&path).map(|space| (space, disk::reserved_on(&path, &needs))) }).await;
            this.update(cx, |this, cx| {
                if this.disk.as_ref().is_some_and(|(k, _)| *k == wanted) {
                    this.disk = Some((wanted, answer));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// "/games: 412 GiB free · the queue needs 63 GiB", with a bar of the
    /// drive's use.
    fn disk_note(&self) -> Option<Div> {
        let ((path, _), Some((space, needs))) = self.disk.as_ref()? else { return None };
        let used = 1. - space.free as f32 / space.total.max(1) as f32;
        let short = (*needs as f64) > space.free as f64;
        let mut text = vec![tr!("{path}: {free} free", path = path.clone(), free = size_text(space.free as f64))];
        if *needs > 0 {
            text.push(tr!("the queue needs {size}", size = size_text(*needs as f64)));
        }
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .mt(px(18.))
                .mx(px(6.))
                .text_size(px(12.))
                .text_color(if short { theme::danger_text().into() } else { Hsla::from(theme::text_dim()) })
                .child(icon(Icon::FaHardDrive, px(14.), theme::text_dim()))
                .child(div().min_w_0().truncate().child(text.join(" \u{b7} ")))
                .child(div().w(px(160.)).flex_none().child(progress_bar(
                    Some(used * 100.),
                    5.,
                    if short { theme::danger_text() } else { theme::accent() },
                ))),
        )
    }
}

impl Render for DownloadsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.covers.drop_evicted(window);
        let o = Overview::read(self.svc.read(cx));
        self.watch_disk(&o, cx);
        let lead = o.summary();
        let column = if o.snap.queue.is_empty() && o.snap.finished.is_empty() && o.updates == 0 {
            stage::empty_page(cx)
        } else {
            // Wide: the queue and the history side by side under the stage.
            let wide = content_width(window) >= 1080.;
            let stage = self.render_stage(&o, window, cx);
            let banner = (o.updates > 0).then(|| self.updates_banner(o.updates));
            let queue = self.render_queue(&o, window, cx);
            let history = self.render_history(&o, window, cx);
            div()
                .flex()
                .flex_col()
                .w_full()
                .child(stage)
                .children(banner)
                .child(div().mt(px(8.)).child(columns(wide, [queue, history])))
                .children(self.disk_note())
        };
        let _ = &self.ctx;
        // No title: the slim header above names the page already.
        sheet("page-downloads", "", Some(lead.into()), None, MAX_W, window, page_close(cx), column).into_any_element()
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
