//! Desktop notifications (freedesktop `org.freedesktop.Notifications`, through
//! `notify-rust`) for results nobody would see otherwise: post-game work and
//! background updates while the window is closed to the tray.
//!
//! Sending blocks on D-Bus, so it runs on its own thread. Every notification
//! is also logged (`[notify] title: body`). `HERONITE_DEBUG_NO_NOTIFY=1`
//! only logs (tests and screenshot runs).

/// Shows a notification; never blocks the caller and never fails loudly.
pub fn send(title: impl Into<String>, body: impl Into<String>) {
    let (title, body) = (title.into(), body.into());
    log::info!("[notify] {title}: {body}");
    if crate::env::flag("HERONITE_DEBUG_NO_NOTIFY") {
        return;
    }
    let spawned = std::thread::Builder::new().name("notify".into()).spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("Heronite").summary(&title).body(&body).show() {
            log::warn!("desktop notification failed: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not start the notification thread: {e}");
    }
}

/// Like [`send`], but waits (at most `wait`) for the notification to be
/// handed to the desktop: for a process that exits right after.
pub fn send_and_wait(title: impl Into<String>, body: impl Into<String>, wait: std::time::Duration) {
    let (title, body) = (title.into(), body.into());
    log::info!("[notify] {title}: {body}");
    if crate::env::flag("HERONITE_DEBUG_NO_NOTIFY") {
        return;
    }
    let (done, finished) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("notify".into()).spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("Heronite").summary(&title).body(&body).show() {
            log::warn!("desktop notification failed: {e}");
        }
        let _ = done.send(());
    });
    if spawned.is_ok() {
        let _ = finished.recv_timeout(wait);
    }
}
