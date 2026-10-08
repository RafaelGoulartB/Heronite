# Agent brief: graceful shutdown (quit confirm, game independence, playtime checkpoints, signals, second instance)

Read docs/agents/common.md first (rules + SAFETY). Another agent ("proc-safety", brief docs/agents/proc-safety.md)
works in parallel on helper processes / move journal / small write-safety fixes. Do not edit its files.

## Context (audit findings, verified)
- `app/src/system/hub.rs` `quit()` (~:409) and `on_window_closed` (~:383) quit without looking at `State.running`
  or `services::session::busy()`. With exitToTray off (default) closing the window while a game runs quits silently.
- Playtime / lastPlayed / sessions.json entry / recent list are only written in `finish()` (crates/launch/src/runtime.rs
  ~:608-640, record.rs) when the game exits. A launcher quit, kill or shutdown loses the whole session.
- The game's stdout/stderr go to a pipe pumped by a launcher thread (`SystemSpawner::spawn_with`, runtime.rs ~:175-240).
  When the launcher exits the read end closes and the game can die of SIGPIPE/EPIPE (native/emulator games).
- No SIGTERM/SIGINT/SIGHUP handling anywhere; GPUI installs none. No `on_app_quit` hook.
- Epic download queue: legendary gets PDEATHSIG SIGTERM when the app dies; the running element stays in
  store/download-manager.json and resumes on next start (good). Nothing pauses it in order on quit.
  DO NOT call `DownloadManager::shutdown()` as is: a cancel without pause_requested moves the element to `finished` as
  Abort (worker_loop ~:243-251) and defeats resume. Use the pause path.
- `app/src/system/single_instance.rs`: if the primary does not answer in 5 s or answers non-ok, `forward` returns Err and
  main.rs (~:76) starts a second full, unlocked instance. The server thread also answers "ok" even when
  `unbounded_send` failed (receiver gone during shutdown).

## What to implement
1. **Quit confirmation when games are running.**
   - Any user quit (window X without staying in tray, tray "Quit") checks running games and post-game work
     (`session::busy()`). If none: quit as today (but through the graceful path of item 4).
   - If some: show a confirm dialog in the main window (create/show the window first if it is hidden, e.g. tray Quit).
     Text (English, our dialog style; see app/src/dialogs/epic_confirm.rs or emulator_confirm.rs for a confirm pattern):
     title "A game is running", body naming the game(s) and saying the launcher records playtime and runs post-game tasks
     (backups, cloud saves) when it ends. Buttons: "Keep running in tray" (hide window, stay alive; only when a tray icon
     exists — otherwise "Keep open" which just cancels), "Quit anyway", "Cancel".
   - Intercept the window close BEFORE it closes (GPUI `on_window_should_close` / equivalent in gpui-kit 0.7.1) so the
     dialog can show; keep the existing hiding/exitToTray behaviour.
   - "Quit anyway": checkpoint playtime of running games (item 3), then the graceful quit. Games keep running (item 2).
   - Headless `--launch` path (`maybe_exit_idle`) must keep working unchanged.
2. **Games must not depend on the launcher.** Spawn games with stdout/stderr going straight to the log file
   (open the file with append and pass `Stdio::from(File)`; `Sink::None` -> `Stdio::null()`), no pipe, no pump thread.
   Check crates/launch/src/log_writer.rs (`GameLog`): keep the launch.log format the log viewer and Heroic expect
   (header, "Game Log:" section, "End of log" line written by our side). If GameLog buffers or post-processes `raw()`
   (size cap, filtering), find an equivalent that does not need a pipe (e.g. both write with O_APPEND to the same file,
   our lines before spawn and after exit). Keep the process_group(0) (game must survive the launcher). Keep the existing
   "wait for exit" logic working (it must not depend on the pipe EOF any more). Update tests.
3. **Playtime checkpoints.** While a game runs, persist progress every 2 minutes so a crash/kill/power loss loses at most
   ~2 minutes. Keep Heroic/Heronite compatibility of timestamp.json (`totalPlayed` minutes, `lastPlayed`) and
   local_library/sessions.json. Design so nothing is double counted: e.g. the session entry is created/updated in place
   (same id) at each checkpoint and finalised by `finish()`; totalPlayed is incremented by the delta since the last
   checkpoint. Read record.rs / sessions.rs to see the exact formats Heronite writes and keep them. Also checkpoint
   immediately on graceful quit and on signals. Unit-test the no-double-count logic.
4. **Signals = Quit.** Add a SIGTERM/SIGINT/SIGHUP handler (`signal-hook` crate, iterator on a thread, forwarded to the
   GPUI foreground loop like single-instance requests). It runs the graceful quit WITHOUT a dialog: checkpoint running
   games, pause the Epic queue in order (pause path, legendary gets SIGTERM, wait asynchronously up to ~5 s for the
   worker to go idle; never block the UI thread), remove the single-instance socket, shut the tray, then `cx.quit()`.
   A second signal while quitting exits immediately (`std::process::exit(130/143)`). The same graceful path is used by
   the normal Quit (minus the checkpoint dialog part). A paused-on-quit download must still resume next start
   (check how DmState / the queue file is persisted; the element must stay in `queue`).
5. **Second instance.** In main.rs/single_instance.rs: connection refused / no socket = stale -> current behaviour
   (become primary). But if a primary exists and does not answer in time or answers an error: do NOT start a second
   instance. Print an error to stderr, show a desktop notification (app/src/system/notify.rs, notify-rust) like
   "Heronite is already running but is not responding", exit with code 1. Server side: answer "ok" only when the
   request was delivered; at quit, stop accepting / remove the socket first.

## Files you own
app/src/main.rs, app/src/system/{hub,single_instance,cli,notify}.rs, app/src/services/session.rs,
app/src/services/downloads.rs (except `Downloads::new`, where proc-safety adds one call — re-read before editing),
crates/epic/src/manager.rs, crates/launch/src/{runtime,record,log_writer}.rs (+ sessions/playtime code in crates/data if
the checkpoint needs it), new dialog file app/src/dialogs/quit_confirm.rs + its registration in app/src/dialogs/mod.rs,
app/Cargo.toml (signal-hook). Small hook points elsewhere are OK if needed; list them in your report.

## Verification
- Unit tests (checkpoint math, log spawning to file, single-instance reply/timeout handling).
- Live (fixture only, scripts/shot.sh, HERONITE_DEBUG_FAKE_LAUNCH / fake session so no real game starts): show the confirm
  dialog with a debug hook (e.g. HERONITE_DEBUG_DIALOG=quit-confirm or a fake running game) and capture it.
- Signal test: start YOUR OWN test instance (HERONITE_NO_SINGLE_INSTANCE=1, fixture dir), send SIGTERM / SIGINT to THAT
  PID ONLY, check the log says graceful quit and the fixture's files are intact. The user's own heronite-rs instance may
  be running: never signal, kill or forward to it. Use `pgrep -f` with your fixture path to find your PID.
- Run the full suite at the end: `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test --workspace --no-fail-fast`.
