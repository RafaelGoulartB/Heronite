//! Opening and closing a client, and following the process that opened it.

use std::path::Path;
use std::time::Instant;

use gpui_kit::*;
use heronite_clients::Client;
use heronite_i18n::tr;

use super::{home, Activity, Clients, Notice};

impl Clients {
    /// Opens the client's window (starting it when it does not run).
    pub fn open(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
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
                        let st = this.state_mut(client);
                        st.activity = Activity::Idle;
                        st.watched = false;
                        st.opened_at = None;
                        st.notice = Some(Notice::error(tr!("Could not open {client}: {error}", client = client.name(), error = e)));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Closes the client and what runs in its prefix, unless one of its
    /// games runs.
    pub fn close(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.frozen == Some(client) || self.state(client).busy() {
            return;
        }
        let Some(prefix) = self.state(client).prefix().map(Path::to_path_buf) else { return };
        let st = self.state_mut(client);
        st.activity = Activity::Closing;
        st.notice = None;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if heronite_clients::run::game_running(client, &prefix) {
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
