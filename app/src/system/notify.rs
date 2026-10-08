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
    if std::env::var("HERONITE_DEBUG_NO_NOTIFY").is_ok_and(|v| !v.is_empty() && v != "0") {
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
