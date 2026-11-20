//! What runs in the background, for console mode: metadata jobs (cover
//! downloads among them), game downloads and installs, a source's database,
//! the app's update and ffmpeg, read from the same places as the desktop
//! header's pills (the metadata hub, the downloads service, the updater,
//! the live stage). A pill beside the clock (top right when the top bar is
//! hidden: pages, game pages, Achievements) says what runs, with a thin
//! progress bar, and a toast in the top right corner says when a task
//! starts and how it ended. Read every [`TICK`] while console mode is on;
//! nothing is drawn while nothing runs. A click on the pill opens Downloads
//! when a game downloads; otherwise it takes no input.
//!
//! `HERONITE_DEBUG_CONSOLE_ACTIVITY=metadata|download|many|done|error|nothing` shows
//! made-up tasks or toasts (nothing runs); `=start:square|wide` starts the
//! real cover download Settings → Metadata starts.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_epic::queue::{DmStatus, ElementKind};
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkState, JobKind, JobTally};

use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::dialogs::metadata_update::hub::{hub, HubEvent, HubGlobal};
use crate::icons::Icon;
use crate::pages::Page;
use crate::services::update::{self, Phase};
use crate::theme;

/// How often the tasks are read.
const TICK: Duration = Duration::from_millis(400);
/// How long a toast stays: a start, an end, a failure.
const START_FOR: Duration = Duration::from_millis(3500);
const DONE_FOR: Duration = Duration::from_secs(5);
const ERROR_FOR: Duration = Duration::from_secs(8);
/// Toasts on screen at most (the oldest goes).
const TOASTS: usize = 3;
/// A toast's width, design px.
const TOAST_W: f32 = 440.;

/// What a task is.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Kind {
    /// A metadata job (`job` id); covers of a family, or fields.
    Metadata {
        job: u64,
        covers: Option<CoverFamily>,
    },
    /// A game downloads, as an install or an update.
    Game {
        app_name: String,
        title: String,
        install: bool,
    },
    /// The app's own update.
    AppUpdate,
    Ffmpeg,
    /// A metadata source's database.
    Database {
        source: String,
    },
}

/// One thing running.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Task {
    pub kind: Kind,
    /// Done of total (games), when counted.
    pub count: Option<(usize, usize)>,
    /// 0..=1, when known.
    pub progress: Option<f32>,
    /// Waiting (paused, offline).
    pub waiting: Option<String>,
}

impl Task {
    /// Stays the same while the task runs.
    fn id(&self) -> String {
        match &self.kind {
            Kind::Metadata { job, .. } => format!("md:{job}"),
            Kind::Game { app_name, .. } => format!("dl:{app_name}"),
            Kind::AppUpdate => "update".into(),
            Kind::Ffmpeg => "ffmpeg".into(),
            Kind::Database { source } => format!("db:{source}"),
        }
    }

    fn icon(&self) -> Icon {
        match &self.kind {
            Kind::Metadata { covers: Some(_), .. } => Icon::ViewCovers,
            Kind::Metadata { .. } => Icon::AutoAwesome,
            Kind::Game { .. } => Icon::Download,
            Kind::AppUpdate => Icon::Update,
            Kind::Ffmpeg => Icon::Animation,
            Kind::Database { .. } => Icon::CloudDownload,
        }
    }

    /// "Downloading square covers", "Installing Elden Ring".
    fn label(&self) -> String {
        match &self.kind {
            Kind::Metadata { covers: Some(CoverFamily::Square), .. } => tr!("Downloading square covers").into(),
            Kind::Metadata { covers: Some(CoverFamily::Wide), .. } => tr!("Downloading wide covers").into(),
            Kind::Metadata { covers: Some(CoverFamily::Tall), .. } => tr!("Downloading covers").into(),
            Kind::Metadata { covers: None, .. } => tr!("Updating metadata").into(),
            Kind::Game { title, install: true, .. } => tr!("Installing {title}", title),
            Kind::Game { title, install: false, .. } => tr!("Updating {title}", title),
            Kind::AppUpdate => tr!("Downloading the update").into(),
            Kind::Ffmpeg => tr!("Installing ffmpeg").into(),
            Kind::Database { .. } => tr!("Downloading a metadata database").into(),
        }
    }

