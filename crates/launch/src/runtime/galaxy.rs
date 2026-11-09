//! Comet beside GOG games (see [`crate::galaxy`]): started right before a
//! game, shared by the GOG games that run at the same time, stopped once the
//! last of them is gone. Nothing here fails a launch, and every step that
//! could hang has a limit (the account, the download, the prefix's Wine
//! commands, the start): past it the game runs without Comet, as it would
//! without GOG Galaxy.
//!
//! Comet is started and waited for by a thread of its own, which lives as
//! long as it does: on Linux it gets Ctrl+C when that thread dies
//! ([`super::Spawner::spawn_helper`]), so it uploads and exits with the app
//! too (a GOG game still running then loses it for the rest of its session,
//! rather than leaving Comet behind with the account's tokens). Its output
//! goes to `logs/runners/comet.log` (Heroic's `comet` runner log), not a
//! game's log: it outlives the session that started it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_i18n::tr;

use super::state::Shared;
use crate::galaxy::{self, GalaxyAccount, GalaxyLogin, DOWNLOAD_LIMIT, LOGIN_LIMIT};
use crate::log_writer::{old_path, GameLog, LogPaths};
use crate::plan::{PlanContext, PrepSpec};
use crate::prepare::{Notice, Prep};
use crate::store_tools::COMET;

/// How long Comet gets after Ctrl+C to upload what the games unlocked before
/// it is ended.
const UPLOAD_GRACE: Duration = Duration::from_secs(15);
/// How often its thread looks whether it is still there.
const WATCH: Duration = Duration::from_millis(500);

/// The Comet of this launcher, if one runs.
#[derive(Default)]
pub(super) struct Slot {
    running: Option<Running>,
    /// Until when a Comet asked to stop may still hold the port.
    stopping_until: Option<Instant>,
}

struct Running {
    id: u64,
    /// The games that use it.
    users: usize,
    /// Asks its thread to stop it.
    stop: mpsc::Sender<()>,
    /// Comet still runs (it may end by itself).
    alive: Arc<AtomicBool>,
}

/// One game's use of the shared Comet, given back with [`stop`].
#[must_use]
pub(super) struct CometUse {
    id: u64,
}

/// Starts Comet for a game whose plan asks for it, when a GOG account is
/// signed in and Comet is on, or joins the one another GOG game started.
/// `None`: the game runs without it (why is in the game's log).
pub(super) fn start(shared: &Shared, log: &Arc<GameLog>, prep: &Prep, spec: &PrepSpec) -> Option<CometUse> {
    if !spec.galaxy {
        return None;
    }
    let account = shared.galaxy.lock().unwrap().clone()?;
    // The Comet of a game that just ended may still be uploading: it frees
    // the port when done (waited for with the lock released).
    let stopping = shared.comet.lock().unwrap().stopping_until;
    if let Some(until) = stopping {
        galaxy::wait_for_port(until.saturating_duration_since(Instant::now()), prep.cancel);
    }
    // No need to ask gogdl for tokens or download anything when the game
    // is served already.
    if let Some(served) = served(&mut shared.comet.lock().unwrap(), log) {
        return served;
    }
    let login = match login_within(account, LOGIN_LIMIT + Duration::from_secs(5)) {
        Ok(Some(login)) => login,
        Ok(None) => return None,
        Err(e) => {
            log.warning(&format!("GOG achievements will not unlock while the game runs: {e}"));
            return None;
        }
    };
    if COMET.find(&PlanContext::load(prep.paths), prep.probe).is_none() {
        prep.notice(Notice::info("Downloading Comet for GOG achievements...", tr!("Downloading Comet for GOG achievements...")));
    }
    let (bin, dummy) = match tools(shared, prep, spec.wine.is_some()) {
        Ok(found) => found,
        Err(e) => {
            log.warning(&format!("GOG achievements will not unlock while the game runs: {e}"));
            return None;
        }
    };
    if let (Some(settings), Some(dummy)) = (&spec.wine, dummy) {
        prep.galaxy_service_in_prefix(settings, &dummy);
    }
    // Checked again under the lock that stays until Comet runs: another GOG
    // game may have started one meanwhile.
    let mut slot = shared.comet.lock().unwrap();
    if let Some(served) = served(&mut slot, log) {
        return served;
    }
    let log_file = LogPaths::for_data(prep.paths).runner("comet");
    log.info(&format!("Starting Comet, GOG Galaxy's service for the game's achievements (its log: {})", log_file.display()));
    match spawn(shared, &bin, &login, &log_file) {
        Ok(running) => {
            let id = running.id;
            slot.running = Some(running);
            Some(CometUse { id })
        }
        Err(e) => {
            log.warning(&format!("could not start Comet: {e}"));
            None
        }
    }
}

