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

/// What asking for the computer's password gave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SystemPassword {
    Given,
    /// Dismissed, or a wrong password.
    Refused,
    /// No way to ask here (no `pkexec`, or not Linux).
    Unavailable,
}

/// "Forgot the PIN?": the desktop asks for this computer's password (an
/// administrator's, through polkit's `pkexec`), which proves the owner is
/// here. Blocks until it is answered: call it off the UI thread.
pub fn ask_system_password() -> SystemPassword {
    // `HERONITE_DEBUG_SYSTEM_PASSWORD=given|refused|unavailable` answers
    // without asking (captures); debug builds only, as it skips the password.
    let fake = if cfg!(debug_assertions) { crate::env::value("HERONITE_DEBUG_SYSTEM_PASSWORD") } else { None };
    match fake.as_deref() {
        Some("given") => return SystemPassword::Given,
        Some("refused") => return SystemPassword::Refused,
        Some(_) => return SystemPassword::Unavailable,
        None => {}
    }
    if !cfg!(target_os = "linux") {
        return SystemPassword::Unavailable;
    }
    let Some(pkexec) = ["/usr/bin/pkexec", "/bin/pkexec"].into_iter().find(|p| std::path::Path::new(p).exists()) else {
        return SystemPassword::Unavailable;
    };
    let status = std::process::Command::new(pkexec).arg("/bin/true").stdin(std::process::Stdio::null()).status();
    match status.map(|s| s.code()) {
        Ok(Some(0)) => SystemPassword::Given,
        // 126: dismissed; 127: not authorised (a wrong password).
        Ok(Some(126 | 127)) => SystemPassword::Refused,
        _ => SystemPassword::Unavailable,
    }
}
