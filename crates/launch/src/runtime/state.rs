//! Shared state of the launcher and of one tracked session.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    pub(super) started_at: chrono::DateTime<Utc>,
    pub(super) t0: Instant,
    /// When the launch began, in `/proc` start-time ticks: stop patterns
    /// only reach processes started since.
    pub(super) since_ticks: u64,
    /// The game is running (the session has playtime to checkpoint).
    pub(super) playing: AtomicBool,
    pub(super) ledger: Mutex<Ledger>,
    /// Notified whenever `abort`, `kill`, `playing` or `ledger.finished`
    /// change, so the tracking threads sleep instead of polling them.
    pub(super) wake: Wake,
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

    pub(super) fn set_playing(&self) {
        self.playing.store(true, Ordering::SeqCst);
        self.wake.notify();
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
        let rec = PlayRecord {
            app_name: app_name.to_string(),
            title: control.title.clone(),
            started_at: control.started_at,
            ended_at: control.started_at + chrono::Duration::milliseconds(control.t0.elapsed().as_millis() as i64),
        };
        if let Err(e) = ledger.cp.write(&self.paths, &rec, self.cfg.min_session_seconds) {
            log::warn!("saving the playtime of {app_name}: {e:#}");
        }
    }

    pub(super) fn send(&self, ev: LaunchEvent) {
        let _ = self.tx.lock().unwrap().send(ev);
    }
}