    /// "12 / 195", "43%", "Paused".
    fn detail(&self) -> Option<String> {
        if let Some(w) = &self.waiting {
            return Some(w.clone());
        }
        match (self.count, self.progress) {
            (Some((done, total)), _) if total > 0 => Some(format!("{done} / {total}")),
            (_, Some(p)) => Some(format!("{}%", (p * 100.).round() as u32)),
            _ => None,
        }
    }
}

/// The tasks that started and the ones that ended between two reads.
fn changes<'a>(before: &'a [Task], now: &'a [Task]) -> (Vec<&'a Task>, Vec<&'a Task>) {
    let started = now.iter().filter(|t| !before.iter().any(|b| b.id() == t.id())).collect();
    let ended = before.iter().filter(|b| !now.iter().any(|t| t.id() == b.id())).collect();
    (started, ended)
}

/// The pill's text for `tasks`: the one task, or how many.
fn pill_text(tasks: &[Task]) -> (String, Option<String>) {
    match tasks {
        [] => (String::new(), None),
        [t] => (t.label(), t.detail()),
        [t, ..] => (trn!(tasks.len(), "{n} task", "{n} tasks"), Some(t.label())),
    }
}

/// How a toast reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Started,
    Done,
    Failed,
    /// Nothing to do: said so.
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Toast {
    id: u64,
    icon: Icon,
    tone: Tone,
    title: String,
    detail: Option<String>,
    at: Instant,
}

impl Toast {
    fn life(&self) -> Duration {
        match self.tone {
            Tone::Started => START_FOR,
            Tone::Done | Tone::Info => DONE_FOR,
            Tone::Failed => ERROR_FOR,
        }
    }
}

/// A cover download that found every game with a cover of the family.
fn nothing_to_download(f: CoverFamily) -> (String, String) {
    let detail = match f {
        CoverFamily::Square => tr!("Every game already has a square cover"),
        CoverFamily::Wide => tr!("Every game already has a wide cover"),
        CoverFamily::Tall => tr!("Every game already has a cover"),
    };
    (tr!("Nothing to download").into(), detail.into())
}

/// How a metadata job ended, from its tally.
fn metadata_end(covers: Option<CoverFamily>, t: &JobTally) -> (Tone, String, Option<String>) {
    let failed = t.failed + t.offline;
    if t.updated == 0 && t.stopped > 0 {
        return (Tone::Done, tr!("Stopped").into(), None);
    }
    if t.updated == 0 && failed > 0 {
        let title = match covers {
            Some(CoverFamily::Square) => tr!("Could not download square covers"),
            Some(CoverFamily::Wide) => tr!("Could not download wide covers"),
            Some(CoverFamily::Tall) => tr!("Could not download covers"),
            None => tr!("Could not update metadata"),
        };
        let detail = if t.offline > 0 { tr!("No connection").to_string() } else { trn!(failed, "{n} failed", "{n} failed") };
        return (Tone::Failed, title.into(), Some(detail));
    }
    let title = match (covers, t.updated > 0) {
        (Some(CoverFamily::Square), true) => tr!("Square covers downloaded"),
        (Some(CoverFamily::Wide), true) => tr!("Wide covers downloaded"),
        (Some(CoverFamily::Tall), true) => tr!("Covers downloaded"),
        (None, true) => tr!("Metadata updated"),
        // Nothing came: no "0 updated" under a title that says it did.
        (Some(CoverFamily::Square), false) => tr!("No new square covers"),
        (Some(CoverFamily::Wide), false) => tr!("No new wide covers"),
        (Some(CoverFamily::Tall), false) => tr!("No new covers"),
        (None, false) => tr!("Nothing new for these games"),
    };
    let mut parts = Vec::new();
    if t.updated > 0 {
        parts.push(trn!(t.updated, "{n} updated", "{n} updated"));
    }
    if t.not_found > 0 {
        parts.push(trn!(t.not_found, "{n} not found", "{n} not found"));
    }
    if failed > 0 {
        parts.push(trn!(failed, "{n} failed", "{n} failed"));
    }
    let detail = (!parts.is_empty()).then(|| parts.join(" \u{b7} "));
    (Tone::Done, title.into(), detail)
}

