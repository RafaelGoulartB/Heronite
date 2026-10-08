# Agent brief: helper processes die with the app, resumable Epic move, small write-safety fixes

Read docs/agents/common.md first (rules + SAFETY). Another agent ("shutdown", brief docs/agents/shutdown.md) works in
parallel on quit confirmation / signals / game spawning / playtime checkpoints / single instance. Do not edit its files
(app/src/main.rs, app/src/system/*, app/src/services/session.rs, crates/launch/src/{runtime,record,log_writer}.rs,
crates/epic/src/manager.rs, app/src/services/downloads.rs except one call in `Downloads::new`).

## Context (audit findings, verified)
- Only crates/epic/src/cmd.rs (legendary) puts children in their own process group with PR_SET_PDEATHSIG (SIGTERM).
  Everything else (rsync/mv in crates/epic/src/ops.rs move_dir, ludusavi in crates/emulation/src/ludusavi.rs, tar in
  crates/wine/src/install.rs and crates/launch/src/{tools,runtimes}.rs, the epic login helper started from
  app/src/pages/epic.rs) uses plain Command::output() and is orphaned when the app quits.
  Note: PDEATHSIG fires when the spawning THREAD dies — fine for callers that block on the child.
- Epic "move install" (ops.rs ~:270-325): rsync --remove-source-files to the new folder, remove the source, then
  `legendary move <app> <new_parent> --skip-move`. Interrupted, the game is split / installed.json points to the old path,
  and retrying fails with "already exists and is not empty".
- crates/wine/src/types.rs `set_download_proton_to_steam` (~:125) writes config.json by hand (to_string_pretty,
  fs::write, no fsync, no path lock, same temp name as json_store).
- crates/launch/src/first_launch.rs (~:93): `record_game` marks the game in the prefix's `installed_games` BEFORE the
  prerequisites/redistributables/known-fixes run (and that write is not atomic, ~:69). Interrupted setup never retries.
- json_store temp files `.<name>.heronite-rs-<pid>.tmp` (crates/data/src/json_store.rs ~:90), `shortcuts.vdf.tmp`,
  `<x>.desktop.tmp` are left behind by a kill and never cleaned. json_store's temp name is per-pid only, so two
  concurrent writers of the same path in one process (one bypassing the lock, e.g. cache_store.rs `clear`) collide.
- crates/shortcuts/src/steam.rs (~:144): shortcuts.vdf written via fixed `.tmp` + rename, no fsync, no backup.

## What to implement
1. **One way to spawn helper processes.** Add a small module (e.g. `heronite_data::proc`, add `libc` dep if needed;
   crates/shortcuts does not depend on heronite-data — fine, it does not spawn) with a `guard(&mut Command)` /
   `Command` extension that sets `process_group(0)` + `PR_SET_PDEATHSIG(SIGTERM)` (Linux, pre_exec; no-op elsewhere),
   and a helper to run-and-wait with cancellation support that kills the whole group (TERM, then KILL after a grace
   period) — reuse/move the logic from crates/epic/src/cmd.rs so legendary uses the same helper. Use it for: rsync/mv,
   ludusavi, tar/archive extraction (wine install, launch tools/runtimes), the epic-login helper.
   NOT for games/emulators (they must survive the launcher) and NOT for detached openers (xdg-open, file managers,
   editors — check app/src/collection/{context_menu,extras}.rs, dialogs/*, pages/* and leave openers alone).
2. **Resumable move.** Before the move starts, write a journal (heronite-rs config dir, e.g.
   `~/.config/heronite-rs/epic-move.json` via AppPaths; NOT inside ~/.config/heroic) with app name, source, destination,
   new_parent, phase. Update the phase (copying -> recording) and delete it when done or when the move fails cleanly
   before touching anything. At startup, `resume_interrupted_move(...)` in the epic crate: if a journal exists, finish it:
   - source still has files -> rerun the copy (rsync is resumable; skip the "destination not empty" check in resume mode),
     remove the emptied source;
   - then `legendary move <app> <new_parent> --skip-move` and refresh the library cache like the normal path;
   - if the destination is missing/empty and the source is complete (move never really started), just drop the journal;
   - report the outcome as a toast ("Finished moving X" / error). Run it in the background through the same
     busy/operation mechanism as other Epic ops so the game shows as busy and legendary is not run concurrently with the
     queue (the queue starts 5 s after launch: run the resume before / serialise). Wire the call with ONE small edit in
     `Downloads::new` (app/src/services/downloads.rs) — re-read the file right before editing, the other agent edits it.
   - Also fix the low bug: when the destination exists and is empty, `mv src dest` nests the folder (`dest/name`).
   Unit-test the reconcile decisions with temp dirs and a fake legendary (see existing tests with `cmd::testing::fake`).
3. **Small items.**
   - wine/types.rs `set_download_proton_to_steam`: use json_store (2-space indent like Heroic's config.json:
     `json_store::update_value_indented(.., INDENT_2, ..)` or the equivalent existing helper).
   - first_launch.rs: mark the game in `installed_games` only AFTER the setup finished successfully (same Heroic file
     format), with an atomic write (temp + rename). Interrupted / failed setup -> retried next launch. Update tests.
   - json_store: unique temp name per write (pid + counter) and `pub fn sweep_stale_temps(dirs)` that removes ONLY our
     own leftovers (`.*.heronite-rs-<pid>.tmp` whose pid is not alive or older than 1 h, `shortcuts.vdf.tmp` next to
     a shortcuts.vdf, `*.desktop.tmp` in the applications dir) — strict name matching, never anything else. Do NOT edit
     main.rs: expose a single function `sweep_stale_temps_at_startup(heroic, app_paths)` (or similar) and tell me in your
     report the one line to call; I will wire it.
   - shortcuts.vdf: before each write copy the current file to `shortcuts.vdf.heronite-bak` (overwrite), write the temp
     with a unique name, `sync_all`, rename. Same fsync for .desktop files.
   - crates/epic/src/cache_store.rs `clear` (~:85): go through the locked update path.

## Verification
- Unit tests for each piece (guard sets the group; group kill on cancel; move journal decisions; installed_games after
  setup; sweep only removes matching names; vdf backup).
- Never run real rsync moves on game folders, real legendary write commands, ludusavi, or tar into the real tools dir:
  use temp dirs and fake binaries (see existing test helpers). Never touch /data/Games, ~/Games, ~/.config/heroic/tools,
  the real Steam userdata.
- Full suite at the end: `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test --workspace --no-fail-fast`.
