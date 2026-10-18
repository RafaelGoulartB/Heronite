//! Library refresh, update detection and `autoUpdateGames`.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::*;
use heronite_data::game_config;
use heronite_data::DataPaths;
use heronite_epic::autoupdate;
use heronite_epic::cmd::Legendary;
use heronite_epic::library;
use heronite_epic::ops;
use heronite_i18n::{tr, trn};
use serde_json::Value;

use super::library_file::{library_entry, read_stats};
use super::stores::any_library_entry;
use super::{Downloads, DownloadsEvent, Store};

/// How often the background check for game updates repeats while the app
/// lives (Heroic only checks at startup and on refresh; a tray-resident app
/// would otherwise never notice an update).
const PERIODIC_CHECK: Duration = Duration::from_secs(3 * 3600);

/// [`PERIODIC_CHECK`], or `HERONITE_DEBUG_UPDATE_INTERVAL_SECS` (tests).
fn periodic_interval() -> Duration {
    crate::env::parse::<u64>("HERONITE_DEBUG_UPDATE_INTERVAL_SECS").map_or(PERIODIC_CHECK, Duration::from_secs)
}

/// `(checkForUpdatesOnStartup, autoUpdateGames)` from the global config.
fn update_settings(data_paths: &DataPaths) -> (bool, bool) {
    let global = game_config::load_global(data_paths, &ops::home());
    let flag = |key: &str, default: bool| global.get(key).and_then(Value::as_bool).unwrap_or(default);
    (flag("checkForUpdatesOnStartup", true), flag("autoUpdateGames", false))
}

