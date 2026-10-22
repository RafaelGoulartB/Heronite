//! Quit requests (with the confirmation) and the graceful quit.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::hub;
use super::window::{close_window_keep_alive, show_window};

/// What a quit would leave behind: the games the launcher is tracking
/// (titles, sorted), whether post-game work (backups, cloud saves) is still
/// running, and the game being installed from a script.
struct Pending {
    titles: Vec<String>,
    busy: bool,
    install: Option<String>,
}

impl Pending {
    fn nothing(&self) -> bool {
        self.titles.is_empty() && !self.busy && self.install.is_none()
    }
}

fn pending_work(cx: &App) -> Pending {
    let hub = hub(cx);
    let (running, runtime) = {
        let s = hub.0.borrow();
        (s.running.keys().cloned().collect::<Vec<_>>(), s.runtime.clone())
    };
    let mut titles: Vec<String> =
        running.into_iter().map(|app| runtime.session_info(&app).map(|i| i.title).filter(|t| !t.is_empty()).unwrap_or(app)).collect();
    titles.sort();
    Pending { titles, busy: crate::services::session::busy(), install: crate::services::script_install::running_title(cx) }
}

/// Opens the quit confirmation in the existing main window.
fn ask_quit_confirm(cx: &mut App, pending: Pending) -> bool {
    let hub = hub(cx);
    let (view, tray) = {
        let s = hub.0.borrow();
        (s.window.as_ref().map(|w| w.view.clone()), s.has_tray())
    };
    let Some(view) = view else { return false };
    view.update(cx, |v, cx| v.system_quit_confirm(pending.titles, pending.busy, pending.install, tray, cx)).is_ok()
}

/// The close button was pressed (called by the platform before the window
/// goes away). Returns whether it may close. The window stays when it was
/// meant to quit the app while games run: the confirmation opens in it.
pub(super) fn allow_user_close(cx: &mut App) -> bool {
    let hub = hub(cx);
    let stays_alive = {
        let mut s = hub.0.borrow_mut();
        if s.quitting {
            return true;
        }
        let settings = s.settings();
        s.has_tray() && (s.hiding || settings.exit_to_tray)
    };
    if stays_alive {
        return true;
    }
    let pending = pending_work(cx);
    if pending.nothing() {
        return true;
    }
    log::info!(
        "close requested while {} game(s) run (post-game work: {}, install: {:?}): asking",
        pending.titles.len(),
        pending.busy,
        pending.install
    );
    // Never `show_window` from here: the window is being updated right now.
    ask_quit_confirm(cx, pending);
    false
}

/// A user's quit (tray menu, a window closed by other means). Asks first when
/// games run or post-game work is in flight; otherwise quits.
pub fn request_quit(cx: &mut App) {
    if hub(cx).0.borrow().quitting {
        return;
    }
    let pending = pending_work(cx);
    if pending.nothing() {
        return graceful_quit(cx);
    }
    log::info!(
        "quit requested while {} game(s) run (post-game work: {}, install: {:?}): asking",
        pending.titles.len(),
        pending.busy,
        pending.install
    );
    // The window may be hidden (tray): bring it up for the question.
    let asked = show_window(cx).is_some() && ask_quit_confirm(cx, pending);
    if !asked {
        log::warn!("could not show the quit confirmation: quitting");
        graceful_quit(cx);
    }
}

/// The confirmation's "Quit anyway".
pub fn quit_anyway(cx: &mut App) {
    graceful_quit(cx);
}

/// The confirmation's "Keep running in tray": the window goes, the app and
/// the games stay.
pub fn keep_in_tray(cx: &mut App) {
    // The click that asked is being handled inside the window: close it after.
    cx.defer(|cx| {
        if !close_window_keep_alive(cx) {
            log::warn!("could not hide the window to the tray");
        }
    });
}

/// The updated AppImage to start once the app has quit.
static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// What a restart would leave behind now: a running game or post-game work
/// (`None` = the app can restart).
pub fn restart_blocker(cx: &App) -> Option<String> {
    cx.try_global::<super::Hub>()?;
    let Pending { titles, busy, install } = pending_work(cx);
    match titles.as_slice() {
        [] if busy => Some(tr!("Saving after your game").into()),
        [] => install.map(|title| tr!("{title} is being installed", title)),
        [one] => Some(tr!("{title} is running", title = one)),
        _ => Some(trn!(titles.len(), "{n} game is running", "{n} games are running")),
    }
}

