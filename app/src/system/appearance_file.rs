//! `appearance.json` changed by something other than the app: a theme script
//! (pywal, matugen, wallust...) writing the custom palette when the wallpaper
//! changes, or the file synced from another PC. The new look applies at once,
//! as a change in Settings → Appearance would.
//!
//! The app's own saves come back here too; they read as the look already in
//! use, which the receiver ignores. Writes are atomic (a rename into the
//! folder), so the folder is watched, not the file.

use std::sync::{mpsc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use heronite_data::appearance::Appearance;
use heronite_data::AppPaths;
use notify::{EventKind, RecursiveMode, Watcher};

/// A script writes the file in a few steps (a temporary file, then the
/// rename); the reading waits this long after the last event.
const SETTLE: Duration = Duration::from_millis(150);

struct Subscribers(Vec<UnboundedSender<Appearance>>);

/// `None` until the first [`watch`] starts the thread.
static SUBSCRIBERS: Mutex<Option<Subscribers>> = Mutex::new(None);

fn publish(a: Appearance) {
    let mut guard = SUBSCRIBERS.lock().unwrap();
    if let Some(s) = guard.as_mut() {
        s.0.retain(|tx| tx.unbounded_send(a).is_ok());
    }
}

/// Sends `appearance.json` each time it changes on disk. One thread for the
/// whole process: the window (and with it each subscriber) comes and goes
/// from the tray.
pub fn watch(paths: &AppPaths) -> UnboundedReceiver<Appearance> {
    let (tx, rx) = unbounded();
    {
        let mut guard = SUBSCRIBERS.lock().unwrap();
        if let Some(s) = guard.as_mut() {
            s.0.push(tx);
            return rx;
        }
        *guard = Some(Subscribers(vec![tx]));
    }
    let paths = paths.clone();
    let spawned = std::thread::Builder::new().name("appearance-file".into()).spawn(move || run(&paths));
    if let Err(err) = spawned {
        log::warn!("watching appearance.json: {err}");
    }
    rx
}

fn run(paths: &AppPaths) {
    let file = paths.appearance();
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return };
    // The folder may not exist yet on a first start; the watch needs it.
    if let Err(err) = std::fs::create_dir_all(dir) {
        log::warn!("watching appearance.json: {err}");
        return;
    }
    let (events_tx, events) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(events_tx) {
        Ok(w) => w,
        Err(err) => {
            log::warn!("watching appearance.json: {err}");
            return;
        }
    };
    if let Err(err) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        log::warn!("watching appearance.json: {err}");
        return;
    }
    let about_file = |ev: &notify::Result<notify::Event>| match ev {
        Ok(ev) => matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) && ev.paths.iter().any(|p| p.file_name() == Some(name)),
        Err(_) => false,
    };
    let mut last = Appearance::load(paths);
    while let Ok(ev) = events.recv() {
        if !about_file(&ev) {
            continue;
        }
        while events.recv_timeout(SETTLE).is_ok() {}
        let now = Appearance::load(paths);
        if now != last {
            last = now;
            publish(now);
        }
    }
}
