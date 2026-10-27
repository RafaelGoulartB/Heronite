//! The store clients under Wine (`heronite_clients`): Battle.net, the EA
//! app, Ubisoft Connect and the Rockstar Games Launcher. Their status, setup,
//! opening and closing, and their installed games kept in the Collection.
//!
//! The service starts with the app (`system/hub/start.rs`) and costs nothing
//! while no client is set up: ten seconds after the start it looks at every
//! client (their files only), then every quarter of an hour it looks again
//! and brings each installed client's games into the Collection. Whether a
//! client runs is only asked while one of its pages is on screen, or while a
//! client this app opened (set up, or started a game through) is expected to
//! run, every few seconds; when such a client closes it is synced, because
//! games may have been installed or removed in it. The Collection reloads on
//! [`ClientsEvent`].
//!
//! `HERONITE_DISABLE_IDLE_JOBS` turns the timer off.

mod debug;
mod run;
mod setup;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_clients::collection::{self, Report};
use heronite_clients::setup::Step;
use heronite_clients::{Account, Client, Status};
use heronite_data::clients::{ClientPrefs, LastSync};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_i18n::tr;

pub use debug::debug_client;

use super::proton_setup::Setup as WineSetup;

const FIRST_DELAY: Duration = Duration::from_secs(10);
const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How often a client whose page is on screen is asked whether it runs.
const POLL: Duration = Duration::from_secs(3);
/// How often a client only expected to run (no page on screen) is asked:
/// it may stay open for hours beside its games.
const POLL_WATCHED: Duration = Duration::from_secs(15);
/// How long a client that was asked to start may take to show up.
const OPENING: Duration = Duration::from_secs(30);

pub enum ClientsEvent {
    /// A client's games were brought into the Collection.
    Synced(Client, Report),
    /// A client was forgotten: this many of its entries left the Collection.
    Removed(usize),
}

/// Where a setup is.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Waiting for the default Proton: there is none on this PC yet.
    Wine(WineSetup),
    Step(Step),
}

/// What a client is doing for the user right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Activity {
    #[default]
    Idle,
    SettingUp(Progress),
    /// Asked to start; waiting for it to show up.
    Opening,
    Closing,
    Adopting,
    Forgetting,
}

/// The line under a client's card.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

impl Notice {
    fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), error: true }
    }

    fn info(text: impl Into<String>) -> Self {
        Self { text: text.into(), error: false }
    }
}

/// Everything known about one client.
#[derive(Clone, Default)]
pub struct ClientState {
    /// `None` until the first look.
    pub status: Option<Status>,
    pub activity: Activity,
    /// Games are being brought into the Collection.
    pub syncing: bool,
    /// The client's program runs (only asked while it matters).
    pub running: bool,
    /// A program other than the client's runs in its prefix.
    pub game_running: bool,
    pub notice: Option<Notice>,
    /// The last sync of this run.
    pub report: Option<Report>,
    pub last: Option<LastSync>,
    /// Entries of this client in the Collection.
    pub in_collection: usize,
    /// App names of its games the user took out of the Collection.
    pub removed: BTreeSet<String>,
    cancel: Option<Arc<Cancel>>,
    visible: bool,
    /// A client this app opened, set up or started a game through: asked
    /// until it closes.
    watched: bool,
    opened_at: Option<Instant>,
    reading: bool,
    /// Another look was asked meanwhile (with a sync or not).
    again: Option<bool>,
}

impl ClientState {
    pub fn installed(&self) -> bool {
        self.status.as_ref().is_some_and(Status::installed)
    }

    pub fn prefix(&self) -> Option<&Path> {
        self.status.as_ref().and_then(|s| s.prefix.as_deref())
    }

    pub fn account(&self) -> Account {
        self.status.as_ref().map(|s| s.account.clone()).unwrap_or_default()
    }

    /// Something other than a sync is going on: the actions wait.
    pub fn busy(&self) -> bool {
        self.activity != Activity::Idle
    }

