//! App updates (`heronite_update`): looks for a newer GitHub release, and
//! when the app runs from an AppImage, downloads it, swaps it in and restarts.
//!
//! Checks run [`FIRST_CHECK`] after the start (skipped when the last one is
//! younger than [`RECHECK_AFTER`]) and every [`CHECK_EVERY`], only for an
//! AppImage with "Check for updates automatically" on; a build that did not
//! come from an AppImage is updated by whatever installed it, so it only
//! checks when asked (Settings → About). A failed background check stays
//! quiet; a failed manual check or install is shown.
//!
//! The Collection draws the banner ([`Updater::banner`]) and Settings → About
//! the full card; both observe this entity.
//!
//! Switches: `HERONITE_NO_UPDATE_CHECK=1` turns the automatic checks off;
//! `HERONITE_UPDATE_API=<url>` asks another endpoint (local tests);
//! `HERONITE_DEBUG_UPDATE=checking|uptodate|available|downloading|ready|failed`
//! shows that state with a made-up release, without any request;
//! `HERONITE_DEBUG_UPDATE_RESTART=1` restarts as soon as an update is
//! installed (end-to-end tests without clicks).

mod debug;
pub mod notes;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_update::{Fetched, InstallError, Prefs, Progress, Release, Target, Unsupported, Version};

const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// A start this soon after the last check waits for the next period instead.
const RECHECK_AFTER: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// No check yet in this session.
    Idle,
    Checking,
    UpToDate,
    /// [`Updater::release`] is newer than this build.
    Available,
    Downloading(Progress),
    /// The new AppImage is in place; it runs from the next start.
    Ready,
    /// A check the user asked for failed.
    CheckFailed(String),
    InstallFailed(String),
}

/// What the Collection's banner shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banner {
    Available,
    Downloading,
    Ready,
    Failed,
}

pub struct Updater {
    app: AppPaths,
    prefs: Prefs,
    target: Result<Target, Unsupported>,
    current: Version,
    phase: Phase,
    /// The newest release when it is newer than this build.
    release: Option<Release>,
    /// Version whose banner was closed in this session.
    dismissed: Option<String>,
    /// Version already announced by a desktop notification.
    notified: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
    job: Option<Task<()>>,
    timer: Option<Task<()>>,
    api_url: String,
    /// `HERONITE_DEBUG_UPDATE`: made-up states, no requests or writes.
    debug: bool,
}

struct UpdaterGlobal(Entity<Updater>);
impl Global for UpdaterGlobal {}

/// Creates the service (once) and schedules the automatic checks.
pub fn init(cx: &mut App, app: AppPaths) -> Entity<Updater> {
    if let Some(g) = cx.try_global::<UpdaterGlobal>() {
        return g.0.clone();
    }
    let entity = cx.new(|cx| Updater::new(app, cx));
    cx.set_global(UpdaterGlobal(entity.clone()));
    entity
}

/// The service, once the hub created it.
pub fn get(cx: &App) -> Option<Entity<Updater>> {
    cx.try_global::<UpdaterGlobal>().map(|g| g.0.clone())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Opens `url` in the browser.
pub fn open_page(url: &str) {
    use std::process::{Command, Stdio};
    match Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => log::warn!("xdg-open {url}: {e}"),
    }
}

/// `14.7 MiB`.
pub fn size_text(bytes: u64) -> String {
    crate::services::widgets::fmt_mib(bytes as f64 / 1_048_576.)
}

