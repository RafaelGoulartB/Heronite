//! The Steam library service: reads the account's library from the running
//! Steam client and joins it to the Collection (`heronite_steam::library`).
//! It runs a few seconds after start, then every few minutes while Steam
//! runs, and when asked (the refresh button, the Steam page). The Collection
//! reloads on [`SteamLibraryEvent::Imported`].

use std::time::Duration;

use gpui_kit::*;
use heronite_data::steam_owned::SteamOwnedFile;
use heronite_data::DataPaths;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::{find_steam_binary, SteamLocator};
use heronite_launch::SystemProbe;
use heronite_steam::library::ImportReport;
use heronite_steam::{LibraryAccess, Steam};

const FIRST_DELAY: Duration = Duration::from_secs(6);
const INTERVAL: Duration = Duration::from_secs(10 * 60);

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

pub struct SteamLibrary {
    paths: DataPaths,
    pub syncing: bool,
    /// The last read asked for by the user (automatic ones are quiet).
    pub last: Option<Result<ImportReport, String>>,
    /// `None` until checked.
    pub access: Option<LibraryAccess>,
    pub summary: Summary,
}

impl EventEmitter<SteamLibraryEvent> for SteamLibrary {}

impl SteamLibrary {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let mut this = Self { paths, syncing: false, last: None, access: None, summary: Summary::default() };
        this.load_summary(cx);
        // Off for tests against real data, like the other idle jobs.
        if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                loop {
                    if this.update(cx, |this, cx| this.sync_if_ready(cx)).is_err() {
                        return;
                    }
                    cx.background_executor().timer(INTERVAL).await;
                }
            })
            .detach();
        }
        this
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