    /// A setup was asked to stop and has not yet.
    pub fn cancelling(&self) -> bool {
        matches!(self.activity, Activity::SettingUp(_)) && self.cancel.as_ref().is_some_and(|c| c.is_requested())
    }

    /// The client's own game, by the Collection's app name, is in the
    /// Collection (it was synced and the user did not take it out).
    pub fn in_collection_game(&self, app_name: &str) -> bool {
        self.last.is_some() && !self.removed.contains(app_name)
    }
}

pub struct Clients {
    paths: DataPaths,
    states: BTreeMap<Client, ClientState>,
    /// Setups waiting for the default Proton to download.
    waiting_wine: BTreeSet<Client>,
    wine_sub: Option<Subscription>,
    poller: Option<Task<()>>,
    /// `HERONITE_DEBUG_CLIENT`: this client shows invented facts and nothing
    /// is run for it.
    frozen: Option<Client>,
}

impl EventEmitter<ClientsEvent> for Clients {}

/// `$HOME`, the way the clients' functions want it.
fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// What one look found.
struct Read {
    status: Status,
    /// `None` when nothing was synced (not asked, or not installed).
    synced: Option<Result<Report, String>>,
    last: Option<LastSync>,
    in_collection: usize,
    removed: BTreeSet<String>,
}

/// Looks at `clients` and, when asked, brings the installed ones' games into
/// the Collection. Blocking: files only.
fn read(paths: &DataPaths, clients: &[Client], sync: bool) -> Vec<(Client, Read)> {
    let statuses: Vec<(Client, Status)> = clients.iter().map(|&c| (c, heronite_clients::status(paths, c, &home()))).collect();
    // Nothing is set up: no other file is read.
    let any = statuses.iter().any(|(_, s)| s.installed());
    let (prefs, counts) = if any { (ClientPrefs::load(paths), collection::counts(paths)) } else { Default::default() };
    statuses
        .into_iter()
        .map(|(client, status)| {
            let synced = (sync && status.installed()).then(|| collection::sync(paths, client, &status.games).map_err(|e| format!("{e:#}")));
            // Read after the sync, which records when it ran.
            let prefs = if matches!(synced, Some(Ok(_))) { ClientPrefs::load(paths) } else { prefs.clone() };
            let read = Read {
                last: prefs.last.get(&client).cloned(),
                removed: prefs.removed.get(&client).cloned().unwrap_or_default(),
                in_collection: counts.get(&client).copied().unwrap_or(0),
                synced,
                status,
            };
            (client, read)
        })
        .collect()
}

