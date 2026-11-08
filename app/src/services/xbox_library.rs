//! The Xbox app's games (`heronite_xbox`): the Microsoft Store and PC Game
//! Pass games installed on this PC as Windows packages, kept in the
//! Collection. The service reads them a few seconds after the start, then
//! every few minutes, and when asked (the Xbox page, the refresh button).
//! A read walks the registry and a few small files; nothing is started.
//! After a read, the games with no size yet (or updated since) are measured
//! in the background (`heronite_xbox::size`), one at a time.
//! The Collection reloads on [`XboxLibraryEvent::Synced`].
//!
//! With an Xbox account signed in (optional, Windows; Heronite signs in as
//! its own app registration, `heronite_xbox::live::client_id`), the
//! account is read after a package read, at most once an hour (or when
//! asked): play time, last play, achievements, art, and the PC games played
//! on it that are not installed here.
//!
//! `HERONITE_DISABLE_IDLE_JOBS` turns the timer off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use heronite_data::xbox::{LastSync, XboxPrefs};
use heronite_data::DataPaths;
use heronite_i18n::{tr, trn};
use heronite_xbox::collection::{self, Report};
use heronite_xbox::live::{self, account, auth::SignIn, http::Net, SignedOut};
use heronite_xbox::XboxGame;

const FIRST_DELAY: Duration = Duration::from_secs(8);
const INTERVAL: Duration = Duration::from_secs(10 * 60);
/// The account is read at most this often by itself.
const ACCOUNT_EVERY: chrono::TimeDelta = chrono::TimeDelta::hours(1);

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
    pub account: AccountState,
}

/// The Xbox account, as the page shows it.
#[derive(Default)]
pub struct AccountState {
    /// Heronite's app registration; `None`: no account is offered.
    pub client_id: Option<String>,
    /// The signed-in account's gamertag (`Some("")` when it has none).
    pub gamertag: Option<String>,
    /// A sign-in under way: its page and secret.
    pub sign_in: Option<SignIn>,
    pub signing_in: bool,
    pub syncing: bool,
    /// The last account read or sign-in, when the user is to see it.
    pub last: Option<Result<String, String>>,
    /// `HERONITE_DEBUG_XBOX_ACCOUNT`: nothing is read or signed in.
    debug: bool,
}

impl AccountState {
    pub fn available(&self) -> bool {
        self.client_id.is_some()
    }
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
        // Windows only: the sign-in is sealed with DPAPI.
        let client_id = live::client_id().filter(|_| cfg!(windows));
        let gamertag = client_id.as_ref().and_then(|_| account::load(&paths)).map(|a| a.gamertag().to_string());
        let mut account = AccountState { client_id, gamertag, ..Default::default() };
        // `HERONITE_DEBUG_XBOX_ACCOUNT=signed|out` shows the account card in
        // that state on any system, without Microsoft (for screenshots).
        if let Some(debug) = crate::env::value("HERONITE_DEBUG_XBOX_ACCOUNT") {
            account.client_id = Some("debug".into());
            account.gamertag = (debug == "signed").then(|| "Heronite Player".into());
            account.debug = true;
        }
        let this =
            Self { paths, syncing: false, measuring: false, stop: Arc::new(AtomicBool::new(false)), found: None, last: None, account };
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
                        this.sync_account(asked, cx);
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

    /// Starts a sign-in: its page, to open in the login window or a browser.
    pub fn begin_sign_in(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let client = self.account.client_id.clone().filter(|_| !self.account.debug)?;
        match SignIn::new(&client) {
            Ok(s) => {
                let url = s.url.clone();
                self.account.sign_in = Some(s);
                self.account.last = None;
                cx.notify();
                Some(url)
            }
            Err(e) => {
                self.account.last = Some(Err(format!("{e:#}")));
                cx.notify();
                None
            }
        }
    }

