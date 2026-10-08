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
    pub account_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub owned: usize,
    pub installed: usize,
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
    pub logging_in: bool,
    /// Last problem of login / refresh, for the Epic page.
    pub last_error: Option<String>,
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
        let manager = DownloadManager::new(heroic.clone(), job, move |ev| {
            let _ = tx.unbounded_send(ev);
        });
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
            logging_in: false,
            last_error: None,
        };
        this.reload_local(cx);
        if let Some(app) = fake {
            this.seed_fake(&app);
        } else if std::env::var_os("HERONITE_DISABLE_IDLE_JOBS").is_none() {
            // Heroic starts the queue 5 s after launch; the update check
            // (network) a little later, if the setting is on.
            let manager = this.manager.clone();
            let heroic = heroic.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                manager.start();
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let check = game_config::load_global(&heroic, &ops::home())
                    .get("checkForUpdatesOnStartup")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if check {
                    this.update(cx, |this, cx| this.refresh_library(cx)).ok();
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

    pub fn is_paused(&self) -> bool {
        self.snapshot.state == DmState::Paused
    }

    /// Items waiting plus the running one.
    pub fn queue_len(&self) -> usize {
        self.snapshot.queue.len()
    }

    fn emit(&self, cx: &mut Context<Self>, ev: DownloadsEvent) {
        cx.emit(ev);
    }

    fn toast(&self, cx: &mut Context<Self>, text: impl Into<String>, error: bool) {
        self.emit(cx, DownloadsEvent::Toast(text.into(), error));
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
            Some(i) => Account { logged_in: true, name: i.display_name, account_id: i.account_id },
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
                        manager.add(DmElement::new(ElementKind::Install, InstallParams { app_name: dlc.clone(), install_dlcs: Vec::new(), ..base.clone() }));
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

/// Owned (without DLC) and installed counts from the library file.
fn read_stats(heroic: &HeroicPaths) -> Stats {
    let doc = json_store::read_value(&heroic.legendary_library()).unwrap_or(Value::Null);
    let list = doc.get("library").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut stats = Stats::default();
    for g in &list {
        let is_dlc = g.pointer("/install/is_dlc").and_then(Value::as_bool).unwrap_or(false);
        if is_dlc {
            continue;
        }
        stats.owned += 1;
        if g.get("is_installed").and_then(Value::as_bool).unwrap_or(false) {
            stats.installed += 1;
        }
    }
    stats
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
