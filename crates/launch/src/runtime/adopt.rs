//! Games that outlive the app. A game runs in its own process group and
//! writes its output straight to its log, so it keeps going when the app
//! quits or crashes. What identifies its processes is written down in
//! `local_library/running.json` once it really runs (and again with every
//! checkpoint); the next run of the app picks the session up
//! ([`Launcher::adopt`]): the game shows as playing again, Stop works, and its
//! playtime goes on from what was saved, without counting it twice.
//!
//! A process is recognised by the boot, its pid and its start time, so a pid
//! reused by another program is never taken for the game. A game that ended
//! while the app was closed keeps the playtime of the last checkpoint (the
//! exact end is unknown) and its entry is dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use heronite_data::{json_store, DataPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::process::{run_script, run_to_completion};
use super::session::{finish, spawn_checkpointer, SessionEnd};
use super::state::{Control, Ledger, Shared};
use super::{LaunchEvent, LaunchRequest, Launcher};
use crate::log_writer::{self, GameLog, GameLogType, LogPaths};
use crate::plan::{CloudSaves, Command, LaunchPlan, PrepSpec, StopSpec};
use crate::prepare::{Notice, Prep};
use crate::procs::{monotonic_ms, signal_tree, Procs, Sig};
use crate::record::{iso, Checkpointer};

/// A session picked up again; what the app needs to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptedGame {
    pub app_name: String,
    pub runner: String,
    pub title: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RunningFile {
    #[serde(default)]
    sessions: BTreeMap<String, RunningEntry>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// One running game, as written to `running.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RunningEntry {
    pub(super) app_name: String,
    pub(super) runner: String,
    pub(super) title: String,
    pub(super) boot_id: String,
    /// The session row's `startedAt`, exactly: the row is found by it.
    pub(super) started_at: String,
    /// [`monotonic_ms`] when the game started: rebuilds the session clock
    /// (suspend not counted, as for a session that never stopped being tracked).
    pub(super) mono_start_ms: u64,
    /// `totalPlayed` before the session (see [`Checkpointer::resume`]).
    #[serde(default)]
    pub(super) base_total: Option<f64>,
    /// A checkpoint already stored part of the session.
    #[serde(default)]
    pub(super) written: bool,
    /// Stop patterns only reach processes started since (see [`Control`]).
    #[serde(default)]
    pub(super) since_ticks: u64,
    pub(super) tracked: Tracked,
}

/// How the game is followed.
// One per running game, read and written now and then: size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(super) enum Tracked {
    /// A process group the app started; also what runs once it is gone.
    #[serde(rename_all = "camelCase")]
    Process {
        pid: u32,
        start_ticks: u64,
        #[serde(default)]
        stop: StopSpec,
        #[serde(default)]
        wait_after_exit: Option<Command>,
        #[serde(default)]
        after_script: Option<Command>,
        #[serde(default)]
        cloud_saves: Option<CloudSaves>,
        #[serde(default)]
        keep_awake: bool,
    },
    /// A Steam game: the pids Steam's logs named, with their start ticks.
    #[serde(rename_all = "camelCase")]
    Steam {
        app_id: String,
        #[serde(default)]
        pids: Vec<(u32, u64)>,
        /// Steam was not running before the launch (see `steam::close_steam`).
        #[serde(default)]
        opened_steam: bool,
    },
}

fn file_path(paths: &DataPaths) -> PathBuf {
    paths.local_library().join("running.json")
}

fn load(paths: &DataPaths) -> RunningFile {
    json_store::read::<RunningFile>(&file_path(paths)).unwrap_or_else(|e| {
        log::warn!("reading the running games: {e:#}");
        RunningFile::default()
    })
}

fn save(paths: &DataPaths, entry: &RunningEntry) {
    let res = json_store::update::<RunningFile, _>(&file_path(paths), |f| {
        f.sessions.insert(entry.app_name.clone(), entry.clone());
        Ok(())
    });
    if let Err(e) = res {
        log::warn!("saving the running game {}: {e:#}", entry.app_name);
    }
}

