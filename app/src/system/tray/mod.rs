//! System tray icon, the counterpart of Heroic's `tray_icon.ts`: recent
//! games, Show / Hide the window, Quit. Runs on its own thread; every action
//! is sent to the UI as a [`TrayCmd`]. Linux only for now (`sni.rs`); other
//! systems have no icon, and the app behaves as if the tray were missing.

#[cfg(target_os = "linux")]
mod common;
#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "linux")]
pub use sni::{spawn, TrayHandle};

#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
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
