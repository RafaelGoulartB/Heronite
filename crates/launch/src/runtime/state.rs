//! Shared state of the launcher and of one tracked session.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use heronite_data::DataPaths;

use super::{LaunchEvent, LauncherConfig, PrepServices, Spawner, SystemInfoFn};
use crate::first_launch::{EpicSetup, StoreSetup};
use crate::plan::StopSpec;
use crate::procs::Procs;
use crate::record::{Checkpointer, PlayRecord};
use crate::steam::SteamLocator;
use crate::wake::Wake;

pub(super) struct Control {
    /// Stop tracking (force clear, or the end of a stop).
    pub(super) abort: AtomicBool,
    /// Kill the tracked process tree.
    pub(super) kill: AtomicBool,
    /// Record the session when tracking ends (default true).
    pub(super) record: AtomicBool,
    /// Steam games are tracked through logs; stopping only ends tracking.
    pub(super) steam: bool,
    pub(super) pid: AtomicU32,
    pub(super) stop: StopSpec,
    pub(super) steam_pids: Arc<Mutex<BTreeSet<u32>>>,
    pub(super) title: String,
    /// When the game really started; preparing the prefix, cloud saves, scripts
    /// or Steam's start-up is not playtime.
    pub(super) clock: OnceLock<(chrono::DateTime<Utc>, Instant)>,
    /// When the launch began, in `/proc` start-time ticks: stop patterns
    /// only reach processes started since.
    pub(super) since_ticks: u64,
    /// The game is running (the session has playtime to checkpoint).
    pub(super) playing: AtomicBool,
    pub(super) ledger: Mutex<Ledger>,
    /// Notified whenever `abort`, `kill`, `playing` or `ledger.finished`
    /// change, so the tracking threads sleep instead of polling them.
    pub(super) wake: Wake,
    /// What `running.json` holds for this session, once the game runs (see
    /// [`super::adopt`]).
    pub(super) running: Mutex<Option<super::adopt::RunningEntry>>,
    /// How the limit can change while the game runs (see [`crate::live_fps`]).
    pub(super) live_fps: Mutex<Option<crate::live_fps::LiveFps>>,
}

impl Control {
    pub(super) fn set_abort(&self) {
        self.abort.store(true, Ordering::SeqCst);
        self.wake.notify();
    }

    pub(super) fn set_kill(&self) {
        self.kill.store(true, Ordering::SeqCst);
        self.wake.notify();
    }

    /// The game is really running now: starts the session clock and returns
    /// when it started.
    pub(super) fn set_playing(&self) -> chrono::DateTime<Utc> {
        let (started_at, _) = *self.clock.get_or_init(|| (Utc::now(), Instant::now()));
        self.playing.store(true, Ordering::SeqCst);
        self.wake.notify();
        started_at
    }

    pub(super) fn started_at(&self) -> Option<chrono::DateTime<Utc>> {
        self.clock.get().map(|(at, _)| *at)
    }

    /// Time played so far; zero if the game never started.
    pub(super) fn played(&self) -> Duration {
        self.clock.get().map_or(Duration::ZERO, |(_, t0)| t0.elapsed())
    }

    /// The user stopped the session (stop, or force clear).
    pub(super) fn stopped(&self) -> bool {
        self.abort.load(Ordering::SeqCst) || self.kill.load(Ordering::SeqCst)
    }
}

/// What was persisted of the session so far.
pub(super) struct Ledger {
    pub(super) cp: Checkpointer,
    /// The final write was made; later checkpoints are no-ops.
    pub(super) finished: bool,
}

pub(super) struct Shared {
    pub(super) system_info: Mutex<Option<SystemInfoFn>>,
    pub(super) services: Mutex<PrepServices>,
    pub(super) epic: Mutex<Option<Arc<dyn EpicSetup>>>,
    pub(super) store_setup: Mutex<Option<Arc<dyn StoreSetup>>>,
    pub(super) paths: DataPaths,
    pub(super) spawner: Arc<dyn Spawner>,
    pub(super) procs: Arc<dyn Procs>,
    pub(super) steam: SteamLocator,
    pub(super) cfg: LauncherConfig,
    /// Milliseconds between playtime checkpoints.
    pub(super) checkpoint_ms: AtomicU64,
    pub(super) tx: Mutex<Sender<LaunchEvent>>,
    pub(super) sessions: Mutex<HashMap<String, Arc<Control>>>,
}

impl Shared {
    /// Saves the playtime of a running session so far (see
    /// [`record::Checkpointer`]). Cheap no-op for games that are not running,
    /// whose recording is off, or that already finished.
    pub(super) fn checkpoint(&self, control: &Control, app_name: &str) {
        if !self.cfg.record || !control.record.load(Ordering::SeqCst) || !control.playing.load(Ordering::SeqCst) {
            return;
        }
        let mut ledger = control.ledger.lock().unwrap();
        if ledger.finished {
            return;
        }
        let Some(started_at) = control.started_at() else {
            return;
        };
        let rec = PlayRecord {
            app_name: app_name.to_string(),
            title: control.title.clone(),
            started_at,
            ended_at: started_at + chrono::Duration::milliseconds(control.played().as_millis() as i64),
        };
        if let Err(e) = ledger.cp.write(&self.paths, &rec, self.cfg.min_session_seconds) {
            log::warn!("saving the playtime of {app_name}: {e:#}");
        }
        super::adopt::refresh(self, control, &ledger);
    }

    pub(super) fn send(&self, ev: LaunchEvent) {
        let _ = self.tx.lock().unwrap_or_else(|e| e.into_inner()).send(ev);
    }
}
