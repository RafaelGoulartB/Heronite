//! One-off operations on a game (verify, uninstall, move, import, cloud saves)
//! and the machinery that runs them on a worker thread.

use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::HeroicPaths;
use heronite_epic::cmd::{Cancel, Legendary};
use heronite_epic::ops::{self, OpResult, RepairUpdate};
use heronite_epic::saves;
use serde_json::Value;

use super::{Busy, Downloads, DownloadsEvent};

enum OpMsg {
    Percent(Option<f32>),
    Done(OpResult),
}

fn locate(heroic: &HeroicPaths) -> Result<Legendary, OpResult> {
    Legendary::locate(heroic).map_err(|e| OpResult { ok: false, aborted: false, message: format!("{e:#}") })
}

impl Downloads {
    pub(super) fn run_op(
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
    pub(super) fn run_op_then(
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

    /// Finishes the install move an earlier run was interrupted in (see
    /// `heronite_epic::moves`). `true` when one is being finished: the game shows
    /// as busy and the download queue starts when it is done.
    pub(super) fn resume_interrupted_move(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(journal) = heronite_epic::moves::pending_move(&self.heroic) else { return false };
        self.run_op_then(
            &journal.app_name,
            "Moving",
            false,
            cx,
            |heroic, lg, _, _| {
                heronite_epic::moves::resume_interrupted_move(heroic, lg)
                    .unwrap_or_else(|| OpResult { ok: true, aborted: false, message: "The interrupted move was already finished".into() })
            },
            |this, _, _| {
                if !heronite_epic::autoupdate::heronite_running() {
                    this.manager.start();
                }
            },
        );
        true
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
}
