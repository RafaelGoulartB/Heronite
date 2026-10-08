# Your brief: EPIC AUTO-UPDATES + POST-GAME TASKS WITHOUT WINDOW (agent "updates")

1. Auto-update Epic games (config.json `autoUpdateGames`, plus per-game ignore-updates if Heroic has it — check
   `ignoreGameUpdates` / IgnoreGameUpdates.tsx): port Heroic's behaviour (src/backend/downloadmanager/**,
   src/backend/storeManagers/legendary/library.ts update checks, main.ts startup flow, updater/ timers): when updates
   are found (startup check `checkForUpdatesOnStartup` and the periodic check if Heroic has one), queue them through our
   download queue (app/src/services/downloads.rs — the Epic service; crates/epic manager). Respect: Heronite running
   (we already don't auto-resume the queue while Heronite runs — do the same for auto-updates), a game currently
   running (don't update it while playing; queue after exit), metered/offline. Notify via toast and, if the window is
   closed (tray), via the system notification mechanism if one exists (if none, add a minimal one: notify-rust or
   org.freedesktop.Notifications over D-Bus with zbus — check what's already in the dependency tree to avoid bloat).
2. Post-game tasks when no window exists (app/src/system/** hub: the window can be closed to the tray; today the
   post-session work — recording the local session/playtime, Ludusavi backup after play, screenshot capture session
   end, cloud-save upload, library reload — only runs when a CollectionView exists, so it's queued until the window
   reopens). Move that work out of the view into a window-independent service (e.g. in app/src/runtime.rs or a new
   app/src/services/session.rs) so it runs immediately when the game exits, with or without a window; the
   CollectionView just observes/reloads. Find every place where post-game work is tied to the view
   (app/src/collection/mod.rs on_launch_event, collection/emulation.rs, collection/screenshots.rs, system_glue.rs,
   system/hub.rs) and fix them. Show results with a system notification when there is no window (backup saved/failed,
   capture saved).
Owned: app/src/services/** (new session service), app/src/system/**, app/src/runtime.rs, the post-game parts of
app/src/collection/{mod.rs,emulation.rs,screenshots.rs,system_glue.rs}, crates/epic update logic.
Another agent ("logs") works in parallel on game/launcher logs and a log viewer (crates/launch log_writer, settings
page logs section, main.rs logger) — keep your edits to main.rs/runtime.rs minimal and don't touch logging.
Testing: never launch real games or emulators (use HERONITE_DEBUG_FAKE_LAUNCH=1 and fake processes); never run a real
Ludusavi backup (fake binary); real legendary only read-only on the REAL config (never a copy); fixture data dirs for
app runs; GUI only via scripts/shot.sh on workspace 7 (HERONITE_NO_TRAY=1 there; for window-closed tests use the
system agent's debug hooks: HERONITE_DEBUG_START_HIDDEN, CLOSE_AFTER_MS, etc. — see PROGRESSO.md "Sistema"). A system
notification smoke test may show a real notification once or twice; keep it minimal.