/// `6.1 MiB of 14.7 MiB · 2.3 MiB/s`.
pub fn progress_text(p: &Progress) -> String {
    let mut text = if p.total > 0 { format!("{} of {}", size_text(p.downloaded), size_text(p.total)) } else { size_text(p.downloaded) };
    if p.bytes_per_sec > 0. {
        text.push_str(&format!(" \u{b7} {}/s", size_text(p.bytes_per_sec as u64)));
    }
    text
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago_text(unix_secs: u64) -> String {
    let secs = now_secs().saturating_sub(unix_secs);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ if secs < 2 * 86_400 => "yesterday".into(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

impl Updater {
    fn new(app: AppPaths, cx: &mut Context<Self>) -> Self {
        let prefs = Prefs::load(&app.update_state());
        let target = Target::detect(std::env::var_os("APPIMAGE"));
        match &target {
            Ok(t) => {
                log::info!("running from {}: updates can be installed", t.appimage.display());
                // A download cut short by a quit or a crash.
                let t = t.clone();
                cx.background_spawn(async move { t.sweep_staging() }).detach();
            }
            Err(Unsupported::NotAppImage) => {}
            Err(why) => log::info!("updates cannot be installed in place: {why}"),
        }
        let mut this = Self {
            app,
            prefs,
            target,
            current: Version::current(),
            phase: Phase::Idle,
            release: None,
            dismissed: None,
            notified: None,
            cancel: None,
            job: None,
            timer: None,
            api_url: crate::env::value("HERONITE_UPDATE_API").unwrap_or_else(|| heronite_update::LATEST_API.to_string()),
            debug: false,
        };
        if let Some(state) = crate::env::value("HERONITE_DEBUG_UPDATE") {
            this.debug_state(&state);
            return this;
        }
        this.schedule(cx);
        this
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn release(&self) -> Option<&Release> {
        self.release.as_ref()
    }

    pub fn current(&self) -> &Version {
        &self.current
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    /// Why this build cannot install updates by itself (`None` = it can).
    pub fn unsupported(&self) -> Option<&Unsupported> {
        self.target.as_ref().err()
    }

    /// The release can be downloaded and swapped in from here.
    pub fn can_install(&self) -> bool {
        self.target.is_ok() && self.release.as_ref().is_some_and(Release::installable)
    }

    /// The release on offer was skipped with "Skip this version".
    pub fn skipped(&self) -> bool {
        self.release.as_ref().is_some_and(|r| self.prefs.skipped_version.as_deref() == Some(r.version.as_str()))
    }

    /// Unix seconds of the last successful check.
    pub fn last_check(&self) -> Option<u64> {
        self.prefs.last_check
    }

    /// The banner the Collection shows, if any.
    pub fn banner(&self) -> Option<Banner> {
        let version = self.release.as_ref().map(|r| r.version.as_str());
        let closed = version.is_some() && self.dismissed.as_deref() == version;
        match &self.phase {
            Phase::Available if !closed && !self.skipped() => Some(Banner::Available),
            Phase::Downloading(_) => Some(Banner::Downloading),
            Phase::Ready if !closed => Some(Banner::Ready),
            Phase::InstallFailed(_) if !closed => Some(Banner::Failed),
            _ => None,
        }
    }

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Downloading(_))
    }

    fn auto_checks(&self) -> bool {
        self.prefs.auto_check && self.target.is_ok() && !crate::env::flag("HERONITE_NO_UPDATE_CHECK")
    }

    /// (Re)arms the automatic checks from the current switches.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if !self.auto_checks() {
            return;
        }
        let since_last = self.prefs.last_check.map(|t| Duration::from_secs(now_secs().saturating_sub(t)));
        let first = match since_last {
            Some(age) if age < RECHECK_AFTER => CHECK_EVERY.saturating_sub(age).max(FIRST_CHECK),
            _ => FIRST_CHECK,
        };
        // What the last check found is offered at once, without a request.
        self.adopt_stored();
        self.timer = Some(cx.spawn(async move |this, cx| {
            let mut wait = first;
            loop {
                cx.background_executor().timer(wait).await;
                if this.update(cx, |u, cx| u.check(false, cx)).is_err() {
                    break;
                }
                wait = CHECK_EVERY;
            }
        }));
    }

    /// Shows the release stored by the last check when it is still newer.
    fn adopt_stored(&mut self) {
        if self.phase != Phase::Idle {
            return;
        }
        if let Some(r) = self.prefs.latest.clone().filter(|r| r.is_newer_than(&self.current)) {
            self.release = Some(r);
            self.phase = Phase::Available;
        }
    }

    /// Asks GitHub for the latest release. `manual`: the user asked, so a
    /// failure is shown and the "skip" choice does not hide the result.
    pub fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.busy() || self.phase == Phase::Ready || self.debug {
            return;
        }
        let previous = std::mem::replace(&mut self.phase, Phase::Checking);
        cx.notify();
        let api = self.api_url.clone();
        let etag = if manual { None } else { self.prefs.etag.clone() };
        let job = cx.background_spawn(async move { heronite_update::release::fetch_latest(&api, etag.as_deref()) });
        self.job = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |u, cx| u.check_finished(result, previous, manual, cx)).ok();
        }));
    }

    fn check_finished(&mut self, result: anyhow::Result<Fetched>, previous: Phase, manual: bool, cx: &mut Context<Self>) {
        self.job = None;
        let latest = match result {
            Ok(Fetched::NotModified) => self.prefs.latest.clone(),
            Ok(Fetched::Latest { release, etag }) => {
                self.prefs.etag = etag;
                self.prefs.latest = Some((*release).clone());
                Some(*release)
            }
            Err(e) => {
                log::warn!("update check failed: {e:#}");
                self.phase = if manual { Phase::CheckFailed(format!("{e:#}")) } else { previous };
                cx.notify();
                return;
            }
        };
        self.prefs.last_check = Some(now_secs());
        self.save_prefs(cx);
        match latest.filter(|r| r.is_newer_than(&self.current)) {
            Some(release) => {
                log::info!("Heronite {} is available (this is {})", release.version, self.current);
                self.release = Some(release);
                self.phase = Phase::Available;
                if manual {
                    // Asked for: show it even if it was skipped or closed.
                    self.dismissed = None;
                }
                if self.prefs.auto_install && self.can_install() && !self.skipped() {
                    self.install(cx);
                    return;
                }
                self.notify_hidden(cx);
            }
            None => {
                self.release = None;
                self.phase = Phase::UpToDate;
            }
        }
        cx.notify();
    }

    /// With the window closed to the tray nobody sees the banner: tell the
    /// desktop once per version.
    fn notify_hidden(&mut self, cx: &App) {
        let Some(r) = &self.release else { return };
        if crate::system::has_window(cx) || self.skipped() || self.notified.as_deref() == Some(r.version.as_str()) {
            return;
        }
        self.notified = Some(r.version.clone());
        crate::system::notify::send(format!("Heronite {} is available", r.version), "Open Heronite to update.");
    }

    /// Downloads the release and swaps it in.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.busy() || self.debug {
            return;
        }
        let (Ok(target), Some(release)) = (self.target.clone(), self.release.clone()) else { return };
        if !release.installable() {
            return;
        }
        log::info!("downloading Heronite {} into {}", release.version, target.appimage.display());
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.phase = Phase::Downloading(Progress::default());
        cx.notify();
        let (tx, mut rx) = unbounded::<Progress>();
        let job = cx.background_spawn(async move {
            heronite_update::install::install(&release, &target, heronite_update::PUBLIC_KEY, &cancel, &mut |p| {
                let _ = tx.unbounded_send(p);
            })
        });
        self.job = Some(cx.spawn(async move |this, cx| {
            // Ends when the download drops its sender.
            while let Some(mut p) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    p = newer;
                }
                let alive = this.update(cx, |u, cx| {
                    if matches!(u.phase, Phase::Downloading(_)) {
                        u.phase = Phase::Downloading(p);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
            let result = job.await;
            this.update(cx, |u, cx| u.install_finished(result, cx)).ok();
        }));
    }

    fn install_finished(&mut self, result: Result<(), InstallError>, cx: &mut Context<Self>) {
        self.job = None;
        self.cancel = None;
        let version = self.release.as_ref().map(|r| r.version.clone()).unwrap_or_default();
        self.phase = match result {
            Ok(()) => {
                log::info!("Heronite {version} is installed; it runs from the next start");
                if !crate::system::has_window(cx) {
                    crate::system::notify::send(format!("Heronite {version} is installed"), "It starts the next time you open Heronite.");
                }
                Phase::Ready
            }
            Err(InstallError::Cancelled) => {
                log::info!("update to {version} cancelled");
                Phase::Available
            }
            Err(InstallError::Failed(e)) => {
                log::error!("update to {version} failed: {e}");
                Phase::InstallFailed(e)
            }
        };
        self.dismissed = None;
        if self.phase == Phase::Ready && crate::env::flag("HERONITE_DEBUG_UPDATE_RESTART") {
            self.restart(cx);
        }
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
        if self.debug {
            self.phase = Phase::Available;
        }
        cx.notify();
    }

    /// "Later": hides the banner of this version for this session.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.dismissed = self.release.as_ref().map(|r| r.version.clone());
        if matches!(self.phase, Phase::InstallFailed(_)) {
            self.phase = Phase::Available;
        }
        cx.notify();
    }

    /// "Skip this version" (or undo it): no banner nor notification for it.
    pub fn set_skipped(&mut self, skip: bool, cx: &mut Context<Self>) {
        self.prefs.skipped_version = if skip { self.release.as_ref().map(|r| r.version.clone()) } else { None };
        self.save_prefs(cx);
        cx.notify();
    }

    pub fn set_auto_check(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prefs.auto_check = on;
        self.save_prefs(cx);
        self.schedule(cx);
        cx.notify();
    }

    pub fn set_auto_install(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prefs.auto_install = on;
        self.save_prefs(cx);
        if on && self.phase == Phase::Available && self.can_install() && !self.skipped() {
            self.install(cx);
        }
        cx.notify();
    }

    /// Restarts into the new version (when no game or post-game work keeps
    /// the app; see [`crate::system::restart_blocker`]).
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Ready || self.debug {
            return;
        }
        let Ok(target) = &self.target else { return };
        let appimage = target.appimage.clone();
        cx.defer(move |cx| crate::system::restart_into(appimage, cx));
    }

    fn save_prefs(&self, cx: &mut Context<Self>) {
        if self.debug {
            return;
        }
        let (prefs, path) = (self.prefs.clone(), self.app.update_state());
        cx.background_spawn(async move {
            if let Err(e) = prefs.save(&path) {
                log::warn!("could not save the update settings: {e:#}");
            }
        })
        .detach();
    }
}