/// Quits gracefully and starts `appimage` (the installed update) at the end.
/// Refused while a game runs or post-game work is in flight.
pub fn restart_into(appimage: PathBuf, cx: &mut App) {
    if hub(cx).0.borrow().quitting {
        return;
    }
    if let Some(why) = restart_blocker(cx) {
        log::warn!("restart for the update refused: {why}");
        return;
    }
    log::info!("restarting into {}", appimage.display());
    *RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(appimage);
    graceful_quit(cx);
}

/// Starts the updated AppImage if a restart asked for it. Called last, once
/// the single-instance socket is gone, so the new process becomes the
/// instance (and by the quit watchdog).
pub fn relaunch_if_requested() {
    let Some(path) = RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    match heronite_update::relaunch::relaunch(&path) {
        Ok(pid) => log::info!("started {} (pid {pid})", path.display()),
        Err(e) => log::error!("could not start {}: {e}", path.display()),
    }
}

/// How long the Epic queue gets to stop its process after the pause request.
const EPIC_STOP_WAIT: Duration = Duration::from_secs(5);
/// A quit that has not finished by then is forced (a stuck step).
const QUIT_WATCHDOG: Duration = Duration::from_secs(15);

/// Ends the app in order, without asking, and never blocks the UI thread:
/// stop accepting second invocations, save the running games' playtime, pause
/// the Epic queue (its element stays queued and resumes at the next start),
/// shut the tray, then end the loop. Games keep running. A second call is a
/// no-op; a signal while this runs ends the process at once.
pub(super) fn graceful_quit(cx: &mut App) {
    let hub = hub(cx);
    let (runtime, tray) = {
        let mut s = hub.0.borrow_mut();
        if s.quitting {
            return;
        }
        s.quitting = true;
        (s.runtime.clone(), s.tray.take())
    };
    log::info!("quitting");
    crate::system::signals::quit_started();
    // A second process from now on finds no socket instead of a dying listener.
    crate::system::single_instance::close_active();
    if let Some(t) = tray {
        t.shutdown();
    }
    // Stops the current encode at once; the next start resumes the queue.
    crate::services::compression::shutdown(cx);
    // The replay recorder ends (it saves nothing).
    crate::services::replay::shutdown(cx);
    let hotkey = crate::overlay::take_hotkey(cx);
    let downloads = crate::services::try_downloads(cx);
    if let Some(d) = &downloads {
        // Only asks (legendary gets SIGTERM); waited for below.
        d.update(cx, |d, _| d.pause_for_shutdown());
    }
    // If a step hangs, the process still ends.
    let _ = std::thread::Builder::new().name("quit-watchdog".into()).spawn(|| {
        std::thread::sleep(QUIT_WATCHDOG);
        log::error!("the quit did not finish in {QUIT_WATCHDOG:?}: exiting");
        relaunch_if_requested();
        std::process::exit(0);
    });
    cx.spawn(async move |cx| {
        // Removes the overlay's keyboard shortcut from the desktop.
        if let Some(hotkey) = hotkey {
            cx.background_spawn(async move { drop(hotkey) }).await;
        }
        let saved = cx.background_spawn(async move { runtime.checkpoint_all() }).await;
        if saved > 0 {
            log::info!("saved the playtime of {saved} running game(s)");
        }
        if let Some(d) = downloads {
            let deadline = Instant::now() + EPIC_STOP_WAIT;
            while !cx.update(|cx| d.read(cx).queue_idle()) {
                if Instant::now() >= deadline {
                    log::warn!("the Epic queue did not stop in {EPIC_STOP_WAIT:?}; legendary is stopped when the app exits");
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
        }
        log::info!("goodbye");
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// The post-game work of a session finished: a headless launch can exit now.
pub fn post_game_idle(cx: &mut App) {
    maybe_exit_idle(cx);
}

pub(super) fn maybe_exit_idle(cx: &mut App) {
    let hub = hub(cx);
    let idle = {
        let s = hub.0.borrow();
        s.exit_when_idle && s.window.is_none() && s.running.is_empty() && !crate::services::session::busy()
    };
    if idle {
        log::info!("game ended and no window was shown: exiting");
        graceful_quit(cx);
    }
}
