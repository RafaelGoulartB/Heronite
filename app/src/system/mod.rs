//! Integration with the desktop: launching games from outside (CLI, protocol
//! URLs, single instance), shortcuts, tray icon and window lifecycle.

pub mod app_icon;
pub mod appearance_file;
pub mod autostart;
pub mod cli;
pub mod desktop_colours;
#[cfg(target_os = "linux")]
pub mod desktop_entry;
mod hub;
pub mod notify;
pub mod own_exe;
pub mod power;
pub mod protocol;
pub mod settings;
pub mod shortcuts;
pub mod signals;
pub mod single_instance;
pub mod tray;
pub mod ui_scale;

#[cfg(windows)]
pub use hub::run_on_quit;
pub use hub::{
    attach_runtime, candidate_keys, close_window, console_in_front, exit_code, fullscreen_asked, has_window, init, keep_in_tray,
    post_game_idle, quit_anyway, refresh_tray, relaunch_if_requested, request_quit, restart_blocker, restart_into, restart_now,
    show_window as show_main_window, summon_console,
};
