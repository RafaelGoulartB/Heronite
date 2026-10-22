//! Public types of the launcher API: requests, events, errors, configuration.

use heronite_i18n::tr;
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
    /// The game is gone (or tracking was cleared). `stopped` is true when the
    /// user ended it (stop, force clear), whatever `exit` says. `record` is
    /// what was written to the data files, `None` if recording failed or was
    /// disabled, or if it was a [`failed_launch`].
    Exited { app_name: String, elapsed: Duration, exit: Option<ExitInfo>, stopped: bool, record: Option<RecordOutcome> },
    /// Spawning failed, Steam never started the game, or recording failed.
    Error { app_name: String, message: String },
    /// Progress or a non-fatal problem while the launch is prepared or
    /// finished ("Preparing prefix...", "Downloading DXVK...", a cloud save
    /// error). The game keeps going; `warning` says something went wrong.
    Status { app_name: String, message: String, warning: bool },
    /// The step the launch is at before the game starts ("Preparing
    /// prefix...", "Installing Redistributables (EPIC)", "Waiting for Steam
    /// to start the game..."). `cancellable` is false while stopping could
    /// do harm (cloud saves being downloaded): a stop then waits for the step.
    Preparing { app_name: String, message: String, cancellable: bool },
}

/// A game that ends this soon, badly, on its own never really started.
pub const FAILED_LAUNCH_WINDOW: Duration = Duration::from_secs(30);

/// The exit of a game that crashed right after starting: not stopped by the
/// user, killed by a signal or with a non-zero code, within
/// [`FAILED_LAUNCH_WINDOW`]. Such a session is not recorded.
pub fn failed_launch(exit: Option<ExitInfo>, stopped: bool, elapsed: Duration) -> Option<ExitInfo> {
    exit.filter(|e| !stopped && elapsed < FAILED_LAUNCH_WINDOW && (e.signal.is_some() || e.code.is_some_and(|c| c != 0)))
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
            LaunchError::Unavailable(r) => f.write_str(&tr!("cannot launch: {reason}", reason = r)),
            LaunchError::AlreadyRunning => f.write_str(tr!("this game is already running")),
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
        Self { probe: Arc::new(SystemProbe), net: Arc::new(SystemNet), cloud: None }
    }
}

/// The mods of a game, put in place around its session (the app provides it,
/// see `heronite_mods::apply`). Called on the session thread before the
/// game starts, Steam games included.
pub trait GameMods: Send + Sync {
    /// `Ok(None)`: the game has no mods to apply. `status` shows a step of
    /// the launch ("Applying mods..."). An error aborts the launch: a game
    /// whose saves expect its mods must not start without them.
    fn apply(&self, app_name: &str, runner: &str, status: &mut dyn FnMut(&str)) -> Result<Option<AppliedMods>, String>;

    /// A game this run did not start has ended (it was picked up again after
    /// the app restarted): its mods were applied by the earlier run, whose
    /// guard is gone, so take them away. Blocking; must be safe to call when
    /// nothing is applied.
    fn release(&self, _app_name: &str, _runner: &str) {}
}

/// Mods in place for one session.
pub struct AppliedMods {
    /// Variables added to the game's command (`WINEDLLOVERRIDES` of the mod
    /// loaders), merged with the command's own by the launcher.
    pub env: Vec<(String, String)>,
    /// One line for the game's log.
    pub summary: String,
    /// Dropped when the session ends: takes the mods away again.
    pub guard: Box<dyn Send>,
}

/// `formatSystemInfo` for the log header (the app provides it).
pub type SystemInfoFn = Arc<dyn Fn() -> String + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(code: Option<i32>, signal: Option<i32>) -> Option<ExitInfo> {
        Some(ExitInfo { code, signal })
    }

    #[test]
    fn only_a_quick_bad_exit_nobody_asked_for_is_a_failed_launch() {
        let quick = Duration::from_secs(3);
        assert!(failed_launch(exit(Some(1), None), false, quick).is_some());
        assert!(failed_launch(exit(None, Some(11)), false, quick).is_some());
        assert!(failed_launch(exit(Some(0), None), false, quick).is_none());
        assert!(failed_launch(exit(None, None), false, quick).is_none());
        assert!(failed_launch(None, false, quick).is_none());
        // The user asked for it.
        assert!(failed_launch(exit(None, Some(15)), true, quick).is_none());
        // It ran for a while.
        assert!(failed_launch(exit(Some(1), None), false, FAILED_LAUNCH_WINDOW).is_none());
    }
}