/// Console mode's activity.
#[derive(Default)]
pub(super) struct ActivityUi {
    tasks: Vec<Task>,
    toasts: Vec<Toast>,
    seq: u64,
    ticker: Option<gpui_kit::Task<()>>,
    /// The hub's finished jobs, for their toast.
    ended_jobs: HashMap<u64, JobTally>,
    hub_sub: Option<Subscription>,
    /// The hub's "nothing to download" count already told.
    nothing_seen: Option<u64>,
    /// `HERONITE_DEBUG_CONSOLE_ACTIVITY`: read once; made-up tasks stay.
    debug: Option<Option<Vec<Task>>>,
}

impl CollectionView {
    /// What runs now, from the same places as the desktop's pills.
    fn read_activity(&mut self, cx: &mut Context<Self>) -> Vec<Task> {
        let mut tasks = Vec::new();
        if cx.has_global::<HubGlobal>() {
            let h = hub(cx).read(cx);
            let p = &h.progress;
            if p.is_running() {
                let covers = match p.kind {
                    JobKind::Covers(f) => Some(f),
                    _ => None,
                };
                let waiting = (p.state == BulkState::Cancelling).then(|| tr!("Stopping").to_string());
                tasks.push(Task {
                    kind: Kind::Metadata { job: p.job, covers },
                    count: Some((p.processed.min(p.total), p.total)),
                    progress: Some(p.percent as f32 / 100.),
                    waiting,
                });
            }
            for (source, snap) in h.running_source_downloads() {
                tasks.push(Task { kind: Kind::Database { source }, count: None, progress: snap.fraction(), waiting: None });
            }
        }
        if crate::services::try_downloads(cx).is_some() {
            let svc = self.epic_service(cx);
            let s = svc.read(cx);
            let snap = s.snapshot();
            if let Some(cur) = snap.queue.first() {
                let progress = s.progress_of(&cur.params.app_name).map(|p| (p.percent / 100.) as f32);
                let waiting = match snap.state {
                    heronite_epic::manager::DmState::Running => None,
                    _ if s.paused_offline() => Some(tr!("Waiting for the connection").to_string()),
                    _ => Some(tr!("Paused").to_string()),
                };
                tasks.push(Task {
                    kind: Kind::Game {
                        app_name: cur.params.app_name.clone(),
                        title: cur.params.title(),
                        install: cur.kind == ElementKind::Install,
                    },
                    count: None,
                    progress,
                    waiting,
                });
            }
        }
        if let Some(u) = update::get(cx) {
            if let Phase::Downloading(p) = u.read(cx).phase() {
                tasks.push(Task { kind: Kind::AppUpdate, count: None, progress: p.percent().map(|p| p / 100.), waiting: None });
            }
        }
        if self.live.getting_ffmpeg {
            tasks.push(Task { kind: Kind::Ffmpeg, count: None, progress: None, waiting: None });
        }
        tasks
    }

    /// How a task that is no longer running ended (`None`: say nothing, it
    /// was cancelled).
    fn activity_end(&mut self, t: &Task, cx: &mut Context<Self>) -> Option<(Tone, String, Option<String>)> {
        match &t.kind {
            Kind::Metadata { job, covers } => {
                let tally = self.console.activity.ended_jobs.remove(job).unwrap_or_default();
                Some(metadata_end(*covers, &tally))
            }
            Kind::Game { app_name, title, install } => {
                let svc = crate::services::try_downloads(cx)?;
                let s = svc.read(cx);
                let last = s.snapshot().finished.iter().filter(|e| e.params.app_name == *app_name).max_by_key(|e| e.end_time)?;
                match (last.status?, install) {
                    (DmStatus::Done, true) => Some((Tone::Done, tr!("{title} installed", title), Some(tr!("ready to play").into()))),
                    (DmStatus::Done, false) => Some((Tone::Done, tr!("{title} updated", title), None)),
                    (DmStatus::Error, true) => Some((Tone::Failed, tr!("Could not install {title}", title), None)),
                    (DmStatus::Error, false) => Some((Tone::Failed, tr!("Could not update {title}", title), None)),
                    (DmStatus::Abort | DmStatus::Paused, _) => None,
                }
            }
            Kind::AppUpdate => {
                let u = update::get(cx)?;
                match u.read(cx).phase() {
                    Phase::Ready => Some((Tone::Done, tr!("Update downloaded").into(), Some(tr!("Restart to update").into()))),
                    Phase::InstallFailed(e) => Some((Tone::Failed, tr!("Could not update Heronite").into(), Some(e.clone()))),
                    _ => None,
                }
            }
            Kind::Ffmpeg => Some(if self.live.wants_ffmpeg {
                (Tone::Failed, tr!("Could not install ffmpeg").into(), None)
            } else {
                (Tone::Done, tr!("ffmpeg installed").into(), None)
            }),
            Kind::Database { source } => {
                let error = cx.has_global::<HubGlobal>().then(|| hub(cx).read(cx).source_download(source).error).flatten();
                Some(match error {
                    Some(e) => (Tone::Failed, tr!("Could not download the metadata database").into(), Some(e)),
                    None => (Tone::Done, tr!("Metadata database downloaded").into(), None),
                })
            }
        }
    }

