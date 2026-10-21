//! Running plans: spawning, tracking until the game is really gone, stopping,
//! and recording the session. All side effects on processes go through the
//! [`Spawner`] and [`crate::procs::Procs`] traits so tests use fakes.
//!
//! Like Heroic's backend, several games may run at once (one per app name);
//! launching an app that is already tracked is refused.

mod adopt;
mod companions;
mod fake;
mod process;
mod sandboxed;
mod session;
mod spawner;
mod state;
mod steam;
mod types;

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use heronite_data::DataPaths;

use crate::first_launch::{EpicSetup, StoreSetup};
use crate::plan::LaunchPlan;
use crate::procs::{signal_tree, Procs, Sig, SystemProcs};
use crate::record::Checkpointer;
use crate::steam::SteamLocator;
use session::run_session;
use state::{Control, Ledger, Shared};

pub use adopt::AdoptedGame;
pub use fake::{FakeHandleState, FakeSpawner};
pub use spawner::{ExitInfo, ProcessHandle, Spawner, SystemSpawner};
pub use types::{
    failed_launch, LaunchError, LaunchEvent, LaunchRequest, LauncherConfig, PrepServices, StopOptions, SystemInfoFn, CHECKPOINT_INTERVAL,
    FAILED_LAUNCH_WINDOW,
};

/// Tracks running games. Cheap to clone.
#[derive(Clone)]
pub struct Launcher {
    shared: Arc<Shared>,
}

