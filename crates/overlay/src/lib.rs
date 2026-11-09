//! Backend of the in-game overlay (the panel the Guide button opens over a
//! game launched by Heronite): controller input with grabs, system audio,
//! controller status and the replay buffer. No GPUI; the app draws the
//! overlay.

pub mod audio;
pub mod controller;
pub mod gamescope;
pub mod input;
mod procs;
pub mod replay;
