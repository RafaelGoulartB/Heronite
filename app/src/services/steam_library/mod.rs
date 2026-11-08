//! The Steam library service: reads the account's library from the running
//! Steam client and joins it to the Collection (`heronite_steam::library`).
//! It runs a few seconds after start, then every few seconds until the
//! library was read once (so a Steam opened later is read at once), then
//! every few minutes, and when asked (the refresh button, the Steam page).
//! It also opens and restarts Steam for the welcome and the Collection's
//! Steam pill ([`SteamLibrary::notice`]). The Collection reloads on
//! [`SteamLibraryEvent::Imported`].
//!
//! With a Steam account signed in ([`account`], optional), the library is
//! read through Steam's Web API whenever the client cannot be read, at most
//! every [`ACCOUNT_EVERY`] by itself.

mod account;

use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::steam_owned::{SteamOwnedFile, SteamSource};
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::{find_steam_binary, installed_app_ids, SteamLocator};
use heronite_launch::SystemProbe;
use heronite_steam::account::http::Net;
use heronite_steam::account::SignedOut;
use heronite_steam::library::ImportReport;
use heronite_steam::{ClientApiFlag, LibraryAccess, LibraryCheck, PortOwner, Steam};

pub use account::{AccountState, QrState};

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

/// How long Steam may run with its client not answering before the page
/// stops saying it is starting and offers a restart (its updater and the
/// sign-in take a while; past this, something keeps the client closed).
const NOT_ANSWERING: Duration = Duration::from_secs(120);

/// The library is read from the account at most this often by itself.
const ACCOUNT_EVERY: Duration = INTERVAL;

/// The id under which "Ignore" on the Collection's Steam pill is remembered
/// (`heronite_data::ui_state::dismiss_note`).
const NOTICE_NOTE: &str = "steam-restart";

/// The `steam` command: `HERONITE_STEAM_BIN` (tests), else the one found.
pub fn steam_bin() -> String {
    crate::env::value("HERONITE_STEAM_BIN").or_else(|| find_steam_binary(&SystemProbe)).unwrap_or_else(|| "steam".into())
}

/// The Steam install, read fresh (the Steam folder setting may change).
pub fn steam(paths: &DataPaths) -> Steam {
    let locator = SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe);
    Steam::new(locator, crate::system::shortcuts::steam_client())
}

/// Whether the client can be read, why not, and what holds the place of
/// the flag file (after the check, which creates it when it can). Blocking.
fn library_check(paths: &DataPaths) -> (LibraryCheck, ClientApiFlag) {
    let steam = steam(paths);
    (steam.library_check(), steam.access_flag())
}

/// How a read failed.
#[derive(Debug)]
enum ReadError {
    /// The client's or the account's message.
    Failed(String),
    /// Steam no longer takes the account's sign-in (deleted already).
    SignedOut,
}

/// Reads the library and joins it: from the client when it can be read,
/// else from the account when `account` (one is signed in). Blocking.
fn sync_blocking(paths: &DataPaths, account: bool) -> Result<(ImportReport, SteamSource), ReadError> {
    let steam = steam(paths);
    let bin = find_steam_binary(&SystemProbe);
    let client_error = match steam.read_library() {
        Ok(owned) => {
            let report = heronite_steam::library::import(paths, owned, bin.as_deref()).map_err(|e| ReadError::Failed(format!("{e:#}")))?;
            return Ok((report, SteamSource::Client));
        }
        Err(e) => e,
    };
    if !account {
        return Err(ReadError::Failed(client_error));
    }
    let installed = installed_app_ids(&steam.locator);
    let owned = heronite_steam::account::read_owned(paths, &Net::default(), &installed).map_err(|e| {
        if e.is::<SignedOut>() {
            ReadError::SignedOut
        } else {
            ReadError::Failed(tr!("Reading your library from your Steam account failed: {error}", error = format!("{e:#}")))
        }
    })?;
    let report = heronite_steam::library::import_from(paths, owned, bin.as_deref(), SteamSource::Account)
        .map_err(|e| ReadError::Failed(format!("{e:#}")))?;
    Ok((report, SteamSource::Account))
}

/// What the cached library holds: games, installed ones, when it was read.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub games: usize,
    pub installed: usize,
    pub updated_at: Option<String>,
    pub source: SteamSource,
}

