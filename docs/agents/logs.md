# Your brief: LOGS + LOG VIEWER (agent "logs")

Port Heroic's logging (HeroicGamesLauncher/src/backend/logger/** — log_writer.ts, paths.ts, constants.ts, formatter.ts,
uploader.ts for reference only, no upload) and its log viewer (src/frontend/screens/Settings/sections/LogSettings/**).
1. Game logs: every launch from our app writes the game log exactly where and how Heroic does
   (~/.config/heroic/GamesConfig/<appName>-lastPlay.log already exists for some games; check log_writer.ts for the
   header Heroic writes — system info, settings, launch command, env — and how the game's stdout/stderr is appended,
   rotation/"lastPlay" naming, and the `disableLogs` / `verboseLogs` settings semantics: config.json global and per-game).
   Our launch pipeline: crates/launch (runtime.rs spawns processes; plan.rs builds commands/env; prepare.rs status),
   app/src/runtime.rs. Capture stdout/stderr of the game process (and of the legendary/umu/wine wrappers) into the log
   without blocking and without unbounded memory.
2. Launcher log: our app's own log (the `log` crate output — app/src/main.rs `env_logger_init`) also written to a file
   in the place Heroic uses for its launcher log if that makes sense for a separate app — otherwise
   ~/.config/heronite-rs/logs/heronite-rs-<date>.log with rotation (keep last N). Respect disableLogs.
3. Viewer: a "Logs" section in the global Settings page (app/src/pages/settings.rs + submodule): pick launcher log or a
   game's log (list games that have logs), view the content (monospace, scrollable, efficient for multi-MB files —
   render only what's visible or tail the last N lines with "load more"), refresh, copy to clipboard, open folder,
   open file externally. Also a "View log" item in the game context menu (app/src/collection/context_menu.rs, one
   small block) that opens the viewer on that game's log (navigate to the Settings page section or open a dialog).
Owned: crates/launch log-writing parts (new module log_writer.rs), app/src/pages/settings/logs.rs, app/src/main.rs
logger part, new app/src/dialogs/log_viewer.rs if you use a dialog. Hook points: pages/settings.rs section list,
context_menu.rs one block, app/src/runtime.rs small.
Another agent ("updates") works in parallel on Epic auto-updates and post-game tasks (services/downloads.rs,
app/src/system/**, runtime post-session work) — coordinate by keeping your runtime.rs edit minimal.
Testing: never launch real games/emulators; use fake game processes (shell scripts) through the launch crate tests
(crates/launch/tests/* already do this). Fixture data dirs only. Captures via scripts/shot.sh (workspace 7) with a debug
hook to open the viewer.
