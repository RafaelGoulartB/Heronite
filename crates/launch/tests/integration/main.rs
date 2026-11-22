//! The crate's integration tests, in one binary so CI links one executable
//! instead of one per file. Tests that need a real machine, the network or
//! downloads live beside this folder behind the `manual-tests` feature.

mod client_games;
mod epic_windows;
mod fixture;
mod game_log;
// The fake legendary and Wine are shell scripts.
#[cfg(unix)]
mod launch_flow;
mod runtime;
#[cfg(unix)]
mod steam_second_drive;
#[cfg(windows)]
mod windows_extras;
