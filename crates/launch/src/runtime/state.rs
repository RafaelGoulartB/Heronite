//! Shared state of the launcher and of one tracked session.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use heronite_data::HeroicPaths;

use super::{LaunchEvent, LauncherConfig, PrepServices, Spawner, SystemInfoFn};
use crate::first_launch::EpicSetup;
use crate::plan::StopSpec;
use crate::procs::Procs;
use crate::record::{Checkpointer, PlayRecord};
use crate::steam::SteamLocator;

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
    /// The game is running (the session has playtime to checkpoint).
    pub(super) playing: AtomicBool,
    pub(super) ledger: Mutex<Ledger>,
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
    pub(super) paths: HeroicPaths,
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