impl Clients {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            paths,
            states: Client::ALL.into_iter().map(|c| (c, ClientState::default())).collect(),
            waiting_wine: BTreeSet::new(),
            wine_sub: None,
            poller: None,
            frozen: None,
        };
        if let Some((client, name)) = debug_client() {
            this.states.insert(client, debug::fake(client, &name));
            this.frozen = Some(client);
        }
        if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FIRST_DELAY).await;
                loop {
                    if this.update(cx, |this, cx| this.refresh(&Client::ALL, true, cx)).is_err() {
                        return;
                    }
                    cx.background_executor().timer(INTERVAL).await;
                }
            })
            .detach();
        }
        this
    }

    /// What was last seen of `client`; `None` until the first look.
    pub fn status(&self, client: Client) -> Option<&Status> {
        self.states[&client].status.as_ref()
    }

    pub fn state(&self, client: Client) -> &ClientState {
        &self.states[&client]
    }

    fn state_mut(&mut self, client: Client) -> &mut ClientState {
        self.states.get_mut(&client).expect("a state for every client")
    }

    /// How many clients are signed in.
    pub fn signed_in(&self) -> usize {
        Client::ALL
            .into_iter()
            .filter(|c| self.status(*c).is_some_and(|s| s.installed() && matches!(s.account, Account::SignedIn(_))))
            .count()
    }

    /// Looks at every client again (files only).
    pub fn look(&mut self, cx: &mut Context<Self>) {
        self.refresh(&Client::ALL, false, cx);
    }

    /// Brings the client's installed games into the Collection now.
    pub fn sync(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) {
            return;
        }
        self.state_mut(client).notice = None;
        self.refresh(&[client], true, cx);
    }

    /// A client page came on screen or went away: the client is asked
    /// whether it runs while one is on screen.
    pub fn set_visible(&mut self, client: Client, on: bool, cx: &mut Context<Self>) {
        let st = self.state_mut(client);
        if std::mem::replace(&mut st.visible, on) == on {
            return;
        }
        if on {
            self.refresh(&[client], false, cx);
            // A fresh loop asks at once.
            self.poller = None;
            self.ensure_polling(cx);
        }
    }

    /// A game of `client` was started: the client opens (or is open) and is
    /// followed until it closes.
    pub fn game_started(&mut self, client: Client, cx: &mut Context<Self>) {
        let st = self.state_mut(client);
        st.watched = true;
        st.opened_at = Some(Instant::now());
        self.ensure_polling(cx);
    }

    /// A game of `client` ended: its play time is the launcher's, but what
    /// is installed may have changed.
    pub fn game_ended(&mut self, client: Client, cx: &mut Context<Self>) {
        self.refresh(&[client], true, cx);
    }

    /// The user took `app_name` (a game of `client`) out of the Collection.
    pub fn note_removed(&mut self, client: Client, app_name: &str, cx: &mut Context<Self>) {
        self.state_mut(client).removed.insert(app_name.to_string());
        let (paths, app_name) = (self.paths.clone(), app_name.to_string());
        cx.background_spawn(async move {
            if let Err(e) = collection::mark_removed(&paths, client, &app_name) {
                log::warn!("{}: leaving {app_name} out of the Collection: {e:#}", client.name());
            }
        })
        .detach();
        cx.notify();
    }

    /// Looks at `clients` (files only) and, with `sync`, brings the games of
    /// the installed ones into the Collection. A look asked while one runs
    /// is done once it ends.
    fn refresh(&mut self, clients: &[Client], sync: bool, cx: &mut Context<Self>) {
        let mut todo = Vec::new();
        for &client in clients {
            if self.frozen == Some(client) {
                continue;
            }
            let st = self.state_mut(client);
            if st.reading {
                st.again = Some(st.again.unwrap_or(false) || sync);
                continue;
            }
            st.reading = true;
            st.syncing = sync && st.installed();
            todo.push(client);
        }
        if todo.is_empty() {
            return;
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let reads = cx.background_spawn(async move { read(&paths, &todo, sync) }).await;
            this.update(cx, |this, cx| this.refreshed(reads, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn refreshed(&mut self, reads: Vec<(Client, Read)>, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        let mut again = Vec::new();
        for (client, read) in reads {
            let st = self.state_mut(client);
            st.reading = false;
            st.syncing = false;
            st.in_collection = read.in_collection;
            st.removed = read.removed;
            st.last = read.last;
            if !read.status.installed() {
                st.running = false;
                st.game_running = false;
                st.watched = false;
            }
            match read.synced {
                Some(Ok(report)) => {
                    st.report = Some(report);
                    // The backend records when it changed something; a sync
                    // that found nothing new still counts.
                    let games = read.status.games.len();
                    st.last = Some(LastSync { at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), games });
                    events.push(ClientsEvent::Synced(client, report));
                }
                Some(Err(e)) => {
                    log::warn!("{}: bringing the games in: {e}", client.name());
                    st.notice = Some(Notice::error(tr!(
                        "Could not bring the games of {client} into the Collection: {error}",
                        client = client.name(),
                        error = e
                    )));
                }
                None => {}
            }
            st.status = Some(read.status);
            if let Some(sync) = st.again.take() {
                again.push((client, sync));
            }
        }
        for (client, sync) in again {
            self.refresh(&[client], sync, cx);
        }
        for ev in events {
            cx.emit(ev);
        }
        self.ensure_polling(cx);
        cx.notify();
    }

    /// Starts asking which clients run, when one is on screen or expected to
    /// run; the loop ends by itself when none is.
    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self.poller.is_some() || self.poll_targets().is_empty() {
            return;
        }
        self.poller = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(targets) = this.update(cx, |this, _| this.poll_targets()) else { return };
                if targets.is_empty() {
                    break;
                }
                let seen = cx
                    .background_spawn(async move {
                        targets
                            .into_iter()
                            .map(|(c, prefix)| {
                                (c, heronite_clients::run::is_running(c, &prefix), heronite_clients::run::game_running(c, &prefix))
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let Ok(visible) = this.update(cx, |this, cx| {
                    this.polled(seen, cx);
                    this.states.values().any(|s| s.visible)
                }) else {
                    return;
                };
                cx.background_executor().timer(if visible { POLL } else { POLL_WATCHED }).await;
            }
            this.update(cx, |this, _| this.poller = None).ok();
        }));
    }

    /// The installed clients whose state matters now, with their prefixes.
    fn poll_targets(&self) -> Vec<(Client, PathBuf)> {
        self.states
            .iter()
            .filter(|(c, s)| (s.visible || s.watched) && self.frozen != Some(**c))
            .filter_map(|(c, s)| s.installed().then(|| s.prefix().map(|p| (*c, p.to_path_buf()))).flatten())
            .collect()
    }

    fn polled(&mut self, seen: Vec<(Client, bool, bool)>, cx: &mut Context<Self>) {
        let mut closed = Vec::new();
        let mut changed = false;
        for (client, running, game) in seen {
            let st = self.state_mut(client);
            let was = std::mem::replace(&mut st.running, running);
            // Only a change repaints: the Collection watches this service too.
            changed |= was != running || st.game_running != game;
            st.game_running = game;
            let activity = st.activity.clone();
            if running {
                st.opened_at = None;
                if st.activity == Activity::Opening {
                    st.activity = Activity::Idle;
                }
            } else if was {
                st.watched = false;
                closed.push(client);
            } else if st.watched && st.opened_at.is_none_or(|t| t.elapsed() > OPENING) {
                // It never showed up.
                st.watched = false;
                st.opened_at = None;
                if st.activity == Activity::Opening {
                    st.activity = Activity::Idle;
                }
            }
            changed |= st.activity != activity;
        }
        // It may have been used to install or remove games.
        if !closed.is_empty() {
            self.refresh(&closed, true, cx);
        }
        if changed {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use heronite_clients::setup::Step;
    use heronite_clients::Account;
    use heronite_data::clients::LastSync;
    use heronite_data::proc::Cancel;

    use super::{Activity, ClientState, Progress};

    #[test]
    fn a_game_is_in_the_collection_once_synced_and_not_taken_out() {
        let mut st = ClientState::default();
        assert!(!st.in_collection_game("ea_a"));
        st.last = Some(LastSync { at: "2026-10-26T10:00:00Z".into(), games: 1 });
        st.removed.insert("ea_b".into());
        assert!(st.in_collection_game("ea_a"));
        assert!(!st.in_collection_game("ea_b"));
    }

    #[test]
    fn only_a_setup_that_was_asked_to_stop_is_cancelling() {
        let mut st = ClientState::default();
        assert!(!st.cancelling());
        let cancel = Cancel::new();
        st.cancel = Some(cancel.clone());
        st.activity = Activity::SettingUp(Progress::Step(Step::Installing));
        assert!(!st.cancelling());
        cancel.request();
        assert!(st.cancelling());
        st.activity = Activity::Idle;
        assert!(!st.cancelling());
    }

    #[test]
    fn nothing_is_installed_before_the_first_look() {
        let st = ClientState::default();
        assert!(!st.installed() && !st.busy());
        assert_eq!(st.account(), Account::Unknown);
        assert_eq!(st.prefix(), None);
    }
}
