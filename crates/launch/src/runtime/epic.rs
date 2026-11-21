//! An Epic game (`ProcessPlan::epic`): Epic signs in and completes the
//! game's command right before it starts (see [`crate::epic_launch`]).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use heronite_i18n::tr;

use super::process::finish_aborted;
use super::session::{fail, finish};
use super::state::{Control, Shared};
use super::{ExitInfo, LaunchRequest};
use crate::epic_launch::{self as el, EpicLaunch, EpicService, Failure, Resolved, SystemEpic};
use crate::log_writer::GameLog;
use crate::plan::Command;
use crate::prepare::Notice;

/// The game's command, or `None` when the session already ended: the launch
/// was stopped, Epic refused, or the game is one of the EA app / Ubisoft
/// Connect that the store's app opens on Windows (its play time is not
/// followed).
pub(super) fn ask_epic(
    shared: &Shared,
    log: &Arc<GameLog>,
    req: &LaunchRequest,
    control: &Control,
    command: &Command,
    launch: &EpicLaunch,
    status: &dyn Fn(Notice),
) -> Option<Command> {
    if !launch.offline {
        status(Notice::info("Logging in to Epic Games...", tr!("Logging in to Epic Games...")));
    }
    log.info(&format!("Asking Epic Games how {} starts{}", req.title, if launch.offline { " (offline)" } else { "" }));
    let service: Arc<dyn EpicService> = match shared.epic_service.lock().unwrap().clone() {
        Some(s) => s,
        None => Arc::new(SystemEpic::new(&shared.paths.legendary_config_dir())),
    };
    let cancelled = || control.abort.load(Ordering::SeqCst) || control.kill.load(Ordering::SeqCst);
    match el::resolve_cancellable(command, launch, service, &cancelled) {
        Ok(Resolved::Game(game)) => Some(game),
        Ok(Resolved::Link(uri)) => {
            let client = el::link_client(&uri);
            if !el::link_handler_installed(&uri) {
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