fn remove(paths: &DataPaths, app_name: &str) {
    let path = file_path(paths);
    if !load(paths).sessions.contains_key(app_name) {
        return;
    }
    let res = json_store::update::<RunningFile, _>(&path, |f| {
        f.sessions.remove(app_name);
        Ok(())
    });
    if let Err(e) = res {
        log::warn!("forgetting the running game {app_name}: {e:#}");
    }
}

/// Writes the session down once the game really runs (right after
/// [`Control::set_playing`]). Nothing is written without a boot id or for a
/// process that is already gone.
pub(super) fn remember(shared: &Shared, control: &Control, req: &LaunchRequest, tracked: Tracked) {
    let Some(boot_id) = shared.procs.boot_id() else { return };
    let Some(started_at) = control.started_at() else { return };
    let mut ledger = control.ledger.lock().unwrap();
    // The base is read now, not at the first checkpoint: a crash before that
    // checkpoint must not leave the next run guessing it.
    if let Err(e) = ledger.cp.begin(&shared.paths, &req.app_name) {
        log::warn!("reading the playtime of {}: {e:#}", req.app_name);
    }
    let entry = RunningEntry {
        app_name: req.app_name.clone(),
        runner: req.runner.clone(),
        title: req.title.clone(),
        boot_id,
        started_at: iso(started_at),
        mono_start_ms: monotonic_ms().saturating_sub(control.played().as_millis() as u64),
        base_total: ledger.cp.base_total(),
        written: ledger.cp.has_written(),
        since_ticks: control.since_ticks,
        tracked,
    };
    drop(ledger);
    save(&shared.paths, &entry);
    *control.running.lock().unwrap() = Some(entry);
}

/// What a process plan's session writes down (see [`remember`]).
pub(super) fn process_entry(shared: &Shared, pid: u32, plan: &crate::plan::ProcessPlan) -> Option<Tracked> {
    Some(Tracked::Process {
        pid,
        start_ticks: shared.procs.start_ticks(pid)?,
        stop: plan.stop.clone(),
        wait_after_exit: plan.wait_after_exit.clone(),
        after_script: plan.after_script.clone(),
        cloud_saves: plan.prep.cloud_saves.clone(),
        keep_awake: plan.keep_awake.is_some(),
    })
}

/// What a Steam session writes down (see [`remember`]).
pub(super) fn steam_entry(shared: &Shared, control: &Control, app_id: &str, opened_steam: bool) -> Tracked {
    Tracked::Steam { app_id: app_id.to_string(), pids: steam_pids(&*shared.procs, &control.steam_pids.lock().unwrap()), opened_steam }
}

/// The Steam game's pids that are still the same processes.
fn steam_pids(procs: &dyn Procs, pids: &BTreeSet<u32>) -> Vec<(u32, u64)> {
    pids.iter().filter_map(|p| procs.start_ticks(*p).map(|t| (*p, t))).collect()
}

/// After a checkpoint: the base, whether something was written, Steam's
/// current pids.
pub(super) fn refresh(shared: &Shared, control: &Control, ledger: &Ledger) {
    let mut running = control.running.lock().unwrap();
    let Some(entry) = running.as_mut() else { return };
    entry.base_total = ledger.cp.base_total();
    entry.written = ledger.cp.has_written();
    if let Tracked::Steam { pids, .. } = &mut entry.tracked {
        let now = steam_pids(&*shared.procs, &control.steam_pids.lock().unwrap());
        // A moment without any pid (the launcher handing over to the game)
        // keeps the last ones known.
        if !now.is_empty() {
            *pids = now;
        }
    }
    save(&shared.paths, entry);
}

/// The session ended: nothing to pick up any more.
pub(super) fn forget(shared: &Shared, control: &Control, app_name: &str) {
    if control.running.lock().unwrap_or_else(|e| e.into_inner()).take().is_some() {
        remove(&shared.paths, app_name);
    }
}

/// Whether the process group the app started for a game still runs. The
/// leader is ours if it started when it did; once it is gone, the group's id
/// cannot be given to another process while any member is left.
fn group_running(procs: &dyn Procs, pid: u32, start_ticks: u64) -> bool {
    match procs.start_ticks(pid) {
        Some(t) => t == start_ticks,
        None => procs.group_alive(pid),
    }
}

