//! App updates (`heronite_update`): looks for a newer GitHub release and
//! installs it the way this copy was installed ([`Method`]):
//! - AppImage: downloads it, swaps it in place, restarts;
//! - our .deb / .rpm / Arch package: downloads the release's package of the
//!   same format and hands it to apt / dnf / pacman through `pkexec` (the
//!   desktop's password prompt), then restarts. Never automatic: it asks for
//!   a password.
//! - Windows, installed by our setup: downloads the release's setup, which
//!   runs silently as the app quits (a restart, or a normal quit) and
//!   replaces the files once this process is gone ([`crate::system::run_on_quit`]);
//! - Windows, portable copy: downloads the release's zip and swaps its
//!   program files in place (the old ones kept as `.old` until the next
//!   start), restarts.
//!
//! Anything else (built from source, a distribution's own package, a
//! Windows build folder) is updated by whatever installed it, so it only
//! checks when asked.
//!
//! Checks run [`FIRST_CHECK`] after the start (skipped when the last one is
//! younger than [`RECHECK_AFTER`]) and every [`CHECK_EVERY`] while "Check for
//! updates automatically" is on. A failed background check stays quiet; a
//! failed manual check or install is shown.
//!
//! The Collection's header draws the pill ([`Updater::banner`]) and Settings
//! → About the full card; both observe this entity. The first start of a
//! version this app installed opens its "What's new" once
//! ([`Updater::take_whats_new`]).
//!
//! Switches: `HERONITE_NO_UPDATE_CHECK=1` turns the automatic checks off;
//! `HERONITE_UPDATE_API=<url>` asks another endpoint (local tests);
//! `HERONITE_DEBUG_UPDATE=checking|uptodate|available|downloading|installing|ready|failed`
//! shows that state with a made-up release, without any request;
//! `HERONITE_DEBUG_UPDATE=simulate` offers a made-up release whose Update
//! fakes the download and whose Restart opens "What's new" instead
//! (nothing is downloaded, written or restarted; `simulate-auto` presses
//! Update by itself); `HERONITE_DEBUG_UPDATE=whatsnew` opens that "What's
//! new" at once;
//! `HERONITE_DEBUG_UPDATE_PACKAGE=deb|rpm|arch` pretends this copy came from
//! that package (with `HERONITE_DEBUG_UPDATE`; on Windows `setup|portable`,
//! the setup by default);
//! `HERONITE_DEBUG_UPDATE_KEY=<minisign public key>` (debug builds only)
//! trusts that key instead of the release key, for end-to-end tests with a
//! local `HERONITE_UPDATE_API`;
//! `HERONITE_DEBUG_UPDATE_RESTART=1` restarts as soon as an update is
//! installed (end-to-end tests without clicks).

mod debug;
pub mod notes;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;
use heronite_update::{Asset, Fetched, InstallError, Package, Prefs, Progress, Release, Target, Unsupported, Version};
#[cfg(windows)]
use heronite_update::{Installer, Portable, WindowsAsset};

const FIRST_CHECK: Duration = Duration::from_secs(3);
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
    /// The package manager is installing the downloaded package.
    Installing,
    /// The new version is in place; it runs from the next start. For a
    /// copy installed by the Windows setup: the setup is downloaded and runs
    /// when the app quits ([`Updater::installs_on_quit`]).
    Ready,
    /// A check the user asked for failed.
    CheckFailed(String),
    InstallFailed(String),
}

/// What the Collection's update pill shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banner {
    Available,
    Downloading,
    Ready,
    Failed,
}

/// How this copy is updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    #[cfg_attr(windows, allow(dead_code))]
    AppImage(Target),
    /// Our system package; `exe` is this binary, read at start (once the
    /// package manager replaced it, `/proc/self/exe` names a deleted file).
    Package { kind: Package, exe: PathBuf },
    /// Installed by our Windows setup.
    #[cfg(windows)]
    Installer(Installer),
    /// A portable copy on Windows.
    #[cfg(windows)]
    Portable(Portable),
}

impl Method {
    /// A copy installed by our setup, else a portable one (the setup owns
    /// the files it installed, even next to a portable marker).
    #[cfg(windows)]
    fn detect() -> Result<Self, Unsupported> {
        let exe = std::env::current_exe().map_err(|_| Unsupported::NotAppImage)?;
        if let Some(installer) = Installer::detect(&exe) {
            return Ok(Method::Installer(installer));
        }
        let root = heronite_platform::dirs::portable_root().ok_or(Unsupported::NotAppImage)?;
        Portable::detect(root, exe).map(Method::Portable)
    }

