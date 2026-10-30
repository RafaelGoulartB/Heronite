//! Controller input of the overlay, read straight from the kernel's evdev
//! nodes (`/dev/input/event*`) so a controller can be *grabbed*: while it
//! is grabbed (`EVIOCGRAB`) its events reach only this thread, never the
//! game. [`core`] decides when (Guide down, the overlay open).
//!
//! Limits: a game that reads the controller through `hidraw` (SDL's HIDAPI
//! drivers, Steam Input) is not affected by the grab, and the game still saw
//! the Guide press itself (the grab starts with it).
//!
//! The thread is Linux only (`linux.rs`); elsewhere `unsupported.rs` reads
//! no controllers.

// Only the evdev thread drives the state machine; other systems use just
// its types.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod core;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

pub use self::core::{Button, Event, Options};
#[cfg(target_os = "linux")]
pub use self::linux::{list_controllers, Watcher};
#[cfg(not(target_os = "linux"))]
pub use self::unsupported::{list_controllers, Watcher};

use std::path::PathBuf;

/// A connected controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    pub name: String,
    /// Its evdev node (`/dev/input/event7`): [`crate::controller::battery`] reads from it.
    pub node: PathBuf,
    /// Bluetooth address, for a controller connected over Bluetooth.
    pub bluetooth: Option<String>,
}
