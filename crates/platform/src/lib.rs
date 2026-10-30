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
//! - [`shell`]: opening a file, folder or URL with the desktop's handler.
//! - [`features`]: which parts of the app this system has, for the UI to
//!   show or hide.
//! - `registry` (Windows): values of the registry.
//!
//! A module of code that only makes sense on one system (`/proc`, evdev,
//! Wine) stays in its crate behind `cfg(target_os = "linux")`; what the rest
//! of the app needs from it on another system is a stub next to it.

pub mod dirs;
pub mod features;
pub mod fs;
pub mod process;
pub mod shell;

#[cfg(windows)]
pub mod registry;

pub use features::{supported, Feature};
