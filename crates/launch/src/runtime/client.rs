//! Games a store client starts (see [`ClientHandoff`]). The client runs in a
//! Wine prefix of its own and stays open after the game ends, maybe started
//! long before: the session follows the game, not the command that asked for
//! it. That command is the client itself, or a request to the client that
//! already runs; either may outlive the game or exit at once, and neither
//! ends the session.
//!
//! The game is any Windows program running from the game's folder (any in
//! the prefix when the folder is not known) that is not one of the client's. The session clock starts when it
//! shows up. A stop signals only the game's programs.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_i18n::tr;

use super::companions::Companions;
use super::process::{finish_aborted, hold_awake, run_script};
use super::session::{fail, finish};
use super::spawner::ProcessHandle;
use super::state::{Control, Shared};
use super::{ExitInfo, LaunchEvent, LaunchRequest};
use crate::keep_awake::Hold;
use crate::log_writer::{self, GameLog};
use crate::plan::{ClientHandoff, Handoff, ProcessPlan};
use crate::prepare::{Notice, Prep};
use crate::probe::Probe;
use crate::procs::{windows_path_under, Procs, Sig};
use crate::record;

/// A launch that has run every preparation step (`run_process`).
pub(super) struct Session<'a> {
    pub(super) shared: &'a Shared,
    pub(super) log: &'a Arc<GameLog>,
    pub(super) req: &'a LaunchRequest,
    pub(super) control: &'a Control,
    pub(super) plan: &'a ProcessPlan,
    pub(super) probe: &'a dyn Probe,
    pub(super) prep: &'a Prep<'a>,
    pub(super) status: &'a dyn Fn(Notice),
}

/// How the wait for the game, and the game, ended.
enum End {
    /// The game is gone (or was stopped).
    Closed,
    /// Tracking was cleared; the game and its programs are left alone.
    Aborted,
    /// Stopped before the game showed up: nothing was played.
    Stopped,
    /// The game cannot be expected any more.
    GaveUp(Notice),
}

/// What runs in the prefix right now.
#[derive(Default)]
struct Seen {
    /// The game's programs, oldest first.
    game: Vec<u32>,
    /// A program of the client (or its helpers) runs.
    client: bool,
    /// The client's main program runs.
    main: bool,
}

/// Tells the game's programs from the client's.
struct Watch<'a> {
    handoff: &'a Handoff,
    client: &'a ClientHandoff,
    /// Programs run alongside the game: neither the game nor the client.
    ours: Vec<String>,
}

impl Watch<'_> {
    fn look(&self, procs: &dyn Procs) -> Seen {
        let mut seen = Seen::default();
        let prefix = self.handoff.prefix.as_deref().unwrap_or_default();
        let folder = self.handoff.folder.as_deref();
        for p in procs.prefix_programs(prefix, folder) {
            let name = p.name.to_lowercase();
            if self.client.programs.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                seen.client = true;
                seen.main |= self.client.main.eq_ignore_ascii_case(&name);
            } else if self.ours.contains(&name) {
                continue;
            } else if folder.is_none_or(|f| p.from_folder || windows_path_under(&p.path, prefix, f)) {
                // With the game's folder known, only what runs from it: the
                // client may run installers (redistributables) first.
                seen.game.push(p.pid);
            }
        }
        seen.game.sort_unstable();
        seen
    }
}

/// What runs while the game does.
struct Play<'a> {
    companions: Companions<'a>,
    awake: Option<Box<dyn ProcessHandle>>,
    held: Option<Hold>,
}

