//! Desktop notifications (freedesktop `org.freedesktop.Notifications`, through
//! `notify-rust`; the Notification portal in a Flatpak) for results nobody
//! would see otherwise: post-game work and background updates while the
//! window is closed to the tray.
//!
//! Sending blocks on D-Bus, so it runs on its own thread. Every notification
//! is also logged (`[notify] title: body`). `HERONITE_DEBUG_NO_NOTIFY=1`
//! only logs (tests and screenshot runs).

/// Shows one notification: through the portal inside a Flatpak.
fn show(title: &str, body: &str) -> Result<(), String> {
    if heronite_platform::flatpak::active() {
        return heronite_platform::flatpak::notify(title, body);
    }
    let mut notification = notify_rust::Notification::new();
    notification.appname("Heronite").summary(title).body(body);
    // The app's icon and launcher entry (what the packages install), else
    // the notification daemon shows a generic icon.
    #[cfg(all(unix, not(target_os = "macos")))]
    notification.icon(APP_ID).hint(notify_rust::Hint::DesktopEntry(APP_ID.into()));
    notification.show().map(|_| ()).map_err(|e| e.to_string())
}

/// The name of the app's icon and `.desktop` entry on Linux.
#[cfg(all(unix, not(target_os = "macos")))]
const APP_ID: &str = "app.heronite.Heronite";

/// Shows a notification; never blocks the caller and never fails loudly.
pub fn send(title: impl Into<String>, body: impl Into<String>) {
    let (title, body) = (title.into(), body.into());
    log::info!("[notify] {title}: {body}");
    if crate::env::flag("HERONITE_DEBUG_NO_NOTIFY") {
        return;
    }
    let spawned = std::thread::Builder::new().name("notify".into()).spawn(move || {
        if let Err(e) = show(&title, &body) {
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
        if let Err(e) = show(&title, &body) {
            log::warn!("desktop notification failed: {e}");
        }
        let _ = done.send(());
    });
    if spawned.is_ok() {
        let _ = finished.recv_timeout(wait);
    }
}