    fn push_activity_toast(&mut self, icon: Icon, tone: Tone, title: String, detail: Option<String>) {
        let ui = &mut self.console.activity;
        ui.seq += 1;
        ui.toasts.push(Toast { id: ui.seq, icon, tone, title, detail, at: Instant::now() });
        let extra = ui.toasts.len().saturating_sub(TOASTS);
        ui.toasts.drain(..extra);
    }

    /// One read: toasts for what started and ended, and a repaint when
    /// anything changed.
    fn activity_tick(&mut self, cx: &mut Context<Self>) {
        let frozen = matches!(self.console.activity.debug, Some(Some(_)));
        let now = if frozen { self.console.activity.tasks.clone() } else { self.read_activity(cx) };
        let before = std::mem::take(&mut self.console.activity.tasks);
        let (started, ended) = changes(&before, &now);
        let (started, ended): (Vec<Task>, Vec<Task>) = (started.into_iter().cloned().collect(), ended.into_iter().cloned().collect());
        let mut changed = before != now;
        self.console.activity.tasks = now;
        for t in started {
            let detail = match t.count {
                Some((_, total)) if total > 0 => Some(trn!(total, "{n} game", "{n} games")),
                _ => None,
            };
            self.push_activity_toast(t.icon(), Tone::Started, format!("{}\u{2026}", t.label()), detail);
        }
        for t in ended {
            if let Some((tone, title, detail)) = self.activity_end(&t, cx) {
                self.push_activity_toast(t.icon(), tone, title, detail);
            }
        }
        if let Some((n, family)) = cx.has_global::<HubGlobal>().then(|| hub(cx).read(cx).nothing_to_download).flatten() {
            if self.console.activity.nothing_seen.replace(n) != Some(n) {
                let (title, detail) = nothing_to_download(family);
                self.push_activity_toast(Icon::ViewCovers, Tone::Info, title, Some(detail));
                changed = true;
            }
        }
        let ui = &mut self.console.activity;
        // A toast stays a little past its life while it fades out.
        let kept = ui.toasts.len();
        if !frozen {
            ui.toasts.retain(|t| t.at.elapsed() < t.life() + Duration::from_millis(400));
        }
        changed |= kept != ui.toasts.len() || ui.toasts.iter().any(|t| t.at.elapsed() >= t.life());
        if changed || !ui.tasks.is_empty() {
            cx.notify();
        }
    }

