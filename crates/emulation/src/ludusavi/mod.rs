//! Ludusavi integration (port of `local-library/ludusavi.ts`): binary and
//! config detection, `find --api` / `backup --api` command construction and
//! JSON parsing, and the after-play backup flow.
//!
//! Processes only run through [`CommandRunner`]; tests inject a fake so no
//! real `ludusavi backup` is ever executed. Results are written back to
//! `settings.json` (`ludusavi.lastBackup*`/`lastError`) like Heronite.

mod backup;
mod commands;
mod detect;
#[cfg(test)]
mod fake;
mod runner;

pub use backup::{
    backup_after_play, backup_for_game, notification, AfterPlay, BackupReason, LudusaviBackupResult, LudusaviEnv,
    LudusaviGame, SKIP_SESSION_SECONDS,
};
pub use commands::{
    backup_command, best_find_match, find_commands, game_names, parse_json, total_games, wine_prefix_for_ludusavi,
    BackupCommand,
};
pub use detect::{detect_ludusavi, parse_config, resolve_binary, yaml_get, LudusaviDetected};
pub use runner::{CommandOutput, CommandRunner, SystemRunner, LUDUSAVI_TIMEOUT};
