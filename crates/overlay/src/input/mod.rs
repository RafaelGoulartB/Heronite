//! Controller input for the overlay. Linux reads evdev nodes and can grab
//! them (`EVIOCGRAB`) while Guide is down or the overlay is open.
//!
//! Linux limits: a game using `hidraw` (SDL's HIDAPI drivers, Steam Input)
//! is not affected by the grab; it also sees the initial Guide press.
//!
//! Windows polls documented XInput. Back+Start acts as Guide; neither that
//! combination nor navigation is exclusive: the game can receive both. The
//! open panel takes the foreground (`heronite_platform::overlay_window`),
//! which keeps navigation out of the games that ignore the controller in the
//! background (SDL, Unity and Unreal ones, most of them).

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

pub use self::core::{Button, Event, Options, CONSOLE_HOLD};
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
    /// Which device it is, for [`crate::controller::battery`].
    pub id: ControllerId,
    /// Known Bluetooth address. `None` does not imply a wired connection.
    pub bluetooth: Option<String>,
}

/// How the system names a controller.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ControllerId {
    /// Its evdev node (`/dev/input/event7`), on Linux.
    Evdev(PathBuf),
    /// Its XInput slot (below `heronite_platform::gamepad::CONTROLLER_COUNT`),
    /// on Windows.
    XInput(u32),
}
