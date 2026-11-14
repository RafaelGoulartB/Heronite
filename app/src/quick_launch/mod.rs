//! The quick launcher: the command palette (Ctrl K) in a small window of its
//! own, opened from anywhere without the main window. Type a few letters,
//! Enter plays the game; Shift Enter shows it in Heronite.
//!
//! It is the same view, ranking and history as Ctrl K
//! (`collection::palette`, `Mode::Launcher`). It runs by itself what needs no
//! main window (play, stop, open the folder, favourites, hide, the logs
//! folder, quit; `collection::palette::launcher::Chosen`); anything else
//! opens Heronite and runs there, marked ↗.
//!
//! Opened by its keyboard shortcut (`hotkey`, none unless set in Settings →
//! Quick launcher), `--quick-launch` and `heronite://quick-launch` (through
//! the running instance's socket; with none running, one starts without its
//! window). The same request closes it when it is open; Esc, running
//! something and leaving it close it too. In console mode in front, it brings
//! console mode forward instead.
//!
//! Split: `window` (the surface on each desktop), `view` (its root view: the
//! palette, the library, the keys), `run` (carrying out a choice) and
//! `hotkey` (the keyboard shortcut).
//!
//! Debug: `HERONITE_DEBUG_QUICK_LAUNCH=<query>` (`-` for none) opens it at
//! start, alone, in a normal window (`scripts/shot.sh` captures it), and
//! `HERONITE_DEBUG_QUICK_LAUNCH_KEYS=down,shift-enter,...` feeds it keys.

mod hotkey;
mod run;
mod view;
mod window;

use std::cell::RefCell;

use gpui_kit::*;

pub use hotkey::{get, init, take_hotkey};
use view::QuickLauncher;

struct Open {
    handle: AnyWindowHandle,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
}

thread_local! {
    /// Work a choice started that the app must not quit before (a folder
    /// being opened, a notification being handed over).
    static PENDING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the launcher is on screen.
fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

/// Whether the launcher is on screen, or work it started is not done: the
/// app does not quit for being idle meanwhile.
pub fn busy(_cx: &App) -> bool {
    is_open() || PENDING.with(|p| p.get()) > 0
}

/// Runs `work` off the UI thread; the app waits for it before quitting.
fn background(cx: &mut App, work: impl FnOnce() + Send + 'static) {
    PENDING.with(|p| p.set(p.get() + 1));
    cx.spawn(async move |cx| {
        cx.background_spawn(async move { work() }).await;
        cx.update(|cx| {
            PENDING.with(|p| p.set(p.get().saturating_sub(1)));
            crate::system::exit_if_idle(cx);
        });
    })
    .detach();
}

/// Opens the launcher, or closes it when it is open (its shortcut and
/// `heronite://quick-launch`).
pub fn toggle(cx: &mut App) {
    if is_open() {
        close(cx);
    } else {
        open(cx);
    }
}

/// Opens the launcher (or gives it the keyboard again).
pub fn open(cx: &mut App) {
    open_with(cx, "", None);
}

fn open_with(cx: &mut App, query: &str, keys: Option<String>) {
    let open = OPEN.with(|o| o.borrow().as_ref().map(|o| o.handle));
    if let Some(handle) = open {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
        // Gone without a word (the compositor): open a new one.
        OPEN.with(|o| o.borrow_mut().take());
    }
    // Console mode is for the controller: the shortcut brings it forward
    // rather than a keyboard panel over it.
    if crate::console::active() && crate::system::has_window(cx) {
        crate::system::summon_console(cx);
        return;
    }
    let query = query.to_string();
    // Windows: a topmost window over a game in exclusive fullscreen would
    // minimise it or switch its display mode (as for the overlay).
    if crate::overlay::window::exclusive_fullscreen() {
        return;
    }
    match window::open(cx, move |window, cx| cx.new(|cx| QuickLauncher::new(&query, keys.clone(), window, cx))) {
        Ok(handle) => {
            log::info!("quick launcher opened");
            watch_close(cx);
            OPEN.with(|o| *o.borrow_mut() = Some(Open { handle }));
        }
        Err(e) => log::error!("quick launcher: cannot open its window: {e:#}"),
    }
}

/// Closes the launcher; the app quits when it was started only for it and
/// nothing else is left (no tray, no game).
pub fn close(cx: &mut App) {
    let Some(open) = OPEN.with(|o| o.borrow_mut().take()) else { return };
    open.handle.update(cx, |_, window, _| window.remove_window()).ok();
    cx.defer(crate::system::exit_if_idle);
}

/// Closes the launcher if it is still the window `id` (a close asked by a
/// launcher that has since been replaced does nothing).
fn close_window(id: WindowId, cx: &mut App) {
    if OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.handle.window_id() == id)) {
        close(cx);
    }
}

/// Forgets the launcher when its window goes away by other means (the
/// compositor). Registered once.
fn watch_close(cx: &mut App) {
    thread_local! {
        static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if WATCHING.with(|w| w.replace(true)) {
        return;
    }
    cx.on_window_closed(|cx, id| {
        let ours = OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.handle.window_id() == id));
        if ours {
            OPEN.with(|o| o.borrow_mut().take());
            cx.defer(crate::system::exit_if_idle);
        }
    })
    .detach();
}

/// `HERONITE_DEBUG_QUICK_LAUNCH` is set: the start opens the launcher, in a
/// normal window, instead of the main window.
pub fn debug_requested() -> bool {
    crate::env::value("HERONITE_DEBUG_QUICK_LAUNCH").is_some()
}

/// The start asked for it (`--quick-launch` with no instance running, or
/// the debug hook with its query and keys).
pub fn open_at_start(cx: &mut App) {
    let query = crate::env::value("HERONITE_DEBUG_QUICK_LAUNCH").filter(|q| q != "-").unwrap_or_default();
    open_with(cx, &query, crate::env::value("HERONITE_DEBUG_QUICK_LAUNCH_KEYS"));
    // It did not open (a game in exclusive fullscreen, no window): a start
    // made only for it must not stay with nothing on screen.
    if !is_open() {
        cx.defer(crate::system::exit_if_idle);
    }
}
