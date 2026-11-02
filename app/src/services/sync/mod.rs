//! The cloud account (`heronite_sync`, Settings → Cloud): the library sync
//! between PCs [`AFTER_START`] after the app starts, [`AFTER_GAME`] after
//! each game and every [`EVERY`] (never while a game launched from here
//! runs), plus "Sync now", and the uploads (`uploads`: backups, game saves,
//! screenshots). Connecting the account runs here too, so the page can close
//! while the browser sign-in waits.
//!
//! A sync that changed the Collection sends [`SyncEvent::Changed`]: the
//! Collection reads it again.

mod restores;
mod uploads;

pub use restores::{Bring, Step};
pub use uploads::Job;

use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::proc::Cancel;
use heronite_data::{AppPaths, DataPaths};
use heronite_sync::{Provider, Server, SyncError};

const AFTER_START: Duration = Duration::from_secs(20);
const AFTER_GAME: Duration = Duration::from_secs(5);
const EVERY: Duration = Duration::from_secs(10 * 60);
/// The screenshots are looked at every this many syncs too (hourly), in case
/// a capture came in without a game or a compression run.
const CAPTURES_EVERY: u32 = 6;

#[derive(Clone, Debug)]
pub enum SyncEvent {
    /// A sync brought changes into the Collection.
    Changed,
}

/// A cloud account being connected.
pub struct Connecting {
    pub provider: Provider,
    /// The sign-in page, once rclone gave it (for "Open the page").
    pub link: Option<String>,
    cancel: Arc<Cancel>,
}

pub struct LibrarySync {
    data_paths: DataPaths,
    app_paths: AppPaths,
    running: bool,
    connecting: Option<Connecting>,
    /// Why the last connection failed.
    connect_error: Option<String>,
    queue: uploads::Queue,
    restores: restores::Restores,
    /// Whether the compression was busy at its last change.
    compressing: bool,
    /// Goes up each time `sync.json` may have changed (a sign-in or
    /// sign-out, a sync, an upload ended): Settings → Cloud reads it again.
    changes: u64,
}

impl EventEmitter<SyncEvent> for LibrarySync {}

struct SyncGlobal(Entity<LibrarySync>);
impl Global for SyncGlobal {}

/// Starts the service and its timer (once, at start).
pub fn init(cx: &mut App, data_paths: DataPaths) {
    if cx.has_global::<SyncGlobal>() {
        return;
    }
    let app_paths = AppPaths::for_data(&data_paths);
    let entity = cx.new(|cx| {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AFTER_START).await;
            let mut round = 0u32;
            loop {
                let alive = this
                    .update(cx, |s: &mut LibrarySync, cx: &mut Context<LibrarySync>| {
                        if crate::system::restart_blocker(cx).is_none() {
                            s.sync(false, cx);
                        }
                        // What came in while the app was closed, then hourly.
                        if round == 0 {
                            s.upload(Job::LatestBackup, false, cx);
                            s.upload(Job::Saves, false, cx);
                        }
                        if round.is_multiple_of(CAPTURES_EVERY) {
                            s.upload(Job::Captures, false, cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                round = round.wrapping_add(1);
                cx.background_executor().timer(EVERY).await;
            }
        })
        .detach();
        // Captures go up once the compression has nothing left.
        if let Some(compression) = crate::services::compression::get(cx) {
            cx.observe(&compression, |s: &mut LibrarySync, c, cx| {
                let busy = c.read(cx).busy();
                if s.compressing && !busy {
                    s.upload(Job::Captures, false, cx);
                }
                s.compressing = busy;
            })
            .detach();
        }
        LibrarySync {
            data_paths,
            app_paths,
            running: false,
            connecting: None,
            connect_error: None,
            queue: Default::default(),
            restores: Default::default(),
            compressing: false,
            changes: 0,
        }
    });
    cx.set_global(SyncGlobal(entity));
}

/// The service, when started (not in a one-shot launch).
pub fn get(cx: &App) -> Option<Entity<LibrarySync>> {
    cx.try_global::<SyncGlobal>().map(|g| g.0.clone())
}

/// A game ended (its post-game work, the Ludusavi backup too, is done):
/// its session goes to the other PCs and its saves to the account. Its
/// screenshots follow the compression (or go now when it is off).
pub fn after_game(cx: &mut App) {
    let Some(svc) = get(cx) else { return };
    let compression_on = crate::services::compression::get(cx).is_some_and(|c| c.read(cx).enabled());
    svc.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AFTER_GAME).await;
            this.update(cx, |s, cx| {
                s.sync(false, cx);
                s.upload(Job::Saves, false, cx);
                if !compression_on {
                    s.upload(Job::Captures, false, cx);
                }
            })
            .ok();
        })
        .detach();
    });
}

