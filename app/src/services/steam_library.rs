//! The Steam library service: reads the account's library from the running
//! Steam client and joins it to the Collection (`heronite_steam::library`).
//! It runs a few seconds after start, then every few seconds until the
//! library was read once (so a Steam opened later is read at once), then
//! every few minutes, and when asked (the refresh button, the Steam page).
//! It also opens and restarts Steam for the welcome. The Collection reloads
//! on [`SteamLibraryEvent::Imported`].

use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::steam_owned::SteamOwnedFile;
use heronite_data::DataPaths;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::{find_steam_binary, SteamLocator};
use heronite_launch::SystemProbe;
use heronite_steam::library::ImportReport;
use heronite_steam::{LibraryAccess, Steam};

const FIRST_DELAY: Duration = Duration::from_secs(6);
/// Until the library was read once: cheap local checks (a pid file, the
/// process list, a request to localhost).
const FAST: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_secs(10 * 60);
/// How long "Opening" shows after Heronite started Steam while Steam does
/// not run yet (its updater can take a while to show up).
const OPENING: Duration = Duration::from_secs(90);

/// After Heronite started Steam, how long an old Steam process may linger.
const SETTLE: Duration = Duration::from_secs(20);

/// The `steam` command: `HERONITE_STEAM_BIN` (tests), else the one found.
pub fn steam_bin() -> String {
    crate::env::value("HERONITE_STEAM_BIN").or_else(|| find_steam_binary(&SystemProbe)).unwrap_or_else(|| "steam".into())
}

/// The Steam install, read fresh (the Steam folder setting may change).
pub fn steam(paths: &DataPaths) -> Steam {
    let locator = SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe);
    Steam::new(locator, crate::system::shortcuts::steam_client())
}

/// Reads the library and joins it. Blocking.
fn sync_blocking(paths: &DataPaths) -> Result<ImportReport, String> {
    let owned = steam(paths).read_library()?;
    heronite_steam::library::import(paths, owned, find_steam_binary(&SystemProbe).as_deref()).map_err(|e| format!("{e:#}"))
}

/// What the cached library holds: games, installed ones, when it was read.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub games: usize,
    pub installed: usize,
    pub updated_at: Option<String>,
}

pub enum SteamLibraryEvent {
    Imported(ImportReport),
}

/// Where getting the library stands, for the welcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteamState {
    Checking,
    NotInstalled,
    Closed,
    /// Starting, updating or waiting for the sign-in.
    Opening,
    Restarting,
    /// Steam was open before Heronite could turn the access on.
    NeedsRestart,
    /// Restarted from here and still without the access: not asked again.
    Failed,
    Reading,
    /// Read: the library had nothing new.
    Connected,
}

pub struct SteamLibrary {
    paths: DataPaths,
    pub syncing: bool,
    /// The last read asked for by the user (automatic ones are quiet).
    pub last: Option<Result<ImportReport, String>>,
    /// `None` until checked.
    pub access: Option<LibraryAccess>,
    pub summary: Summary,
    /// When Heronite last started Steam.
    opened_at: Option<Instant>,
    restarting: bool,
    /// Steam was restarted from here: if the access still does not come,
    /// asking again would not help.
    restarted: bool,
    /// The last open or restart failed.
    pub error: Option<String>,
}

impl EventEmitter<SteamLibraryEvent> for SteamLibrary {}

impl SteamLibrary {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            paths,
            syncing: false,
            last: None,
            access: None,
            summary: Summary::default(),
            opened_at: None,
            restarting: false,
            restarted: false,
            error: None,
        };
        this.load_summary(cx);
        // Off for tests against real data, like the other idle jobs.
        if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                loop {
                    let Ok(read) = this.update(cx, |this, cx| {
                        this.sync_if_ready(cx);
                        this.summary.updated_at.is_some()
                    }) else {
                        return;
                    };
                    cx.background_executor().timer(if read { INTERVAL } else { FAST }).await;
                }
            })
            .detach();
        }
        this
    }

    pub fn state(&self) -> SteamState {
        if self.restarting {
            return SteamState::Restarting;
        }
        if self.syncing {
            return SteamState::Reading;
        }
        let opening = self.opened_at.is_some_and(|t| t.elapsed() < OPENING);
        match self.access {
            None => SteamState::Checking,
            Some(LibraryAccess::Ready) => SteamState::Connected,
            Some(LibraryAccess::Starting) => SteamState::Opening,
            Some(LibraryAccess::Closed) if opening => SteamState::Opening,
            Some(LibraryAccess::Closed) => SteamState::Closed,
            // Right after the restart, what is left of the old Steam may still show.
            Some(LibraryAccess::NeedsRestart) if self.opened_at.is_some_and(|t| t.elapsed() < SETTLE) => SteamState::Opening,
            Some(LibraryAccess::NeedsRestart) if self.restarted => SteamState::Failed,
            Some(LibraryAccess::NeedsRestart) => SteamState::NeedsRestart,
            Some(LibraryAccess::NotInstalled) => SteamState::NotInstalled,
        }
    }

    /// Starts Steam (with the access on); the checks read it once it answers.
    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.opened_at = Some(Instant::now());
        self.error = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { steam(&paths).open(&steam_bin()) }).await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    this.opened_at = None;
                    this.error = Some(e);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Closes Steam and starts it again with the access on.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.restarting {
            return;
        }
        self.restarting = true;
        self.error = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { steam(&paths).restart(&steam_bin()) }).await;
            this.update(cx, |this, cx| {
                this.restarting = false;
                match res {
                    Ok(()) => {
                        this.restarted = true;
                        this.opened_at = Some(Instant::now());
                        // Not the old "needs a restart" until the new Steam is checked.
                        this.access = Some(LibraryAccess::Closed);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn load_summary(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let summary = cx
                .background_spawn(async move {
                    let f = SteamOwnedFile::read(&paths);
                    Summary { games: f.games.len(), installed: f.games.iter().filter(|g| g.installed).count(), updated_at: f.updated_at }
                })
                .await;
            this.update(cx, |this, cx| {
                this.summary = summary;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Checks whether the client can be read now (for the Steam page).
    pub fn check(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let access = cx.background_spawn(async move { steam(&paths).library_access() }).await;
            this.update(cx, |this, cx| {
                this.access = Some(access);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A quiet read (the timer, the refresh button): only when the client answers.
    pub fn sync_if_ready(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let access = cx.background_spawn(async move { steam(&paths).library_access() }).await;
            this.update(cx, |this, cx| {
                this.access = Some(access);
                if access == LibraryAccess::Ready {
                    this.run(false, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the library now. `asked`: the user asked (the result is kept
    /// for the Steam page); otherwise failures are only logged.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.run(true, cx);
    }

    fn run(&mut self, asked: bool, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { sync_blocking(&paths) }).await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                match &res {
                    Ok(report) => {
                        this.access = Some(LibraryAccess::Ready);
                        cx.emit(SteamLibraryEvent::Imported(*report));
                    }
                    Err(e) => log::warn!("Steam library: {e}"),
                }
                if asked || res.is_ok() {
                    this.last = Some(res);
                }
                this.load_summary(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
