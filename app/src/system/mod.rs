//! Integration with the desktop: launching games from outside (CLI, protocol
//! URLs, single instance), shortcuts, tray icon and window lifecycle.

pub mod cli;
mod hub;
#[allow(dead_code)]
pub mod shortcuts;
pub mod settings;
pub mod single_instance;
pub mod tray;

pub use hub::{attach_runtime, candidate_keys, exit_code, init};