    /// Starts reading the tasks (from render, once console mode draws).
    fn ensure_activity_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.debug_activity(cx);
        if self.console.activity.hub_sub.is_none() && cx.has_global::<HubGlobal>() {
            let h = hub(cx);
            self.console.activity.hub_sub = Some(cx.subscribe(&h, |this, _, ev: &HubEvent, _| {
                if let HubEvent::JobDone(p) = ev {
                    this.console.activity.ended_jobs.insert(p.job, p.tally());
                }
            }));
        }
        if self.console.activity.ticker.is_some() {
            return;
        }
        self.console.activity.ticker = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            if this.update(cx, |this, cx| this.activity_tick(cx)).is_err() {
                break;
            }
        }));
    }

    /// `HERONITE_DEBUG_CONSOLE_ACTIVITY`: made-up tasks and toasts.
    fn debug_activity(&mut self, cx: &mut Context<Self>) {
        if self.console.activity.debug.is_some() {
            return;
        }
        let Some(which) = crate::env::value("HERONITE_DEBUG_CONSOLE_ACTIVITY") else {
            self.console.activity.debug = Some(None);
            return;
        };
        // `start:square|wide`: starts the real download the Settings row
        // starts (it goes to the network), and shows it as it runs.
        if let Some(family) = which.strip_prefix("start:").and_then(CoverFamily::parse) {
            self.console.activity.debug = Some(None);
            if cx.has_global::<HubGlobal>() {
                hub(cx).update(cx, |h, cx| h.download_family(family, None, cx));
            }
            return;
        }
        let covers = Task {
            kind: Kind::Metadata { job: 1, covers: Some(CoverFamily::Square) },
            count: Some((12, 195)),
            progress: Some(12. / 195.),
            waiting: None,
        };
        let game = Task {
            kind: Kind::Game { app_name: "fake".into(), title: "Elden Ring".into(), install: true },
            count: None,
            progress: Some(0.43),
            waiting: None,
        };
        let db = Task { kind: Kind::Database { source: "launchbox".into() }, count: None, progress: Some(0.71), waiting: None };
        let tasks = match which.as_str() {
            "metadata" => vec![covers.clone()],
            "download" => vec![game.clone()],
            "many" => vec![covers.clone(), game, db],
            _ => Vec::new(),
        };
        match which.as_str() {
            "metadata" => self.push_activity_toast(
                covers.icon(),
                Tone::Started,
                format!("{}\u{2026}", covers.label()),
                Some(trn!(195, "{n} game", "{n} games")),
            ),
            "done" => {
                let t = JobTally { updated: 182, not_found: 13, ..Default::default() };
                let (tone, title, detail) = metadata_end(Some(CoverFamily::Square), &t);
                self.push_activity_toast(covers.icon(), tone, title, detail);
                self.push_activity_toast(
                    Icon::Download,
                    Tone::Done,
                    tr!("{title} installed", title = "Elden Ring"),
                    Some(tr!("ready to play").into()),
                );
            }
            "nothing" => {
                let (title, detail) = nothing_to_download(CoverFamily::Square);
                self.push_activity_toast(Icon::ViewCovers, Tone::Info, title, Some(detail));
            }
            "error" => {
                let t = JobTally { offline: 195, ..Default::default() };
                let (tone, title, detail) = metadata_end(Some(CoverFamily::Square), &t);
                self.push_activity_toast(covers.icon(), tone, title, detail);
            }
            _ => {}
        }
        // Made-up toasts stay for the screenshots.
        let stay = Instant::now() + Duration::from_secs(3600);
        self.console.activity.toasts.iter_mut().for_each(|t| t.at = stay);
        self.console.activity.tasks = tasks.clone();
        self.console.activity.debug = Some(Some(tasks));
    }

    /// The pill (`None` while nothing runs). `floating`: over a screen
    /// without the top bar.
    fn activity_pill(&self, floating: bool) -> Option<AnyElement> {
        let tasks = &self.console.activity.tasks;
        let first = tasks.first()?;
        let (label, detail) = pill_text(tasks);
        let many = tasks.len() > 1;
        let game = tasks.iter().any(|t| matches!(t.kind, Kind::Game { .. }));
        let progress = first.progress;
        let moving = motion::on() && first.waiting.is_none();
        let glyph = crate::ui::icon(first.icon(), u(16.), theme::console_focus());
        let glyph: AnyElement = if moving {
            div()
                .flex_none()
                .child(glyph)
                .with_animation("console-activity-pulse", Animation::new(Duration::from_millis(1600)).repeat().with_max_fps(20.), |d, t| {
                    d.opacity(0.55 + 0.45 * (1. - (2. * t - 1.).abs()))
                })
                .into_any_element()
        } else {
            div().flex_none().child(glyph).into_any_element()
        };
        let bar =
            div().absolute().left(u(16.)).right(u(16.)).bottom(u(5.)).h(u(2.)).rounded_full().bg(kit::ink(0.12)).overflow_hidden().child(
                match progress {
                    Some(p) => div().h_full().w(relative(p.clamp(0.02, 1.))).rounded_full().bg(theme::console_focus()).into_any_element(),
                    // Not counted: a light sweeping along.
                    None if motion::on() => div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(relative(0.3))
                        .rounded_full()
                        .bg(theme::console_focus())
                        .with_animation(
                            "console-activity-sweep",
                            Animation::new(Duration::from_millis(1400)).repeat().with_easing(ease_in_out).with_max_fps(30.),
                            |d, t| d.left(relative(-0.3 + 1.3 * t)),
                        )
                        .into_any_element(),
                    None => div().into_any_element(),
                },
            );
        let pill = div()
            .id("console-activity")
            .relative()
            .flex()
            .items_center()
            .gap(u(8.))
            .h(u(40.))
            .pb(u(2.))
            .min_w_0()
            .max_w(u(460.))
            .pl(u(12.))
            .pr(u(16.))
            .rounded_full()
            .bg(Rgba { a: 0.86, ..theme::console_sidebar() })
            .border_1()
            .border_color(kit::ink(0.12))
            .when(floating, |d| d.shadow(kit::shadow(0.4, 6., 18.)))
            .child(glyph)
            // What gives way when it is narrow: one task's name (its count
            // stays), or the first task's name after "3 tasks".
            .child(
                div()
                    .when(many, |d| d.flex_none())
                    .when(!many, |d| d.min_w_0().truncate())
                    .text_size(u(14.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(label),
            )
            .children(detail.map(|d| {
                div()
                    .when(many, |d| d.min_w_0().truncate())
                    .when(!many, |d| d.flex_none())
                    .text_size(u(13.))
                    .text_color(theme::text_muted())
                    .child(d)
            }))
            .child(bar)
            // A game downloading: a click opens Downloads.
            .when(game, |d| {
                d.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, self.console_click(|this, window, cx| this.navigate(Page::Downloads, window, cx)))
            });
        Some(motion::fade_in("console-activity-in", pill, 260))
    }

    /// The pill beside the clock (`console_top_bar`).
    pub(super) fn activity_in_bar(&self) -> Option<AnyElement> {
        // A fifth of the bar at most, clear of tabs centred in it: on a narrow
        // window the label truncates.
        let pill = self.activity_pill(false)?;
        Some(div().flex().min_w_0().max_w(relative(0.2)).child(pill).into_any_element())
    }

    /// The toasts, newest last, for the column in the top right corner.
    fn activity_toasts(&self) -> Vec<AnyElement> {
        self.console.activity.toasts.iter().map(toast).collect()
    }

    /// The top right corner: the session card, the toasts and, over a screen
    /// without the top bar, the pill.
    pub(super) fn render_activity_corner(
        &mut self,
        top_bar: bool,
        card: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.ensure_activity_ticker(window, cx);
        let pill = (!top_bar).then(|| self.activity_pill(true)).flatten();
        let toasts = self.activity_toasts();
        if pill.is_none() && card.is_none() && toasts.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .top(if top_bar { u(92.) } else { u(68.) })
                // Both sides: its children are never wider than the window.
                .left(u(28.))
                .right(u(28.))
                .flex()
                .flex_col()
                .items_end()
                .gap(u(12.))
                .children(pill)
                .children(card)
                .children(toasts)
                .into_any_element(),
        )
    }
}

