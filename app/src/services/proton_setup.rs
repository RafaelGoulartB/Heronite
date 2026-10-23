//! A Proton to run Windows games with when this PC has none: the first time
//! an install dialog finds no Wine or Proton, the newest GE-Proton is
//! downloaded and installed on its own, and becomes the default
//! (`heronite_wine::default::ensure_default`, run by the install). The dialogs
//! show its progress and pick it once it is there.
//!
//! It also keeps the installed `-latest` builds up to date (Wine Manager's
//! "Keep the -latest builds up to date", `autoUpdateLatestProton`, on unless
//! turned off): a few minutes after the start and then every
//! [`UPDATE_EVERY`], only the sources that have one are checked
//! (`heronite_wine::auto`), which reads their release feed and calls GitHub's
//! API only when something was published. A new release downloads in the
//! background, never while a game runs: a game starting meanwhile stops the
//! download, which tries again later.
//!
//! The service owns the app's `WineManager`, which the Wine Manager page
//! shares: the page shows the download, and neither can start it twice.
//! `HERONITE_WINE_OFFLINE=1` keeps it from asking the network.

use std::time::Duration;

use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_wine::{CheckMode, JobPhase, WineConfig, WineManager};

/// After the start, before the first look for new `-latest` releases.
const FIRST_UPDATE_CHECK: Duration = Duration::from_secs(3 * 60);
/// Between two looks; the release feeds are not read more often anyway
/// (`heronite_wine::checks::AUTO_EVERY`).
const UPDATE_EVERY: Duration = heronite_wine::checks::AUTO_EVERY;
/// While a game runs, the update waits this long and tries again.
const WHILE_PLAYING: Duration = Duration::from_secs(10 * 60);
/// How often a check or a download in progress is looked at.
const POLL: Duration = Duration::from_millis(500);

/// Where the automatic GE-Proton is.
#[derive(Clone, Debug, PartialEq)]
pub enum Setup {
    /// Not asked for, or a Wine or Proton is already there.
    Idle,
    /// Looking for the newest release.
    Looking,
    Downloading {
        version: String,
        /// 0-100.
        percentage: f64,
    },
    Unzipping {
        version: String,
    },
    /// Installed and made the default.
    Ready {
        version: String,
    },
    Failed(String),
}

impl Setup {
    /// Looking, downloading or unzipping.
    pub fn busy(&self) -> bool {
        matches!(self, Setup::Looking | Setup::Downloading { .. } | Setup::Unzipping { .. })
    }
}

pub struct ProtonSetup {
    data_paths: DataPaths,
    manager: WineManager,
    state: Setup,
}

struct ProtonSetupGlobal(Entity<ProtonSetup>);
impl Global for ProtonSetupGlobal {}

/// The service, created on first use.
pub fn get(cx: &mut App) -> Entity<ProtonSetup> {
    if let Some(g) = cx.try_global::<ProtonSetupGlobal>() {
        return g.0.clone();
    }
    let data_paths = DataPaths::detect();
    let manager = WineManager::new(data_paths.clone());
    let entity = cx.new(|_| ProtonSetup { data_paths, manager, state: Setup::Idle });
    cx.set_global(ProtonSetupGlobal(entity.clone()));
    entity
}

/// Starts the service and the automatic update of the installed `-latest`
/// builds. Called by the hub at start.
pub fn init(cx: &mut App) {
    let entity = get(cx);
    if !cfg!(target_os = "linux") || crate::env::flag("HERONITE_WINE_OFFLINE") {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FIRST_UPDATE_CHECK).await;
        loop {
            let wait = update_latest(&entity, cx).await;
            cx.background_executor().timer(wait).await;
        }
    })
    .detach();
}