/// What the Collection's Steam pill shows ([`SteamLibrary::notice`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteamNotice {
    /// The library was never read and Steam runs without the access: a
    /// restart brings in the whole library.
    Restart,
    /// Steam was restarted for it and the library is not read yet.
    Restarting,
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
    /// Steam runs with the access on, but its client has not answered for
    /// [`NOT_ANSWERING`]: still waiting for the sign-in, or something keeps
    /// it closed. Checked again all the time.
    NotAnswering,
    /// Steam was open before Heronite could turn the access on.
    NeedsRestart,
    /// Steam runs without the access and another tool (Millennium) holds
    /// the place of its flag file: a restart does not open it.
    Blocked,
    /// Another program holds the port of Steam's access
    /// ([`SteamLibrary::taken`]): it has to close (or move) first.
    PortTaken,
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
    /// What holds the place of the flag file that opens the access, as of
    /// the last check (`None` until checked).
    pub flag: Option<ClientApiFlag>,
    pub summary: Summary,
    /// The cached library was looked at: until then nothing tells whether
    /// it was ever read.
    summary_loaded: bool,
    /// "Ignore" on the Collection's Steam pill, remembered for good.
    notice_ignored: bool,
    /// When Heronite last started Steam.
    opened_at: Option<Instant>,
    restarting: bool,
    /// Steam was restarted from here: if the access still does not come,
    /// asking again would not help.
    restarted: bool,
    /// The last open or restart failed.
    pub error: Option<String>,
    /// Since when Steam runs with its client not answering ([`LibraryAccess::Starting`]).
    starting_since: Option<Instant>,
    /// The last access written to the log, with why the client did not
    /// answer: logged again only when it changes.
    logged: Option<(LibraryAccess, Option<String>)>,
    /// The program holding the port of Steam's access, as of the last check.
    pub taken: Option<PortOwner>,
    /// The Steam account signed in with a QR code (optional).
    pub account: AccountState,
    /// A read was asked for while another ran: it runs next.
    pending: bool,
}

impl EventEmitter<SteamLibraryEvent> for SteamLibrary {}