    /// Ends the sign-in with the address it ended on (the login window
    /// prints it; the user may paste it), then reads the account.
    pub fn finish_sign_in(&mut self, address: String, cx: &mut Context<Self>) {
        let (Some(client), Some(sign_in)) = (self.account.client_id.clone(), self.account.sign_in.clone()) else {
            self.account.last = Some(Err(tr!("Open the sign-in page first").to_string()));
            cx.notify();
            return;
        };
        if self.account.signing_in {
            return;
        }
        self.account.signing_in = true;
        self.account.last = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let code = sign_in.code_from(&address)?;
                    account::finish(&paths, &Net::default(), &client, &sign_in, &code)
                })
                .await;
            this.update(cx, |this, cx| {
                this.account.signing_in = false;
                match res {
                    Ok(a) => {
                        log::info!("Signed in to Xbox");
                        this.account.gamertag = Some(a.gamertag().to_string());
                        this.account.sign_in = None;
                        this.sync_account(true, cx);
                    }
                    Err(e) => this.account.last = Some(Err(format!("{e:#}"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Reads the account (`asked`: now, else at most once an hour).
    pub fn sync_account(&mut self, asked: bool, cx: &mut Context<Self>) {
        let Some(client) = self.account.client_id.clone() else { return };
        if self.account.gamertag.is_none() || self.account.syncing || self.account.debug {
            return;
        }
        let paths = self.paths.clone();
        let due = XboxPrefs::load(&paths)
            .account_synced
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .is_none_or(|t| chrono::Utc::now() - t.with_timezone(&chrono::Utc) >= ACCOUNT_EVERY);
        if !asked && !due {
            return;
        }
        let installed: Vec<String> = self.found.as_ref().map(|f| f.games.iter().map(|g| g.family.clone()).collect()).unwrap_or_default();
        self.account.syncing = true;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let synced = live::sync(&paths, &Net::default(), &client, &installed)?;
                    heronite_data::json_store::update_if_changed::<XboxPrefs, _>(&paths.xbox(), |p| {
                        p.account_synced = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                        Ok(())
                    })?;
                    Ok::<_, anyhow::Error>(synced)
                })
                .await;
            this.update(cx, |this, cx| {
                this.account.syncing = false;
                let outcome = match res {
                    Ok(s) => {
                        let a = s.applied;
                        if a.any() {
                            cx.emit(XboxLibraryEvent::Synced(Report { added: a.added, updated: a.updated + a.promoted }));
                        }
                        Ok(trn!(s.titles, "Read {n} title from the account.", "Read {n} titles from the account."))
                    }
                    Err(e) if e.is::<SignedOut>() => {
                        this.account.gamertag = None;
                        Err(tr!("Xbox signed you out. Sign in again to keep the play time up to date.").to_string())
                    }
                    Err(e) => {
                        log::warn!("Xbox account: {e:#}");
                        Err(format!("{e:#}"))
                    }
                };
                if asked || outcome.is_err() {
                    this.account.last = Some(outcome);
                } else if matches!(this.account.last, Some(Err(_))) {
                    // A quiet read that worked: an earlier failure is over.
                    this.account.last = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Forgets the account. What it brought stays in the Collection.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        // A read under way may renew the tokens and save them again.
        if self.account.syncing || self.account.signing_in {
            return;
        }
        match account::sign_out(&self.paths) {
            Ok(()) => {
                self.account.gamertag = None;
                // Microsoft still lets Heronite in until the user says no there.
                self.account.last = Some(Ok(tr!(
                    "Signed out. To also take away Heronite's access to your account, go to account.live.com/consent/Manage."
                )
                .to_string()));
            }
            Err(e) => self.account.last = Some(Err(format!("{e:#}"))),
        }
        cx.notify();
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

/// Opens a game's page in the Microsoft Store (where an Xbox game not
/// installed here is installed): by its product id, else the Xbox app.
pub fn open_store_page(product_id: Option<&str>) -> std::io::Result<()> {
    let valid = product_id.filter(|p| !p.is_empty() && p.len() <= 16 && p.chars().all(|c| c.is_ascii_alphanumeric()));
    match valid {
        Some(id) => heronite_platform::shell::open(format!("ms-windows-store://pdp/?productid={id}")),
        None => open_xbox_app(true),
    }
}