    #[cfg(not(windows))]
    fn detect() -> Result<Self, Unsupported> {
        // Not `$APPIMAGE` as is: it may be another AppImage's, inherited.
        match Target::detect(crate::system::own_exe::own_appimage().map(Into::into)) {
            Ok(t) => Ok(Method::AppImage(t)),
            Err(Unsupported::NotAppImage) => {
                let exe = std::env::current_exe().map_err(|_| Unsupported::NotAppImage)?;
                let kind = Package::detect(&exe).ok_or(Unsupported::NotAppImage)?;
                Ok(Method::Package { kind, exe })
            }
            Err(e) => Err(e),
        }
    }

    /// What to start after a restart.
    fn executable(&self) -> PathBuf {
        match self {
            Method::AppImage(t) => t.appimage.clone(),
            Method::Package { exe, .. } => exe.clone(),
            #[cfg(windows)]
            Method::Installer(i) => i.exe.clone(),
            #[cfg(windows)]
            Method::Portable(p) => p.exe.clone(),
        }
    }
}

/// What the background install reports.
enum Step {
    Progress(Progress),
    Installing,
}

pub struct Updater {
    app: AppPaths,
    prefs: Prefs,
    method: Result<Method, Unsupported>,
    current: Version,
    phase: Phase,
    /// The newest release when it is newer than this build.
    release: Option<Release>,
    /// Version whose banner was closed in this session.
    dismissed: Option<String>,
    /// Version already announced by a desktop notification.
    notified: Option<String>,
    /// The downloaded package, while it waits for or failed its install (on
    /// Windows the setup, until the app quits).
    downloaded: Option<PathBuf>,
    cancel: Option<Arc<AtomicBool>>,
    job: Option<Task<()>>,
    timer: Option<Task<()>>,
    api_url: String,
    /// The release this start is the first run of, for its "What's new".
    whats_new: Option<Release>,
    /// `HERONITE_DEBUG_UPDATE`: made-up states, no requests or writes.
    debug: bool,
    /// `HERONITE_DEBUG_UPDATE=simulate`: Update and Restart are faked.
    simulate: bool,
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
    if let Err(e) = heronite_platform::shell::open(url) {
        log::warn!("opening {url}: {e}");
    }
}

/// `14.7 MiB`.
pub fn size_text(bytes: u64) -> String {
    crate::services::widgets::fmt_mib(bytes as f64 / 1_048_576.)
}

