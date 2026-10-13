//! The programs run alongside a game (see [`crate::companions`]) while it is
//! tracked: started when their delay is up, closed when the game ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_i18n::tr;

use super::spawner::ProcessHandle;
use super::state::Shared;
use crate::log_writer::{self, GameLog};
use crate::plan::CompanionPlan;
use crate::prepare::Notice;
use crate::probe::Probe;
use crate::procs::Procs;

/// Wine's own programs, running in every prefix in use, and Proton's
/// `steam.exe`, which starts the game and stays after it.
const WINE_SERVICES: &[&str] = &[
    "steam.exe",
    "services.exe",
    "winedevice.exe",
    "plugplay.exe",
    "svchost.exe",
    "explorer.exe",
    "rpcss.exe",
    "conhost.exe",
    "tabtip.exe",
    "wineboot.exe",
    "winemenubuilder.exe",
    "start.exe",
];

/// How long the game must be gone from its prefix before it counts as over
/// (a launcher may hand over to the game).
const GONE_FOR: Duration = Duration::from_secs(3);

pub(super) struct Companions<'a> {
    waiting: Vec<(Instant, &'a CompanionPlan)>,
    running: Vec<(String, Box<dyn ProcessHandle>)>,
    /// The game was seen running in its prefix.
    seen: bool,
    /// Since when the game is not in its prefix any more.
    gone_since: Option<Instant>,
}

impl<'a> Companions<'a> {
    /// The game started at `started`.
    pub(super) fn new(plans: &'a [CompanionPlan], started: Instant) -> Self {
        Self { waiting: plans.iter().map(|p| (started + p.delay, p)).collect(), running: Vec::new(), seen: false, gone_since: None }
    }

    /// Some program is waiting for its turn or still running.
    pub(super) fn active(&mut self) -> bool {
        self.running.retain_mut(|(_, h)| matches!(h.poll(), Ok(None)));
        !self.waiting.is_empty() || !self.running.is_empty()
    }

    /// Whether the game, after it was seen in `prefix`, has been gone from it
    /// for [`GONE_FOR`]. With plain Proton or Wine the processes that started
    /// the game do not tell: they may exit long before the game does, or wait
    /// for the programs started here.
    pub(super) fn game_left(&mut self, procs: &dyn Procs, prefix: &str) -> bool {
        if self.game_in_prefix(procs, prefix) {
            self.seen = true;
            self.gone_since = None;
            return false;
        }
        self.seen && self.gone_since.get_or_insert_with(Instant::now).elapsed() >= GONE_FOR
    }

    /// Whether the game may still be in `prefix` after the process that
    /// started it exited: it is there, or it was and left less than
    /// [`GONE_FOR`] ago.
    pub(super) fn game_may_run(&mut self, procs: &dyn Procs, prefix: &str) -> bool {
        if self.game_in_prefix(procs, prefix) {
            self.seen = true;
            self.gone_since = None;
            return true;
        }
        self.seen && self.gone_since.get_or_insert_with(Instant::now).elapsed() < GONE_FOR
    }

    /// Whether a Windows program other than Wine's own and the ones started
    /// here runs in `prefix`.
    fn game_in_prefix(&self, procs: &dyn Procs, prefix: &str) -> bool {
        let ours: Vec<String> = self
            .waiting
            .iter()
            .map(|(_, p)| p.path.as_str())
            .chain(self.running.iter().map(|(path, _)| path.as_str()))
            .filter_map(|p| p.rsplit('/').next())
            .map(str::to_lowercase)
            .collect();
        procs.windows_programs(prefix).iter().any(|name| !WINE_SERVICES.contains(&name.as_str()) && !ours.contains(name))
    }

    /// How long until the next one is due (`None`: none left to start).
    pub(super) fn next_due(&self, now: Instant) -> Option<Duration> {
        self.waiting.iter().map(|(at, _)| at.saturating_duration_since(now)).min()
    }

    /// Starts the programs whose time has come.
    pub(super) fn start_due(&mut self, shared: &Shared, probe: &dyn Probe, log: &Arc<GameLog>, status: &dyn Fn(Notice)) {
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|(at, _)| *at <= now);
        self.waiting = later;
        for (_, plan) in due {
            let path = &plan.path;
            if !probe.exists(path) {
                status(Notice::warn(format!("Program not found: {path}"), tr!("Program not found: {path}", path)));
                continue;
            }
            if plan.command.program == *path && !probe.is_executable(path) {
                if let Err(e) = shared.spawner.make_executable(path) {
                    log::warn!("chmod {path}: {e}");
                }
            }
            log.info(&format!("Starting {path} alongside the game: {}", log_writer::command_line_for_log(&plan.command)));
            match shared.spawner.spawn_logged(&plan.command, Some(log.clone())) {
                Ok(handle) => self.running.push((path.clone(), handle)),
                Err(e) => status(Notice::warn(
                    format!("could not start {path}: {e}"),
                    tr!("could not start {program}: {error}", program = path, error = e),
                )),
            }
        }
    }

    /// Closes the programs still running: SIGTERM, then SIGKILL after
    /// `grace`. Programs not started yet never will be.
    pub(super) fn stop(&mut self, grace: Duration, log: &GameLog) {
        self.waiting.clear();
        self.running.retain_mut(|(_, h)| matches!(h.poll(), Ok(None)));
        if self.running.is_empty() {
            return;
        }
        for (path, h) in &mut self.running {
            log.info(&format!("Closing {path}: the game ended"));
            h.terminate();
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            self.running.retain_mut(|(_, h)| matches!(h.poll(), Ok(None)));
            if self.running.is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for (_, h) in &mut self.running {
            h.kill();
        }
    }

    /// Tracking stopped without stopping the game: its programs stay too.
    pub(super) fn leave_running(self) {}
}
