//! An Epic game on Windows (`ProcessPlan::ask_legendary`): legendary signs in
//! and names the game's program before it starts (see
//! [`crate::legendary_windows`]).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use heronite_i18n::tr;

use super::process::finish_aborted;
use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{ExitInfo, LaunchRequest};
use crate::legendary_windows::{self as lw, Failure, Resolved};
use crate::log_writer::GameLog;
use crate::plan::Command;
use crate::prepare::Notice;

/// The game's program, or `None` when the session already ended: the
/// launch was stopped, legendary failed, or the game is one of the EA app /
/// Ubisoft Connect, which the store's app opens (its play time is not
/// followed).
pub(super) fn ask_legendary(
    shared: &Shared,
    log: &Arc<GameLog>,
    req: &LaunchRequest,
    control: &Control,
    command: &Command,
    status: &dyn Fn(Notice),
) -> Option<Command> {
    status(Notice::info("Logging in to Epic Games...", tr!("Logging in to Epic Games...")));
    log.info(&format!("Asking legendary how {} starts: {}", req.title, command.display()));
    let cancelled = || control.abort.load(Ordering::SeqCst) || control.kill.load(Ordering::SeqCst);
    match lw::resolve(command, &cancelled) {
        Ok(Resolved::Game(game)) => Some(game),
        Ok(Resolved::Link(uri)) => {
            let client = lw::link_client(&uri);
            if !lw::link_handler_installed(&uri) {
                let notice = Notice::warn(
                    format!("{client} is not installed: it opens this game"),
                    tr!("{client} opens this game, and it is not installed. Install {client}, then play again.", client),
                );
                fail(shared, log, req, notice);
                return None;
            }
            if let Err(e) = shared.spawner.open_uri(&uri) {
                let notice =
                    Notice::warn(format!("Could not open {client}: {e}"), tr!("Could not open {client}: {error}", client, error = e));
                fail(shared, log, req, notice);
                return None;
            }
            log.info(&format!("{client} was asked to start the game; its play time is not followed"));
            status(Notice::info(format!("{client} starts the game"), tr!("{client} starts the game", client)));
            finish(shared, log, req, control, Duration::ZERO, Some(ExitInfo { code: Some(0), signal: None }));
            None
        }
        Err(Failure::Cancelled) => {
            finish_aborted(shared, log, req);
            None
        }
        Err(Failure::Message(message)) => {
            fail(shared, log, req, Notice::warn(message.clone(), message));
            None
        }
    }
}
