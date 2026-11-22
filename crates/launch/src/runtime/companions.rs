//! The programs run alongside a game (see [`crate::companions`]) while it is
//! tracked: started when their delay is up, closed when the game ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_i18n::tr;

use super::spawner::ProcessHandle;
use super::state::Shared;
use crate::log_writer::{self, GameLog};
use crate::plan::{Command, CompanionPlan};
use crate::prepare::Notice;
use crate::probe::Probe;
use crate::procs::{Procs, WINE_SERVICES};

/// How long the game must be gone from its prefix before it counts as over
/// (a launcher may hand over to the game).
const GONE_FOR: Duration = Duration::from_secs(3);

/// A program that hands its work to an instance already running exits right
/// after it starts, and with success. One that exits later, or with an error,
/// crashed or was closed: there is no other instance holding its work, and
/// its [`CompanionPlan::undo`] would only start a new one.
const HANDS_OVER_WITHIN: Duration = Duration::from_secs(30);

/// How long a program with an [`CompanionPlan::undo`] may still be handing
/// its work over when the game ends: closing it then would cut the handover
/// short and leave the work in the other instance. Bounds the wait at the end.
const HANDOVER_TAKES: Duration = Duration::from_secs(5);

pub(super) struct Companions<'a> {
    waiting: Vec<(Instant, &'a CompanionPlan)>,
    /// The game's process, when it runs in the sandbox: the programs join
    /// the sandbox below it (see [`crate::sandbox::join`]).
    sandbox_root: Option<u32>,
    started: Instant,
    /// The programs started, with the time each one started.
    running: Vec<(&'a CompanionPlan, Box<dyn ProcessHandle>, Instant)>,
    /// Exited by themselves with something to take back when the game ends
    /// ([`CompanionPlan::undo`]).
    handed_over: Vec<&'a CompanionPlan>,
    /// Ran as the instance itself and then crashed or was closed: nothing of
    /// their [`CompanionPlan::undo`] is left to take back.
    stayed: Vec<&'a CompanionPlan>,
    /// The game was seen running in its prefix.
    seen: bool,
    /// Since when the game is not in its prefix any more.
    gone_since: Option<Instant>,
}

impl<'a> Companions<'a> {
    /// The game started at `started`; `sandbox_root`: its process, when it
    /// runs in the sandbox.
    pub(super) fn new(plans: &'a [CompanionPlan], started: Instant, sandbox_root: Option<u32>) -> Self {
        Self {
            waiting: plans.iter().map(|p| (started + p.delay, p)).collect(),
            sandbox_root,
            started,
            running: Vec::new(),
            handed_over: Vec::new(),
            stayed: Vec::new(),
            seen: false,
            gone_since: None,
        }
    }

    /// Some program is waiting for its turn or still running.
    pub(super) fn active(&mut self) -> bool {
        self.reap();
        !self.waiting.is_empty() || !self.running.is_empty()
    }

    /// Forgets the programs that exited. Of those with something to take
    /// back, notes which handed their work over (see [`HANDS_OVER_WITHIN`])
    /// and which were the instance itself: ran past [`HANDOVER_TAKES`] or
    /// were killed. One that failed right away was neither and held nothing.
    fn reap(&mut self) {
        let (handed_over, stayed) = (&mut self.handed_over, &mut self.stayed);
        self.running.retain_mut(|(plan, h, since)| {
            let exit = match h.poll() {
                Ok(None) => return true,
                Ok(Some(exit)) => Some(exit),
                Err(_) => None,
            };
            if plan.undo.is_some() {
                let ran = since.elapsed();
                match exit {
                    Some(e) if e.code == Some(0) && ran <= HANDS_OVER_WITHIN => handed_over.push(plan),
                    // Also the instance itself when it dies this early with an
                    // error code (no signals on Windows): the unload may then
                    // find none and start an idle one. Rare and harmless,
                    // where counting it would block every unload.
                    Some(e) if e.signal.is_none() && ran < HANDOVER_TAKES => {}
                    _ => stayed.push(plan),
                }
            }
            false
        });
    }

    /// Lets the programs that may still be handing their work over (see
    /// [`HANDOVER_TAKES`]) finish before they are closed.
    fn let_handovers_finish(&mut self) {
        let pending = |running: &[(&CompanionPlan, Box<dyn ProcessHandle>, Instant)]| {
            running.iter().filter(|(p, _, since)| p.undo.is_some() && since.elapsed() < HANDOVER_TAKES).map(|(_, _, since)| *since).max()
        };
        while let Some(last) = pending(&self.running) {
            if last.elapsed() >= HANDOVER_TAKES {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            self.reap();
        }
    }

    /// The [`CompanionPlan::undo`] commands to run now, each once: the ones
    /// of programs that handed their work over, unless a program with the
    /// same one runs as the instance itself (it is closed, which takes the
    /// work with it) or did and crashed (it held the work, which is gone).
    fn undos_due(&mut self) -> Vec<(&'a CompanionPlan, &'a Command)> {
        let mut due: Vec<(&'a CompanionPlan, &'a Command)> = Vec::new();
        for plan in std::mem::take(&mut self.handed_over) {
            let Some(undo) = plan.undo.as_ref() else { continue };
            let kept = self.running.iter().map(|(p, _, _)| *p).chain(self.stayed.iter().copied()).any(|p| p.undo.as_ref() == Some(undo));
            if !kept && !due.iter().any(|(_, u)| *u == undo) {
                due.push((plan, undo));
            }
        }
        due
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
            .chain(self.running.iter().map(|(p, _, _)| p.path.as_str()))
            .filter_map(|p| p.rsplit('/').next())
            .map(str::to_lowercase)
            .collect();
        procs.windows_programs(prefix).iter().any(|name| !WINE_SERVICES.contains(&name.as_str()) && !ours.contains(name))
    }

    /// How long until the next one is due (`None`: none left to start).
    pub(super) fn next_due(&self, now: Instant) -> Option<Duration> {
        self.waiting.iter().map(|(at, _)| at.saturating_duration_since(now)).min()
    }

    /// Starts the programs whose time has come. Every runtime calls it on
    /// each poll, so it also notes, right when they exit, the programs that
    /// exited (see [`Self::reap`]).
    pub(super) fn start_due(&mut self, shared: &Shared, probe: &dyn Probe, log: &Arc<GameLog>, status: &dyn Fn(Notice)) {
        self.reap();
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|(at, _)| *at <= now);
        self.waiting = later;
        // Looked up once for all of them (it reads every process).
        let mut sandbox: Option<Option<u32>> = None;
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
            let command = match self.sandbox_root.filter(|_| !plan.on_host) {
                None => plan.command.clone(),
                Some(root) => match *sandbox.get_or_insert_with(|| crate::sandbox::find_sandbox(&*shared.procs, root)) {
                    Some(pid) => crate::sandbox::join(&plan.command, pid),
                    // The sandbox may still be starting.
                    None if now.duration_since(self.started) < crate::sandbox::FOUND_WITHIN => {
                        self.waiting.push((now + Duration::from_secs(1), plan));
                        continue;
                    }
                    None => {
                        status(Notice::warn(
                            format!("{path} did not start: the game's sandbox was not found"),
                            tr!("{path} did not start: the game's sandbox was not found", path),
                        ));
                        continue;
                    }
                },
            };
            log.info(&format!("Starting {path} alongside the game: {}", log_writer::command_line_for_log(&command)));
            match shared.spawner.spawn_logged(&command, Some(log.clone())) {
                Ok(handle) => self.running.push((plan, handle, Instant::now())),
                Err(e) => status(Notice::warn(
                    format!("could not start {path}: {e}"),
                    tr!("could not start {program}: {error}", program = path, error = e),
                )),
            }
        }
    }

    /// Closes the programs still running: SIGTERM, then SIGKILL after the
    /// kill grace. The ones that handed their work over get it taken back.
    /// Programs not started yet never will be.
    pub(super) fn stop(&mut self, shared: &Shared, log: &Arc<GameLog>) {
        self.waiting.clear();
        self.reap();
        self.let_handovers_finish();
        for (plan, undo) in self.undos_due() {
            log.info(&format!("Taking back {}: the game ended ({})", plan.path, log_writer::command_line_for_log(undo)));
            // Not waited for: it only passes a request to the running instance.
            if let Err(e) = shared.spawner.spawn_logged(undo, Some(log.clone())) {
                log.warning(&format!("could not take back {}: {e}", plan.path));
            }
        }
        if self.running.is_empty() {
            return;
        }
        for (plan, h, _) in &mut self.running {
            log.info(&format!("Closing {}: the game ended", plan.path));
            h.terminate();
        }
        let deadline = Instant::now() + shared.cfg.kill_grace;
        while Instant::now() < deadline {
            self.running.retain_mut(|(_, h, _)| matches!(h.poll(), Ok(None)));
            if self.running.is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for (_, h, _) in &mut self.running {
            h.kill();
        }
    }

    /// Tracking stopped without stopping the game: its programs stay too.
    pub(super) fn leave_running(self) {}
}
