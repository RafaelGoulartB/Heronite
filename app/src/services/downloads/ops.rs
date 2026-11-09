//! One-off operations on a game (verify, uninstall, move, import, cloud saves)
//! and the machinery that runs them on a worker thread.

use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::DataPaths;
use heronite_epic::cmd::{Cancel, Legendary};
use heronite_epic::ops::{self, OpResult, RepairUpdate};
use heronite_epic::saves;
use heronite_i18n::tr;
use serde_json::Value;

use super::{Busy, Downloads, DownloadsEvent};

/// The busy key of [`Downloads::import_from_launcher`].
pub const LAUNCHER_IMPORT: &str = "epic-games-launcher";

enum OpMsg {
    Percent(Option<f32>),
    Done(OpResult),
}

fn locate(data_paths: &DataPaths) -> Result<Legendary, OpResult> {
    Legendary::locate(data_paths).map_err(|e| OpResult { ok: false, aborted: false, warning: false, message: format!("{e:#}") })
}

impl Downloads {
    pub(super) fn run_op(
        &mut self,
        app_name: &str,
        label: &str,
        cancellable: bool,
        cx: &mut Context<Self>,
        work: impl FnOnce(&DataPaths, &Legendary, &Arc<Cancel>, &dyn Fn(Option<f32>)) -> OpResult + Send + 'static,
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
        work: impl FnOnce(&DataPaths, &Legendary, &Arc<Cancel>, &dyn Fn(Option<f32>)) -> OpResult + Send + 'static,
        then: impl FnOnce(&mut Self, &OpResult, &mut Context<Self>) + 'static,
    ) {
        self.run_task_then(
            app_name,
            label,
            cancellable,
            cx,
            move |data_paths, cancel, report| match locate(data_paths) {
                Err(r) => r,
                Ok(lg) => work(data_paths, &lg, cancel, report),
            },
            then,
        );
    }

    /// Whether the game's files are in use (it is running, or its download is
    /// queued): a verify, move or install-location change must not start; a
    /// toast says why. The menus hide these already, this is the backstop.
    pub(super) fn files_in_use(&mut self, app_name: &str, cx: &mut Context<Self>) -> bool {
        let reason = if self.playing.contains(app_name) {
            tr!("The game is running: close it first")
        } else if self.queued(app_name).is_some() {
            tr!("This game is in the download queue")
        } else {
            return false;
        };
        self.toast(cx, reason, true);
        true
    }

