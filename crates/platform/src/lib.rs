//! What differs between operating systems.
//!
//! Heronite runs on Linux and, in part, on Windows. Every call into the
//! operating system that is not the same on both goes through this crate, so
//! the other crates stay free of `cfg(unix)` / `cfg(windows)`:
//!
//! - [`process`]: whether a process lives, stopping it, process groups (a
//!   job object on Windows), detached starts, a clock that survives restarts.
//! - [`fs`]: permission bits, executables, file identity (hard links).
//! - [`dirs`]: where the app keeps its data, settings and caches.
//! - [`disks`]: the disks things live on (Windows' volumes) and the space
//!   files take on them, links not followed.
//! - [`net`]: which process listens on a local TCP port.
//! - [`shell`]: opening a file, folder or URL with the desktop's handler.
//! - [`flatpak`]: running inside a Flatpak sandbox, and the host's programs
//!   from there.
//! - [`scoop`]: a copy installed by Scoop on Windows, and the path that
//!   follows its updates.
//! - [`features`]: which parts of the app this system has, for the UI to
//!   show or hide.
//! - [`packages`]: the Microsoft Store packages of this user (none on other
//!   systems).
//! - `registry` (Windows): values of the registry.
//! - `tray` (Windows): an icon in the taskbar's notification area, with a
//!   menu.
//! - `power` (Windows): keeping the computer awake.
//! - `shortcut` (Windows): `.lnk` shortcuts.
//! - `system` (Windows): what the computer is (processor, memory, graphics
//!   cards, Windows version), for the logs.
//! - `gamepad` (Windows): XInput controllers, for the overlay.
//! - `gpu_adapters` (Windows): the graphics adapters DXGI picks for power
//!   saving and for high performance, for a game's GPU preference.
//! - `controllers` (Windows): every kind of controller through SDL, for
//!   console mode.
//! - `audio` (Windows): Core Audio volumes, mutes and per-app sessions, for
//!   the overlay.
//! - `overlay_window` (Windows): placing the overlay's windows over the game
//!   and handing the focus back.
//! - `com` (Windows, private): COM on the current thread while a guard lives.
//!
//! A module of code that only makes sense on one system (`/proc`, evdev,
//! Wine) stays in its crate behind `cfg(target_os = "linux")`; what the rest
//! of the app needs from it on another system is a stub next to it.

pub mod dirs;
pub mod disks;
pub mod features;
pub mod flatpak;
pub mod fs;
pub mod net;
pub mod packages;
pub mod process;
pub mod scoop;
pub mod secret;
pub mod shell;

#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
mod com;
#[cfg(windows)]
pub mod controllers;
#[cfg(windows)]
pub mod gamepad;
#[cfg(windows)]
pub mod gpu_adapters;
#[cfg(windows)]
pub mod overlay_window;
#[cfg(windows)]
pub mod power;
#[cfg(windows)]
pub mod registry;
#[cfg(windows)]
pub mod shortcut;
#[cfg(windows)]
pub mod system;
#[cfg(windows)]
pub mod tray;

pub use features::{supported, Feature};
