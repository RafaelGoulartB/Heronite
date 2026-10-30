//! Shortcuts for games, mirroring Heroic's `backend/shortcuts/**`:
//!
//! * [`menu`]: a game's shortcuts on the Desktop and in the menu of
//!   programs, what the app calls on every system. It writes
//!   [`desktop`]'s `.desktop` files on Linux (Desktop and application menu,
//!   `~/.local/share/applications`) and `lnk`'s `.lnk` files on Windows
//!   (Desktop and Start menu; [`ico`] makes their icons).
//! * [`vdf`] + [`steam`]: the binary `shortcuts.vdf` of every Steam user
//!   ("Add to Steam" as a non-Steam game) and its `grid/` artwork.
//! * [`steam_client`] + [`steam_running`]: the same while Steam runs, through
//!   its CEF debugging interface or the `steam://addnonsteamgame` handler.
//!
//! Nothing here knows about GPUI or the library: callers pass plain data, and
//! every root (Desktop, applications, Steam) is a parameter so tests (and the
//! `HERONITE_*_DIR` overrides of the app) never touch the real ones.

pub mod desktop;
pub mod ico;
#[cfg(windows)]
pub mod lnk;
pub mod menu;
pub mod steam;
pub mod steam_client;
pub mod steam_running;
pub mod sweep;
pub mod vdf;

pub use sweep::sweep_stale_temps_at_startup;

/// Identity of a game for shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameId {
    pub runner: String,
    pub app_name: String,
    pub title: String,
}

/// Command line pieces of a launch request for this app.
///
/// `--launch <runner> <appName>` is what `.desktop` and `.lnk` files use; Steam gets the
/// Heroic-compatible protocol URL (`heronite://launch?appName=..&runner=..`).
pub fn launch_url(scheme: &str, runner: &str, app_name: &str) -> String {
    format!("{scheme}://launch?appName={app_name}&runner={runner}")
}
