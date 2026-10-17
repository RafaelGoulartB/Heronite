//! GOG and Amazon Games: accounts, library refreshes and update checks, and
//! the one-off operations on their games (verify, uninstall, move, import).
//! Installs and updates run in the shared download queue (`queue.rs`), which
//! hands their elements to gogdl / nile.

use gpui_kit::*;
use heronite_amazon::cmd::Nile;
use heronite_data::DataPaths;
use heronite_epic::cmd::Cancel;
use heronite_epic::ops::{self as epic_ops, OpResult};
use heronite_gog::cmd::Gogdl;
use heronite_i18n::{tr, trn};
use serde_json::Value;

use super::{Account, Downloads, DownloadsEvent, Store, StoreState};

/// A store's files as read in the background.
struct Local {
    account: Option<Account>,
    owned: usize,
    installed: usize,
    updates: Vec<String>,
}

fn read_local(paths: &DataPaths, store: Store) -> Local {
    match store {
        Store::Gog => {
            let stats = heronite_gog::library::stats(paths);
            Local {
                account: heronite_gog::user::account(paths)
                    .filter(|_| heronite_gog::user::is_logged_in(paths))
                    .map(|a| Account { logged_in: true, name: a.username }),
                owned: stats.owned,
                installed: stats.installed,
                updates: heronite_gog::updates::cached_updates(paths),
            }
        }
        Store::Amazon => {
            let stats = heronite_amazon::library::stats(paths);
            Local {
                account: heronite_amazon::user::account(paths)
                    .filter(|_| heronite_amazon::user::is_logged_in(paths))
                    .map(|a| Account { logged_in: true, name: a.name }),
                owned: stats.owned,
                installed: stats.installed,
                updates: heronite_amazon::updates::cached_updates(paths),
            }
        }
    }
}

/// Library refresh + update check, over the network.
fn refresh_blocking(paths: &DataPaths, store: Store) -> anyhow::Result<Local> {
    match store {
        Store::Gog => {
            let gogdl = Gogdl::locate_or_install(paths)?;
            heronite_gog::library::refresh(paths, &gogdl)?;
            if let Err(err) = heronite_gog::updates::check(paths, &gogdl) {
                log::warn!("gog: checking for updates: {err:#}");
            }
        }
        Store::Amazon => {
            let nile = Nile::locate_or_install(paths)?;
            heronite_amazon::library::refresh(paths, &nile)?;
            if let Err(err) = heronite_amazon::updates::check(paths, &nile) {
                log::warn!("amazon: checking for updates: {err:#}");
            }
        }
    }
    Ok(read_local(paths, store))
}

pub(super) fn failed(err: anyhow::Error) -> OpResult {
    OpResult { ok: false, aborted: false, message: format!("{err:#}") }
}

pub(super) fn done(message: &str) -> OpResult {
    OpResult { ok: true, aborted: false, message: message.to_string() }
}

/// A finished operation that may have been cancelled.
fn ended(result: anyhow::Result<()>, cancel: &Cancel, message: &str) -> OpResult {
    match result {
        _ if cancel.is_requested() => OpResult { ok: false, aborted: true, message: tr!("Cancelled").into() },
        Ok(()) => done(message),
        Err(err) => failed(err),
    }
}

/// The library entry of a game of any store (Epic, GOG, Amazon).
pub fn any_library_entry(paths: &DataPaths, app_name: &str) -> Option<Value> {
    super::library_entry(paths, app_name)
        .or_else(|| heronite_gog::library::game_info(paths, app_name))
        .or_else(|| heronite_amazon::library::game_info(paths, app_name))
}

impl Downloads {
    pub fn store(&self, store: Store) -> &StoreState {
        match store {
            Store::Gog => &self.gog,
            Store::Amazon => &self.amazon,
        }
    }

    fn store_mut(&mut self, store: Store) -> &mut StoreState {
        match store {
            Store::Gog => &mut self.gog,
            Store::Amazon => &mut self.amazon,
        }
    }