/// `6.1 MiB of 14.7 MiB · 2.3 MiB/s`.
pub fn progress_text(p: &Progress) -> String {
    let mut text = if p.total > 0 {
        tr!("{done} of {total}", done = size_text(p.downloaded), total = size_text(p.total))
    } else {
        size_text(p.downloaded)
    };
    if p.bytes_per_sec > 0. {
        text.push_str(&format!(" \u{b7} {}/s", size_text(p.bytes_per_sec as u64)));
    }
    text
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago_text(unix_secs: u64) -> String {
    let secs = now_secs().saturating_sub(unix_secs);
    match secs {
        0..=59 => tr!("just now").into(),
        60..=3599 => tr!("{n} min ago", n = secs / 60),
        3600..=86_399 => tr!("{n} h ago", n = secs / 3600),
        _ if secs < 2 * 86_400 => tr!("yesterday").into(),
        _ => tr!("{n} days ago", n = secs / 86_400),
    }
}

impl Updater {
    fn new(app: AppPaths, cx: &mut Context<Self>) -> Self {
        let prefs = Prefs::load(&app.update_state());
        let method = Method::detect();
        match &method {
            Ok(Method::AppImage(t)) => {
                log::info!("running from {}: updates can be installed", t.appimage.display());
                // A download cut short by a quit or a crash.
                let t = t.clone();
                cx.background_spawn(async move { t.sweep_staging() }).detach();
            }
            #[cfg(windows)]
            Ok(Method::Installer(i)) => {
                log::info!("installed by the setup in {}: updates run the new setup as the app quits", i.exe.display());
                // Setups of earlier updates. The one that started this
                // process may still be finishing: its file stays until the
                // next start (each version downloads into its own folder).
                let dir = Self::downloads_dir(&app);
                cx.background_spawn(async move {
                    let _ = std::fs::remove_dir_all(dir);
                })
                .detach();
            }
            #[cfg(windows)]
            Ok(Method::Portable(p)) => {
                log::info!("portable copy in {}: updates are swapped in place", p.root.display());
                // An unfinished update and the files the last one replaced
                // (the previous process may still be exiting: it waits a bit).
                let p = p.clone();
                let _ = std::thread::Builder::new().name("update-sweep".into()).spawn(move || p.sweep());
            }
            Ok(Method::Package { kind, .. }) => {
                log::info!("installed from a {} package: updates go through its package manager", kind.label());
                // Packages of earlier updates (installed or not).
                let dir = Self::downloads_dir(&app);
                cx.background_spawn(async move {
                    let _ = std::fs::remove_dir_all(dir);
                })
                .detach();
            }
            Err(Unsupported::NotAppImage) => {}
            Err(why) => log::info!("updates cannot be installed in place: {why}"),
        }
        let mut this = Self {
            app,
            prefs,
            method,
            current: Version::current(),
            phase: Phase::Idle,
            release: None,
            dismissed: None,
            notified: None,
            downloaded: None,
            cancel: None,
            job: None,
            timer: None,
            api_url: crate::env::value("HERONITE_UPDATE_API").unwrap_or_else(|| heronite_update::LATEST_API.to_string()),
            whats_new: None,
            debug: false,
            simulate: false,
        };
        if let Some(state) = crate::env::value("HERONITE_DEBUG_UPDATE") {
            this.debug_state(&state, cx);
            return this;
        }
        if let Some(arrived) = arrival(&mut this.prefs, &this.current) {
            this.whats_new = arrived;
            this.save_prefs(cx);
        }
        this.schedule(cx);
        this
    }

    /// A "What's new" waits for the Collection.
    pub fn has_whats_new(&self) -> bool {
        self.whats_new.is_some()
    }

    /// The release whose "What's new" opens now (once).
    pub fn take_whats_new(&mut self) -> Option<Release> {
        self.whats_new.take()
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
        self.method.as_ref().err()
    }

    /// The package format this copy was installed from.
    pub fn package(&self) -> Option<Package> {
        match &self.method {
            Ok(Method::Package { kind, .. }) => Some(*kind),
            _ => None,
        }
    }

    /// The Windows download this copy updates from.
    #[cfg(windows)]
    fn windows_asset(&self) -> Option<WindowsAsset> {
        match &self.method {
            Ok(Method::Installer(_)) => Some(WindowsAsset::Setup),
            Ok(Method::Portable(_)) => Some(WindowsAsset::Portable),
            _ => None,
        }
    }

    /// The file an update downloads (the AppImage, this format's package,
    /// the Windows setup or portable zip).
    pub fn download_asset(&self) -> Option<&Asset> {
        let release = self.release.as_ref()?;
        #[cfg(windows)]
        if let Some(kind) = self.windows_asset() {
            return release.windows_asset(kind);
        }
        match &self.method {
            Ok(Method::Package { kind, .. }) => release.package(*kind),
            _ => release.appimage.as_ref(),
        }
    }

    /// The release can be downloaded and installed from here.
    pub fn can_install(&self) -> bool {
        let Some(release) = &self.release else { return false };
        match &self.method {
            Ok(Method::AppImage(_)) => release.installable(),
            Ok(Method::Package { kind, .. }) => release.package_signature(*kind).is_some(),
            #[cfg(windows)]
            Ok(Method::Installer(_) | Method::Portable(_)) => {
                self.windows_asset().is_some_and(|k| release.windows_asset(k).is_some() && release.windows_signature(k).is_some())
            }
            Err(_) => false,
        }
    }

    /// Updates may install without a click (not packages: they ask for a
    /// password).
    pub fn can_auto_install(&self) -> bool {
        match &self.method {
            Ok(Method::AppImage(_)) => true,
            #[cfg(windows)]
            Ok(Method::Installer(_) | Method::Portable(_)) => true,
            _ => false,
        }
    }

    /// A ready update installs when the app quits (the Windows setup)
    /// instead of being in place already.
    pub fn installs_on_quit(&self) -> bool {
        #[cfg(windows)]
        if let Ok(Method::Installer(_)) = &self.method {
            return true;
        }
        false
    }

    /// The terminal command for a downloaded package whose install failed.
    pub fn manual_command(&self) -> Option<String> {
        let file = self.downloaded.as_ref()?;
        Some(self.package()?.install_command(&file.display().to_string()))
    }

    fn downloads_dir(app: &AppPaths) -> PathBuf {
        app.cache.join("updates")
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
            Phase::Downloading(_) | Phase::Installing => Some(Banner::Downloading),
            Phase::Ready if !closed => Some(Banner::Ready),
            Phase::InstallFailed(_) if !closed => Some(Banner::Failed),
            _ => None,
        }
    }

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Downloading(_) | Phase::Installing)
    }

    fn auto_checks(&self) -> bool {
        self.prefs.auto_check && self.method.is_ok() && !crate::env::flag("HERONITE_NO_UPDATE_CHECK")
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
                if self.prefs.auto_install && self.can_auto_install() && self.can_install() && !self.skipped() {
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
        crate::system::notify::send(tr!("Heronite {version} is available", version = r.version), tr!("Open Heronite to update."));
    }

    /// Downloads the release and installs it ([`Method`]).
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.busy() || !self.can_install() {
            return;
        }
        if self.debug {
            if self.simulate {
                self.debug_install(cx);
            }
            return;
        }
        let (Ok(method), Some(release)) = (self.method.clone(), self.release.clone()) else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.phase = Phase::Downloading(Progress::default());
        self.downloaded = None;
        cx.notify();
        let (tx, mut rx) = unbounded::<Step>();
        let dir = Self::downloads_dir(&self.app);
        // The result, and the downloaded package when its install failed (or
        // the Windows setup, which runs as the app quits).
        let key = debug::public_key();
        let job = cx.background_spawn(async move {
            let key = key.as_str();
            let mut progress = |p| {
                let _ = tx.unbounded_send(Step::Progress(p));
            };
            match method {
                Method::AppImage(target) => {
                    log::info!("downloading Heronite {} into {}", release.version, target.appimage.display());
                    (heronite_update::install::install(&release, &target, key, &cancel, &mut progress), None)
                }
                #[cfg(windows)]
                Method::Installer(_) => {
                    log::info!("downloading the setup of Heronite {} into {}", release.version, dir.display());
                    match Installer::download(&release, &dir.join(&release.version), key, &cancel, &mut progress) {
                        // Kept: it runs as the app quits.
                        Ok(setup) => (Ok(()), Some(setup)),
                        Err(e) => (Err(e), None),
                    }
                }
                #[cfg(windows)]
                Method::Portable(copy) => {
                    log::info!("downloading Heronite {} into {}", release.version, copy.root.display());
                    (heronite_update::portable::install(&release, &copy, key, &cancel, &mut progress), None)
                }
                Method::Package { kind, .. } => {
                    let (Some(asset), Some(sig)) = (release.package(kind), release.package_signature(kind)) else {
                        return (Err(InstallError::Failed(tr!("This release has no signed package for this system").into())), None);
                    };
                    log::info!("downloading {} into {}", asset.name, dir.display());
                    let file =
                        match heronite_update::install::download_signed(asset, sig, &release.version, &dir, key, &cancel, &mut progress) {
                            Ok(file) => file,
                            Err(e) => return (Err(e), None),
                        };
                    let _ = tx.unbounded_send(Step::Installing);
                    log::info!("installing {} with the package manager", file.display());
                    match kind.install(&file) {
                        Ok(()) => {
                            let _ = std::fs::remove_file(&file);
                            (Ok(()), None)
                        }
                        Err(e) => (Err(e), Some(file)),
                    }
                }
            }
        });
        self.job = Some(cx.spawn(async move |this, cx| {
            // Ends when the job drops its sender.
            while let Some(mut step) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    step = newer;
                }
                let alive = this.update(cx, |u, cx| {
                    match step {
                        Step::Progress(p) if matches!(u.phase, Phase::Downloading(_)) => u.phase = Phase::Downloading(p),
                        Step::Installing => u.phase = Phase::Installing,
                        Step::Progress(_) => {}
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let (result, kept) = job.await;
            this.update(cx, |u, cx| {
                u.downloaded = kept;
                u.install_finished(result, cx)
            })
            .ok();
        }));
    }

    fn install_finished(&mut self, result: Result<(), InstallError>, cx: &mut Context<Self>) {
        self.job = None;
        self.cancel = None;
        let version = self.release.as_ref().map(|r| r.version.clone()).unwrap_or_default();
        self.phase = match result {
            Ok(()) if self.installs_on_quit() => {
                log::info!("the setup of Heronite {version} is downloaded; it runs when the app quits");
                self.prefs.updated_to = Some(version.clone());
                self.save_prefs(cx);
                self.setup_on_quit();
                if !crate::system::has_window(cx) {
                    crate::system::notify::send(
                        tr!("Heronite {version} is ready to install", version),
                        tr!("It installs when you quit Heronite."),
                    );
                }
                Phase::Ready
            }
            Ok(()) => {
                log::info!("Heronite {version} is installed; it runs from the next start");
                self.prefs.updated_to = Some(version.clone());
                self.save_prefs(cx);
                if !crate::system::has_window(cx) {
                    crate::system::notify::send(
                        tr!("Heronite {version} is installed", version),
                        tr!("It starts the next time you open Heronite."),
                    );
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

    /// The downloaded Windows setup runs once the app has quit: it waits for
    /// this process to end, installs silently and, after a restart, starts
    /// the new version itself. A failure to start it is only logged (a
    /// restart then starts this version again).
    fn setup_on_quit(&self) {
        #[cfg(windows)]
        if let Some(setup) = self.downloaded.clone() {
            let log = self.app.cache.join("update-setup.log");
            crate::system::run_on_quit(Some(Box::new(move |relaunch| match Installer::run_setup(&setup, relaunch, Some(&log)) {
                Ok(pid) => {
                    log::info!("started the setup {} (pid {pid}, log {})", setup.display(), log.display());
                    relaunch
                }
                Err(e) => {
                    log::error!("could not start the setup {}: {e}", setup.display());
                    false
                }
            })));
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
        if self.debug {
            self.job = None;
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
        if on && self.phase == Phase::Available && self.can_auto_install() && self.can_install() && !self.skipped() {
            self.install(cx);
        }
        cx.notify();
    }

    /// Restarts into the new version (when no game or post-game work keeps
    /// the app; see [`crate::system::restart_blocker`]).
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Ready {
            return;
        }
        if self.debug {
            if self.simulate {
                self.debug_restart(cx);
            }
            return;
        }
        let Ok(method) = &self.method else { return };
        let exe = method.executable();
        cx.defer(move |cx| crate::system::restart_into(exe, cx));
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

/// What a start finds of the update this app installed: `None` when nothing
/// changed, `Some(release)` when this start is the first of the version it
/// installed (with its notes when the last check described it). The
/// version is forgotten once it runs or once an older one is running.
fn arrival(prefs: &mut Prefs, current: &Version) -> Option<Option<Release>> {
    let to = Version::parse(prefs.updated_to.as_deref()?);
    if to.as_ref().is_some_and(|v| v > current) {
        // Not running yet (a restart is still due).
        return None;
    }
    prefs.updated_to = None;
    let notes = prefs.latest.clone().filter(|r| to.as_ref() == Some(current) && Version::parse(&r.version).as_ref() == Some(current));
    Some(notes)
}

#[cfg(test)]
mod tests {
    use super::arrival;
    use heronite_update::{Prefs, Release, Version};

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            name: String::new(),
            notes: "### Fixes\n- A fix".into(),
            page_url: String::new(),
            published_at: String::new(),
            appimage: None,
            signature: None,
            packages: Vec::new(),
            windows: Vec::new(),
        }
    }

    #[test]
    fn the_first_start_of_an_installed_version_shows_its_notes_once() {
        let current = Version::parse("0.17.1").unwrap();
        let mut prefs = Prefs { updated_to: Some("0.17.1".into()), latest: Some(release("0.17.1")), ..Default::default() };
        assert_eq!(arrival(&mut prefs, &current), Some(Some(release("0.17.1"))));
        assert_eq!(prefs.updated_to, None);
        assert_eq!(arrival(&mut prefs, &current), None, "only once");

        // Installed but not restarted yet: kept for the next start.
        let mut waiting = Prefs { updated_to: Some("0.17.2".into()), ..Default::default() };
        assert_eq!(arrival(&mut waiting, &current), None);
        assert_eq!(waiting.updated_to.as_deref(), Some("0.17.2"));

        // An older one (updated again by other means): forgotten, no notes.
        let mut old = Prefs { updated_to: Some("0.16.0".into()), latest: Some(release("0.17.1")), ..Default::default() };
        assert_eq!(arrival(&mut old, &current), Some(None));
        assert_eq!(old.updated_to, None);

        // The notes of another version are not shown.
        let mut other = Prefs { updated_to: Some("0.17.1".into()), latest: Some(release("0.18.0")), ..Default::default() };
        assert_eq!(arrival(&mut other, &current), Some(None));
    }
}
