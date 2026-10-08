//! Integration with the desktop: launching games from outside (CLI, protocol
//! URLs, single instance), shortcuts, tray icon and window lifecycle.

pub mod cli;
mod hub;
pub mod notify;
pub mod shortcuts;
pub mod settings;
pub mod signals;
pub mod single_instance;
pub mod tray;

pub use hub::{attach_runtime, candidate_keys, exit_code, has_window, init, keep_in_tray, post_game_idle, quit_anyway, request_quit};
