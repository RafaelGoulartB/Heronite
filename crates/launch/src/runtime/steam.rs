//! Steam games: open the `steam://` URI and follow the game through Steam's logs.

use std::sync::atomic::Ordering;
use std::time::Instant;

use chrono::Utc;

use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{LaunchEvent, LaunchRequest};
use crate::log_writer::GameLog;
use crate::playtime_watch::{wait_for_steam_session, SteamWaitOutcome};
use crate::record;

pub(super) fn run_steam(
    shared: &Shared,
    log: &GameLog,
    req: &LaunchRequest,
    control: &Control,
    app_id: &str,
    uri: &str,
    started_at: chrono::DateTime<Utc>,
    t0: Instant,
) {
    log.info(&format!("Opening {uri}"));
    if let Err(e) = shared.spawner.open_uri(uri) {
        return fail(shared, log, req, format!("could not open {uri}: {e}"));
    }
    let locator = shared.steam.clone();
    let mut announced: Option<u32> = None;
    let outcome = wait_for_steam_session(
        app_id,
        &*shared.procs,
        &move || locator.log_files(),
        &shared.cfg.steam,
        &control.abort,
        &control.wake,
        &control.steam_pids,
        &mut |pid| {
            announced = Some(pid);
            log.info(&format!("Steam started the game (pid {pid})"));
            control.pid.store(pid, Ordering::SeqCst);
            // Only now do we know the game is really running.
            let _ = record::mark_started(&shared.paths, &req.app_name, started_at);
            control.set_playing();
            shared.send(LaunchEvent::Running { app_name: req.app_name.clone(), pid });
        },
    );
    match outcome {
        SteamWaitOutcome::NotStarted => {
            fail(shared, log, req, format!("Steam did not start {} within {}s", req.title, shared.cfg.steam.start_timeout.as_secs()))
        }
        SteamWaitOutcome::Finished | SteamWaitOutcome::Aborted => {
            log.info("Steam game finished");
            finish(shared, log, req, control, started_at, t0.elapsed(), None)
        }
    }
}
