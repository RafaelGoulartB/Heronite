//! Backend of the in-game overlay (the panel the Guide button opens over a
//! game launched by Heronite): controller input with grabs, system audio,
//! controller status and the replay buffer. No GPUI; the app draws the
//! overlay.

pub mod audio;
pub mod controller;
pub mod gamescope;
#[cfg(target_os = "linux")]
pub mod input;
mod procs;
#[cfg(target_os = "linux")]
pub mod replay;