/// A toast: the task's icon in a disc of its tone, what happened and a line
/// more, on the session card's glass.
fn toast(t: &Toast) -> AnyElement {
    let colour: Rgba = match t.tone {
        Tone::Started => theme::console_focus(),
        Tone::Done => theme::console_on(),
        Tone::Failed => rgb(0xffb74d),
        Tone::Info => theme::text_muted(),
    };
    let glyph = match t.tone {
        Tone::Started => t.icon,
        Tone::Done => Icon::Check,
        Tone::Failed => Icon::Warning,
        Tone::Info => Icon::InfoOutlined,
    };
    let leaving = t.at.elapsed() >= t.life();
    let body = div()
        .w(u(TOAST_W))
        .max_w(relative(1.))
        .flex()
        .items_center()
        .gap(u(14.))
        .px(u(16.))
        .py(u(12.))
        .rounded(u(14.))
        .bg(Rgba { a: 0.94, ..theme::console_sidebar() })
        .border_1()
        .border_color(if t.tone == Tone::Failed { Hsla { a: 0.45, ..colour.into() } } else { kit::ink(0.12) })
        .shadow(kit::shadow(0.45, 8., 28.))
        .child(
            div()
                .flex_none()
                .size(u(36.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(Rgba { a: 0.18, ..colour })
                .child(crate::ui::icon(glyph, u(18.), kit::tint(colour))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(u(2.))
                .child(
                    div()
                        .text_size(u(16.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .line_clamp(2)
                        .child(t.title.clone()),
                )
                .children(t.detail.clone().map(|d| div().text_size(u(13.)).text_color(theme::text_muted()).line_clamp(2).child(d))),
        );
    let id = t.id;
    let shown = motion::enter(SharedString::from(format!("console-activity-toast-in-{id}")), body.relative(), 300, 40., 0.);
    motion::spring(SharedString::from(format!("console-activity-toast-out-{id}")), div().child(shown), motion::SOFT, !leaving, |d, k| {
        d.opacity(k.clamp(0., 1.))
    })
}

#[cfg(test)]
mod tests {
    use super::{changes, metadata_end, nothing_to_download, pill_text, Kind, Task, Tone};
    use heronite_data::cover_family::CoverFamily;
    use heronite_metadata::JobTally;

    fn covers(job: u64, done: usize) -> Task {
        Task { kind: Kind::Metadata { job, covers: Some(CoverFamily::Square) }, count: Some((done, 195)), progress: None, waiting: None }
    }

    fn game(app: &str, p: f32) -> Task {
        Task {
            kind: Kind::Game { app_name: app.into(), title: "Elden Ring".into(), install: true },
            count: None,
            progress: Some(p),
            waiting: None,
        }
    }

    #[test]
    fn a_task_keeps_its_place_while_its_progress_moves() {
        let (before, now) = ([covers(1, 3)], [covers(1, 40)]);
        let (started, ended) = changes(&before, &now);
        assert!(started.is_empty() && ended.is_empty());
        let (before, now) = ([covers(1, 195)], [covers(2, 0), game("a", 0.1)]);
        let (started, ended) = changes(&before, &now);
        assert_eq!(started.len(), 2);
        assert_eq!(ended, [&covers(1, 195)]);
    }

    #[test]
    fn the_pill_names_one_task_or_counts_them() {
        assert_eq!(pill_text(&[covers(1, 12)]), ("Downloading square covers".into(), Some("12 / 195".into())));
        assert_eq!(pill_text(&[game("a", 0.43)]), ("Installing Elden Ring".into(), Some("43%".into())));
        let (label, detail) = pill_text(&[covers(1, 12), game("a", 0.43), game("b", 0.1)]);
        assert_eq!((label.as_str(), detail.as_deref()), ("3 tasks", Some("Downloading square covers")));
        let paused = Task { waiting: Some("Paused".into()), ..game("a", 0.43) };
        assert_eq!(pill_text(&[paused]).1.as_deref(), Some("Paused"));
    }

    #[test]
    fn a_metadata_job_ends_well_or_says_why_not() {
        let done = metadata_end(Some(CoverFamily::Square), &JobTally { updated: 182, not_found: 13, ..Default::default() });
        assert_eq!(done, (Tone::Done, "Square covers downloaded".into(), Some("182 updated \u{b7} 13 not found".into())));
        let offline = metadata_end(Some(CoverFamily::Square), &JobTally { offline: 195, ..Default::default() });
        assert_eq!(offline.0, Tone::Failed);
        assert_eq!(offline.2.as_deref(), Some("No connection"));
        assert_eq!(metadata_end(None, &JobTally { stopped: 4, ..Default::default() }).1, "Stopped");
        assert_eq!(metadata_end(None, &JobTally { updated: 2, ..Default::default() }).1, "Metadata updated");
        let none = metadata_end(Some(CoverFamily::Square), &JobTally { not_found: 14, ..Default::default() });
        assert_eq!(none, (Tone::Done, "No new square covers".into(), Some("14 not found".into())), "never \"0 updated\"");
        assert_eq!(metadata_end(None, &JobTally::default()), (Tone::Done, "Nothing new for these games".into(), None));
    }

    #[test]
    fn a_download_with_nothing_to_fetch_says_so() {
        assert_eq!(
            nothing_to_download(CoverFamily::Square),
            ("Nothing to download".into(), "Every game already has a square cover".into())
        );
        assert_eq!(nothing_to_download(CoverFamily::Wide).1, "Every game already has a wide cover");
    }
}
