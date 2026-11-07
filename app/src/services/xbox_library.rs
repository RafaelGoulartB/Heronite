//! The Xbox app's games (`heronite_xbox`): the Microsoft Store and PC Game
//! Pass games installed on this PC as Windows packages, kept in the
//! Collection. The service reads them a few seconds after the start, then
//! every few minutes, and when asked (the Xbox page, the refresh button).
//! A read walks the registry and a few small files; nothing is started.
//! After a read, the games with no size yet (or updated since) are measured
//! in the background (`heronite_xbox::size`), one at a time.
//! The Collection reloads on [`XboxLibraryEvent::Synced`].
//!
//! `HERONITE_DISABLE_IDLE_JOBS` turns the timer off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use heronite_data::xbox::{LastSync, XboxPrefs};
use heronite_data::DataPaths;
use heronite_xbox::collection::{self, Report};
use heronite_xbox::XboxGame;

const FIRST_DELAY: Duration = Duration::from_secs(8);
const INTERVAL: Duration = Duration::from_secs(10 * 60);

pub enum XboxLibraryEvent {
    Synced(Report),
}

/// What a read found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub xbox_app: bool,
    pub games: Vec<XboxGame>,
    pub last: Option<LastSync>,
}

pub struct XboxLibrary {
    paths: DataPaths,
    pub syncing: bool,
    /// A measure of the games' sizes runs.
    measuring: bool,
    /// Set when the service goes away: a measure stops.
    stop: Arc<AtomicBool>,
    /// `None` until the first read.
    pub found: Option<Found>,
    /// The last read the user asked for (automatic ones are quiet).
    pub last: Option<Result<Report, String>>,
}

impl EventEmitter<XboxLibraryEvent> for XboxLibrary {}

/// Reads the packages and brings their games into the Collection. Blocking.
fn sync_blocking(paths: &DataPaths) -> Result<(Report, Found), String> {
    let scan = heronite_xbox::scan();
    let report = collection::sync(paths, &scan.games).map_err(|e| format!("{e:#}"))?;
    let found = Found { xbox_app: scan.xbox_app, games: scan.games, last: XboxPrefs::load(paths).last };
    Ok((report, found))
}

impl XboxLibrary {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let this = Self { paths, syncing: false, measuring: false, stop: Arc::new(AtomicBool::new(false)), found: None, last: None };
        if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") && heronite_platform::supported(heronite_platform::Feature::XboxStore) {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                loop {
                    if this.update(cx, |this, cx| this.run(false, cx)).is_err() {
                        return;
                    }
                    cx.background_executor().timer(INTERVAL).await;
                }
            })
            .detach();
        }
        this
    }

    /// Reads the games now; the outcome is kept for the Xbox page.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.run(true, cx);
    }

    /// A quiet read (the timer, the Collection's refresh button).
    pub fn sync_quietly(&mut self, cx: &mut Context<Self>) {
        self.run(false, cx);
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
                let outcome = match res {
                    Ok((report, found)) => {
                        this.measure(found.games.clone(), cx);
                        this.found = Some(found);
                        cx.emit(XboxLibraryEvent::Synced(report));
                        Ok(report)
                    }
                    Err(e) => {
                        log::warn!("Xbox games: {e}");
                        Err(e)
                    }
                };
                if asked {
                    this.last = Some(outcome);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Measures the games with no size yet, or updated since, and has the
    /// Collection reload when one got a size.
    fn measure(&mut self, games: Vec<XboxGame>, cx: &mut Context<Self>) {
        if self.measuring || games.is_empty() {
            return;
        }
        self.measuring = true;
        let (paths, stop) = (self.paths.clone(), self.stop.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let format = |bytes: u64| heronite_epic::util::file_size(bytes as f64);
                    heronite_xbox::size::measure(&paths, &games, &format, &|| stop.load(Ordering::Relaxed))
                })
                .await;
            this.update(cx, |this, cx| {
                this.measuring = false;
                match res {
                    Ok(0) => {}
                    Ok(n) => cx.emit(XboxLibraryEvent::Synced(Report { added: 0, updated: n })),
                    Err(e) => log::warn!("Xbox games: measuring their size: {e:#}"),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The user took one of the Xbox app's games out of the Collection.
    pub fn note_removed(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let (paths, app_name) = (self.paths.clone(), app_name.to_string());
        cx.background_spawn(async move {
            if let Err(e) = collection::mark_removed(&paths, &app_name) {
                log::warn!("Xbox games: could not remember that {app_name} was removed: {e:#}");
            }
        })
        .detach();
    }
}

impl Drop for XboxLibrary {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Opens the Xbox app, or its page in the Microsoft Store when it is not
/// installed.
pub fn open_xbox_app(installed: bool) -> std::io::Result<()> {
    if installed {
        heronite_platform::shell::open(format!(r"shell:AppsFolder\{}", heronite_xbox::XBOX_APP))
    } else {
        heronite_platform::shell::open(format!("ms-windows-store://pdp/?productid={}", heronite_xbox::XBOX_APP_STORE_ID))
    }
}