/// One look for new releases of the installed `-latest` builds, and their
/// downloads; gives the time until the next look.
async fn update_latest(entity: &Entity<ProtonSetup>, cx: &mut AsyncApp) -> Duration {
    let (paths, manager) = cx.update(|cx| {
        let s = entity.read(cx);
        (s.data_paths.clone(), s.manager.clone())
    });
    let playing = |cx: &mut AsyncApp| cx.update(|cx| crate::services::session::any_running(cx));
    if playing(cx) {
        return WHILE_PLAYING;
    }
    let p = paths.clone();
    let (on, sources) = cx
        .background_spawn(async move {
            (WineConfig::load(&p).auto_update_latest, heronite_wine::auto::latest_sources(&heronite_wine::store::read_releases(&p)))
        })
        .await;
    if !on || sources.is_empty() {
        return UPDATE_EVERY;
    }
    // Through the manager, in line with the Wine Manager's own checks.
    for repo in &sources {
        manager.start_check(*repo, CheckMode::Auto);
    }
    while sources.iter().any(|r| manager.checking(*r)) {
        cx.background_executor().timer(POLL).await;
    }
    let p = paths.clone();
    let updates = cx.background_spawn(async move { heronite_wine::auto::latest_updates(&heronite_wine::store::read_releases(&p)) }).await;
    for info in updates {
        if playing(cx) {
            return WHILE_PLAYING;
        }
        let version = info.version.clone();
        log::info!("proton update: {version} has a new release, downloading it");
        manager.start_install(info);
        while let Some(job) = manager.job(&version) {
            // The build may be in use: a game starting stops a download (not
            // the unpacking, which takes seconds), tried again later.
            if matches!(job.phase, JobPhase::Starting | JobPhase::Downloading { .. }) && playing(cx) {
                log::info!("proton update: a game started, {version} waits");
                manager.cancel(&version);
                while manager.job(&version).is_some() {
                    cx.background_executor().timer(POLL).await;
                }
                return WHILE_PLAYING;
            }
            cx.background_executor().timer(POLL).await;
        }
    }
    UPDATE_EVERY
}

/// The app's Wine Manager jobs (installs, removals, release checks).
pub fn manager(cx: &mut App) -> WineManager {
    get(cx).read(cx).manager.clone()
}

/// Where it is, without creating the service.
pub fn state(cx: &App) -> Setup {
    cx.try_global::<ProtonSetupGlobal>().map(|g| g.0.read(cx).state.clone()).unwrap_or(Setup::Idle)
}

/// Downloads the newest GE-Proton when there is no Wine or Proton (an
/// install dialog found none). Nothing happens while it runs or once done.
pub fn ensure(cx: &mut App) {
    get(cx).update(cx, |s, cx| s.ensure(cx));
}

impl ProtonSetup {
    pub fn state(&self) -> &Setup {
        &self.state
    }

    fn set(&mut self, state: Setup, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    fn ensure(&mut self, cx: &mut Context<Self>) {
        if self.state.busy() || matches!(self.state, Setup::Ready { .. }) || !cfg!(target_os = "linux") {
            return;
        }
        if crate::env::flag("HERONITE_WINE_OFFLINE") {
            return;
        }
        self.set(Setup::Looking, cx);
        let data_paths = self.data_paths.clone();
        let manager = self.manager.clone();
        cx.spawn(async move |this, cx| {
            // Another Wine may have appeared since the dialog looked.
            let found = cx
                .background_spawn(async move {
                    if heronite_wine::list_installations(&data_paths).iter().any(heronite_wine::default::usable) {
                        return Ok(None);
                    }
                    heronite_wine::auto::latest_ge(&data_paths).map(Some)
                })
                .await;
            let info = match found {
                Ok(Some(info)) => info,
                Ok(None) => {
                    this.update(cx, |s, cx| s.set(Setup::Idle, cx)).ok();
                    return;
                }
                Err(why) => {
                    log::warn!("proton setup: {why}");
                    this.update(cx, |s, cx| s.set(Setup::Failed(why), cx)).ok();
                    return;
                }
            };
            let version = info.version.clone();
            if !info.installed() {
                log::info!("proton setup: no Wine or Proton here, downloading {version}");
                // `false`: the Wine Manager is installing it already; it is followed all the same.
                manager.start_install(info);
            }
            // Follows the job until it ends.
            while let Some(job) = manager.job(&version) {
                let state = match job.phase {
                    JobPhase::Downloading { percentage, .. } => Setup::Downloading { version: version.clone(), percentage },
                    JobPhase::Unzipping => Setup::Unzipping { version: version.clone() },
                    _ => Setup::Downloading { version: version.clone(), percentage: 0. },
                };
                if this.update(cx, |s, cx| s.set(state, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_millis(250)).await;
            }
            let paths = this.read_with(cx, |s, _| s.data_paths.clone());
            let Ok(paths) = paths else { return };
            let v = version.clone();
            let installed = cx
                .background_spawn(
                    async move { heronite_wine::store::read_releases(&paths).iter().any(|r| r.version == v && r.installed()) },
                )
                .await;
            let state = if installed {
                log::info!("proton setup: {version} installed");
                Setup::Ready { version }
            } else {
                Setup::Failed(heronite_i18n::tr!("The download of {version} did not finish", version = &version))
            };
            this.update(cx, |s, cx| s.set(state, cx)).ok();
        })
        .detach();
    }
}
