//! Integration with the desktop: launching games from outside (CLI, protocol
//! URLs, single instance), shortcuts, tray icon and window lifecycle.

pub mod cli;
mod hub;
pub mod notify;
pub mod settings;
pub mod shortcuts;
pub mod signals;
pub mod single_instance;
pub mod tray;

pub use hub::{
    attach_runtime, candidate_keys, exit_code, has_window, init, keep_in_tray, post_game_idle, quit_anyway, refresh_tray,
    relaunch_if_requested, request_quit, restart_blocker, restart_into,
};