/// [`GalaxyAccount::login`] on a thread of its own, given up after `limit`
/// whatever the account does.
fn login_within(account: Arc<dyn GalaxyAccount>, limit: Duration) -> Result<Option<GalaxyLogin>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("comet-login".into())
        .spawn(move || {
            let _ = tx.send(account.login());
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(limit).map_err(|_| format!("the GOG account did not answer within {} s", limit.as_secs()))?
}

/// Comet (and, with `dummy`, its dummy service for a Wine prefix), found or
/// downloaded on a thread of its own: the launch waits for it at most
/// [`DOWNLOAD_LIMIT`] (a stalled download never returns), and a download
/// still going then finishes for the next launch.
fn tools(shared: &Shared, prep: &Prep, dummy: bool) -> Result<(String, Option<PathBuf>), String> {
    let services = shared.services.lock().unwrap().clone();
    let paths = prep.paths.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let work = move || {
        let bin = COMET.ensure(&paths, &*services.probe, &*services.net, &|| false);
        let dummy = bin.is_ok() && dummy;
        let dir = dummy.then(|| galaxy::ensure_dummy_service(&paths, &*services.net, &|| false));
        let _ = tx.send((bin, dir));
    };
    std::thread::Builder::new().name("comet-download".into()).spawn(work).map_err(|e| e.to_string())?;
    let until = Instant::now() + DOWNLOAD_LIMIT;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((bin, dir)) => {
                let bin = bin?.to_string_lossy().into_owned();
                let dir = dir.and_then(|d| d.inspect_err(|e| log::warn!("{e}")).ok());
                return Ok((bin, dir));
            }
            Err(RecvTimeoutError::Timeout) if (prep.cancel)() => return Err("the launch was stopped".into()),
            Err(RecvTimeoutError::Timeout) if Instant::now() >= until => {
                return Err(format!("Comet was not downloaded within {} s (it goes on for the next launch)", DOWNLOAD_LIMIT.as_secs()))
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("getting Comet failed".into()),
        }
    }
}

/// Whether the game is served without a Comet of its own: `Some` with the
/// Comet of another GOG game it joins, or `Some(None)` when something else
/// holds the port (GOG Galaxy, another launcher's Comet). `None`: start one.
fn served(slot: &mut Slot, log: &GameLog) -> Option<Option<CometUse>> {
    if let Some(r) = slot.running.as_mut().filter(|r| r.alive.load(Ordering::SeqCst)) {
        r.users += 1;
        log.info("Comet already runs for another GOG game: the game uses it");
        return Some(Some(CometUse { id: r.id }));
    }
    if galaxy::port_taken() {
        log.info("GOG Galaxy's service already runs on this computer (GOG Galaxy, or Comet from another launcher): the game uses it");
        return Some(None);
    }
    None
}

/// Starts Comet on a thread that owns it until it ends: asked to stop, it
/// gets Ctrl+C, then is ended after [`UPLOAD_GRACE`].
fn spawn(shared: &Shared, bin: &str, login: &GalaxyLogin, log_file: &Path) -> Result<Running, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    rotate(log_file, &galaxy::log_line(bin, login));
    let cmd = galaxy::comet_command(bin, login);
    let spawner = shared.spawner.clone();
    let log_file: PathBuf = log_file.to_path_buf();
    let alive = Arc::new(AtomicBool::new(true));
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let still = alive.clone();
    let body = move || {
        let mut handle = match spawner.spawn_helper(&cmd, Some(&log_file)) {
            Ok(h) => h,
            Err(e) => {
                still.store(false, Ordering::SeqCst);
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));
        loop {
            match stop_rx.recv_timeout(WATCH) {
                Err(RecvTimeoutError::Timeout) if matches!(handle.poll(), Ok(None)) => {}
                Err(RecvTimeoutError::Timeout) => {
                    log::warn!("Comet ended by itself (see {})", log_file.display());
                    break;
                }
                // Asked to stop, or the launcher is gone.
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    handle.interrupt();
                    let until = Instant::now() + UPLOAD_GRACE;
                    while Instant::now() < until && matches!(handle.poll(), Ok(None)) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if matches!(handle.poll(), Ok(None)) {
                        log::warn!("Comet did not stop in time: ended");
                        handle.kill();
                    }
                    break;
                }
            }
        }
        still.store(false, Ordering::SeqCst);
    };
    std::thread::Builder::new().name("comet".into()).spawn(body).map_err(|e| e.to_string())?;
    // Starting a program takes milliseconds: a spawn stuck longer is given up.
    ready_rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "Comet did not start".to_string())??;
    Ok(Running { id: NEXT_ID.fetch_add(1, Ordering::SeqCst), users: 1, stop: stop_tx, alive })
}

/// Starts Comet's log afresh (the last one becomes `.old`) with the command
/// it runs, its tokens hidden.
fn rotate(log_file: &Path, line: &str) {
    if let Some(dir) = log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::rename(log_file, old_path(log_file));
    if let Err(e) = std::fs::write(log_file, format!("Launching: {line}\n\n")) {
        log::debug!("{}: {e}", log_file.display());
    }
}

/// A game is done with Comet: the last one stops it (Ctrl+C, so it uploads
/// what is left). Returns at once: the wait is not the game's time.
pub(super) fn stop(shared: &Shared, used: CometUse, log: &GameLog) {
    let mut slot = shared.comet.lock().unwrap();
    let Some(r) = slot.running.as_mut().filter(|r| r.id == used.id) else { return };
    r.users = r.users.saturating_sub(1);
    if r.users > 0 {
        return;
    }
    let r = slot.running.take().expect("checked above");
    if r.alive.load(Ordering::SeqCst) {
        log.info("Stopping Comet: the game ended");
        let _ = r.stop.send(());
        slot.stopping_until = Some(Instant::now() + UPLOAD_GRACE + Duration::from_secs(1));
    }
}