    fn apply_local(&mut self, store: Store, local: Local) {
        let st = self.store_mut(store);
        st.account = local.account.unwrap_or_default();
        st.owned = local.owned;
        st.installed = local.installed;
        st.updates = local.updates.into_iter().collect();
    }

    /// Account, counts and known updates of `store` from its files.
    pub(super) fn store_reload_local(&mut self, store: Store, cx: &mut Context<Self>) {
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let local = cx.background_spawn(async move { read_local(&paths, store) }).await;
            this.update(cx, |this, cx| {
                this.apply_local(store, local);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Amazon's sign-in starts with nile (downloaded first when missing):
    /// `then` gets the page to open, `None` when nile is already logged in.
    pub fn amazon_begin_login(
        &mut self,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Result<Option<String>, String>, &mut Context<Self>) + 'static,
    ) {
        let paths = self.data_paths.clone();
        self.amazon.last_error = None;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let nile = Nile::locate_or_install(&paths)?;
                    heronite_amazon::user::start_login(&nile)
                })
                .await;
            this.update(cx, |this, cx| {
                let res = match res {
                    Ok(Some(request)) => {
                        let url = request.url.clone();
                        this.amazon_login = Some(request);
                        Ok(Some(url))
                    }
                    Ok(None) => {
                        this.store_reload_local(Store::Amazon, cx);
                        Ok(None)
                    }
                    Err(err) => Err(format!("{err:#}")),
                };
                then(this, res, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Logs in with the code the login window returned or the user pasted
    /// (Amazon: after [`amazon_begin_login`](Self::amazon_begin_login)), then
    /// refreshes the library.
    pub fn store_login(&mut self, store: Store, input: String, cx: &mut Context<Self>) {
        if self.store(store).logging_in {
            return;
        }
        let request = match store {
            Store::Gog => None,
            Store::Amazon => match self.amazon_login.clone() {
                Some(r) => Some(r),
                None => {
                    self.toast(cx, tr!("Open the Amazon sign-in page first"), true);
                    return;
                }
            },
        };
        let st = self.store_mut(store);
        st.logging_in = true;
        st.last_error = None;
        cx.notify();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let name = match (store, request) {
                        (Store::Amazon, Some(request)) => {
                            let nile = Nile::locate_or_install(&paths)?;
                            heronite_amazon::user::register(&paths, &nile, &request, &input)?.name
                        }
                        _ => {
                            let gogdl = Gogdl::locate_or_install(&paths)?;
                            heronite_gog::user::login(&paths, &gogdl, &input)?.username
                        }
                    };
                    // The library right away, like after an Epic login.
                    let local = match refresh_blocking(&paths, store) {
                        Ok(local) => local,
                        Err(err) => {
                            log::warn!("{}: refreshing the library after the login: {err:#}", store.runner());
                            read_local(&paths, store)
                        }
                    };
                    anyhow::Ok((name, local))
                })
                .await;
            this.update(cx, |this, cx| {
                this.store_mut(store).logging_in = false;
                match res {
                    Ok((name, local)) => {
                        if store == Store::Amazon {
                            this.amazon_login = None;
                        }
                        this.apply_local(store, local);
                        this.toast(cx, tr!("Logged in as {name}", name), false);
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                    }
                    Err(err) => {
                        let msg = format!("{err:#}");
                        this.store_mut(store).last_error = Some(msg.clone());
                        this.toast(cx, msg, true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn store_logout(&mut self, store: Store, cx: &mut Context<Self>) {
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match store {
                        Store::Gog => heronite_gog::user::logout(&paths),
                        Store::Amazon => {
                            let nile = Nile::locate(&paths)?;
                            heronite_amazon::user::logout(&paths, &nile)
                        }
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(()) => {
                        *this.store_mut(store) = StoreState::default();
                        this.toast(cx, tr!("Logged out of {store}", store = store.name()), false);
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                        this.store_reload_local(store, cx);
                    }
                    Err(err) => this.toast(cx, format!("{err:#}"), true),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Library refresh and update check. `quiet`: a background check (no
    /// toast unless it fails); `auto_update`: queue the updates it finds when
    /// `autoUpdateGames` allows.
    pub fn store_refresh(&mut self, store: Store, quiet: bool, auto_update: bool, cx: &mut Context<Self>) {
        let st = self.store_mut(store);
        if st.refreshing || !st.account.logged_in {
            return;
        }
        st.refreshing = true;
        st.last_error = None;
        cx.notify();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { refresh_blocking(&paths, store) }).await;
            this.update(cx, |this, cx| {
                this.store_mut(store).refreshing = false;
                match res {
                    Ok(local) => {
                        this.apply_local(store, local);
                        let n = this.store(store).updates.len();
                        let msg = if n > 0 {
                            trn!(
                                n,
                                "{store} library refreshed: {n} update available",
                                "{store} library refreshed: {n} updates available",
                                store = store.name()
                            )
                        } else {
                            tr!("{store} library refreshed", store = store.name()).to_string()
                        };
                        if quiet {
                            log::info!("{}: {msg}", store.runner());
                        } else {
                            this.toast(cx, msg, false);
                        }
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                        if auto_update {
                            this.queue_auto_updates(None, cx);
                        }
                    }
                    Err(err) => {
                        let msg = format!("{err:#}");
                        this.store_mut(store).last_error = Some(msg.clone());
                        let first = msg.lines().next().unwrap_or("").to_string();
                        this.toast(cx, tr!("Could not refresh the {store} library: {error}", store = store.name(), error = first), true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Queues an update for every game of `store` that has one.
    pub fn store_update_all(&mut self, store: Store, cx: &mut Context<Self>) {
        let apps: Vec<String> = self.store(store).updates.iter().cloned().collect();
        self.queue_updates_of(apps, cx);
    }

    /// Queues updates for `apps` (any store) that are not queued yet.
    pub(super) fn queue_updates_of(&mut self, apps: Vec<String>, cx: &mut Context<Self>) {
        let apps = self.updatable(apps, cx);
        if apps.is_empty() {
            return;
        }
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let entries: Vec<Value> =
                cx.background_spawn(async move { apps.iter().filter_map(|a| any_library_entry(&paths, a)).collect() }).await;
            this.update(cx, |this, cx| {
                for info in entries {
                    this.queue_update(info, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Verify and repair the game's files.
    pub fn store_verify(&mut self, store: Store, app_name: &str, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        self.run_task_then(
            app_name,
            tr!("Verifying"),
            true,
            cx,
            move |paths, cancel, report| {
                let mut on_percent = |p: f64| report(Some(p as f32));
                let res = match store {
                    Store::Gog => Gogdl::locate(paths).and_then(|g| heronite_gog::ops::repair(paths, &g, &app, cancel, &mut on_percent)),
                    Store::Amazon => {
                        Nile::locate(paths).and_then(|n| heronite_amazon::ops::verify(paths, &n, &app, cancel, &mut on_percent))
                    }
                };
                ended(res, cancel, tr!("Finished repairing"))
            },
            |_, _, _| {},
        );
    }

    /// Uninstall; optionally removes the Wine prefix and the game settings.
    pub fn store_uninstall(&mut self, store: Store, app_name: &str, remove_prefix: bool, remove_settings: bool, cx: &mut Context<Self>) {
        let app = app_name.to_string();
        self.run_task_then(
            app_name,
            tr!("Uninstalling"),
            false,
            cx,
            move |paths, _, _| {
                let res = match store {
                    Store::Gog => heronite_gog::ops::uninstall(paths, &app),
                    Store::Amazon => Nile::locate(paths).and_then(|n| heronite_amazon::ops::uninstall(paths, &n, &app)),
                };
                if let Err(err) = res {
                    return failed(err);
                }
                // The prefix is looked up before the settings are deleted.
                if remove_prefix {
                    if let Err(err) = epic_ops::remove_prefix(paths, &app) {
                        log::warn!("removing the prefix of {app}: {err:#}");
                    }
                }
                if remove_settings {
                    epic_ops::remove_settings_and_logs(paths, &app);
                }
                done(tr!("Uninstalled"))
            },
            |_, _, _| {},
        );
    }

    /// Moves the install folder into `new_parent`.
    pub fn store_move(&mut self, store: Store, app_name: &str, new_parent: &str, cx: &mut Context<Self>) {
        let (app, to) = (app_name.to_string(), new_parent.to_string());
        self.run_task_then(
            app_name,
            tr!("Moving"),
            true,
            cx,
            move |paths, cancel, report| {
                let mut on_percent = |p: f64| report(Some(p as f32));
                let res = match store {
                    Store::Gog => heronite_gog::ops::move_install(paths, &app, &to, cancel, &mut on_percent),
                    Store::Amazon => heronite_amazon::ops::move_install(paths, &app, &to, cancel, &mut on_percent),
                };
                ended(res.map(|_| ()), cancel, tr!("Game moved"))
            },
            |_, _, _| {},
        );
    }

    /// Adds a copy that is already on disk. GOG finds the game in `folder`
    /// by itself; Amazon needs `app_name`. `key` names the game for the busy
    /// state (the app name, or the folder for GOG).
    pub fn store_import(
        &mut self,
        store: Store,
        app_name: Option<String>,
        folder: &str,
        wine_prefix: Option<String>,
        wine_version: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        let key = app_name.clone().unwrap_or_else(|| folder.to_string());
        let folder = folder.to_string();
        self.run_task_then(
            &key,
            tr!("Importing"),
            false,
            cx,
            move |paths, _, _| {
                let res = match (store, app_name) {
                    (Store::Gog, _) => Gogdl::locate_or_install(paths).and_then(|g| heronite_gog::ops::import(paths, &g, &folder)),
                    (Store::Amazon, Some(app)) => {
                        Nile::locate_or_install(paths).and_then(|n| heronite_amazon::ops::import(paths, &n, &app, &folder)).map(|_| app)
                    }
                    (Store::Amazon, None) => Err(anyhow::anyhow!(tr!("Choose the game to import"))),
                };
                let app = match res {
                    Ok(app) => app,
                    Err(err) => return failed(err),
                };
                if wine_prefix.is_some() || wine_version.is_some() {
                    if let Err(err) = epic_ops::write_wine_config(paths, &app, wine_prefix.as_deref().unwrap_or(""), wine_version.as_ref())
                    {
                        log::warn!("writing the Wine settings of {app}: {err:#}");
                    }
                }
                done(tr!("Game imported"))
            },
            |_, _, _| {},
        );
    }
}

/// Cloud saves of a GOG game around a launch: `flag` is `--skip-upload`
/// (download before the game starts) or `--skip-download` (upload after it
/// exits). Blocking.
pub fn gog_sync_saves_flag(paths: &DataPaths, app_name: &str, flag: &str) -> Result<String, String> {
    let gogdl = Gogdl::locate_or_install(paths).map_err(|e| format!("{e:#}"))?;
    heronite_gog::saves::sync_flag(paths, &gogdl, app_name, flag).map_err(|e| format!("{e:#}"))
}

/// The first launch of a GOG game in a prefix asks gogdl for the game's
/// installers and redistributables (the launch crate cannot depend on the
/// GOG crate).
pub struct StoreLaunchSetup(pub DataPaths);

impl heronite_launch::first_launch::StoreSetup for StoreLaunchSetup {
    fn steps(&self, runner: &str, app_name: &str, install_path: &str) -> Result<Vec<heronite_launch::plan::SetupStep>, String> {
        if runner != "gog" {
            return Ok(Vec::new());
        }
        let gogdl = Gogdl::locate_or_install(&self.0).map_err(|e| format!("{e:#}"))?;
        heronite_gog::setup::first_launch_steps(&self.0, &gogdl, app_name, install_path).map_err(|e| format!("{e:#}"))
    }
}