impl SteamLibrary {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let paths_for_account = paths.clone();
        let mut this = Self {
            paths,
            syncing: false,
            last: None,
            access: None,
            flag: None,
            summary: Summary::default(),
            summary_loaded: false,
            notice_ignored: false,
            opened_at: None,
            restarting: false,
            restarted: false,
            error: None,
            starting_since: None,
            logged: None,
            taken: None,
            account: AccountState::load(&paths_for_account),
            pending: false,
        };
        this.account.debug();
        this.load_summary(cx);
        // Off for tests against real data, like the other idle jobs.
        if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                // Ticks fast so a Steam opened from here is followed at
                // once even after the library was read; reads only when
                // something is awaited or the interval passed.
                let mut last: Option<Instant> = None;
                loop {
                    let alive = this.update(cx, |this, cx| {
                        if this.awaiting() || last.is_none_or(|t| t.elapsed() >= INTERVAL) {
                            last = Some(Instant::now());
                            this.sync_if_ready(cx);
                        }
                    });
                    if alive.is_err() {
                        return;
                    }
                    cx.background_executor().timer(FAST).await;
                }
            })
            .detach();
        }
        this
    }

    /// Whether the checks should follow Steam closely: the library was
    /// never read, or Steam is opening, restarting or not read yet.
    fn awaiting(&self) -> bool {
        self.summary.updated_at.is_none()
            || matches!(
                self.state(),
                SteamState::Checking
                    | SteamState::Opening
                    | SteamState::Restarting
                    | SteamState::NotAnswering
                    | SteamState::NeedsRestart
                    | SteamState::Blocked
                    | SteamState::PortTaken
            )
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
            Some(LibraryAccess::Starting) if self.starting_since.is_some_and(|t| t.elapsed() >= NOT_ANSWERING) => SteamState::NotAnswering,
            Some(LibraryAccess::Starting) => SteamState::Opening,
            Some(LibraryAccess::Closed) if opening => SteamState::Opening,
            Some(LibraryAccess::Closed) => SteamState::Closed,
            // Right after the restart, what is left of the old Steam may still show.
            Some(LibraryAccess::NeedsRestart) if self.opened_at.is_some_and(|t| t.elapsed() < SETTLE) => SteamState::Opening,
            Some(LibraryAccess::NeedsRestart) if self.flag == Some(ClientApiFlag::Blocked) => SteamState::Blocked,
            Some(LibraryAccess::NeedsRestart) if self.restarted => SteamState::Failed,
            Some(LibraryAccess::NeedsRestart) => SteamState::NeedsRestart,
            Some(LibraryAccess::PortTaken) => SteamState::PortTaken,
            Some(LibraryAccess::NotInstalled) => SteamState::NotInstalled,
        }
    }

    /// Whether the client cannot be read for a reason the user has to deal
    /// with (not while Steam is closed, starting or loading): what the
    /// account's sign-in is offered for.
    pub fn client_stuck(&self) -> bool {
        matches!(
            self.state(),
            SteamState::NotAnswering | SteamState::NeedsRestart | SteamState::Blocked | SteamState::PortTaken | SteamState::Failed
        )
    }

    /// What the Collection's Steam pill shows. Only before the library was
    /// ever read (afterwards the Steam page tells what is wrong), only while
    /// a restart is what helps (not while Steam is closed, starting or
    /// loading, and not once a restart from here did not help), and never
    /// after "Ignore". `HERONITE_DEBUG_STEAM_NOTICE=restart|restarting`
    /// shows one (for screenshots).
    pub fn notice(&self) -> Option<SteamNotice> {
        if let Some(debug) = crate::env::value("HERONITE_DEBUG_STEAM_NOTICE") {
            return Some(if debug == "restarting" { SteamNotice::Restarting } else { SteamNotice::Restart });
        }
        if !self.summary_loaded || self.summary.updated_at.is_some() || self.notice_ignored {
            return None;
        }
        match self.state() {
            SteamState::NeedsRestart if heronite_steam::can_restart() => Some(SteamNotice::Restart),
            SteamState::Restarting => Some(SteamNotice::Restarting),
            SteamState::Opening | SteamState::Reading if self.restarted => Some(SteamNotice::Restarting),
            _ => None,
        }
    }

    /// "Ignore" on the Collection's Steam pill: it does not show again.
    pub fn ignore_notice(&mut self, cx: &mut Context<Self>) {
        self.notice_ignored = true;
        let app = AppPaths::for_data(&self.paths);
        cx.background_spawn(async move {
            if let Err(e) = heronite_data::ui_state::dismiss_note(&app, NOTICE_NOTE) {
                log::warn!("Remembering the ignored Steam notice: {e:#}");
            }
        })
        .detach();
        cx.notify();
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
                        this.set_access(LibraryAccess::Closed, None);
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
            let (summary, ignored) = cx
                .background_spawn(async move {
                    let f = SteamOwnedFile::read(&paths);
                    let summary = Summary {
                        games: f.games.len(),
                        installed: f.games.iter().filter(|g| g.installed).count(),
                        updated_at: f.updated_at,
                        source: f.source.unwrap_or_default(),
                    };
                    (summary, heronite_data::ui_state::note_dismissed(&AppPaths::for_data(&paths), NOTICE_NOTE))
                })
                .await;
            this.update(cx, |this, cx| {
                this.summary = summary;
                this.summary_loaded = true;
                this.notice_ignored |= ignored;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps what a check found: when the client stopped answering, and
    /// the log line when it changed (a check runs every few seconds).
    /// Whether it wrote a log line.
    fn set_access(&mut self, access: LibraryAccess, detail: Option<String>) -> bool {
        if access != LibraryAccess::Starting {
            self.starting_since = None;
        } else if self.starting_since.is_none() {
            self.starting_since = Some(Instant::now());
        }
        self.access = Some(access);
        let entry = (access, detail);
        if self.logged.as_ref() != Some(&entry) {
            match &entry.1 {
                Some(why) => log::info!("Steam library access: {access:?} ({why})"),
                None => log::info!("Steam library access: {access:?}"),
            }
            self.logged = Some(entry);
            return true;
        }
        false
    }

    /// Keeps what a check found. `HERONITE_DEBUG_STEAM_PORT=<port>` makes
    /// every check find that port held by another program (for screenshots).
    fn take_check(&mut self, mut check: LibraryCheck, flag: ClientApiFlag) {
        if let Some(port) = crate::env::parse::<u16>("HERONITE_DEBUG_STEAM_PORT") {
            let owner = PortOwner { port, pid: Some(4242), name: Some("qbittorrent.exe".into()) };
            check = LibraryCheck { access: LibraryAccess::PortTaken, why: Some("debug".into()), taken: Some(owner) };
        }
        self.flag = Some(flag);
        self.taken = check.taken;
        self.set_access(check.access, check.why);
    }

    /// Checks whether the client can be read now (for the Steam page).
    pub fn check(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let (check, flag) = cx.background_spawn(async move { library_check(&paths) }).await;
            this.update(cx, |this, cx| {
                this.take_check(check, flag);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A quiet read (the timer, the refresh button): when the client
    /// answers, else from the account when one is signed in and it is due.
    pub fn sync_if_ready(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let (check, flag) = cx.background_spawn(async move { library_check(&paths) }).await;
            this.update(cx, |this, cx| {
                let ready = check.access == LibraryAccess::Ready;
                this.take_check(check, flag);
                if ready || (this.account_allowed() && this.account.due(ACCOUNT_EVERY)) {
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

    /// Whether the library may be read from the account now: one is signed
    /// in, and the client's richer read (hidden and Family Sharing games)
    /// is not what the cache holds, or the client is stuck. While Steam is
    /// only closed, a library the client read stays as it is.
    fn account_allowed(&self) -> bool {
        // Not while signing out: a renewal would save the tokens again.
        self.account.signed_in()
            && !self.account.busy
            && self.summary_loaded
            && (self.client_stuck() || self.summary.updated_at.is_none() || self.summary.source == SteamSource::Account)
    }

    fn run(&mut self, asked: bool, cx: &mut Context<Self>) {
        if self.syncing {
            // A sign-in (or the user) asked while a quiet read ran: next.
            self.pending |= asked;
            return;
        }
        self.syncing = true;
        let paths = self.paths.clone();
        let account = self.account_allowed();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { sync_blocking(&paths, account) }).await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                let res = match res {
                    Ok((report, source)) => {
                        match source {
                            SteamSource::Client => {
                                this.set_access(LibraryAccess::Ready, None);
                            }
                            SteamSource::Account => this.account.read_now(),
                        }
                        cx.emit(SteamLibraryEvent::Imported(report));
                        Ok(report)
                    }
                    Err(ReadError::SignedOut) => {
                        log::warn!("Steam library: Steam signed the account out");
                        this.account.signed_out();
                        Err(tr!("Steam signed you out of your account in Heronite. Sign in again with a QR code to keep reading your library from it.").to_string())
                    }
                    Err(ReadError::Failed(e)) => {
                        log::warn!("Steam library: {e}");
                        if account {
                            this.account.read_now();
                        }
                        Err(e)
                    }
                };
                if asked || res.is_ok() {
                    this.last = Some(res);
                }
                this.load_summary(cx);
                if std::mem::take(&mut this.pending) {
                    this.run(true, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use heronite_data::DataPaths;
    use heronite_steam::{ClientApiFlag, LibraryAccess};

    use super::{SteamLibrary, SteamNotice, SteamState, Summary, NOT_ANSWERING};

    fn library() -> SteamLibrary {
        SteamLibrary {
            paths: DataPaths::at("/nonexistent"),
            syncing: false,
            last: None,
            access: None,
            flag: None,
            summary: Summary::default(),
            summary_loaded: true,
            notice_ignored: false,
            opened_at: None,
            restarting: false,
            restarted: false,
            error: None,
            starting_since: None,
            logged: None,
            taken: None,
            account: Default::default(),
            pending: false,
        }
    }

    #[test]
    fn the_pill_asks_for_a_restart_only_until_the_library_was_read() {
        let mut s = library();
        s.set_access(LibraryAccess::NeedsRestart, None);
        if heronite_steam::can_restart() {
            assert_eq!(s.notice(), Some(SteamNotice::Restart));
        }
        // Read once: the Steam page says what is wrong from then on.
        s.summary.updated_at = Some("2026-11-07T10:00:00Z".into());
        assert_eq!(s.notice(), None);
        // Before the cache was looked at, nothing tells it was never read.
        let mut s = library();
        s.summary_loaded = false;
        s.set_access(LibraryAccess::NeedsRestart, None);
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn the_pill_shows_only_while_a_restart_helps() {
        let mut s = library();
        for access in [LibraryAccess::Closed, LibraryAccess::Starting, LibraryAccess::NotInstalled, LibraryAccess::Ready] {
            s.set_access(access, None);
            assert_eq!(s.notice(), None, "{access:?}");
        }
        // Another tool keeps the access closed: a restart does not open it.
        s.set_access(LibraryAccess::NeedsRestart, None);
        s.flag = Some(ClientApiFlag::Blocked);
        assert_eq!(s.notice(), None);
        // Restarted from here and still without the access: not asked again.
        s.flag = Some(ClientApiFlag::On);
        s.restarted = true;
        assert_eq!(s.state(), SteamState::Failed);
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn the_pill_follows_the_restart_and_ignore_is_for_good() {
        let mut s = library();
        s.restarting = true;
        assert_eq!(s.notice(), Some(SteamNotice::Restarting));
        s.restarting = false;
        s.restarted = true;
        s.opened_at = Some(Instant::now());
        s.set_access(LibraryAccess::Closed, None);
        assert_eq!(s.state(), SteamState::Opening);
        assert_eq!(s.notice(), Some(SteamNotice::Restarting));
        s.notice_ignored = true;
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn a_client_that_never_answers_stops_showing_as_starting() {
        let mut s = library();
        s.set_access(LibraryAccess::Starting, Some("steamwebhelper not running".into()));
        assert_eq!(s.state(), SteamState::Opening);
        // Checked again later, still not answering: the clock is not reset.
        // (No earlier instant right after boot: nothing to check then.)
        let Some(since) = Instant::now().checked_sub(NOT_ANSWERING) else { return };
        s.starting_since = Some(since);
        s.set_access(LibraryAccess::Starting, Some("client: refused".into()));
        assert_eq!(s.starting_since, Some(since));
        assert_eq!(s.state(), SteamState::NotAnswering);
        // Answering, or closed, starts over.
        s.set_access(LibraryAccess::Ready, None);
        assert_eq!(s.state(), SteamState::Connected);
        assert_eq!(s.starting_since, None);
        s.set_access(LibraryAccess::Starting, None);
        assert_eq!(s.state(), SteamState::Opening);
    }

    #[test]
    fn the_account_does_not_replace_a_library_the_client_read() {
        use heronite_data::steam_owned::SteamSource;
        let mut s = library();
        s.account.name = Some("gaben".into());
        // Never read: the account brings it.
        assert!(s.account_allowed());
        // Read from the client, Steam only closed: kept as the client read it.
        s.summary.updated_at = Some("2026-11-07T10:00:00Z".into());
        s.set_access(LibraryAccess::Closed, None);
        assert!(!s.account_allowed());
        // The client is stuck: the account steps in.
        s.set_access(LibraryAccess::PortTaken, None);
        assert!(s.account_allowed());
        // Read from the account already: it keeps it current.
        s.set_access(LibraryAccess::Closed, None);
        s.summary.source = SteamSource::Account;
        assert!(s.account_allowed());
        // Signed out: never.
        s.account.name = None;
        assert!(!s.account_allowed());
    }

    #[test]
    fn the_log_says_each_change_once() {
        let mut s = library();
        assert!(s.set_access(LibraryAccess::Starting, Some("a".into())));
        assert!(!s.set_access(LibraryAccess::Starting, Some("a".into())));
        assert!(s.set_access(LibraryAccess::Starting, Some("b".into())));
        assert!(s.set_access(LibraryAccess::Closed, None));
        assert!(!s.set_access(LibraryAccess::Closed, None));
    }

    #[test]
    fn another_tool_s_stand_in_for_the_flag_is_not_a_restart_away() {
        let mut s = library();
        s.set_access(LibraryAccess::NeedsRestart, None);
        s.flag = Some(ClientApiFlag::On);
        assert_eq!(s.state(), SteamState::NeedsRestart);
        s.restarted = true;
        assert_eq!(s.state(), SteamState::Failed);
        // Millennium's link: restarting Steam would not open the access.
        s.flag = Some(ClientApiFlag::Blocked);
        assert_eq!(s.state(), SteamState::Blocked);
        s.restarted = false;
        assert_eq!(s.state(), SteamState::Blocked);
        // Followed closely, so the access is noticed once the tool lets it open.
        s.summary.updated_at = Some("2026-11-07T10:00:00Z".into());
        assert!(s.awaiting());
        // Steam closed or answering: the flag changes nothing.
        s.set_access(LibraryAccess::Ready, None);
        assert_eq!(s.state(), SteamState::Connected);
    }

    #[test]
    fn checks_follow_steam_closely_until_it_is_read() {
        let mut s = library();
        assert!(s.awaiting());
        s.summary.updated_at = Some("2026-11-07T10:00:00Z".into());
        s.set_access(LibraryAccess::Closed, None);
        assert!(!s.awaiting());
        // Opened from here after a first read: followed at once again.
        s.opened_at = Some(Instant::now());
        assert!(s.awaiting());
        s.set_access(LibraryAccess::Ready, None);
        assert!(!s.awaiting());
    }
}
