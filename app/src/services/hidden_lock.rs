//! The session side of the hidden-games PIN (`heronite_data::hidden_lock`):
//! whether it was typed since the app started, and the wrong tries in a row.
//!
//! The PIN unlocks hidden games until "Show hidden games" is turned off or the
//! app quits. Every reader of the switch asks [`shown`], so a switch left on
//! in `ui-state.json` never shows them on its own.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use heronite_data::{hidden_lock, AppPaths};

static UNLOCKED: AtomicBool = AtomicBool::new(false);
/// Whether a PIN is saved: 0 not read yet, 1 no, 2 yes.
static PIN_SET: AtomicU8 = AtomicU8::new(0);

struct Tries {
    failures: u32,
    until: Option<Instant>,
}

static TRIES: Mutex<Tries> = Mutex::new(Tries { failures: 0, until: None });

fn tries() -> MutexGuard<'static, Tries> {
    TRIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether hidden games are behind a PIN (the file is read once, then kept).
pub fn pin_set(app: &AppPaths) -> bool {
    match PIN_SET.load(Ordering::Relaxed) {
        0 => {
            let set = hidden_lock::is_set(app);
            note_pin_set(set);
            set
        }
        n => n == 2,
    }
}

/// After the PIN was saved or removed.
pub fn note_pin_set(set: bool) {
    PIN_SET.store(if set { 2 } else { 1 }, Ordering::Relaxed);
}

/// Whether hidden games may show now: there is no PIN, or it was typed.
pub fn open(app: &AppPaths) -> bool {
    UNLOCKED.load(Ordering::Relaxed) || !pin_set(app)
}

/// "Show hidden games" as it applies: `flag` (the saved switch) while the
/// games may show.
pub fn shown(flag: bool, app: &AppPaths) -> bool {
    flag && open(app)
}

/// Hidden games may show until [`lock`] (the PIN was typed, or saved while
/// they were shown).
pub fn unlock() {
    UNLOCKED.store(true, Ordering::Relaxed);
}

/// The right PIN was typed: the wrong tries start again from zero.
pub fn succeeded() {
    let mut t = tries();
    t.failures = 0;
    t.until = None;
}

pub fn lock() {
    UNLOCKED.store(false, Ordering::Relaxed);
}

/// A wrong PIN; gives how long to wait before the next try.
pub fn failed() -> Duration {
    let mut t = tries();
    t.failures = t.failures.saturating_add(1);
    let wait = hidden_lock::wait_after(t.failures);
    t.until = (!wait.is_zero()).then(|| Instant::now() + wait);
    wait
}

/// The time left before the PIN can be tried again.
pub fn wait_left() -> Option<Duration> {
    let until = tries().until?;
    until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
}
