//! Heroic-compatible log files (`src/backend/logger/**` of Heroic).
//!
//! * Where: `<data>/logs` (see [`LogPaths`]). A game's log is
//!   `games/<appName>_<runner>/launch.log`; the
//!   previous run is kept as `launch.log.old`.
//! * How: [`GameLog`] mirrors Heroic's `LogWriter`. The first write of a run
//!   renames the existing file to `.old` (overwriting an older one) and
//!   starts a new file; lines look like `(21:04:41) [INFO]:    message`; a
//!   finished log ends with `============= End of log =============`.
//! * `disableLogs` (global `config.json`) and `verboseLogs` (per game) switch
//!   the launch log off: only "forced" lines (the "Logs are disabled"
//!   warning) are written then.
//!
//! The game writes its stdout / stderr straight into the log file (the launcher
//! hands it an `O_APPEND` descriptor, see [`GameLog::child_stdio`]), so there
//! is no pipe for the launcher to pump and the game does not depend on the
//! launcher staying alive. The launcher appends its own lines (header before
//! the spawn, "exited" and "End of log" after) to the same file.

mod format;
mod game_log;
mod header;
mod launcher_log;
mod paths;

pub use format::{format_line, GameLogType, Level};
pub use game_log::{GameLog, END_OF_LOG, MAX_LOG_BYTES};
pub use header::{
    command_line_for_log, launch_log_switches, merged_settings, settings_for_log, write_disabled_notice, write_launch_header, HeaderInfo,
};
pub use launcher_log::{prune_launcher_logs, LauncherLogger, KEEP_LAUNCHER_LOGS, MAX_LAUNCHER_LOG_BYTES};
pub use paths::{old_path, LogPaths};
