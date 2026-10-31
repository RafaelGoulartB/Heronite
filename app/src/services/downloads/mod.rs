//! Store service: owns the download queue (running one install/update at a
//! time on a worker thread, for Epic, GOG and Amazon Games), the accounts, the
//! "update available" sets and the one-off operations on games (verify,
//! uninstall, move, import, cloud saves). Pages, dialogs and the Collection
//! observe it.
//!
//! `Downloads` is split by responsibility: `queue` (install/update/queue
//! control, pausing while offline with `connectivity`), `ops` (one-off operations), `updates` (library refresh and
//! auto-update), `eos` (EOS overlay), `account` (Epic login/logout), `stores`
//! (GOG and Amazon Games: accounts, refreshes, operations) and `debug`
//! (`HERONITE_DEBUG_*` hooks).

mod account;
mod amazon_app;
mod connectivity;
mod debug;
mod eos;
mod gog_galaxy;
mod gog_saves;
mod launch_setup;
mod library_file;
mod ops;
mod queue;
mod relocate;
mod stores;
mod types;
mod updates;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_epic::manager::{DownloadManager, Event as DmEvent, Job, Snapshot};
use heronite_epic::progress::Progress;
use heronite_epic::queue::DmElement;
use heronite_epic::user::UserInfo;
use heronite_epic::{library, user};
use heronite_i18n::tr;

use eos::EosEntry;
use queue::LazyJob;

pub use eos::global as eos_global;
pub use launch_setup::EpicLaunchSetup;
pub use library_file::library_entry;
pub use ops::LAUNCHER_IMPORT;
pub use stores::{gog_sync_saves_flag, StoreLaunchSetup};
pub use types::*;

pub struct Downloads {
    data_paths: DataPaths,
    manager: DownloadManager,
    snapshot: Snapshot,
    progress: HashMap<String, Progress>,
    /// Download speeds (MiB/s) of the running element's last reports, oldest
    /// first, for the Downloads page's chart: `(app name, samples)`.
    speed: (String, VecDeque<f32>),
    /// Pauses the queue while the connection is down.
    offline: connectivity::OfflineWatch,
    updates: HashSet<String>,
    /// `HERONITE_DEBUG_FAKE_UPDATE=<appName>`: shows an update for screenshots.
    fake_update: Option<String>,
    busy: HashMap<String, Busy>,
    account: Account,
    stats: Stats,
    pub refreshing: bool,
    /// Queue the updates found by the refresh in progress (`autoUpdateGames`).
    auto_update_after_refresh: bool,
    /// The refresh in progress is a background check: no success toast.
    quiet_refresh: bool,
    /// Games being played (the hub tells us): they are updated after they exit.
    playing: HashSet<String>,
    /// Auto-updates waiting for their game to exit.
    deferred: HashSet<String>,
    /// Games whose automatic update failed: not retried until the next start
    /// (the periodic check would otherwise fail again every few hours).
    auto_failed: HashSet<String>,
    pub logging_in: bool,
    /// An Epic logout runs: the account going away is not an expired sign-in.
    logging_out: bool,
    /// Last problem of login / refresh, for the Epic page.
    pub last_error: Option<String>,
    eos: EosStatus,
    eos_games: HashMap<String, EosEntry>,
    gog: StoreState,
    amazon: StoreState,
    /// The Amazon sign-in in progress (nile's verifier, needed to register).
    amazon_login: Option<heronite_amazon::user::LoginRequest>,
    /// Games of the Amazon Games app are being brought in (`amazon_app`).
    amazon_app_importing: bool,
    /// GOG Galaxy's games are being looked for (`gog_galaxy`).
    gog_galaxy_importing: bool,
}

impl EventEmitter<DownloadsEvent> for Downloads {}

