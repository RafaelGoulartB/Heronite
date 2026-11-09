//! Opening and closing a client, and following the process that opened it.

use std::time::Instant;

use gpui_kit::*;
use heronite_clients::Client;
use heronite_i18n::tr;

use super::{home, Activity, Clients, Notice};

impl Clients {
    /// Opens the client's window (starting it when it does not run).
    pub fn open(&mut self, client: Client, cx: &mut Context<Self>) {
        self.open_with(client, false, cx);
    }

    /// [`Self::open`] asked by the client's shortcut: Heronite's window may
    /// not be on screen, so a failure is also a desktop notification.
    pub fn open_from_shortcut(&mut self, client: Client, cx: &mut Context<Self>) {
        self.open_with(client, true, cx);
    }

    fn open_with(&mut self, client: Client, notify: bool, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            log::info!("{}: not opened, it is busy", client.name());
            // Opening already: a second click of the shortcut. Anything else
            // would leave the click unanswered, the window may be hidden.
            if notify && self.state(client).activity != Activity::Opening {
                let text = tr!("{client} is busy right now. Try again in a moment.", client = client.name());
                crate::system::notify::send("Heronite", text);
            }
            return;
        }
        let st = self.state_mut(client);
        st.activity = Activity::Opening;
        st.notice = None;
        // Followed until it closes: its games may have changed by then.
        st.watched = true;
        st.opened_at = Some(Instant::now());
        self.ensure_polling(cx);
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_clients::run::open(&paths, client, &home()) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(child) => this.reap(client, child, cx),
                    Err(e) => {
                        let text = tr!("Could not open {client}: {error}", client = client.name(), error = e);
                        if notify {
                            crate::system::notify::send("Heronite", text.clone());
                        }
                        let st = this.state_mut(client);
                        st.activity = Activity::Idle;
                        st.watched = false;
                        st.opened_at = None;
                        st.notice = Some(Notice::error(text));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Closes the client and what runs in its prefix (on Windows, its
    /// programs), unless one of its games runs.
    pub fn close(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            return;
        }
        let Some(status) = self.state(client).status.clone().filter(|s| s.installed()) else { return };
        let st = self.state_mut(client);
        st.activity = Activity::Closing;
        st.notice = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if heronite_clients::run::game_running(client, &status) {
                        return Err(tr!("{client} stays open while one of its games runs. Close the game first.", client = client.name()));
                    }
                    heronite_clients::run::close(&paths, client, &home())
                        .map_err(|e| tr!("Could not close {client}: {error}", client = client.name(), error = e))
                })
                .await;
            this.update(cx, |this, cx| {
                let st = this.state_mut(client);
                st.activity = Activity::Idle;
                match res {
                    Ok(()) => {
                        st.running = false;
                        st.game_running = false;
                        st.watched = false;
                        this.refresh(&[client], true, cx);
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

    /// Waits for the process that opened the client on a thread of its own
    /// (it lives as long as the client when it started it, and ends at once
    /// when it only handed the request to a client that was open), then
    /// looks again.
    fn reap(&mut self, client: Client, mut child: std::process::Child, cx: &mut Context<Self>) {
        let (tx, rx) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new().name("client-reaper".into()).spawn(move || {
            let _ = child.wait();
            let _ = tx.send(());
        });
        if let Err(e) = spawned {
            log::warn!("waiting for {}: {e}", client.name());
        }
        cx.spawn(async move |this, cx| {
            let _ = rx.await;
            this.update(cx, |this, cx| this.refresh(&[client], true, cx)).ok();
        })
        .detach();
    }
}
