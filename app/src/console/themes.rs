//! `console.json` at run time: read once when first needed (console mode
//! starting, its Settings section shown), changed in memory, and written off
//! the UI thread (the latest version wins; writes never overlap).
//!
//! [`update`] is the one way to change the theme in use: it also installs it
//! in [`super::look`] while console mode is on, so the screen follows at once.
//! The controller's options (`controller`) are installed in `look` whatever
//! the mode, and mirrored in `ui-state.json` for the next start.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use heronite_data::console_theme::{ConsolePrefs, ConsoleTheme, ControllerPrefs, KeptFilters};
use heronite_data::paths::AppPaths;

static PREFS: Mutex<Option<ConsolePrefs>> = Mutex::new(None);
static PENDING: Mutex<Option<(AppPaths, ConsolePrefs)>> = Mutex::new(None);
static WRITING: AtomicBool = AtomicBool::new(false);

/// The prefs, read from disk the first time.
pub fn prefs(app: &AppPaths) -> ConsolePrefs {
    read(app, ConsolePrefs::clone)
}

/// `f` over the prefs without copying them (it must be quick: it holds the lock).
pub fn read<R>(app: &AppPaths, f: impl FnOnce(&ConsolePrefs) -> R) -> R {
    let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
    f(cur.get_or_insert_with(|| loaded(app)))
}

/// The prefs from disk, their controller options installed (they may differ
/// from the mirror read at start: a file from before them, or edited).
fn loaded(app: &AppPaths) -> ConsolePrefs {
    let p = ConsolePrefs::load(app);
    install_controls(app, p.controller);
    p
}

/// Installs the controller's options and mirrors them when they changed.
fn install_controls(app: &AppPaths, c: ControllerPrefs) {
    if super::look::set_controls(c) {
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = heronite_data::ui_state::set_controller(&app, c) {
                log::warn!("mirroring the controller options: {e:#}");
            }
        });
    }
}

/// Keeps `tab` as the one console mode reopens on. Unlike [`update`] it leaves
/// the theme in use alone (installing it again would restart the screen's animations).
pub fn remember_tab(app: &AppPaths, tab: usize) {
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur.get_or_insert_with(|| loaded(app));
        if p.last_tab == tab {
            return;
        }
        p.last_tab = tab;
        p.clone()
    };
    save(app, next);
}

/// Keeps the filters console mode starts with next time (like
/// [`remember_tab`], the theme in use is left alone).
pub fn remember_filters(app: &AppPaths, filters: KeptFilters) {
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur.get_or_insert_with(|| loaded(app));
        if p.kept_filters == filters {
            return;
        }
        p.kept_filters = filters;
        p.clone()
    };
    save(app, next);
}

/// Changes the prefs, installs the theme while console mode is on and saves.
pub fn update(app: &AppPaths, change: impl FnOnce(&mut ConsolePrefs)) -> ConsolePrefs {
    let (before, next) = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur.get_or_insert_with(|| loaded(app));
        let before = p.theme.clone();
        change(p);
        p.theme = std::mem::take(&mut p.theme).normalized();
        (before, p.clone())
    };
    // Covers of a shape the new theme draws and the old one did not.
    crate::dialogs::metadata_update::families::console_theme_changed(&before, &next.theme);
    if super::active() {
        super::look::set(next.theme.clone());
    }
    install_controls(app, next.controller);
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
