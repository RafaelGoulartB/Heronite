//! Epic service: owns the download queue (running one install/update at a
//! time on a worker thread), the account state, the "update available" set
//! and the one-off operations on games (verify, uninstall, move, import,
//! cloud saves). Pages, dialogs and the Collection observe it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::json_store;
use heronite_data::HeroicPaths;
use heronite_epic::cmd::{Cancel, Legendary};
use heronite_epic::eos_overlay::{self, PrefixFolder};
use heronite_epic::manager::{DmState, DownloadManager, Event as DmEvent, Job, JobResult, Snapshot};
use heronite_epic::ops::{self, LegendaryJob, OpResult, RepairUpdate};
use heronite_epic::progress::Progress;
use heronite_epic::queue::{DmElement, DmStatus, ElementKind, InstallParams};
use heronite_epic::user::UserInfo;
use heronite_epic::util::file_size;
use heronite_epic::{info, library, saves, user};
use serde_json::Value;

/// A one-off operation running for a game.
#[derive(Clone)]
pub struct Busy {
    pub label: SharedString,
    /// 0..=100 when known.
    pub percent: Option<f32>,
    pub cancel: Option<Arc<Cancel>>,
}

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub logged_in: bool,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub owned: usize,
    pub installed: usize,
    /// Facts the Collection's slimmed-down game model leaves out.
    flags: Arc<HashMap<String, GameFlags>>,
}

/// What the library file says about a game that [`CollectionGame`] drops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameFlags {
    /// `dlcList` is not empty.
    pub has_dlcs: bool,
    /// EA App / Ubisoft Connect games (`thirdPartyManagedApp`).
    pub third_party: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    Downloading,
    Paused,
    Waiting,
}

/// What the install dialog (or an update) asks the queue for.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub game_info: Value,
    pub path: String,
    pub platform: String,
    pub sdl_tags: Vec<String>,
    pub dlcs: Vec<String>,
    pub language: String,
    /// Download size as text.
    pub size: Option<String>,
    /// Wine settings to store for the game (Linux): prefix and wine version.
    pub wine_prefix: Option<String>,
    pub wine_version: Option<Value>,
}

#[derive(Clone, Debug)]
pub enum DownloadsEvent {
    /// The Epic library file or the install state changed.
    LibraryChanged,
    Toast(String, bool),
    /// Text for a message dialog (cloud saves output).
    Message { title: String, text: String },
    /// "Enable EOS Overlay" was asked for a game but the overlay is not
    /// installed: the Collection offers to install it now.
    EosNotInstalled { app_name: String },
}

/// The EOS Overlay as the Epic page shows it.
#[derive(Clone, Debug, Default)]
pub struct EosStatus {
    pub installed: bool,
    pub version: String,
    pub latest: String,
    /// "Check for updates" is running.
    pub checking: bool,
}

/// Whether the overlay is on for the prefix of one game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EosGame {
    Loading,
    Enabled,
    Disabled,
    /// Not applicable here (the overlay is per prefix on Linux only).
    Unavailable,
}

pub struct Downloads {
    heroic: HeroicPaths,
    manager: DownloadManager,
    snapshot: Snapshot,
    progress: HashMap<String, Progress>,
    updates: HashSet<String>,
    /// `HERONITE_DEBUG_FAKE_UPDATE=<appName>`: shows an update for screenshots.
    fake_update: Option<String>,
    busy: HashMap<String, Busy>,
    account: Account,
    stats: Stats,
    pub refreshing: bool,
    /// Queue the updates found by the refresh in progress (`autoUpdateGames`).
    auto_update_after_refresh: bool,
    pub logging_in: bool,
    /// Last problem of login / refresh, for the Epic page.
    pub last_error: Option<String>,
    eos: EosStatus,
    eos_games: HashMap<String, EosGame>,
}

impl EventEmitter<DownloadsEvent> for Downloads {}

enum OpMsg {
    Percent(Option<f32>),
    Done(OpResult),
}

fn locate(heroic: &HeroicPaths) -> Result<Legendary, OpResult> {
    Legendary::locate(heroic).map_err(|e| OpResult { ok: false, aborted: false, message: format!("{e:#}") })
}