fn still_running(procs: &dyn Procs, boot_id: Option<&str>, entry: &RunningEntry) -> bool {
    if boot_id != Some(entry.boot_id.as_str()) {
        return false;
    }
    match &entry.tracked {
        Tracked::Process { pid, start_ticks, .. } => group_running(procs, *pid, *start_ticks),
        Tracked::Steam { pids, .. } => pids.iter().any(|(p, t)| procs.start_ticks(*p) == Some(*t)),
    }
}

impl Launcher {
    /// Picks up the games a previous run of the app left running (see the
    /// module docs). `on_adopt` is called for each before its `Running` event,
    /// so the app can register what it shows. Returns how many were adopted.
    pub fn adopt(&self, on_adopt: &mut dyn FnMut(&AdoptedGame)) -> usize {
        let shared = &self.shared;
        let file = load(&shared.paths);
        if file.sessions.is_empty() {
            return 0;
        }
        let boot_id = shared.procs.boot_id();
        let mut adopted = 0;
        for (app_name, entry) in file.sessions {
            if shared.sessions.lock().unwrap().contains_key(&app_name) {
                continue;
            }
            if !still_running(&*shared.procs, boot_id.as_deref(), &entry) {
                log::info!("{} ended while the app was closed; its playtime is kept up to the last checkpoint", entry.title);
                remove(&shared.paths, &app_name);
                continue;
            }
            let control = Arc::new(adopted_control(&entry));
            shared.sessions.lock().unwrap().insert(app_name.clone(), control.clone());
            on_adopt(&AdoptedGame { app_name: app_name.clone(), runner: entry.runner.clone(), title: entry.title.clone() });
            log::info!("{} is still running: following it again", entry.title);
            shared.send(LaunchEvent::Running { app_name: app_name.clone(), pid: control.pid.load(Ordering::SeqCst) });
            let shared = shared.clone();
            let spawned =
                std::thread::Builder::new().name(format!("adopted-{app_name}")).spawn(move || run_adopted(shared, entry, control));
            match spawned {
                Ok(_) => adopted += 1,
                Err(e) => log::error!("following {app_name} again: {e}"),
            }
        }
        adopted
    }
}

fn adopted_control(entry: &RunningEntry) -> Control {
    let started_at = DateTime::parse_from_rfc3339(&entry.started_at).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now());
    let played = Duration::from_millis(monotonic_ms().saturating_sub(entry.mono_start_ms));
    let t0 = Instant::now().checked_sub(played).unwrap_or_else(Instant::now);
    let (pid, stop, steam_pids, steam) = match &entry.tracked {
        Tracked::Process { pid, stop, .. } => (*pid, stop.clone(), BTreeSet::new(), false),
        Tracked::Steam { pids, .. } => {
            let set: BTreeSet<u32> = pids.iter().map(|(p, _)| *p).collect();
            (set.iter().next().copied().unwrap_or(0), StopSpec::default(), set, true)
        }
    };
    Control {
        abort: AtomicBool::new(false),
        kill: AtomicBool::new(false),
        record: AtomicBool::new(true),
        steam,
        pid: AtomicU32::new(pid),
        stop,
        steam_pids: Arc::new(Mutex::new(steam_pids)),
        title: entry.title.clone(),
        clock: OnceLock::from((started_at, t0)),
        since_ticks: entry.since_ticks,
        playing: AtomicBool::new(true),
        ledger: Mutex::new(Ledger { cp: Checkpointer::resume(entry.base_total, entry.written), finished: false }),
        wake: Default::default(),
        running: Mutex::new(Some(entry.clone())),
    }
}

