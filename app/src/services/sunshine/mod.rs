//! Settings → Integrations → Sunshine: keeps the picked games in Sunshine's
//! application list (`heronite_sunshine`), so Moonlight can start them.
//!
//! A sync runs when the Collection loads a library whose picked games
//! differ from the last synced ones ([`library_loaded`], a few seconds after
//! the change settles), when an option changes and on "Sync now". Turning
//! the integration off removes Heronite's entries. A failure is shown on the
//! Settings card and logged once until it changes: no toasts, no repeats.
//! While Sunshine does not answer, a quiet retry runs every few minutes (it
//! often starts after Heronite at login).
//!
//! A Sunshine on another computer has its certificate pinned on the first
//! sync that goes through; a different one later stops syncing until the
//! user accepts it ([`Sunshine::accept_certificate`]).

mod secret;
pub mod wanted;

use std::time::{Duration, SystemTime};

use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths, Library};
use heronite_i18n::tr;
use heronite_sunshine::prefs::Prefs;
use heronite_sunshine::{install, Client, Error, Report};

/// Wait after a library change before syncing (installs come in bursts).
const SETTLE: Duration = Duration::from_secs(3);
/// Retry while Sunshine does not answer.
const RETRY: Duration = Duration::from_secs(5 * 60);

/// Why the last sync did not go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoPassword,
    /// The password store failed (keyring locked or missing, DPAPI).
    Secret(String),
    BadAddress,
    NotInstalled,
    NotRunning,
    Unauthorized,
    NotSetUp,
    Forbidden,
    /// Not the certificate pinned on first use; the new fingerprint.
    CertificateChanged(String),
    Other(String),
}

impl Problem {
    fn from_api(e: Error) -> Self {
        match e {
            Error::BadAddress => Problem::BadAddress,
            Error::Unreachable(_) if install::installed() => Problem::NotRunning,
            Error::Unreachable(_) => Problem::NotInstalled,
            Error::Unauthorized => Problem::Unauthorized,
            Error::NotSetUp => Problem::NotSetUp,
            Error::Forbidden => Problem::Forbidden,
            Error::CertificateChanged { seen } => Problem::CertificateChanged(seen),
            Error::Api(e) => Problem::Other(e),
        }
    }

