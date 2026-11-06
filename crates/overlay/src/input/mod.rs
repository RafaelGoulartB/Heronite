//! Controller input for the overlay. Linux reads evdev nodes and can grab
//! them (`EVIOCGRAB`) while Guide is down or the overlay is open.
//!
//! Linux limits: a game using `hidraw` (SDL's HIDAPI drivers, Steam Input)
//! is not affected by the grab; it also sees the initial Guide press.
//!
//! Windows polls documented XInput. Back+Start acts as Guide; neither that
//! combination nor navigation is exclusive: the game can receive both.

// Unsupported systems use only the public input types.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
mod core;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))]
mod xinput;

pub use self::core::{Button, Event, Options};
#[cfg(target_os = "linux")]
pub use self::linux::{list_controllers, Watcher};
#[cfg(not(any(target_os = "linux", windows)))]
pub use self::unsupported::{list_controllers, Watcher};
#[cfg(windows)]
pub use self::windows::{list_controllers, Watcher};

use std::path::PathBuf;

/// A connected controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    pub name: String,
    /// An evdev node on Linux, an opaque `xinput-<slot>` ID on Windows.
    /// [`crate::controller::battery`] accepts either identifier.
    pub node: PathBuf,
    /// Known Bluetooth address. `None` does not imply a wired connection.
    pub bluetooth: Option<String>,
}

#[cfg(windows)]
pub(crate) fn xinput_index(node: &std::path::Path) -> Option<u32> {
    let index = node.to_str()?.strip_prefix("xinput-")?.parse().ok()?;
    (index < heronite_platform::gamepad::CONTROLLER_COUNT).then_some(index)
}
