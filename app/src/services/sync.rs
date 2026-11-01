//! The library sync between PCs (`heronite_sync`, Settings → Sync): a sync
//! [`AFTER_START`] after the app starts, [`AFTER_GAME`] after each game and
//! every [`EVERY`] (never while a game launched from here runs), plus "Sync
//! now". Connecting a cloud account runs here too, so the page can close
//! while the browser sign-in waits.
//!
//! A sync that changed the Collection sends [`SyncEvent::Changed`]: the
//! Collection reads it again.

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
            loop {
                let alive = this
                    .update(cx, |s: &mut LibrarySync, cx: &mut Context<LibrarySync>| {
                        if crate::system::restart_blocker(cx).is_none() {
                            s.sync(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(EVERY).await;
            }
        })
        .detach();
        LibrarySync { data_paths, app_paths, running: false, connecting: None, connect_error: None }
    });
    cx.set_global(SyncGlobal(entity));
}

/// The service, when started (not in a one-shot launch).
pub fn get(cx: &App) -> Option<Entity<LibrarySync>> {
    cx.try_global::<SyncGlobal>().map(|g| g.0.clone())
}

/// A game ended: its session goes to the other PCs.
pub fn after_game(cx: &mut App) {
    let Some(svc) = get(cx) else { return };
    svc.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AFTER_GAME).await;
            this.update(cx, |s, cx| s.sync(cx)).ok();
        })
        .detach();
    });
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

    /// Syncs now ("Sync now", the timer, after a game). Does nothing while
    /// one runs or when the sync is off.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.running = true;
        cx.notify();
        let (data, app) = (self.data_paths.clone(), self.app_paths.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_sync::run(&data, &app) }).await;
            this.update(cx, |s, cx| {
                s.running = false;
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
                            match result {
                                Ok(()) => s.sync(cx),
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