impl Downloads {
    pub fn new(data_paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = unbounded::<DmEvent>();
        let fake = crate::env::value("HERONITE_DEBUG_FAKE_DOWNLOAD");
        let job: Arc<dyn Job> = if fake.is_some() {
            Arc::new(debug::FakeJob)
        } else {
            // legendary is located when the first element runs.
            Arc::new(LazyJob { data_paths: data_paths.clone() })
        };
        let notify = move |ev| {
            let _ = tx.unbounded_send(ev);
        };
        // The fake queue must never reach download-manager.json.
        let manager = if fake.is_some() {
            DownloadManager::new_ephemeral(data_paths.clone(), job, notify)
        } else {
            DownloadManager::new(data_paths.clone(), job, notify)
        };
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                if this.update(cx, |this, cx| this.on_manager_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let snapshot = manager.snapshot();
        let mut this = Self {
            data_paths,
            manager,
            snapshot,
            progress: HashMap::new(),
            speed: Default::default(),
            offline: Default::default(),
            updates: HashSet::new(),
            fake_update: crate::env::value("HERONITE_DEBUG_FAKE_UPDATE"),
            busy: HashMap::new(),
            account: Account::default(),
            stats: Stats::default(),
            refreshing: false,
            auto_update_after_refresh: false,
            quiet_refresh: false,
            playing: HashSet::new(),
            deferred: HashSet::new(),
            auto_failed: HashSet::new(),
            logging_in: false,
            logging_out: false,
            last_error: None,
            eos: EosStatus::default(),
            eos_games: HashMap::new(),
            gog: StoreState::default(),
            amazon: StoreState::default(),
            amazon_login: None,
            amazon_app_importing: false,
            gog_galaxy_importing: false,
        };
        this.reload_local(cx);
        this.eos_reload(false, cx);
        // Runs one operation at startup (end-to-end checks with a fake legendary); see `debug_op`.
        if let Some(op) = crate::env::value("HERONITE_DEBUG_EPIC_OP") {
            this.debug_op(&op, cx);
        }
        if let Some(app) = fake {
            this.seed_fake(&app);
        } else if let Some(app) = crate::env::value("HERONITE_DEBUG_QUEUE_INSTALL") {
            // Integration check with a fake legendary (HERONITE_LEGENDARY_BIN): queues a real install.
            this.debug_queue_install(&app, cx);
        } else if !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") && heronite_platform::features::any_store() {
            this.spawn_idle_jobs(cx);
            this.spawn_offline_watch(cx);
        }
        this
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// `dlcList` / `thirdPartyManagedApp` of an Epic game from the library file.
    pub fn game_flags(&self, app_name: &str) -> GameFlags {
        self.stats.flags.get(app_name).copied().unwrap_or_default()
    }

    pub fn has_update(&self, app_name: &str) -> bool {
        self.updates.contains(app_name)
            || self.gog.updates.contains(app_name)
            || self.amazon.updates.contains(app_name)
            || self.fake_update.as_deref() == Some(app_name)
    }

    /// Updates of every store.
    pub fn updates_count(&self) -> usize {
        self.updates.len()
            + self.gog.updates.len()
            + self.amazon.updates.len()
            + usize::from(self.fake_update.as_ref().is_some_and(|a| !self.has_real_update(a)))
    }

    /// App names of every game with an update waiting (any store).
    pub fn update_apps(&self) -> HashSet<String> {
        let mut all: HashSet<String> = self.updates.iter().chain(&self.gog.updates).chain(&self.amazon.updates).cloned().collect();
        all.extend(self.fake_update.clone());
        all
    }

    fn has_real_update(&self, app_name: &str) -> bool {
        self.updates.contains(app_name) || self.gog.updates.contains(app_name) || self.amazon.updates.contains(app_name)
    }

    /// Epic's updates only (the Epic page).
    pub fn epic_updates_count(&self) -> usize {
        self.updates.len() + usize::from(self.fake_update.as_ref().is_some_and(|a| !self.updates.contains(a)))
    }

    pub fn busy_of(&self, app_name: &str) -> Option<&Busy> {
        self.busy.get(app_name)
    }

    pub fn progress_of(&self, app_name: &str) -> Option<&Progress> {
        self.progress.get(app_name)
    }

    /// The download speeds (MiB/s) of `app_name`'s last reports, oldest
    /// first; empty unless it is the running element.
    pub fn speed_samples(&self, app_name: &str) -> &VecDeque<f32> {
        static NONE: VecDeque<f32> = VecDeque::new();
        if self.speed.0 == app_name {
            &self.speed.1
        } else {
            &NONE
        }
    }

    /// Queue element for the game (running first, then waiting).
    pub fn queued(&self, app_name: &str) -> Option<(usize, &DmElement)> {
        self.snapshot.queue.iter().enumerate().find(|(_, e)| e.params.app_name == app_name)
    }

    fn emit(&self, cx: &mut Context<Self>, ev: DownloadsEvent) {
        cx.emit(ev);
    }

    fn toast(&self, cx: &mut Context<Self>, text: impl Into<String>, error: bool) {
        let text = text.into();
        log::info!("downloads: {text}");
        self.emit(cx, DownloadsEvent::Toast(text, error));
    }

    /// Account, stats and updates of every store from files on disk.
    pub fn reload_local(&mut self, cx: &mut Context<Self>) {
        for store in Store::ALL {
            self.store_reload_local(store, cx);
        }
        let data_paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (info, signed_in, updates, stats) = cx
                .background_spawn(async move {
                    let info = user::sync_user_info(&data_paths).ok().flatten();
                    let signed_in = user::is_logged_in(&data_paths);
                    (info, signed_in, library::updates_available(&data_paths), library_file::read_stats(&data_paths))
                })
                .await;
            this.update(cx, |this, cx| {
                // legendary deletes its login when Epic refuses it.
                let lost = this.account.logged_in && !signed_in && !this.logging_out;
                this.apply_account(info);
                if lost {
                    this.mark_expired(None, cx);
                }
                this.updates = updates.into_iter().collect();
                this.stats = stats;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_account(&mut self, info: Option<UserInfo>) {
        self.account = match info {
            Some(i) => Account { logged_in: true, name: i.display_name, expired: false },
            None => Account { expired: self.account.expired, ..Account::default() },
        };
    }

    /// The saved sign-in of a store (`None`: Epic) stopped working: it shows
    /// as logged out with "Log in again" and a message in place of the raw
    /// failure, said once.
    pub(super) fn mark_expired(&mut self, store: Option<Store>, cx: &mut Context<Self>) {
        let name = store.map_or("Epic Games", Store::name);
        let message = tr!("Your {store} sign-in expired. Log in again to download, update and sync saves.", store = name);
        let (account, last_error) = match store {
            None => (&mut self.account, &mut self.last_error),
            Some(s) => {
                let st = self.store_mut(s);
                (&mut st.account, &mut st.last_error)
            }
        };
        let first = !account.expired;
        account.logged_in = false;
        account.expired = true;
        *last_error = Some(message.clone());
        log::warn!("downloads: the {name} sign-in expired");
        if first {
            if !crate::system::has_window(cx) {
                crate::system::notify::send(name.to_string(), message.clone());
            }
            self.toast(cx, message, true);
        }
        cx.notify();
    }
}