fn run_adopted(shared: Arc<Shared>, entry: RunningEntry, control: Arc<Control>) {
    let _end = SessionEnd::new(shared.clone(), &entry.app_name, &entry.title, control.clone());
    spawn_checkpointer(shared.clone(), entry.app_name.clone(), control.clone());
    // `finish` only reads the names.
    let req = LaunchRequest {
        app_name: entry.app_name.clone(),
        runner: entry.runner.clone(),
        title: entry.title.clone(),
        plan: LaunchPlan::Unavailable(String::new()),
    };
    let (disabled_globally, verbose, _) = log_writer::launch_log_switches(&shared.paths, &entry.app_name);
    let path = LogPaths::for_data(&shared.paths).game(&entry.app_name, &entry.runner, GameLogType::Launch);
    // The game still writes to its log: it is appended to, never archived.
    let log = GameLog::continuing(path, disabled_globally || !verbose);
    log.info("The app was restarted; following the game again");
    match &entry.tracked {
        Tracked::Process { pid, start_ticks, wait_after_exit, after_script, cloud_saves, keep_awake, .. } => {
            let services = shared.services.lock().unwrap().clone();
            let status = |n: Notice| {
                if n.warning {
                    log.warning(&n.log);
                } else {
                    log.info(&n.log);
                }
                shared.send(LaunchEvent::Status { app_name: entry.app_name.clone(), message: n.message, warning: n.warning })
            };
            // The lock held for the previous run died with it.
            let mut awake = keep_awake
                .then(|| crate::keep_awake::command(&*services.probe, &entry.title, std::process::id()))
                .flatten()
                .and_then(|cmd| shared.spawner.spawn(&cmd, None).ok());
            let aborted = follow_group(&shared, &control, *pid, *start_ticks);
            if let Some(h) = awake.as_mut() {
                h.terminate();
            }
            let stopped = control.kill.load(Ordering::SeqCst);
            if !aborted && !stopped {
                if let Some(cmd) = wait_after_exit {
                    run_to_completion(&shared, &control, cmd, Some(&log));
                }
            }
            let elapsed = control.played();
            if !aborted {
                if let Some(script) = after_script {
                    run_script(&shared, &control, &*services.probe, script, &log, &status);
                }
                let never = || false;
                let prep = Prep {
                    paths: &shared.paths,
                    probe: &*services.probe,
                    net: &*services.net,
                    spawner: &*shared.spawner,
                    cloud: services.cloud.as_deref(),
                    cancel: &never,
                    notify: &status,
                };
                prep.after_exit(&entry.app_name, &PrepSpec { cloud_saves: cloud_saves.clone(), ..Default::default() });
            }
            finish(&shared, &log, &req, &control, elapsed, None);
        }
        Tracked::Steam { app_id, opened_steam, .. } => {
            let locator = shared.steam.clone();
            let seeded: BTreeSet<u32> = control.steam_pids.lock().unwrap().clone();
            let outcome = crate::playtime_watch::follow_steam_session(
                app_id,
                &*shared.procs,
                &move || locator.log_files(),
                &shared.cfg.steam,
                &control.abort,
                &control.wake,
                &control.steam_pids,
                seeded,
            );
            log.info("Steam game finished");
            finish(&shared, &log, &req, &control, control.played(), None);
            if *opened_steam && matches!(outcome, crate::playtime_watch::SteamWaitOutcome::Finished) {
                super::steam::close_steam(&shared, &entry.title);
            }
        }
    }
}

/// Waits for the game's process group to end, terminating it when stopped.
/// Returns true when tracking was aborted (force clear).
fn follow_group(shared: &Shared, control: &Control, pid: u32, start_ticks: u64) -> bool {
    let procs = &*shared.procs;
    let mut terminated_at: Option<Instant> = None;
    loop {
        if !group_running(procs, pid, start_ticks) {
            return false;
        }
        if control.abort.load(Ordering::SeqCst) {
            return true;
        }
        if control.kill.load(Ordering::SeqCst) {
            // The same as a spawned game's handle: the tree, then the group.
            let sig = match terminated_at {
                None => {
                    terminated_at = Some(Instant::now());
                    Some(Sig::Term)
                }
                Some(t) if t.elapsed() > shared.cfg.kill_grace => {
                    if t.elapsed() > shared.cfg.kill_grace + Duration::from_secs(2) {
                        return false;
                    }
                    Some(Sig::Kill)
                }
                _ => None,
            };
            if let Some(sig) = sig {
                signal_tree(procs, pid, sig);
                procs.signal_group(pid, sig);
            }
        }
        let kill_pending = terminated_at.is_none();
        control.wake.wait_while(shared.cfg.poll_interval, || {
            !control.abort.load(Ordering::SeqCst) && !(kill_pending && control.kill.load(Ordering::SeqCst))
        });
    }
}