impl Downloads {
    pub fn new(heroic: HeroicPaths, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = unbounded::<DmEvent>();
        let fake = std::env::var("HERONITE_DEBUG_FAKE_DOWNLOAD").ok().filter(|s| !s.is_empty());
        let job: Arc<dyn Job> = if fake.is_some() {
            Arc::new(FakeJob)
        } else {
            // legendary is located when the first element runs.
            Arc::new(LazyJob { heroic: heroic.clone() })
        };
        let notify = move |ev| {
            let _ = tx.unbounded_send(ev);
        };
        // The fake queue must never reach download-manager.json.
        let manager = if fake.is_some() {
            DownloadManager::new_ephemeral(heroic.clone(), job, notify)
        } else {
            DownloadManager::new(heroic.clone(), job, notify)
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
            heroic: heroic.clone(),
            manager,
            snapshot,
            progress: HashMap::new(),
            updates: HashSet::new(),
            fake_update: std::env::var("HERONITE_DEBUG_FAKE_UPDATE").ok().filter(|s| !s.is_empty()),
            busy: HashMap::new(),
            account: Account::default(),
            stats: Stats::default(),
            refreshing: false,
            auto_update_after_refresh: false,
            logging_in: false,
            last_error: None,
            eos: EosStatus::default(),
            eos_games: HashMap::new(),
        };
        this.reload_local(cx);
        this.eos_reload(false, cx);
        if let Ok(op) = std::env::var("HERONITE_DEBUG_EPIC_OP") {
            this.debug_op(&op, cx);
        }
        if let Some(app) = fake {
            this.seed_fake(&app);
        } else if let Some(app) = std::env::var("HERONITE_DEBUG_QUEUE_INSTALL").ok().filter(|s| !s.is_empty()) {
            // Integration check with a fake legendary (HERONITE_LEGENDARY_BIN): queues a real install.
            this.debug_queue_install(&app, cx);
        } else if std::env::var_os("HERONITE_DISABLE_IDLE_JOBS").is_none() {
            // Heroic starts the queue 5 s after launch; the update check
            // (network) a little later, if the setting is on.
            let manager = this.manager.clone();
            let heroic = heroic.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                // Heronite resumes the same persisted queue on its own start;
                // two legendary processes on one game would fight, so leave
                // it to Heronite while it is open (resume from Downloads).
                if heronite_running() {
                    log::info!("Heronite is running: not resuming the download queue automatically");
                } else {
                    manager.start();
                }
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let check = game_config::load_global(&heroic, &ops::home())
                    .get("checkForUpdatesOnStartup")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if check {
                    this.update(cx, |this, cx| {
                        this.refresh_library(cx);
                        this.auto_update_after_refresh = true;
                    })
                    .ok();
                }
            })
            .detach();
        }
        this
    }

    // --- state ---------------------------------------------------------

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
        self.updates.contains(app_name) || self.fake_update.as_deref() == Some(app_name)
    }

    pub fn updates_count(&self) -> usize {
        self.updates.len() + usize::from(self.fake_update.as_ref().is_some_and(|a| !self.updates.contains(a)))
    }

    pub fn busy_of(&self, app_name: &str) -> Option<&Busy> {
        self.busy.get(app_name)
    }

    pub fn progress_of(&self, app_name: &str) -> Option<&Progress> {
        self.progress.get(app_name)
    }

    /// Queue element for the game (running first, then waiting).
    pub fn queued(&self, app_name: &str) -> Option<(usize, &DmElement)> {
        self.snapshot.queue.iter().enumerate().find(|(_, e)| e.params.app_name == app_name)
    }

    /// Where the game is in the queue.
    pub fn queue_state(&self, app_name: &str) -> Option<QueueState> {
        let (index, _) = self.queued(app_name)?;
        Some(match (index, self.snapshot.state) {
            (0, DmState::Running) => QueueState::Downloading,
            (0, DmState::Paused) => QueueState::Paused,
            _ => QueueState::Waiting,
        })
    }

    pub fn is_running(&self) -> bool {
        self.snapshot.state == DmState::Running
    }

    fn emit(&self, cx: &mut Context<Self>, ev: DownloadsEvent) {
        cx.emit(ev);
    }

    fn toast(&self, cx: &mut Context<Self>, text: impl Into<String>, error: bool) {
        let text = text.into();
        log::info!("epic: {text}");
        self.emit(cx, DownloadsEvent::Toast(text, error));
    }

    /// Account, stats and updates from files on disk.
    pub fn reload_local(&mut self, cx: &mut Context<Self>) {
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let (info, updates, stats) = cx
                .background_spawn(async move {
                    let info = user::sync_user_info(&heroic).ok().flatten();
                    (info, library::updates_available(&heroic), read_stats(&heroic))
                })
                .await;
            this.update(cx, |this, cx| {
                this.apply_account(info);
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
            Some(i) => Account { logged_in: true, name: i.display_name },
            None => Account::default(),
        };
    }

    fn on_manager_event(&mut self, ev: DmEvent, cx: &mut Context<Self>) {
        match ev {
            DmEvent::Progress { app_name, progress } => {
                self.progress.insert(app_name, progress);
            }
            DmEvent::Changed => {
                self.snapshot = self.manager.snapshot();
                let alive: HashSet<&str> = self.snapshot.queue.iter().map(|e| e.params.app_name.as_str()).collect();
                self.progress.retain(|app, _| alive.contains(app.as_str()));
            }
            DmEvent::Finished { element, error } => {
                let title = element.params.title();
                let verb = if element.kind == ElementKind::Install { "Installation" } else { "Update" };
                match element.status {
                    Some(DmStatus::Done) => {
                        self.toast(cx, format!("{title}: {} finished", verb.to_lowercase()), false);
                        self.progress.remove(&element.params.app_name);
                        self.emit(cx, DownloadsEvent::LibraryChanged);
                        self.reload_local(cx);
                        if element.kind == ElementKind::Install {
                            // Desktop / menu / Steam shortcuts from the General settings.
                            crate::system::shortcuts::spawn_on_game_added(cx, &self.heroic, "legendary", &element.params.app_name, |this, o, cx| {
                                if let Some(m) = o.summary() {
                                    this.toast(cx, m.text, m.error);
                                }
                            });
                        }
                    }
                    Some(DmStatus::Error) => {
                        let why = error.unwrap_or_default();
                        let first = why.lines().next().unwrap_or("").to_string();
                        self.toast(cx, format!("{title}: {} failed{}", verb.to_lowercase(), if first.is_empty() { String::new() } else { format!(" ({first})") }), true);
                    }
                    _ => {}
                }
            }
        }
        cx.notify();
    }

    // --- queue ---------------------------------------------------------

    /// Queues an install (and its DLCs) the way Heroic's `install` handler does.
    pub fn queue_install(&mut self, req: InstallRequest, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let heroic = self.heroic.clone();
        let app_name = req.game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if req.wine_prefix.is_some() || req.wine_version.is_some() {
                        ops::write_wine_config(&heroic, &app_name, req.wine_prefix.as_deref().unwrap_or(""), req.wine_version.as_ref())?;
                    }
                    let base = InstallParams {
                        app_name: app_name.clone(),
                        path: req.path.clone(),
                        install_dlcs: req.dlcs.clone(),
                        sdl_list: req.sdl_tags.clone(),
                        install_language: req.language.clone(),
                        runner: "legendary".into(),
                        platform_to_install: req.platform.clone(),
                        game_info: req.game_info.clone(),
                        size: req.size.clone(),
                        ..Default::default()
                    };
                    manager.add(DmElement::new(ElementKind::Install, base.clone()));
                    for dlc in &req.dlcs {
                        // The queue lists the DLC under its own title and cover.
                        let game_info = library_entry(&heroic, dlc).unwrap_or_else(|| base.game_info.clone());
                        manager.add(DmElement::new(
                            ElementKind::Install,
                            InstallParams { app_name: dlc.clone(), install_dlcs: Vec::new(), game_info, size: None, ..base.clone() },
                        ));
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.toast(cx, "Added to the download queue", false),
                Err(err) => this.toast(cx, format!("Could not queue the installation: {err:#}"), true),
            })
            .ok();
        })
        .detach();
    }

    /// `updateGame`: queues an update of an installed game.
    pub fn queue_update(&mut self, game_info: Value, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        let heroic = self.heroic.clone();
        let app = game_info.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string();
        let path = game_info.pointer("/install/install_path").and_then(Value::as_str).unwrap_or_default().to_string();
        let platform = game_info.pointer("/install/platform").and_then(Value::as_str).unwrap_or("Windows").to_string();
        cx.background_spawn(async move {
            let size = Legendary::locate(&heroic)
                .ok()
                .and_then(|lg| info::install_info(&heroic, &lg, &app, &platform).ok())
                .map(|i| file_size(i.download_size()));
            manager.add(DmElement::new(
                ElementKind::Update,
                InstallParams {
                    app_name: app,
                    path,
                    runner: "legendary".into(),
                    platform_to_install: platform,
                    game_info,
                    size,
                    ..Default::default()
                },
            ));
        })
        .detach();
        self.toast(cx, "Update added to the download queue", false);
    }

    pub fn pause(&mut self) {
        self.manager.pause();
    }

    pub fn resume(&mut self) {
        self.manager.resume();
    }

    pub fn cancel_current(&mut self, remove_downloaded: bool) {
        self.manager.cancel_current(remove_downloaded);
    }

    pub fn remove_from_queue(&mut self, app_name: &str) {
        self.manager.remove(app_name);
    }

    pub fn move_in_queue(&mut self, app_name: &str, up: bool) {
        self.manager.move_element(app_name, up);
    }

    pub fn clear_finished(&mut self) {
        self.manager.clear_finished();
    }

    pub fn clear_finished_item(&mut self, app_name: &str) {
        self.manager.clear_finished_item(app_name);
    }

    // --- one-off operations ---------------------------------------------

    fn run_op(
        &mut self,
        app_name: &str,
        label: &str,
        cancellable: bool,
        cx: &mut Context<Self>,
        work: impl FnOnce(&HeroicPaths, &Legendary, &Arc<Cancel>, &dyn Fn(Option<f32>)) -> OpResult + Send + 'static,
    ) {
        self.run_op_then(app_name, label, cancellable, cx, work, |_, _, _| {});
    }

    /// [`run_op`](Self::run_op) that also calls `then` with the result on the
    /// UI thread, after the toast and the library reload.
    fn run_op_then(
        &mut self,
        app_name: &str,
        label: &str,
        cancellable: bool,
        cx: &mut Context<Self>,
        work: impl FnOnce(&HeroicPaths, &Legendary, &Arc<Cancel>, &dyn Fn(Option<f32>)) -> OpResult + Send + 'static,
        then: impl FnOnce(&mut Self, &OpResult, &mut Context<Self>) + 'static,
    ) {
        let mut then = Some(then);
        if self.busy.contains_key(app_name) {
            self.toast(cx, "Another operation is running for this game", true);
            return;
        }
        let cancel = cancellable.then(Cancel::new);
        self.busy.insert(
            app_name.to_string(),
            Busy { label: label.to_string().into(), percent: None, cancel: cancel.clone() },
        );
        cx.notify();
        let (tx, mut rx) = unbounded::<OpMsg>();
        let heroic = self.heroic.clone();
        let c = cancel.unwrap_or_else(Cancel::new);
        std::thread::Builder::new()
            .name("epic-op".into())
            .spawn(move || {
                let result = match locate(&heroic) {
                    Err(r) => r,
                    Ok(lg) => {
                        let progress_tx: UnboundedSender<OpMsg> = tx.clone();
                        work(&heroic, &lg, &c, &move |p| {
                            let _ = progress_tx.unbounded_send(OpMsg::Percent(p));
                        })
                    }
                };
                let _ = tx.unbounded_send(OpMsg::Done(result));
            })
            .expect("spawning an epic operation");
        let app = app_name.to_string();
        let label = label.to_string();
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = this
                    .update(cx, |this, cx| match msg {
                        OpMsg::Percent(p) => {
                            if let Some(b) = this.busy.get_mut(&app) {
                                b.percent = p;
                            }
                            cx.notify();
                            false
                        }
                        OpMsg::Done(r) => {
                            this.busy.remove(&app);
                            if r.aborted {
                                this.toast(cx, format!("{label} cancelled"), false);
                            } else if r.ok {
                                this.toast(cx, r.message.clone(), false);
                            } else {
                                let first = r.message.lines().next().unwrap_or("Failed").to_string();
                                this.toast(cx, format!("{label} failed: {first}"), true);
                            }
                            this.emit(cx, DownloadsEvent::LibraryChanged);
                            this.reload_local(cx);
                            if let Some(then) = then.take() {
                                then(this, &r, cx);
                            }
                            cx.notify();
                            true
                        }
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    /// Verify and repair (`legendary repair`).
    pub fn verify(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        self.run_op(app_name, "Verifying", true, cx, move |heroic, lg, cancel, report| {
            ops::repair(heroic, lg, &app, cancel, |u| match u {
                RepairUpdate::Verifying(p) => report(Some(p as f32)),
                RepairUpdate::Downloading(p) => report(Some(p.percent as f32)),
            })
        });
    }

    /// Uninstall; optionally removes the Wine prefix and the game settings.
    pub fn uninstall(&mut self, app_name: &str, remove_prefix: bool, remove_settings: bool, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        self.run_op(app_name, "Uninstalling", false, cx, move |heroic, lg, _, _| {
            // The prefix is looked up before the settings are deleted.
            let prefix_to_remove = remove_prefix;
            let r = ops::uninstall(heroic, lg, &app);
            if r.ok {
                if prefix_to_remove {
                    if let Err(err) = ops::remove_prefix(heroic, &app) {
                        log::warn!("removing the prefix of {app}: {err:#}");
                    }
                }
                if remove_settings {
                    ops::remove_settings_and_logs(heroic, &app);
                }
            }
            r
        });
    }

    pub fn move_install(&mut self, app_name: &str, install_path: &str, new_parent: &str, cx: &mut Context<Self>) {
        let (app, from, to) = (app_name.to_string(), install_path.to_string(), new_parent.to_string());
        self.run_op(app_name, "Moving", false, cx, move |heroic, lg, _, _| {
            let r = ops::move_install(heroic, lg, &app, &from, &to);
            if r.ok {
                OpResult { ok: true, aborted: false, message: "Game moved".into() }
            } else {
                r
            }
        });
    }

    pub fn import_game(
        &mut self,
        app_name: &str,
        folder: &str,
        platform: &str,
        wine_prefix: Option<String>,
        wine_version: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        let (app, folder, platform) = (app_name.to_string(), folder.to_string(), platform.to_string());
        self.run_op(app_name, "Importing", false, cx, move |heroic, lg, _, _| {
            let r = ops::import_game(heroic, lg, &app, &folder, &platform);
            if r.ok && (wine_prefix.is_some() || wine_version.is_some()) {
                if let Err(err) = ops::write_wine_config(heroic, &app, wine_prefix.as_deref().unwrap_or(""), wine_version.as_ref()) {
                    log::warn!("writing the Wine settings of {app}: {err:#}");
                }
            }
            r
        });
    }

    /// Cloud saves: syncs with the folder in the game settings (detecting it
    /// when empty) and shows legendary's output.
    pub fn sync_saves(&mut self, app_name: &str, title: &str, cx: &mut Context<Self>) {
        let (app, title_owned) = (app_name.to_string(), title.to_string());
        let (tx, mut rx) = unbounded::<Result<String, String>>();
        self.run_op(app_name, "Syncing saves", false, cx, move |heroic, _, _, _| {
            let configured = game_config::load_game(heroic, &app, &ops::home())
                .get("savesPath")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let path = if configured.is_empty() {
                match saves::detect_path(heroic, &app) {
                    Ok(Some(p)) => p,
                    Ok(None) => return OpResult { ok: false, aborted: false, message: "The save folder could not be detected; set it in the game settings".into() },
                    Err(err) => return OpResult { ok: false, aborted: false, message: format!("{err:#}") },
                }
            } else {
                configured
            };
            match saves::sync(heroic, &app, &path, saves::SyncMode::Both) {
                Ok(text) => {
                    let _ = tx.unbounded_send(Ok(text));
                    OpResult { ok: true, aborted: false, message: "Cloud saves synced".into() }
                }
                Err(err) => OpResult { ok: false, aborted: false, message: format!("{err:#}") },
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some(Ok(text)) = rx.next().await {
                this.update(cx, |this, cx| {
                    this.emit(cx, DownloadsEvent::Message { title: format!("Cloud saves: {title_owned}"), text });
                })
                .ok();
            }
        })
        .detach();
    }

    /// `legendary list` + rebuild of the library file + update detection.
    pub fn refresh_library(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.last_error = None;
        cx.notify();
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&heroic)?;
                    let r = library::refresh(&heroic, &lg)?;
                    anyhow::Ok((r, library::updates_available(&heroic), read_stats(&heroic)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match res {
                    Ok((r, updates, stats)) => {
                        this.updates = updates.into_iter().collect();
                        this.stats = stats;
                        match r.list_error {
                            Some(e) => {
                                this.last_error = Some(e.clone());
                                this.toast(cx, format!("Could not refresh the Epic library: {}", e.lines().next().unwrap_or("")), true);
                            }
                            None => {
                                let n = this.updates.len();
                                let msg = if n > 0 { format!("Epic library refreshed - {n} update(s) available") } else { "Epic library refreshed".to_string() };
                                this.toast(cx, msg, false);
                            }
                        }
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                        if std::mem::take(&mut this.auto_update_after_refresh) {
                            this.queue_auto_updates(cx);
                        }
                    }
                    Err(err) => {
                        this.last_error = Some(format!("{err:#}"));
                        this.toast(cx, format!("{err:#}"), true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Queues an update for every installed game that has one.
    pub fn update_all(&mut self, cx: &mut Context<Self>) {
        let apps: Vec<String> = self.updates.iter().cloned().collect();
        for app in apps {
            if self.queued(&app).is_some() {
                continue;
            }
            if let Some(info) = library_entry(&self.heroic, &app) {
                self.queue_update(info, cx);
            }
        }
    }

    /// `autoUpdateGames`: queues the updates found, except for games that
    /// ignore updates (`ignoreGameUpdates` in their settings).
    fn queue_auto_updates(&mut self, cx: &mut Context<Self>) {
        let home = ops::home();
        let enabled = game_config::load_global(&self.heroic, &home).get("autoUpdateGames").and_then(Value::as_bool).unwrap_or(false);
        if !enabled {
            return;
        }
        let apps: Vec<String> = self.updates.iter().cloned().collect();
        for app in apps {
            let ignored = game_config::load_game(&self.heroic, &app, &home).get("ignoreGameUpdates").and_then(Value::as_bool).unwrap_or(false);
            if ignored || self.queued(&app).is_some() {
                continue;
            }
            if let Some(info) = library_entry(&self.heroic, &app) {
                self.queue_update(info, cx);
            }
        }
    }

    // --- modify installation (DLCs) ---------------------------------------

    /// Installs one owned DLC of an installed game into the game's folder
    /// (Heroic's `DLC` row: a normal install of the DLC with the base game's
    /// path and platform, no components of its own).
    pub fn install_dlc(&mut self, dlc_info: Value, base_path: &str, platform: &str, size: Option<String>, cx: &mut Context<Self>) {
        self.queue_install(
            InstallRequest {
                game_info: dlc_info,
                path: base_path.to_string(),
                platform: platform.to_string(),
                sdl_tags: Vec::new(),
                dlcs: Vec::new(),
                language: "en-US".into(),
                size,
                wine_prefix: None,
                wine_version: None,
            },
            cx,
        );
    }

    // --- EOS overlay -----------------------------------------------------

    pub fn eos(&self) -> &EosStatus {
        &self.eos
    }

    /// Reads the overlay state; `check` also looks up the latest version
    /// (which may ask legendary, so it only runs when the page is open).
    pub fn eos_reload(&mut self, check: bool, cx: &mut Context<Self>) {
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let (status, latest) = cx
                .background_spawn(async move {
                    let status = eos_overlay::status(&heroic);
                    let latest = if check && status.installed {
                        Legendary::locate(&heroic).map(|lg| eos_overlay::latest_version(&heroic, &lg)).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    (status, latest)
                })
                .await;
            this.update(cx, |this, cx| {
                this.eos.installed = status.installed;
                this.eos.version = status.version.unwrap_or_default();
                if !status.installed {
                    this.eos.latest.clear();
                } else if check {
                    this.eos.latest = latest;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Check for updates": `legendary status`, then the latest version.
    pub fn eos_check_updates(&mut self, cx: &mut Context<Self>) {
        if self.eos.checking {
            return;
        }
        self.eos.checking = true;
        cx.notify();
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let latest = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&heroic).ok()?;
                    eos_overlay::update_info(&heroic, &lg);
                    Some(eos_overlay::latest_version(&heroic, &lg))
                })
                .await;
            this.update(cx, |this, cx| {
                this.eos.checking = false;
                if let Some(latest) = latest {
                    this.eos.latest = latest;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Installs the overlay, or updates it when it is already installed.
    pub fn eos_install(&mut self, cx: &mut Context<Self>) {
        self.eos_install_then(None, cx);
    }

    fn eos_install_then(&mut self, enable_for: Option<String>, cx: &mut Context<Self>) {
        let label = if self.eos.installed { "Updating EOS Overlay" } else { "Installing EOS Overlay" };
        self.run_op_then(
            eos_overlay::APP_NAME,
            label,
            true,
            cx,
            |heroic, lg, cancel, report| eos_overlay::install(heroic, lg, cancel, &|p| report(Some(p))),
            move |this, r, cx| {
                this.eos_reload(true, cx);
                if r.ok {
                    if let Some(app) = enable_for {
                        this.eos_enable_game(&app, cx);
                    }
                }
            },
        );
    }

    /// Uninstalls the overlay (the caller has asked the user).
    pub fn eos_remove(&mut self, cx: &mut Context<Self>) {
        self.run_op_then(
            eos_overlay::APP_NAME,
            "Removing EOS Overlay",
            false,
            cx,
            |_, lg, _, _| eos_overlay::remove(lg),
            |this, r, cx| {
                if r.ok {
                    this.eos_games.clear();
                }
                this.eos_reload(true, cx);
            },
        );
    }

    /// Whether the overlay is on for `app_name`'s prefix (once loaded).
    pub fn eos_game(&self, app_name: &str) -> Option<EosGame> {
        self.eos_games.get(app_name).copied()
    }

    /// Looks the state of a game up in the background, once (the context menu
    /// calls this on every frame it is open).
    pub fn eos_load_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        if self.eos_games.contains_key(app_name) {
            return;
        }
        self.eos_games.insert(app_name.to_string(), EosGame::Loading);
        let (heroic, app) = (self.heroic.clone(), app_name.to_string());
        cx.spawn(async move |this, cx| {
            let state = cx
                .background_spawn({
                    let app = app.clone();
                    async move {
                        match eos_overlay::prefix_folder(&heroic, &app) {
                            PrefixFolder::NotApplicable => EosGame::Unavailable,
                            PrefixFolder::Invalid => EosGame::Disabled,
                            PrefixFolder::Valid(prefix) => {
                                let enabled = Legendary::locate(&heroic).map(|lg| eos_overlay::is_enabled(&lg, Some(&prefix))).unwrap_or(false);
                                if enabled { EosGame::Enabled } else { EosGame::Disabled }
                            }
                        }
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.eos_games.insert(app, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The "Enable / Disable EOS Overlay" item of a game.
    pub fn eos_toggle_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        match self.eos_game(app_name) {
            Some(EosGame::Enabled) => self.eos_set_game(app_name, false, cx),
            Some(EosGame::Disabled) => self.eos_enable_game(app_name, cx),
            _ => {}
        }
    }

    /// Enabling needs the overlay: when it is missing the Collection asks
    /// whether to install it first ([`DownloadsEvent::EosNotInstalled`]).
    fn eos_enable_game(&mut self, app_name: &str, cx: &mut Context<Self>) {
        if !self.eos.installed {
            self.emit(cx, DownloadsEvent::EosNotInstalled { app_name: app_name.to_string() });
            return;
        }
        self.eos_set_game(app_name, true, cx);
    }

    /// Installs the overlay and enables it for the game (the answer to
    /// "The EOS Overlay is not installed. Do you want to install it now?").
    pub fn eos_install_and_enable(&mut self, app_name: &str, cx: &mut Context<Self>) {
        self.eos_install_then(Some(app_name.to_string()), cx);
    }

    fn eos_set_game(&mut self, app_name: &str, enable: bool, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        let label = if enable { "Enabling EOS Overlay" } else { "Disabling EOS Overlay" };
        let app_for_work = app.clone();
        self.run_op_then(
            app_name,
            label,
            false,
            cx,
            move |heroic, lg, _, _| {
                let prefix = match eos_overlay::prefix_folder(heroic, &app_for_work) {
                    PrefixFolder::Valid(p) => Some(p),
                    PrefixFolder::NotApplicable => None,
                    // Nothing to enable / disable without a prefix (Heroic returns quietly).
                    PrefixFolder::Invalid if !enable => {
                        return OpResult { ok: true, aborted: false, message: "EOS Overlay disabled".into() }
                    }
                    PrefixFolder::Invalid => {
                        return OpResult {
                            ok: false,
                            aborted: false,
                            message: "The Wine prefix of this game does not exist yet; start the game once first".into(),
                        }
                    }
                };
                if enable {
                    eos_overlay::enable(lg, prefix.as_deref())
                } else {
                    eos_overlay::disable(lg, prefix.as_deref())
                }
            },
            move |this, r, _| {
                if r.ok {
                    this.eos_games.insert(app, if enable { EosGame::Enabled } else { EosGame::Disabled });
                }
            },
        );
    }

    // --- account ---------------------------------------------------------

    /// Logs in with what the user pasted or the webview returned.
    pub fn login(&mut self, input: String, cx: &mut Context<Self>) {
        if self.logging_in {
            return;
        }
        self.logging_in = true;
        self.last_error = None;
        cx.notify();
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&heroic)?;
                    let info = user::login(&heroic, &lg, &input)?;
                    // Heroic refreshes the library right after logging in.
                    let refresh = library::refresh(&heroic, &lg);
                    anyhow::Ok((info, refresh.ok(), library::updates_available(&heroic), read_stats(&heroic)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.logging_in = false;
                match res {
                    Ok((info, _refresh, updates, stats)) => {
                        let name = info.display_name.clone();
                        this.apply_account(Some(info));
                        this.updates = updates.into_iter().collect();
                        this.stats = stats;
                        this.toast(cx, format!("Logged in as {name}"), false);
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                    }
                    Err(err) => {
                        let msg = format!("{err:#}");
                        this.last_error = Some(msg.clone());
                        this.toast(cx, msg, true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&heroic)?;
                    user::logout(&heroic, &lg)
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(()) => {
                        this.apply_account(None);
                        this.updates.clear();
                        this.stats = Stats::default();
                        this.toast(cx, "Logged out of Epic Games", false);
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                    }
                    Err(err) => this.toast(cx, format!("{err:#}"), true),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // --- debugging -------------------------------------------------------

    /// `HERONITE_DEBUG_EPIC_OP=verify:<app>|uninstall:<app>[:prefix][:settings]|move:<app>:<dir>|import:<app>:<dir>|sync:<app>|eos-install|eos-remove|eos-enable:<app>|eos-disable:<app>`:
    /// runs one operation at startup (for end-to-end checks with a fake legendary).
    fn debug_op(&mut self, op: &str, cx: &mut Context<Self>) {
        let parts: Vec<&str> = op.split(':').collect();
        let app = parts.get(1).copied().unwrap_or_default().to_string();
        let install_path = library_entry(&self.heroic, &app)
            .and_then(|g| g.pointer("/install/install_path").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        match parts.first().copied() {
            Some("verify") => self.verify(&app, cx),
            Some("uninstall") => self.uninstall(&app, parts.contains(&"prefix"), parts.contains(&"settings"), cx),
            Some("move") => self.move_install(&app, &install_path, parts.get(2).copied().unwrap_or_default(), cx),
            Some("import") => self.import_game(&app, parts.get(2).copied().unwrap_or_default(), "Windows", None, None, cx),
            Some("sync") => self.sync_saves(&app, &app, cx),
            Some("eos-install") => self.eos_install(cx),
            Some("eos-remove") => self.eos_remove(cx),
            Some("eos-enable") => {
                self.eos.installed = eos_overlay::status(&self.heroic).installed;
                self.eos_games.insert(app.clone(), EosGame::Disabled);
                self.eos_toggle_game(&app, cx)
            }
            Some("eos-disable") => {
                self.eos_games.insert(app.clone(), EosGame::Enabled);
                self.eos_toggle_game(&app, cx)
            }
            _ => log::warn!("unknown HERONITE_DEBUG_EPIC_OP {op}"),
        }
    }

    /// `HERONITE_DEBUG_QUEUE_INSTALL=<appName>`: queues an install of that game
    /// into `$HERONITE_DEBUG_INSTALL_DIR` the way the install dialog would.
    fn debug_queue_install(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let Some(game_info) = library_entry(&self.heroic, app_name) else {
            log::warn!("debug install: {app_name} is not in the Epic library");
            return;
        };
        let path = std::env::var("HERONITE_DEBUG_INSTALL_DIR").unwrap_or_else(|_| "/tmp/heronite-fake-games".into());
        let dlcs = std::env::var("HERONITE_DEBUG_INSTALL_DLCS").ok().map(|d| d.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
        self.queue_install(
            InstallRequest {
                game_info,
                path,
                platform: "Windows".into(),
                sdl_tags: Vec::new(),
                dlcs,
                language: String::new(),
                size: Some("1 GiB".into()),
                wine_prefix: None,
                wine_version: None,
            },
            cx,
        );
    }

    /// `HERONITE_DEBUG_FAKE_DOWNLOAD=<appName>`: a queue with a fake running
    /// install (no legendary), one waiting and one finished, for screenshots.
    fn seed_fake(&mut self, app_name: &str) {
        let games = json_store::read_value(&self.heroic.legendary_library()).unwrap_or(Value::Null);
        let list: Vec<Value> = games.get("library").and_then(Value::as_array).cloned().unwrap_or_default();
        let find = |app: &str| list.iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app)).cloned();
        let mut chosen: Vec<Value> = Vec::new();
        if let Some(g) = find(app_name) {
            chosen.push(g);
        }
        for g in list.iter().filter(|g| g.get("app_name").and_then(Value::as_str) != Some(app_name)).take(3) {
            chosen.push(g.clone());
        }
        let mk = |g: &Value, kind: ElementKind, size: &str| {
            DmElement::new(
                kind,
                InstallParams {
                    app_name: g.get("app_name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    path: "/tmp/heronite-fake/Games".into(),
                    runner: "legendary".into(),
                    platform_to_install: "Windows".into(),
                    game_info: g.clone(),
                    size: Some(size.into()),
                    ..Default::default()
                },
            )
        };
        let manager = self.manager.clone();
        if let Some(g) = chosen.first() {
            manager.add(mk(g, ElementKind::Install, "25.87 GiB"));
        }
        for (g, size) in chosen.iter().skip(1).take(2).zip(["4.2 GiB", "812.5 MiB"]) {
            manager.add(mk(g, ElementKind::Update, size));
        }
    }
}

/// Whether Heronite/Heroic (Electron, process name `heroic`) is running.
fn heronite_running() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "heroic")
    })
}

/// The library file's `GameInfo` of one game.
pub fn library_entry(heroic: &HeroicPaths, app_name: &str) -> Option<Value> {
    let doc = json_store::read_value(&heroic.legendary_library()).ok()?;
    doc.get("library")?
        .as_array()?
        .iter()
        .find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name))
        .cloned()
}

/// Owned (without DLC) and installed counts from the library file.
fn read_stats(heroic: &HeroicPaths) -> Stats {
    let doc = json_store::read_value(&heroic.legendary_library()).unwrap_or(Value::Null);
    let list = doc.get("library").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut stats = Stats::default();
    let mut flags = HashMap::new();
    for g in &list {
        if let Some(app) = g.get("app_name").and_then(Value::as_str) {
            let has_dlcs = g.get("dlcList").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
            let third_party = g.get("thirdPartyManagedApp").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            flags.insert(app.to_string(), GameFlags { has_dlcs, third_party });
        }
        let is_dlc = g.pointer("/install/is_dlc").and_then(Value::as_bool).unwrap_or(false);
        if is_dlc {
            continue;
        }
        stats.owned += 1;
        if g.get("is_installed").and_then(Value::as_bool).unwrap_or(false) {
            stats.installed += 1;
        }
    }
    stats.flags = Arc::new(flags);
    stats
}

/// What the first launch of an Epic game in a prefix asks legendary for
/// (the launcher crate cannot depend on the Epic crate): the game's
/// prerequisites and the EOS overlay of the prefix.
pub struct EpicLaunchSetup(pub HeroicPaths);

impl heronite_launch::first_launch::EpicSetup for EpicLaunchSetup {
    fn prerequisites(&self, app_name: &str, platform: &str) -> Result<Option<heronite_launch::first_launch::Prerequisite>, String> {
        let lg = Legendary::locate(&self.0).map_err(|e| format!("{e:#}"))?;
        let info = info::install_info(&self.0, &lg, app_name, platform).map_err(|e| format!("{e:#}"))?;
        Ok(info.prerequisites().map(|(path, args)| heronite_launch::first_launch::Prerequisite { path, args }))
    }

    fn eos_overlay_installed(&self) -> bool {
        eos_overlay::status(&self.0).installed
    }

    fn eos_overlay_enabled(&self, prefix: &str) -> bool {
        Legendary::locate(&self.0).map(|lg| eos_overlay::is_enabled(&lg, Some(prefix))).unwrap_or(false)
    }

    fn eos_overlay_enable(&self, prefix: &str) -> Result<(), String> {
        let lg = Legendary::locate(&self.0).map_err(|e| format!("{e:#}"))?;
        let r = eos_overlay::enable(&lg, Some(prefix));
        if r.ok { Ok(()) } else { Err(r.message) }
    }
}

/// Locates legendary when the first element runs (cheap to create, and picks
/// up an `altLegendaryBin` changed in the settings).
struct LazyJob {
    heroic: HeroicPaths,
}

impl Job for LazyJob {
    fn run(&self, element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        match Legendary::locate(&self.heroic) {
            Ok(legendary) => LegendaryJob { heroic: self.heroic.clone(), legendary }.run(element, cancel, on_progress),
            Err(err) => JobResult { status: DmStatus::Error, error: Some(format!("{err:#}")) },
        }
    }
}

/// Debug job: slowly advances a percentage and never finishes.
struct FakeJob;

impl Job for FakeJob {
    fn run(&self, _element: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let mut percent = 37.4;
        while !cancel.is_requested() {
            on_progress(Progress {
                percent,
                downloaded_mib: percent * 120.,
                eta: "00:08:20".into(),
                down_speed: 5.43,
                disk_speed: 5.12,
            });
            percent = (percent + 0.1).min(99.);
            std::thread::sleep(Duration::from_millis(1000));
        }
        JobResult { status: DmStatus::Abort, error: None }
    }
}