    /// For the Settings card.
    pub fn text(&self) -> String {
        match self {
            Problem::NoPassword => tr!("Type Sunshine's user name and password, then Connect.").into(),
            Problem::Secret(e) => tr!("The password could not be kept safely: {error}", error = e.as_str()),
            Problem::BadAddress => tr!("That is not a Sunshine address. It looks like https://localhost:47990.").into(),
            Problem::NotInstalled => tr!("Sunshine is not installed on this computer, or it is not at this address.").into(),
            Problem::NotRunning => tr!("Sunshine is not running, or it is not at this address. Start it and try again.").into(),
            Problem::Unauthorized => tr!("Sunshine refused the user name or password.").into(),
            Problem::NotSetUp => tr!("Sunshine has no account yet: open its web page once to create one.").into(),
            Problem::Forbidden => tr!("Sunshine does not accept this computer at this address.").into(),
            Problem::CertificateChanged(_) => tr!(
                "Sunshine's certificate is not the one it had before. If Sunshine was reinstalled or moved, accept the new one; if not, check the address."
            )
            .into(),
            Problem::Other(e) => tr!("Sunshine answered with an error: {error}", error = e.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing happened yet (or the integration is off).
    Idle,
    Syncing,
    /// `entries`: Heronite's applications in Sunshine now.
    Synced {
        report: Report,
        entries: usize,
        at: SystemTime,
    },
    Failed(Problem),
}

/// What the background job does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Job {
    Sync,
    /// Remove every entry of Heronite's (turned off).
    Clear,
}

pub struct Sunshine {
    data_paths: DataPaths,
    app_paths: AppPaths,
    prefs: Prefs,
    /// The password once read or typed this session.
    password: Option<String>,
    status: Status,
    /// [`wanted::fingerprint`] of the last library a sync was started for.
    synced: Option<u64>,
    running: bool,
    /// Asked again while a job ran.
    again: Option<Job>,
    /// The settle delay or the retry timer.
    pending: Option<Task<()>>,
    /// The last problem written to the log.
    logged: Option<Problem>,
}

struct SunshineGlobal(Entity<Sunshine>);
impl Global for SunshineGlobal {}

/// The service (created on first use).
pub fn get(cx: &mut App) -> Entity<Sunshine> {
    if let Some(g) = cx.try_global::<SunshineGlobal>() {
        return g.0.clone();
    }
    let data_paths = DataPaths::detect();
    let app_paths = AppPaths::for_data(&data_paths);
    let prefs = Prefs::load(&app_paths);
    let entity = cx.new(|_| Sunshine {
        data_paths,
        app_paths,
        prefs,
        password: None,
        status: Status::Idle,
        synced: None,
        running: false,
        again: None,
        pending: None,
        logged: None,
    });
    cx.set_global(SunshineGlobal(entity.clone()));
    entity
}

/// The Collection loaded `lib`: sync when the picked games changed.
pub fn library_loaded(lib: &Library, cx: &mut App) {
    get(cx).update(cx, |s, cx| s.on_library(lib, cx));
}

impl Sunshine {
    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// A password is kept (on Windows; elsewhere the keyring is only read
    /// when a sync needs it, so this says whether one was typed or read).
    pub fn has_password(&self) -> bool {
        self.password.is_some() || !self.prefs.sealed_password.is_empty()
    }

    fn on_library(&mut self, lib: &Library, cx: &mut Context<Self>) {
        if !self.prefs.enabled {
            return;
        }
        let fp = wanted::fingerprint(lib, &self.prefs);
        if self.synced == Some(fp) {
            return;
        }
        self.synced = Some(fp);
        self.later(SETTLE, cx);
    }

    /// Runs a sync after `delay` (replacing a pending one).
    fn later(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |s, cx| s.start(Job::Sync, cx)).ok();
        }));
    }

    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        if self.prefs.enabled {
            self.start(Job::Sync, cx);
        }
    }

    /// Changes options that shape the entries and syncs again. Turning
    /// the integration off removes Heronite's entries.
    pub fn set_prefs(&mut self, change: impl FnOnce(&mut Prefs), cx: &mut Context<Self>) {
        let was_on = self.prefs.enabled;
        change(&mut self.prefs);
        self.save(cx);
        self.synced = None;
        match (was_on, self.prefs.enabled) {
            (_, true) => self.start(Job::Sync, cx),
            (true, false) => {
                self.pending = None;
                self.start(Job::Clear, cx);
            }
            (false, false) => {}
        }
        cx.notify();
    }

    /// Keeps the address, user name and password, turns the integration on
    /// and syncs.
    pub fn connect(&mut self, address: String, username: String, password: String, cx: &mut Context<Self>) {
        let address = heronite_sunshine::api::normalize_address(&address).unwrap_or(address);
        if address != self.prefs.address {
            // Another Sunshine: its certificate is a first use again.
            self.prefs.certificate.clear();
        }
        self.prefs.address = address;
        self.prefs.username = username;
        self.prefs.enabled = true;
        self.synced = None;
        if password.is_empty() {
            self.save(cx);
            self.start(Job::Sync, cx);
            return;
        }
        let stored = secret::store(&mut self.prefs, &password, cx);
        self.save(cx);
        self.password = Some(password);
        self.status = Status::Syncing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = stored.await;
            this.update(cx, |s, cx| match result {
                Ok(()) => {
                    s.save(cx);
                    s.start(Job::Sync, cx);
                }
                Err(e) => {
                    // Not kept: it is not used either, so nothing works half-saved.
                    s.password = None;
                    s.fail(Problem::Secret(e), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Trusts the certificate Sunshine shows now, in place of the pinned one,
    /// and syncs.
    pub fn accept_certificate(&mut self, cx: &mut Context<Self>) {
        let Status::Failed(Problem::CertificateChanged(seen)) = &self.status else { return };
        log::info!("Sunshine: the new certificate {seen} was accepted");
        self.prefs.certificate = seen.clone();
        self.logged = None;
        self.synced = None;
        self.save(cx);
        self.start(Job::Sync, cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.prefs.save(&self.app_paths) {
            log::error!("saving the Sunshine settings: {e:#}");
        }
        cx.notify();
    }

    fn fail(&mut self, problem: Problem, cx: &mut Context<Self>) {
        if self.logged.as_ref() != Some(&problem) {
            log::warn!("Sunshine: {}", problem.text());
            self.logged = Some(problem.clone());
        }
        if matches!(problem, Problem::NotRunning) && self.prefs.enabled {
            self.later(RETRY, cx);
        }
        self.status = Status::Failed(problem);
        cx.notify();
    }

    fn start(&mut self, job: Job, cx: &mut Context<Self>) {
        if self.running {
            // A clear asked for after a sync wins, and the other way round.
            self.again = Some(job);
            return;
        }
        self.pending = None;
        self.running = true;
        self.status = Status::Syncing;
        cx.notify();
        let password = match self.password.clone() {
            Some(p) => Task::ready(Ok(Some(p))),
            None => secret::load(&self.prefs, cx),
        };
        let (data_paths, prefs) = (self.data_paths.clone(), self.prefs.clone());
        cx.spawn(async move |this, cx| {
            let result = match password.await {
                Ok(Some(password)) => {
                    let pw = password.clone();
                    let done = cx.background_spawn(async move { run(&data_paths, &prefs, &pw, job) }).await;
                    Ok((password, done))
                }
                Ok(None) => Err(Problem::NoPassword),
                Err(e) => Err(Problem::Secret(e)),
            };
            this.update(cx, |s, cx| {
                s.running = false;
                match result {
                    Ok((password, done)) => {
                        s.password = Some(password);
                        s.finish(job, done, cx);
                    }
                    Err(problem) => s.fail(problem, cx),
                }
                if let Some(next) = s.again.take() {
                    s.start(next, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish(&mut self, job: Job, done: Ran, cx: &mut Context<Self>) {
        let Ran { address, certificate, result } = done;
        match result {
            Ok((report, entries)) => {
                // Trust on first use: the certificate of the first sync that
                // went through (for the address it went to).
                if let Some(fp) = certificate.filter(|_| self.prefs.certificate.is_empty() && self.prefs.address == address) {
                    log::info!("Sunshine: trusting its certificate {fp}");
                    self.prefs.certificate = fp;
                    self.save(cx);
                }
                if report.changed() {
                    log::info!(
                        "Sunshine: {} added, {} updated, {} removed ({entries} entries)",
                        report.added,
                        report.updated,
                        report.removed
                    );
                }
                self.logged = None;
                self.status = if job == Job::Clear { Status::Idle } else { Status::Synced { report, entries, at: SystemTime::now() } };
                cx.notify();
            }
            Err(problem) => {
                // The next library load tries again.
                self.synced = None;
                self.fail(problem, cx);
            }
        }
    }
}

/// What a job did.
struct Ran {
    /// The address it used (the settings may have changed meanwhile).
    address: String,
    /// The certificate seen, when certificates are pinned for the address.
    certificate: Option<String>,
    /// What changed and how many entries Heronite has there now.
    result: Result<(Report, usize), Problem>,
}

/// The blocking part of a job: read the library, build the entries and
/// bring Sunshine in line.
fn run(data_paths: &DataPaths, prefs: &Prefs, password: &str, job: Job) -> Ran {
    let client = Client::new(&prefs.address, &prefs.username, password, &prefs.certificate);
    let (certificate, result) = match &client {
        Ok(client) => {
            let result = run_with(client, data_paths, prefs, job);
            (client.certificate(), result)
        }
        Err(e) => (None, Err(Problem::from_api(e.clone()))),
    };
    Ran { address: prefs.address.clone(), certificate, result }
}

fn run_with(client: &Client, data_paths: &DataPaths, prefs: &Prefs, job: Job) -> Result<(Report, usize), Problem> {
    if job == Job::Clear {
        let removed = heronite_sunshine::sync::remove_all(client).map_err(Problem::from_api)?;
        return Ok((Report { removed, ..Default::default() }, 0));
    }
    // Reach Sunshine before the covers are rendered: a wrong password or a
    // stopped Sunshine is told at once.
    client.apps().map_err(Problem::from_api)?;
    let lib = Library::load(data_paths).map_err(|e| Problem::Other(format!("{e:#}")))?;
    let entries = wanted::entries(data_paths, &lib, prefs);
    let report = heronite_sunshine::sync(client, &entries).map_err(Problem::from_api)?;
    wanted::remove_stale_covers(data_paths, &entries);
    Ok((report, entries.len()))
}
