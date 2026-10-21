//! The replay buffer: while a game launched by the app runs,
//! gpu-screen-recorder keeps its last seconds in memory and saves them when
//! asked (Guide + X, the panel). The app picks the folder and marks the
//! saved file as a moment in the game's journal.
//!
//! Chosen to cost the game as little as possible:
//! - the monitor is captured straight from the display (KMS, `-w <monitor>`),
//!   with no copy through the CPU nor the compositor;
//! - the graphics card's own encoder (NVENC, VA-API), never the CPU;
//! - a constant bitrate, so the buffer's memory is known in advance
//!   ([`plan::estimate`]);
//! - it only runs while a game does, and anything else (cutting the panel
//!   off a save) runs at idle priority.
//!
//! [`info`] reads what the recorder can do, [`plan`] builds the command and
//! the estimate, [`recorder`] runs it, [`ipc`] saves through its socket,
//! [`target`] finds the game's monitor and [`trim`] cuts the end of a save.

pub mod info;
pub mod ipc;
pub mod plan;
pub mod recorder;
pub mod target;
pub mod trim;

pub use info::{Info, Screen, MIN_VERSION};
pub use plan::{estimate, Estimate, Plan};
pub use recorder::{Program, Recorder, State};