impl Downloads {
    /// Heroic starts the queue 5 s after launch; the update check (network) a
    /// little later, then periodically while the app lives.
    pub(super) fn spawn_idle_jobs(&mut self, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        // An install move cut short by a quit / kill is finished first; the
        // queue starts after it (two legendary runs would both write installed.json).
        let move_first = self.resume_interrupted_move(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            if !move_first && this.update(cx, |this, cx| this.start_queue_when_online(cx)).is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_secs(15)).await;
            let mut first = true;
            loop {
                let h = data_paths.clone();
                let (on_startup, auto) = cx.background_spawn(async move { update_settings(&h) }).await;
                // The startup check is what `checkForUpdatesOnStartup` asks
                // for (Heroic checks game updates then regardless); the
                // periodic ones only matter for unattended auto-updates.
                if ((first && on_startup) || auto) && this.update(cx, |this, cx| this.check_for_updates(true, cx)).is_err() {
                    break;
                }
                first = false;
                cx.background_executor().timer(periodic_interval()).await;
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
        let data_paths = self.data_paths.clone();
        let quiet = std::mem::take(&mut self.quiet_refresh);
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let lg = Legendary::locate(&data_paths)?;
                    let r = library::refresh(&data_paths, &lg)?;
                    let signed_in = heronite_epic::user::is_logged_in(&data_paths);
                    anyhow::Ok((r, signed_in, library::updates_available(&data_paths), read_stats(&data_paths)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match res {
                    Ok((r, signed_in, updates, stats)) => {
                        this.updates = updates.into_iter().collect();
                        this.stats = stats;
                        // legendary refused the login (and deleted it).
                        let expired = this.account.logged_in
                            && !this.logging_out
                            && (!signed_in || r.list_error.as_deref().is_some_and(heronite_epic::session::legendary_expired));
                        match r.list_error {
                            _ if expired => this.mark_expired(None, cx),
                            Some(e) => {
                                this.last_error = Some(e.clone());
                                this.toast(
                                    cx,
                                    tr!("Could not refresh the Epic library: {error}", error = e.lines().next().unwrap_or("")),
                                    true,
                                );
                            }
                            None => {
                                let n = this.updates.len();
                                let msg = if n > 0 {
                                    trn!(n, "Epic library refreshed: {n} update available", "Epic library refreshed: {n} updates available")
                                } else {
                                    tr!("Epic library refreshed").to_string()
                                };
                                if quiet {
                                    log::info!("epic: {msg}");
                                } else {
                                    this.toast(cx, msg, false);
                                }
                            }
                        }
                        this.emit(cx, DownloadsEvent::LibraryChanged);
                        if std::mem::take(&mut this.auto_update_after_refresh) {
                            this.queue_auto_updates(None, cx);
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

    /// Queues an update for every installed game of every store that has one.
    pub fn update_all(&mut self, cx: &mut Context<Self>) {
        self.update_all_epic(cx);
        for store in Store::ALL {
            self.store_update_all(store, cx);
        }
    }

    /// Queues an update for every installed Epic game that has one.
    pub fn update_all_epic(&mut self, cx: &mut Context<Self>) {
        let apps: Vec<String> = self.updates.iter().cloned().collect();
        for app in self.updatable(apps, cx) {
            if let Some(info) = library_entry(&self.data_paths, &app) {
                self.queue_update(info, cx);
            }
        }
    }

    /// The background update check (startup and periodic): skipped offline,
    /// otherwise refreshes the libraries of the stores logged in and, with
    /// `autoUpdateGames`, queues the updates found (see [`autoupdate`]).
    pub fn check_for_updates(&mut self, auto: bool, cx: &mut Context<Self>) {
        let epic = !self.refreshing && self.account.logged_in;
        let stores: Vec<Store> =
            Store::ALL.into_iter().filter(|s| self.store(*s).account.logged_in && !self.store(*s).refreshing).collect();
        if !epic && stores.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let online = cx.background_spawn(async { heronite_launch::net::Net::is_online(&heronite_launch::net::SystemNet) }).await;
            if !online {
                log::info!("offline, not checking for game updates");
                return;
            }
            this.update(cx, |this, cx| {
                if epic {
                    this.quiet_refresh = true;
                    this.auto_update_after_refresh = auto;
                    this.refresh_library(cx);
                }
                for store in stores {
                    this.store_refresh(store, true, auto, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Of `apps`, those an update can be queued for now. A game that is
    /// running or has an operation in progress is left out (one toast counts
    /// them); one already in the queue is left out silently.
    pub(super) fn updatable(&mut self, apps: Vec<String>, cx: &mut Context<Self>) -> Vec<String> {
        let (free, held): (Vec<String>, Vec<String>) =
            apps.into_iter().filter(|a| self.queued(a).is_none()).partition(|a| !self.busy.contains_key(a) && !self.playing.contains(a));
        if !held.is_empty() {
            let n = held.len();
            self.toast(
                cx,
                trn!(n, "{n} game is running or busy and was not updated", "{n} games are running or busy and were not updated"),
                true,
            );
        }
        free
    }

    /// The hub tells which games are being played: such a game is not
    /// updated; its update is queued once it exits.
    pub fn game_running(&mut self, app_name: &str, running: bool, cx: &mut Context<Self>) {
        if running {
            self.playing.insert(app_name.to_string());
            return;
        }
        if self.playing.remove(app_name) && self.deferred.remove(app_name) {
            log::info!("epic: {app_name} exited, queueing its update");
            self.queue_auto_updates(Some(app_name.to_string()), cx);
        }
    }

    /// `autoUpdateGames`: queues the updates found, except for games that
    /// ignore updates (`ignoreGameUpdates` in their settings), games being
    /// played and games with a verify or move running (those wait for their
    /// exit / the end of the operation); not at all offline or on a
    /// metered connection. `only`: just that game.
    pub(super) fn queue_auto_updates(&mut self, only: Option<String>, cx: &mut Context<Self>) {
        let data_paths = self.data_paths.clone();
        let apps: Vec<String> = self
            .updates
            .iter()
            .chain(&self.gog.updates)
            .chain(&self.amazon.updates)
            .filter(|a| only.as_ref().is_none_or(|o| o == *a) && !self.auto_failed.contains(*a))
            .cloned()
            .collect();
        if apps.is_empty() {
            return;
        }
        let queued: HashSet<String> = self.snapshot.queue.iter().map(|e| e.params.app_name.clone()).collect();
        let playing = self.playing.clone();
        let busy: HashSet<String> = self.busy.keys().cloned().collect();
        cx.spawn(async move |this, cx| {
            let (plan, entries) = cx
                .background_spawn(async move {
                    let home = ops::home();
                    let enabled =
                        game_config::load_global(&data_paths, &home).get("autoUpdateGames").and_then(Value::as_bool).unwrap_or(false);
                    let candidates: Vec<autoupdate::Candidate> = apps
                        .iter()
                        .map(|app| autoupdate::Candidate {
                            app_name: app.clone(),
                            ignored: game_config::load_game(&data_paths, app, &home)
                                .get("ignoreGameUpdates")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                            queued: queued.contains(app),
                            playing: playing.contains(app),
                            busy: busy.contains(app),
                        })
                        .collect();
                    let gates = if enabled {
                        autoupdate::Gates {
                            enabled,
                            offline: !heronite_launch::net::Net::is_online(&heronite_launch::net::SystemNet),
                            metered: autoupdate::connection_metered(),
                        }
                    } else {
                        autoupdate::Gates::default()
                    };
                    let plan = autoupdate::plan(&gates, &candidates);
                    // An operation that just ended may have uninstalled the game.
                    let entries: Vec<Value> = plan
                        .queue
                        .iter()
                        .filter_map(|app| any_library_entry(&data_paths, app))
                        .filter(|e| e.get("is_installed").and_then(Value::as_bool) != Some(false))
                        .collect();
                    (plan, entries)
                })
                .await;
            this.update(cx, |this, cx| this.apply_auto_plan(plan, entries, cx)).ok();
        })
        .detach();
    }

    fn apply_auto_plan(&mut self, plan: autoupdate::Plan, entries: Vec<Value>, cx: &mut Context<Self>) {
        match plan.blocked {
            Some(autoupdate::Block::Disabled) | None => {}
            Some(why) => log::info!("epic: not auto-updating: {}", why.describe()),
        }
        for app in &plan.ignored {
            log::info!("epic: skipping the auto-update of {app} (updates ignored)");
        }
        for app in plan.deferred {
            log::info!("epic: {app} is being played: its update waits for the exit");
            self.deferred.insert(app);
        }
        let mut titles = Vec::new();
        for info in entries {
            titles.push(info.get("title").and_then(Value::as_str).unwrap_or(tr!("a game")).to_string());
            log::info!("epic: auto-updating {}", titles.last().map(String::as_str).unwrap_or_default());
            self.queue_update(info, cx);
        }
        if !titles.is_empty() && !crate::system::has_window(cx) {
            crate::system::notify::send("Heronite", tr!("Updating {titles} in the background", titles = titles.join(", ")));
        }
    }
}