impl Launcher {
    pub fn new(
        paths: DataPaths,
        steam: SteamLocator,
        spawner: Arc<dyn Spawner>,
        procs: Arc<dyn Procs>,
        cfg: LauncherConfig,
        events: Sender<LaunchEvent>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                system_info: Mutex::new(None),
                services: Mutex::new(PrepServices::default()),
                epic: Mutex::new(None),
                store_setup: Mutex::new(None),
                paths,
                spawner,
                procs,
                steam,
                cfg,
                checkpoint_ms: AtomicU64::new(CHECKPOINT_INTERVAL.as_millis() as u64),
                tx: Mutex::new(events),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Replaces what launch preparation uses (network, machine, cloud saves).
    pub fn set_prep_services(&self, services: PrepServices) {
        *self.shared.services.lock().unwrap() = services;
    }

    /// Text for the "System Info" part of every launch log.
    pub fn set_system_info(&self, f: SystemInfoFn) {
        *self.shared.system_info.lock().unwrap() = Some(f);
    }

    /// What the first launch of an Epic game in a prefix asks legendary
    /// (prerequisites, EOS overlay).
    pub fn set_epic_setup(&self, epic: Arc<dyn EpicSetup>) {
        *self.shared.epic.lock().unwrap() = Some(epic);
    }

    /// What the first launch of a GOG game in a prefix asks gogdl
    /// (redistributables, the game's install scripts).
    pub fn set_store_setup(&self, store: Arc<dyn StoreSetup>) {
        *self.shared.store_setup.lock().unwrap() = Some(store);
    }

    /// Changes how often running games' playtime is saved (tests).
    pub fn set_checkpoint_interval(&self, every: Duration) {
        self.shared.checkpoint_ms.store(every.as_millis().max(1) as u64, Ordering::SeqCst);
    }

    /// Saves the playtime of every running game right now (before the app
    /// quits). Blocking file I/O: call it off the UI thread. Returns how
    /// many games were running.
    pub fn checkpoint_all(&self) -> usize {
        let running: Vec<(String, Arc<Control>)> =
            self.shared.sessions.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let n = running.iter().filter(|(_, c)| c.playing.load(Ordering::SeqCst)).count();
        for (app_name, control) in &running {
            self.shared.checkpoint(control, app_name);
        }
        n
    }

    /// Real processes, default timings.
    pub fn system(paths: DataPaths, steam: SteamLocator, events: Sender<LaunchEvent>) -> Self {
        let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
        Self::new(paths, steam, Arc::new(SystemSpawner::new(procs.clone())), procs, LauncherConfig::default(), events)
    }

    pub fn is_running(&self, app_name: &str) -> bool {
        self.shared.sessions.lock().unwrap().contains_key(app_name)
    }

    pub fn running_apps(&self) -> Vec<String> {
        let mut v: Vec<String> = self.shared.sessions.lock().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    /// Pid of the tracked process, once known.
    pub fn pid_of(&self, app_name: &str) -> Option<u32> {
        let s = self.shared.sessions.lock().unwrap();
        s.get(app_name).map(|c| c.pid.load(Ordering::SeqCst)).filter(|p| *p != 0)
    }

    /// How the FPS limit of a running game can change, if it can.
    pub fn live_fps(&self, app_name: &str) -> Option<crate::live_fps::LiveFps> {
        let s = self.shared.sessions.lock().unwrap();
        s.get(app_name).and_then(|c| c.live_fps.lock().unwrap().clone())
    }

    /// Starts the game in the background; progress arrives as [`LaunchEvent`]s.
    pub fn launch(&self, req: LaunchRequest) -> Result<(), LaunchError> {
        if let LaunchPlan::Unavailable(reason) = &req.plan {
            return Err(LaunchError::Unavailable(reason.clone()));
        }
        let stop = req.plan.process().map(|p| p.stop.clone()).unwrap_or_default();
        let control = Arc::new(Control {
            abort: AtomicBool::new(false),
            kill: AtomicBool::new(false),
            record: AtomicBool::new(true),
            steam: matches!(req.plan, LaunchPlan::SteamUri { .. }),
            pid: AtomicU32::new(0),
            stop,
            steam_pids: Arc::new(Mutex::new(BTreeSet::new())),
            title: req.title.clone(),
            clock: Default::default(),
            since_ticks: crate::procs::boot_ticks_now(),
            playing: AtomicBool::new(false),
            ledger: Mutex::new(Ledger { cp: Checkpointer::default(), finished: false }),
            wake: Default::default(),
            running: Mutex::new(None),
            live_fps: Mutex::new(None),
        });
        {
            let mut sessions = self.shared.sessions.lock().unwrap();
            if sessions.contains_key(&req.app_name) {
                return Err(LaunchError::AlreadyRunning);
            }
            sessions.insert(req.app_name.clone(), control.clone());
        }
        self.shared.send(LaunchEvent::Launching { app_name: req.app_name.clone() });
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name(format!("launch-{}", req.app_name))
            .spawn(move || run_session(shared, req, control))
            .map_err(|e| LaunchError::Unavailable(e.to_string()))?;
        Ok(())
    }

    /// Stops a game: kills its process tree (not for Steam games unless
    /// asked), runs the plan's stop commands and `pkill` patterns.
    /// Returns false if the game is not being tracked.
    pub fn stop(&self, app_name: &str) -> bool {
        self.stop_with(app_name, StopOptions::default())
    }

    pub fn stop_with(&self, app_name: &str, opts: StopOptions) -> bool {
        let Some(control) = self.shared.sessions.lock().unwrap().get(app_name).cloned() else {
            return false;
        };
        let steam_pids: Vec<u32> = control.steam_pids.lock().unwrap().iter().copied().collect();
        if opts.kill_steam_processes {
            for pid in &steam_pids {
                signal_tree(&*self.shared.procs, *pid, Sig::Term);
            }
        }
        if control.steam {
            // Heronite's stop only ends tracking for Steam games.
            control.set_abort();
        } else {
            control.set_kill();
        }
        let spawner = self.shared.spawner.clone();
        let stop = control.stop.clone();
        let since = control.since_ticks;
        // Off the caller's thread: wineserver -k / pkill can take a moment.
        std::thread::spawn(move || {
            for pattern in &stop.kill_patterns {
                if pattern.len() >= 3 {
                    spawner.kill_pattern(pattern, since);
                }
            }
            for cmd in &stop.commands {
                if let Ok(mut h) = spawner.spawn(cmd, None) {
                    let deadline = Instant::now() + Duration::from_secs(15);
                    while Instant::now() < deadline {
                        if !matches!(h.poll(), Ok(None)) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        });
        true
    }

    /// "Force clear playing": forget the game without killing anything. The
    /// session is still recorded with the time elapsed so far (as Heroic does).
    pub fn force_clear(&self, app_name: &str) -> bool {
        self.force_clear_with(app_name, true)
    }

    pub fn force_clear_with(&self, app_name: &str, record: bool) -> bool {
        let Some(control) = self.shared.sessions.lock().unwrap().get(app_name).cloned() else {
            return false;
        };
        control.record.store(record, Ordering::SeqCst);
        control.set_abort();
        true
    }
}