/// A game started: uploads stop until it ends, so they do not take its
/// bandwidth.
pub fn game_started(cx: &mut App) {
    if let Some(svc) = get(cx) {
        svc.update(cx, |s, cx| s.pause_for_game(cx));
    }
}

/// The app is closing: the upload running and a sign-in waiting stop.
pub fn shutdown(cx: &mut App) {
    if let Some(svc) = get(cx) {
        svc.update(cx, |s, cx| {
            s.stop_for_quit();
            s.stop_bringing(cx);
            if let Some(c) = &s.connecting {
                c.cancel.request();
            }
        });
    }
}

/// A backup of the data folder was made at `path`.
pub fn backup_made(path: std::path::PathBuf, cx: &mut App) {
    if let Some(svc) = get(cx) {
        svc.update(cx, |s, cx| s.upload(Job::Backup(path), false, cx));
    }
}

impl LibrarySync {
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn connecting(&self) -> Option<&Connecting> {
        self.connecting.as_ref()
    }

    pub fn connect_error(&self) -> Option<&str> {
        self.connect_error.as_deref()
    }

    /// Moves each time `sync.json` may have changed: Settings → Cloud reads
    /// it again then.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    pub(super) fn changed(&mut self) {
        self.changes = self.changes.wrapping_add(1);
    }

    /// Syncs now ("Sync now": `manual`, the timer, after a game). Does
    /// nothing while one runs, when the sync is off, or when it is manual and
    /// this is not.
    pub fn sync(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.running = true;
        cx.notify();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_sync::run(&data, &app, manual) }).await;
            this.update(cx, |s, cx| {
                s.running = false;
                s.changed();
                // How it went is kept in `sync.json` for the settings page.
                match result {
                    Ok(report) if report.summary.changed() => cx.emit(SyncEvent::Changed),
                    Ok(_) | Err(SyncError::Off) => {}
                    Err(e) => log::warn!("sync: {}", e.localized()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Connects `provider` (a browser sign-in, or `server`'s login) and syncs
    /// once it works.
    pub fn connect(&mut self, provider: Provider, server: Server, cx: &mut Context<Self>) {
        if self.connecting.is_some() {
            return;
        }
        let cancel = Cancel::new();
        self.connecting = Some(Connecting { provider, link: None, cancel: cancel.clone() });
        self.connect_error = None;
        cx.notify();
        enum Msg {
            Link(String),
            Done(Result<(), SyncError>),
        }
        let (tx, mut rx) = unbounded::<Msg>();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        // The sign-in can wait minutes for the browser: a thread of its own.
        std::thread::spawn(move || {
            let link_tx = tx.clone();
            let mut on_link = |url: &str| {
                link_tx.unbounded_send(Msg::Link(url.to_owned())).ok();
            };
            let result = heronite_sync::connect(&data, &app, provider, &server, &cancel, &mut on_link);
            tx.unbounded_send(Msg::Done(result)).ok();
        });
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let done = matches!(msg, Msg::Done(_));
                this.update(cx, |s, cx| {
                    match msg {
                        Msg::Link(url) => {
                            if let Some(c) = &mut s.connecting {
                                c.link = Some(url);
                            }
                        }
                        Msg::Done(result) => {
                            s.connecting = None;
                            s.changed();
                            s.forget_cloud_backups();
                            match result {
                                // What is automatic starts; the rest waits for "Send now".
                                Ok(()) => {
                                    s.sync(false, cx);
                                    s.upload_all(false, cx);
                                }
                                Err(SyncError::Cancelled) => {}
                                Err(e) => s.connect_error = Some(e.localized()),
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.connecting {
            c.cancel.request();
        }
        cx.notify();
    }

    /// Signs this PC out of the connected account.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_sync::disconnect(&data, &app) }).await;
            this.update(cx, |s, cx| {
                s.changed();
                s.forget_cloud_backups();
                if let Err(e) = result {
                    s.connect_error = Some(e.localized());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
