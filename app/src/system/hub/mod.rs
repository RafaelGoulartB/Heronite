//! Application lifecycle: the main window (created on demand and dropped when
//! it is closed to the tray), the game runtime shared by every window, launch
//! requests from the command line / protocol URLs / other instances / the
//! tray, and when the process quits.
//!
//! Quitting has two parts. A *request* (window closed without a tray to stay
//! in, tray "Quit") asks first when games are running or post-game work is in
//! flight ([`request_quit`]). The *graceful quit* ([`graceful_quit`], also run
//! by the signals and by the confirmation's "Quit anyway") saves the running
//! games' playtime, pauses the Epic queue in order, releases the
//! single-instance socket and the tray, and only then ends the loop. Games are
//! independent processes: quitting never stops them.
//!
//! State lives in a global [`Hub`] (`Rc<RefCell<..>>`): borrow it only for
//! short scopes and never while calling into GPUI (opening a window builds
//! the Collection, which calls back into [`attach_runtime`]).
//!
//! Split by responsibility: `start` (setup), `window` (main window), `quit`
//! (quit requests and the graceful quit), `events` (launch events, tray and
//! second-instance messages), `launch` (launch requests) and `debug`
//! (`HERONITE_DEBUG_*` hooks).

mod debug;
mod events;
mod launch;
mod quit;
mod start;
mod window;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_launch::LaunchEvent;

use super::settings::SysSettings;
use super::tray::TrayHandle;
use crate::collection::CollectionView;
use crate::runtime::GameRuntime;

pub use launch::candidate_keys;
pub use quit::{keep_in_tray, post_game_idle, quit_anyway, relaunch_if_requested, request_quit, restart_blocker, restart_into};
pub use start::init;
pub use window::has_window;

/// Process exit code (non-zero when a launch from outside failed).
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

pub fn exit_code() -> i32 {
    EXIT_CODE.load(Ordering::Relaxed)
}

struct MainWindow {
    handle: AnyWindowHandle,
    view: WeakEntity<CollectionView>,
}

struct State {
    data_paths: DataPaths,
    runtime: Arc<GameRuntime>,
    window: Option<MainWindow>,
    /// The open Collection's channel for launch events.
    subscriber: Option<UnboundedSender<LaunchEvent>>,
    /// Games started by this process (pid once known).
    running: HashMap<String, Option<u32>>,
    tray: Option<TrayHandle>,
    /// `HERONITE_DEBUG_FAKE_TRAY`: behave as if a tray icon existed (measure
    /// the app closed to the tray without putting an icon in the user's bar).
    fake_tray: bool,
    /// Quit when the last game ends and no window was shown (started only to
    /// launch a game without UI).
    exit_when_idle: bool,
    /// The next window close is ours (tray / minimize on launch): stay alive.
    hiding: bool,
    /// The graceful quit began: no new windows, requests or quits.
    quitting: bool,
    fake_launch: bool,
    /// `--fullscreen` was asked for: every window of this run opens fullscreen.
    fullscreen: bool,
    settings: SysSettings,
    settings_stamp: Option<SystemTime>,
}

impl State {
    fn has_tray(&self) -> bool {
        self.tray.is_some() || self.fake_tray
    }

    /// Settings, re-read only when the file changed (a `stat` per call).
    fn settings(&mut self) -> SysSettings {
        let stamp = self.data_paths.global_config().metadata().and_then(|m| m.modified()).ok();
        if stamp != self.settings_stamp {
            self.settings = SysSettings::load(&self.data_paths);
            self.settings_stamp = stamp;
        }
        self.settings
    }
}

#[derive(Clone)]
pub struct Hub(Rc<RefCell<State>>);
impl Global for Hub {}

fn hub(cx: &App) -> Hub {
    cx.global::<Hub>().clone()
}

/// Rebuilds the tray menu (its labels follow the interface language).
pub fn refresh_tray(cx: &App) {
    if let Some(hub) = cx.try_global::<Hub>() {
        if let Some(tray) = &hub.0.borrow().tray {
            tray.refresh();
        }
    }
}

/// Called by `CollectionView::new`: the shared runtime plus a fresh stream of
/// launch events (games already running are announced first).
pub fn attach_runtime(data_paths: &DataPaths, cx: &mut App) -> (Arc<GameRuntime>, UnboundedReceiver<LaunchEvent>) {
    let Some(hub) = cx.try_global::<Hub>().cloned() else {
        let (runtime, rx) = GameRuntime::new(data_paths.clone());
        return (Arc::new(runtime), rx);
    };
    let (tx, rx) = unbounded();
    let mut s = hub.0.borrow_mut();
    for (app_name, pid) in &s.running {
        let _ = tx.unbounded_send(LaunchEvent::Launching { app_name: app_name.clone() });
        if let Some(pid) = pid {
            let _ = tx.unbounded_send(LaunchEvent::Running { app_name: app_name.clone(), pid: *pid });
        }
    }
    s.subscriber = Some(tx);
    (s.runtime.clone(), rx)
}
