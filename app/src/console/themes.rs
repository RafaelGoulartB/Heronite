//! `console.json` at run time: read once when first needed (console mode
//! starting, its Settings section shown), changed in memory, and written off
//! the UI thread (the latest version wins; writes never overlap).
//!
//! [`update`] is the one way to change the theme in use: it also installs it
//! in [`super::look`] while console mode is on, so the screen follows at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use heronite_data::console_theme::{ConsolePrefs, ConsoleTheme};
use heronite_data::paths::AppPaths;

static PREFS: Mutex<Option<ConsolePrefs>> = Mutex::new(None);
static PENDING: Mutex<Option<(AppPaths, ConsolePrefs)>> = Mutex::new(None);
static WRITING: AtomicBool = AtomicBool::new(false);

/// The prefs, read from disk the first time.
pub fn prefs(app: &AppPaths) -> ConsolePrefs {
    let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
    cur.get_or_insert_with(|| ConsolePrefs::load(app)).clone()
}

/// Changes the prefs, installs the theme while console mode is on and saves.
pub fn update(app: &AppPaths, change: impl FnOnce(&mut ConsolePrefs)) -> ConsolePrefs {
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur.get_or_insert_with(|| ConsolePrefs::load(app));
        change(p);
        p.theme = std::mem::take(&mut p.theme).normalized();
        p.clone()
    };
    if super::active() {
        super::look::set(next.theme.clone());
    }
    save(app, next.clone());
    next
}

/// Uses `theme` (from a preset or a saved theme when `based_on` is given).
pub fn apply(app: &AppPaths, theme: ConsoleTheme, based_on: Option<&str>) -> ConsolePrefs {
    update(app, |p| {
        p.theme = theme;
        if let Some(b) = based_on {
            p.based_on = b.to_string();
        }
    })
}

/// Forgets the copy in memory (console mode ended and Settings is closed).
pub fn forget() {
    *PREFS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn save(app: &AppPaths, prefs: ConsolePrefs) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((app.clone(), prefs));
    if WRITING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| loop {
        let next = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some((app, prefs)) = next else {
            WRITING.store(false, Ordering::Release);
            // A change may have come in between the take and the store.
            if PENDING.lock().unwrap_or_else(|e| e.into_inner()).is_some() && !WRITING.swap(true, Ordering::AcqRel) {
                continue;
            }
            break;
        };
        if let Err(e) = prefs.save(&app) {
            log::warn!("saving console.json: {e:#}");
        }
    });
}
