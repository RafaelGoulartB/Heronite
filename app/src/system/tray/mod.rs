//! System tray icon, the counterpart of Heroic's `tray_icon.ts`: recent
//! games, Show / Hide the window, Quit. Runs on its own thread; every action
//! is sent to the UI as a [`TrayCmd`]. Linux has a StatusNotifierItem
//! (`sni.rs`), Windows an icon in the notification area (`windows.rs`); other
//! systems have no icon, and the app behaves as if the tray were missing.

#[cfg(any(target_os = "linux", windows))]
mod common;

#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "linux")]
pub use sni::{spawn, TrayHandle};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{spawn, TrayHandle};

#[cfg(not(any(target_os = "linux", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "linux", windows)))]
pub use unsupported::{spawn, TrayHandle};

/// What the user asked for in the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCmd {
    /// Bring the window up (create it when it was closed to the tray).
    Show,
    /// Left click: hide when `exitToTray` and visible, else show.
    Toggle,
    Quit,
    /// Launch a recent game (`appName`; the runner is searched).
    Launch(String),
}
