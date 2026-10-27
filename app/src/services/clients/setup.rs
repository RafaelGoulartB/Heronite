//! Setting a client up (or using one that is installed), and forgetting it.
//!
//! A setup needs a Wine: when this PC has none, the newest GE-Proton comes
//! first (`services::proton_setup`, which the Wine Manager and the install
//! dialogs share) and the setup goes on once it is there.

use std::path::PathBuf;
use std::time::Instant;

use futures::StreamExt;
use gpui_kit::*;
use heronite_clients::collection;
use heronite_clients::setup::Step;
use heronite_clients::{Client, Status};
use heronite_data::proc::Cancel;
use heronite_i18n::tr;

use super::{home, Activity, Clients, ClientsEvent, Notice, Progress};
use crate::services::proton_setup::{self, Setup as WineSetup};

impl Clients {
    /// Sets the client up in a prefix of its own.
    pub fn install(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            return;
        }
        let st = self.state_mut(client);
        st.activity = Activity::SettingUp(Progress::Wine(WineSetup::Idle));
        st.notice = None;
        st.cancel = Some(Cancel::new());
        cx.notify();
        self.check_wine(client, false, cx);
    }

    /// Stops a setup: at once while it waits for the Proton, else when the
    /// installer lets go.
    pub fn cancel_install(&mut self, client: Client, cx: &mut Context<Self>) {
        let Some(cancel) = self.state(client).cancel.clone() else { return };
        cancel.request();
        if self.waiting_wine.remove(&client) {
            self.setup_stopped(client, cx);
        }
        cx.notify();
    }

    fn setting_up(&self, client: Client) -> bool {
        matches!(self.state(client).activity, Activity::SettingUp(_))
    }

    fn cancel_requested(&self, client: Client) -> bool {
        self.state(client).cancel.as_ref().is_some_and(|c| c.is_requested())
    }

    fn check_wine(&mut self, client: Client, retried: bool, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let has = cx.background_spawn(async move { heronite_clients::setup::has_wine(&paths, &home()) }).await;
            this.update(cx, |this, cx| this.wine_checked(client, has, retried, cx)).ok();
        })
        .detach();
    }

    /// `retried`: the Proton download was waited for already.
    fn wine_checked(&mut self, client: Client, has: bool, retried: bool, cx: &mut Context<Self>) {
        if !self.setting_up(client) {
            return;
        }
        if self.cancel_requested(client) {
            return self.setup_stopped(client, cx);
        }
        match (has, retried) {
            (true, _) => self.run_setup(client, cx),
            (false, false) => self.wait_for_wine(client, cx),
            (false, true) => self.setup_failed(
                client,
                tr!(
                    "No Wine or Proton to run {client} with. Install one in Wine Manager (the menu) and try again.",
                    client = client.name()
                ),
                cx,
            ),
        }
    }

    fn wait_for_wine(&mut self, client: Client, cx: &mut Context<Self>) {
        self.waiting_wine.insert(client);
        if self.wine_sub.is_none() {
            let svc = proton_setup::get(cx);
            self.wine_sub = Some(cx.observe(&svc, |this, _, cx| this.wine_changed(cx)));
        }
        proton_setup::ensure(cx);
        // `ensure` leaves the state as it was when there is nothing to do.
        self.wine_changed(cx);
    }

    fn wine_changed(&mut self, cx: &mut Context<Self>) {
        if self.waiting_wine.is_empty() {
            return;
        }
        let wine = proton_setup::state(cx);
        if wine.busy() {
            for client in self.waiting_wine.clone() {
                self.state_mut(client).activity = Activity::SettingUp(Progress::Wine(wine.clone()));
            }
            cx.notify();
            return;
        }
        for client in std::mem::take(&mut self.waiting_wine) {
            match &wine {
                WineSetup::Failed(why) => self.setup_failed(
                    client,
                    tr!("Could not download a Proton to run {client} with: {why}", client = client.name(), why = why),
                    cx,
                ),
                _ => self.check_wine(client, true, cx),
            }
        }
    }

    fn run_setup(&mut self, client: Client, cx: &mut Context<Self>) {
        let Some(cancel) = self.state(client).cancel.clone() else { return };
        self.state_mut(client).activity = Activity::SettingUp(Progress::Step(Step::Downloading(None)));
        let paths = self.paths.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Step>();
        let (done_tx, done_rx) = futures::channel::oneshot::channel();
        // A thread of its own, like a store's install: the setup blocks for
        // minutes (download, prefix, installer), which would hold one of the
        // few threads every background task of the app shares.
        let spawned = std::thread::Builder::new().name("client-setup".into()).spawn(move || {
            let mut on_step = move |step| {
                let _ = tx.unbounded_send(step);
            };
            let res = heronite_clients::setup::install(&paths, client, &home(), &cancel, &mut on_step).map_err(|e| format!("{e:#}"));
            let _ = done_tx.send(res);
        });
        if let Err(e) = spawned {
            return self.setup_failed(client, tr!("Could not start the setup of {client}: {error}", client = client.name(), error = e), cx);
        }
        cx.spawn(async move |this, cx| {
            while let Some(step) = rx.next().await {
                let shown = this.update(cx, |this, cx| {
                    if this.setting_up(client) {
                        this.state_mut(client).activity = Activity::SettingUp(Progress::Step(step));
                        cx.notify();
                    }
                });
                if shown.is_err() {
                    return;
                }
            }
            // Dropped without an answer only when the setup panicked.
            let res = done_rx.await.unwrap_or_else(|_| Err(tr!("The setup of {client} stopped unexpectedly", client = client.name())));
            this.update(cx, |this, cx| this.setup_done(client, res, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn setup_done(&mut self, client: Client, res: Result<(), String>, cx: &mut Context<Self>) {
        let cancelled = self.cancel_requested(client);
        match res {
            Ok(()) => {
                let st = self.state_mut(client);
                st.activity = Activity::Idle;
                st.cancel = None;
                // The installer often opens the client by itself: followed
                // until it closes, then its games are looked at.
                st.watched = true;
                st.opened_at = Some(Instant::now());
                self.refresh(&[client], true, cx);
            }
            Err(_) if cancelled => self.setup_stopped(client, cx),
            Err(e) => self.setup_failed(client, e, cx),
        }
        cx.notify();
    }

    fn setup_stopped(&mut self, client: Client, cx: &mut Context<Self>) {
        self.end_setup(client, Notice::info(tr!("Setup cancelled.")), cx);
    }

    fn setup_failed(&mut self, client: Client, error: String, cx: &mut Context<Self>) {
        log::warn!("setting up {}: {error}", client.name());
        self.end_setup(client, Notice::error(error), cx);
    }

    fn end_setup(&mut self, client: Client, notice: Notice, cx: &mut Context<Self>) {
        let st = self.state_mut(client);
        st.activity = Activity::Idle;
        st.cancel = None;
        st.notice = Some(notice);
        // A setup cut short leaves its settings file: look at what is there.
        self.refresh(&[client], false, cx);
        cx.notify();
    }

    /// Uses the client already installed in `prefix`.
    pub fn adopt(&mut self, client: Client, prefix: PathBuf, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            return;
        }
        let st = self.state_mut(client);
        st.activity = Activity::Adopting;
        st.notice = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(
                    async move { heronite_clients::setup::adopt(&paths, client, &home(), &prefix).map_err(|e| format!("{e:#}")) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.state_mut(client).activity = Activity::Idle;
                match res {
                    // Its games come in at once.
                    Ok(()) => this.refresh(&[client], true, cx),
                    Err(e) => this.state_mut(client).notice = Some(Notice::error(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Forgets the client: its prefix and games stay on disk, its entries
    /// leave the Collection.
    pub fn forget(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            return;
        }
        let st = self.state_mut(client);
        st.activity = Activity::Forgetting;
        st.notice = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    heronite_clients::setup::forget(&paths, client).map_err(|e| format!("{e:#}"))?;
                    collection::remove(&paths, client).map_err(|e| format!("{e:#}"))
                })
                .await;
            this.update(cx, |this, cx| {
                let st = this.state_mut(client);
                st.activity = Activity::Idle;
                match res {
                    Ok(removed) => {
                        st.status = Some(Status::default());
                        st.running = false;
                        st.game_running = false;
                        st.watched = false;
                        st.report = None;
                        st.last = None;
                        st.in_collection = 0;
                        st.removed.clear();
                        cx.emit(ClientsEvent::Removed(removed));
                        this.refresh(&[client], false, cx);
                    }
                    Err(e) => st.notice = Some(Notice::error(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