    /// Runs `work` for a game on a worker thread: the game shows as busy
    /// (with the percents `work` reports) until it ends with a toast.
    pub(super) fn run_task_then(
        &mut self,
        app_name: &str,
        label: &str,
        cancellable: bool,
        cx: &mut Context<Self>,
        work: impl FnOnce(&DataPaths, &Arc<Cancel>, &dyn Fn(Option<f32>)) -> OpResult + Send + 'static,
        then: impl FnOnce(&mut Self, &OpResult, &mut Context<Self>) + 'static,
    ) {
        let mut then = Some(then);
        if self.busy.contains_key(app_name) {
            self.toast(cx, tr!("Another operation is running for this game"), true);
            return;
        }
        let cancel = cancellable.then(Cancel::new);
        self.busy.insert(app_name.to_string(), Busy { label: label.to_string().into(), percent: None, cancel: cancel.clone() });
        cx.notify();
        let (tx, mut rx) = unbounded::<OpMsg>();
        let data_paths = self.data_paths.clone();
        let c = cancel.unwrap_or_else(Cancel::new);
        std::thread::Builder::new()
            .name("store-op".into())
            .spawn(move || {
                let progress_tx: UnboundedSender<OpMsg> = tx.clone();
                let result = work(&data_paths, &c, &move |p| {
                    let _ = progress_tx.unbounded_send(OpMsg::Percent(p));
                });
                let _ = tx.unbounded_send(OpMsg::Done(result));
            })
            .expect("spawning a store operation");
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
                            // An auto-update skipped because of this operation goes now.
                            if this.deferred.remove(&app) {
                                this.queue_auto_updates(Some(app.clone()), cx);
                            }
                            if r.aborted {
                                this.toast(cx, tr!("{label} cancelled", label), false);
                            } else if r.ok {
                                // A warning is a finished operation with a problem to read.
                                this.toast(cx, r.message.clone(), r.warning);
                            } else {
                                let first = r.message.lines().next().unwrap_or(tr!("Failed")).to_string();
                                this.toast(cx, tr!("{label} failed: {first}", label, first), true);
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
        if self.files_in_use(app_name, cx) {
            return;
        }
        let app = app_name.to_string();
        self.run_op(app_name, tr!("Verifying"), true, cx, move |data_paths, lg, cancel, report| {
            ops::repair(data_paths, lg, &app, cancel, |u| match u {
                RepairUpdate::Verifying(p) => report(Some(p as f32)),
                RepairUpdate::Downloading(p) => report(Some(p.percent as f32)),
            })
        });
    }

    /// Uninstall; optionally removes the Wine prefix and the game settings.
    pub fn uninstall(&mut self, app_name: &str, remove_prefix: bool, remove_settings: bool, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        self.run_op(app_name, tr!("Uninstalling"), false, cx, move |data_paths, lg, _, _| {
            // The prefix is looked up before the settings are deleted.
            let prefix_to_remove = remove_prefix;
            let mut r = ops::uninstall(data_paths, lg, &app);
            if r.ok {
                let note = prefix_to_remove.then(|| ops::remove_prefix_note(data_paths, &app)).flatten();
                if remove_settings {
                    ops::remove_settings_and_logs(data_paths, &app);
                }
                if let Some(note) = note {
                    r = OpResult::warn(format!("{}. {note}", r.message));
                }
            }
            r
        });
    }

    /// Finishes the install move an earlier run was interrupted in (see
    /// `heronite_epic::moves`). `true` when one is being finished: the game shows
    /// as busy and the download queue starts when it is done.
    pub(super) fn resume_interrupted_move(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(journal) = heronite_epic::moves::pending_move(&self.data_paths) else { return false };
        self.run_op_then(
            &journal.app_name,
            tr!("Moving"),
            false,
            cx,
            |data_paths, lg, _, _| {
                heronite_epic::moves::resume_interrupted_move(data_paths, lg).unwrap_or_else(|| OpResult {
                    ok: true,
                    aborted: false,
                    warning: false,
                    message: tr!("The interrupted move was already finished").into(),
                })
            },
            |this, _, _| this.manager.start(),
        );
        true
    }

    pub fn move_install(&mut self, app_name: &str, install_path: &str, new_parent: &str, cx: &mut Context<Self>) {
        if self.files_in_use(app_name, cx) {
            return;
        }
        let (app, from, to) = (app_name.to_string(), install_path.to_string(), new_parent.to_string());
        self.run_op(app_name, tr!("Moving"), false, cx, move |data_paths, lg, _, _| {
            let r = ops::move_install(data_paths, lg, &app, &from, &to);
            if r.ok {
                OpResult { ok: true, aborted: false, warning: false, message: tr!("Game moved").into() }
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
        self.run_op(app_name, tr!("Importing"), false, cx, move |data_paths, lg, _, _| {
            let r = ops::import_game(data_paths, lg, &app, &folder, &platform);
            if r.ok && (wine_prefix.is_some() || wine_version.is_some()) {
                if let Err(err) = ops::write_wine_config(data_paths, &app, wine_prefix.as_deref().unwrap_or(""), wine_version.as_ref()) {
                    log::warn!("writing the Wine settings of {app}: {err:#}");
                }
            }
            r
        });
    }

    /// Windows: takes in the games the Epic Games Launcher installed on this
    /// PC, without downloading them (see `heronite_epic::egl`).
    pub fn import_from_launcher(&mut self, cx: &mut Context<Self>) {
        self.run_op(LAUNCHER_IMPORT, tr!("Importing from the Epic Games Launcher"), false, cx, |data_paths, lg, _, _| {
            heronite_epic::egl::import(data_paths, lg)
        });
    }

    /// Cloud saves: syncs with the folder in the game settings (detecting it
    /// when empty) and shows legendary's output.
    pub fn sync_saves(&mut self, app_name: &str, title: &str, cx: &mut Context<Self>) {
        let (app, title_owned) = (app_name.to_string(), title.to_string());
        let (tx, mut rx) = unbounded::<Result<String, String>>();
        self.run_op(app_name, tr!("Syncing saves"), false, cx, move |data_paths, _, _, _| {
            let configured = game_config::load_game(data_paths, &app, &ops::home())
                .get("savesPath")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let path = if configured.is_empty() {
                match saves::detect_path(data_paths, &app) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return OpResult {
                            ok: false,
                            aborted: false,
                            warning: false,
                            message: tr!("The save folder could not be detected; set it in the game settings").into(),
                        }
                    }
                    Err(err) => return OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
                }
            } else {
                configured
            };
            match saves::sync(data_paths, &app, &path, saves::SyncMode::Both) {
                Ok(text) => {
                    let _ = tx.unbounded_send(Ok(text));
                    OpResult { ok: true, aborted: false, warning: false, message: tr!("Cloud saves synced").into() }
                }
                Err(err) => OpResult { ok: false, aborted: false, warning: false, message: format!("{err:#}") },
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some(Ok(text)) = rx.next().await {
                this.update(cx, |this, cx| {
                    this.emit(cx, DownloadsEvent::Message { title: tr!("Cloud saves: {title}", title = title_owned), text });
                })
                .ok();
            }
        })
        .detach();
    }
}
