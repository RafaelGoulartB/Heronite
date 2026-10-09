//! Public types of the launcher API: requests, events, errors, configuration.

use std::sync::Arc;
use std::time::Duration;

use super::ExitInfo;
use crate::net::{Net, SystemNet};
use crate::plan::LaunchPlan;
use crate::playtime_watch::SteamWaitConfig;
use crate::prepare::CloudSync;
use crate::probe::{Probe, SystemProbe};
use crate::record::{RecordOutcome, MIN_SESSION_SECONDS};


#[derive(Debug, Clone, PartialEq)]
pub enum LaunchEvent {
    /// Accepted; the process (or the Steam URI) is being started.
    Launching { app_name: String },
    /// The game process exists. For Steam this is the pid found in Steam's logs.
    Running { app_name: String, pid: u32 },
    /// The game is gone (or tracking was cleared). `record` is what was
    /// written to the data files, `None` if recording failed or was disabled.
    Exited {
        app_name: String,
        elapsed: Duration,
        exit: Option<ExitInfo>,
        record: Option<RecordOutcome>,
    },
    /// Spawning failed, Steam never started the game, or recording failed.
    Error { app_name: String, message: String },
    /// Progress or a non-fatal problem while the launch is prepared or
    /// finished ("Preparing prefix...", "Downloading DXVK...", a cloud save
    /// error). The game keeps going; `warning` says something went wrong.
    Status {
        app_name: String,
        message: String,
        warning: bool,
    },
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    pub plan: LaunchPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    Unavailable(String),
    AlreadyRunning,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Unavailable(r) => write!(f, "cannot launch: {r}"),
            LaunchError::AlreadyRunning => write!(f, "this game is already running"),
        }
    }
}

impl std::error::Error for LaunchError {}

#[derive(Debug, Clone)]
pub struct LauncherConfig {
    /// How often a running process is polled.
    pub poll_interval: Duration,
    /// After SIGTERM, how long before SIGKILL.
    pub kill_grace: Duration,
    /// Sessions shorter than this are not stored (Heroic: 5 s).
    pub min_session_seconds: i64,
    pub steam: SteamWaitConfig,
    /// Write timestamps / sessions / recent games on exit.
    pub record: bool,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            kill_grace: Duration::from_secs(5),
            min_session_seconds: MIN_SESSION_SECONDS,
            steam: SteamWaitConfig::default(),
            record: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StopOptions {
    /// Steam games: also SIGTERM the game processes found in Steam's logs.
    /// Heronite's own stop only ends tracking for Steam games.
    pub kill_steam_processes: bool,
}


/// How often a running game's playtime is saved.
pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(120);

/// What the pre-launch preparation talks to (replaceable for tests and the
/// app: the Epic crate provides the cloud saves).
#[derive(Clone)]
pub struct PrepServices {
    pub probe: Arc<dyn Probe>,
    pub net: Arc<dyn Net>,
    pub cloud: Option<Arc<dyn CloudSync>>,
}

impl Default for PrepServices {
    fn default() -> Self {
        Self {
            probe: Arc::new(SystemProbe),
            net: Arc::new(SystemNet),
            cloud: None,
        }
    }
}

/// `formatSystemInfo` for the log header (the app provides it).
pub type SystemInfoFn = Arc<dyn Fn() -> String + Send + Sync>;