impl Session<'_> {
    pub(super) fn run(&self, handoff: &Handoff, client: &ClientHandoff) {
        let (shared, log, control, plan) = (self.shared, self.log, self.control, self.plan);
        let cfg = shared.cfg.handoff;
        let name = client.name.as_str();
        let ours = plan.companions.iter().filter_map(|c| c.path.rsplit('/').next()).map(str::to_lowercase).collect();
        let watch = Watch { handoff, client, ours };
        if control.stopped() {
            return finish_aborted(shared, log, self.req);
        }

        // A client that already runs gets the request; one that does not is started with it.
        let open = watch.look(&*shared.procs).main;
        let command = if open { &client.join } else { &plan.command };
        if open {
            log.info(&format!("{name} is already open: asking it to start the game"));
        }
        (self.status)(Notice::info(
            format!("Waiting for {name} to start the game..."),
            tr!("Waiting for {client} to start the game…", client = name),
        ));
        log.info(&format!("Launching {} through {name}: {}\n", self.req.title, log_writer::command_line_for_log(command)));
        log.info("Game Output:");
        let mut handle = match shared.spawner.spawn_logged(command, Some(log.clone())) {
            Ok(h) => h,
            Err(e) => {
                let program = &command.program;
                let notice =
                    Notice::warn(format!("could not start {program}: {e}"), tr!("could not start {program}: {error}", program, error = e));
                return fail(shared, log, self.req, notice);
            }
        };
        let spawned = Instant::now();

        let mut play: Option<Play> = None;
        let mut exit: Option<ExitInfo> = None;
        let mut gone_since: Option<Instant> = None;
        let mut stopping: Option<Instant> = None;
        let mut termed = BTreeSet::new();
        let end = loop {
            if control.abort.load(Ordering::SeqCst) {
                break End::Aborted;
            }
            if exit.is_none() {
                match handle.poll() {
                    Ok(Some(info)) => {
                        log.info(&format!("The command {}", describe(info)));
                        exit = Some(info);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::error!("polling {}: {e}", self.req.app_name);
                        exit = Some(ExitInfo::default());
                    }
                }
            }
            let seen = watch.look(&*shared.procs);
            if play.is_none() && !seen.game.is_empty() {
                play = Some(self.game_started(seen.game[0]));
            }
            if control.kill.load(Ordering::SeqCst) {
                // Only the game: the client stays open.
                if stopping.is_none() {
                    log.info("Stopping: asking the game's programs to close");
                }
                let kill_due = stopping.get_or_insert_with(Instant::now).elapsed() > shared.cfg.kill_grace;
                for pid in &seen.game {
                    if kill_due {
                        shared.procs.signal(*pid, Sig::Kill);
                    } else if termed.insert(*pid) {
                        shared.procs.signal(*pid, Sig::Term);
                    }
                }
                if seen.game.is_empty() || kill_due {
                    break if play.is_some() { End::Closed } else { End::Stopped };
                }
            } else if let Some(play) = play.as_mut() {
                if seen.game.is_empty() {
                    if gone_since.get_or_insert_with(Instant::now).elapsed() >= cfg.client_gone {
                        log.info("The game closed");
                        break End::Closed;
                    }
                } else {
                    gone_since = None;
                    play.companions.start_due(shared, self.probe, log, self.status);
                }
            } else if let Some(notice) = self.given_up(name, spawned, exit, &seen) {
                break End::GaveUp(notice);
            }
            let kill_pending = stopping.is_none();
            let wait = play
                .as_ref()
                .and_then(|p| p.companions.next_due(Instant::now()))
                .map_or(shared.cfg.poll_interval, |d| d.min(shared.cfg.poll_interval));
            control
                .wake
                .wait_while(wait, || !control.abort.load(Ordering::SeqCst) && !(kill_pending && control.kill.load(Ordering::SeqCst)));
        };

        // The command is not waited for, killed or signalled: it may be the client.
        let Some(mut play) = play else {
            return match end {
                End::GaveUp(notice) => fail(shared, log, self.req, notice),
                _ => finish_aborted(shared, log, self.req),
            };
        };
        let aborted = matches!(end, End::Aborted);
        if aborted {
            play.companions.leave_running();
        } else {
            play.companions.stop(shared.cfg.kill_grace, log);
        }
        if let Some(h) = play.awake.as_mut() {
            h.terminate();
        }
        play.held = None;
        // The session ends when the game did, not when its grace ran out;
        // saving to the cloud is not playtime either.
        let gone = gone_since.filter(|_| !aborted).map_or(Duration::ZERO, |t| t.elapsed());
        let elapsed = control.played().saturating_sub(gone);
        if !aborted {
            if let Some(script) = &plan.after_script {
                run_script(shared, control, self.probe, script, log, self.status);
            }
            self.prep.after_exit(&self.req.app_name, &plan.prep);
        }
        finish(shared, log, self.req, control, elapsed, None);
    }

    /// The game is running: the session clock starts here, not when the client was asked.
    fn game_started(&self, pid: u32) -> Play<'_> {
        let (shared, log, control, plan) = (self.shared, self.log, self.control, self.plan);
        log.info(&format!("The game started (pid {pid})"));
        control.pid.store(pid, Ordering::SeqCst);
        let started_at = control.set_playing();
        let _ = record::mark_started(&shared.paths, &self.req.app_name, started_at);
        // The next run of the app can pick the game up if this one quits
        // first. It follows one process, the game's oldest.
        if let Some(tracked) = super::adopt::process_entry(shared, pid, plan) {
            super::adopt::remember(shared, control, self.req, tracked);
        }
        shared.send(LaunchEvent::Running { app_name: self.req.app_name.clone(), pid });
        let awake = plan.keep_awake.as_ref().and_then(|cmd| match shared.spawner.spawn(cmd, None) {
            Ok(h) => Some(h),
            Err(e) => {
                log.warning(&format!("Could not keep the computer awake ({}): {e}", cmd.program));
                None
            }
        });
        let held = plan.hold_awake.then(|| hold_awake(log, &self.req.title)).flatten();
        Play { companions: Companions::new(&plan.companions, Instant::now(), None), awake, held }
    }

    /// Why the game cannot be expected any more, if so. Before the game
    /// shows up the client may be updating itself or waiting for the user to
    /// sign in, so only a client that is gone, or a command that failed, ends the wait early.
    fn given_up(&self, name: &str, spawned: Instant, exit: Option<ExitInfo>, seen: &Seen) -> Option<Notice> {
        let cfg = self.shared.cfg.handoff;
        if spawned.elapsed() >= cfg.client_wait {
            let waited = cfg.client_wait.as_secs();
            let waited = if waited >= 60 { format!("{} min", waited / 60) } else { format!("{waited} s") };
            return Some(Notice::warn(
                format!("The game did not start within {waited}"),
                tr!("{client} did not start the game", client = name),
            ));
        }
        // The command may be the client, still starting.
        let exit = exit.filter(|_| !seen.client)?;
        if exit.signal.is_some() || exit.code.is_some_and(|c| c != 0) {
            return Some(Notice::warn(
                format!("{name} {} before the game started", describe(exit)),
                tr!("{client} closed with an error before it started the game", client = name),
            ));
        }
        (spawned.elapsed() >= cfg.client_settle).then(|| {
            Notice::warn(
                format!("{name} was closed before the game started"),
                tr!("{client} was closed before it started the game", client = name),
            )
        })
    }
}

fn describe(info: ExitInfo) -> String {
    match (info.code, info.signal) {
        (Some(c), _) => format!("exited with code {c}"),
        (None, Some(s)) => format!("was terminated by signal {s}"),
        _ => "ended".to_string(),
    }
}
