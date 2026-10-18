//! Backend of the in-game overlay (the panel the Guide button opens over a
//! game launched by Heronite): controller input with grabs, system audio and
//! controller status. No GPUI; the app draws the overlay.

pub mod audio;
pub mod controller;
#[cfg(target_os = "linux")]
pub mod input;
