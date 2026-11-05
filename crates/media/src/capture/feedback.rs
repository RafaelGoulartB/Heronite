//! User feedback: Omarchy OSD on Hyprland, desktop notifications elsewhere.

use super::hyprland::is_hyprland_session;
use std::process::{Command, Stdio};

/// `showScreenshotOsd`: `omarchy-shell osd show {...}`; false when unavailable.
pub fn show_osd(message: &str, value: &str) -> bool {
    if !is_hyprland_session() {
        return false;
    }
    let payload = serde_json::json!({ "icon": "\u{F0100}", "message": message, "value": value, "duration": 2000 });
    Command::new("omarchy-shell")
        .args(["osd", "show", &payload.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Desktop notification via `notify-rust` (freedesktop Notifications), or the
/// Notification portal inside a Flatpak.
pub fn notify(title: &str, body: &str) {
    let shown = if heronite_platform::flatpak::active() {
        heronite_platform::flatpak::notify(title, body)
    } else {
        notify_rust::Notification::new().appname("Heronite").summary(title).body(body).show().map(|_| ()).map_err(|e| e.to_string())
    };
    if let Err(e) = shown {
        log::error!("Collection screenshot notification failed: {e}");
    }
}
