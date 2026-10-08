# Your brief: EPIC GAMES (agent "epic")

Goal: full Epic Games Store support like Heroic: log in to the Epic account, Epic library integrated in the
Collection, install/download (with a download queue and progress), update, verify/repair, uninstall, import an
existing install, move install, cloud saves sync — all through the `legendary` CLI exactly as Heroic does it.

Heroic sources to port (read them thoroughly):
- src/backend/storeManagers/legendary/{library.ts,games.ts,user.ts,setup.ts,constants.ts,electronStores.ts,commands/**}
- src/backend/downloadmanager/** (queue, store/download-manager.json format), src/backend/storeManagers/storeManagerCommon/games.ts
- src/backend/launcher.ts (runRunnerCommand / callRunner: env such as LEGENDARY_CONFIG_PATH, progress parsing)
- frontend: src/frontend/screens/Login/**, src/frontend/screens/WebView/index.tsx (Epic login flow),
  src/frontend/components/UI/DialogHandler, src/frontend/screens/Library/components/InstallModal/** (install dialog),
  src/frontend/screens/DownloadManager/** (downloads page).
Existing Rust to reuse: crates/launch/src/legendary.rs (find_legendary, launch), crates/data (paths, library loading
of store_cache/legendary_library.json, json_store), app/src/runtime.rs (`GameRuntime::install` currently returns
"Installing Epic games is not available yet" for legendary — replace that path).

Owned files (create/edit freely):
- crates/epic/** (skeleton exists; KEEP the `saves::{sync, detect_path, SyncMode}` signatures — the settings agent
  calls them from the game-settings "Cloud saves" tab; implement them).
- app/src/services/** (new: e.g. `services/mod.rs`, `services/downloads.rs` — a GPUI global/entity DownloadManager
  that owns the queue, runs one install at a time on a background thread, exposes progress and emits events;
  register `mod services;` in app/src/main.rs).
- app/src/pages/downloads.rs, app/src/pages/epic.rs (replace the placeholders; keep `new(ctx, window, cx)` and
  `EventEmitter<PageEvent>`).
- app/src/dialogs/epic_install.rs (+ any other new dialog files you need, e.g. epic_uninstall / confirm / move).
- app/src/collection/epic.rs (new: Collection-side glue: install dialog opening, Epic context-menu items, progress
  helpers). You may also create a separate binary crate for the webview login helper (see below).
Allowed hook points in shared files (keep each edit minimal, a few lines):
- app/src/collection/mod.rs: `mod epic;`, a field for download progress/subscription, calling your glue from
  `primary_action` for uninstalled legendary games (open install dialog instead of runtime.install), reload when an
  install finishes. app/src/runtime.rs: the legendary branch of `install`.
- app/src/collection/context_menu.rs: ONE call that appends your Epic items (update, verify/repair, move, uninstall,
  "Cloud saves sync") — implement them in collection/epic.rs.
- app/src/collection/card.rs and focus.rs: a progress indicator for games being installed/updated (e.g. thin
  progress bar + percent on the card, progress + cancel in the focus panel footer where the play button is), and an
  "Update available" hint. Keep the existing look otherwise.
- app/src/dialogs/mod.rs: `pub mod epic_install;` (and your other dialogs). app/src/main.rs: `mod services;`.
- app/Cargo.toml: dependencies you need. Workspace Cargo.toml only if you add a new crate dir (crates/* glob already
  covers crates/epic-login if you create it — make sure it has a valid Cargo.toml + src before saving anything else).

Requirements:
1. Login (pages/epic.rs): Heroic logs in with a WebView at https://www.epicgames.com/id/login?responseType=code and
   reads the authorizationCode from the redirect JSON, then runs `legendary auth --code <code>`. Implement:
   a) Primary: an embedded login window in a SEPARATE helper binary (new crate `crates/epic-login`, bin
      `heronite-epic-login`) using `wry` + `tao` (webkit2gtk-4.1 2.52 and gtk3 dev files are installed) that opens the
      Epic login URL, detects the code (the JSON page https://www.epicgames.com/id/api/redirect... contains
      "authorizationCode") and prints it to stdout, then exits. The main app spawns it and reads stdout. Keep it
      out of the main binary so GPUI memory stays low. If wry/tao integration turns out infeasible, document why.
      IMPORTANT: you cannot click in it, so do not open it except via a non-interactive smoke test that verifies the
      window starts on workspace 2 (you may add a `--smoke` flag that loads a local page and exits). Never type
      credentials.
   b) Fallback always available: "Open login page in browser" (xdg-open https://legendary.gl/epiclogin) + an input
      to paste the authorization code or the JSON blob (Heroic's SIDLogin), then `legendary auth --code`.
   Show account (displayName from legendaryConfig/legendary/user.json — never print tokens), logout
   (`legendary auth --delete` + configStore userInfo removal like Heroic), "Refresh library", "Import game"
   (legendary import <app> <path>), basic stats (owned/installed counts). Update config.json `userInfo` exactly as
   Heroic does so Heronite shows the same login state.
2. Library refresh: port Heroic's legendary library refresh so store_cache/legendary_library.json (and
   legendary_install_info.json etc. as Heroic writes them) are produced in the SAME format; the Collection then
   reloads (PageEvent::LibraryChanged / the existing mtime watcher). Include update detection
   (`legendary list-installed --check-updates --json` or what Heroic uses) and expose "update available" per game.
3. Install: install dialog (path defaulting to config.json defaultInstallPath, platform choice when available,
   download/install size from `legendary info --json`, free disk space, DLC selection, SDL (selective download) list
   like Heroic's thirdParty/sdl handling if present, language if relevant). Queue → `legendary install` with the same
   args Heroic uses, progress parsing (percent, downloaded, speed, ETA) like Heroic's regexes, pause/cancel/resume
   (Heroic kills the process and resumes later using legendary's resume file), finish → library refresh + write
   GamesConfig defaults only if Heroic does so at install time. Persist queue/finished in store/download-manager.json
   in Heroic's DMQueueElement format so Heronite sees the history.
4. Downloads page: current download with progress bar, speed, ETA, pause/resume/cancel; queued items (reorder/remove);
   finished history (clear). Badge/indicator: show progress in the ☰ menu entry if simple (optional).
5. Collection integration: uninstalled Epic game → Play button becomes Install (opens the dialog); installing game shows
   progress on card + focus footer; context menu: Update (if available), Verify & repair, Move install, Uninstall
   (confirm dialog with "remove prefix" option like Heroic), Cloud saves sync. After each operation reload library.
6. Cloud saves: implement `saves::sync` / `saves::detect_path` (Heroic: games.ts syncSaves + getSavesPath logic).

Testing constraints (VERY IMPORTANT):
- legendary rotates the OAuth refresh token in legendaryConfig/legendary/user.json on most network commands. NEVER run
  legendary against a COPY of the config (that would invalidate the real token and log Heronite out). For manual tests
  with real network use the REAL legendary config dir (LEGENDARY_CONFIG_PATH=~/.config/heroic/legendaryConfig/legendary)
  but only READ-ONLY commands: `list --json`, `info <app> --json`, `list-installed --check-updates --json`,
  `install <app> --dry-run --base-path /tmp/...` is OK only if legendary's --dry-run does not download (check first).
  Never `auth --delete`, never install/uninstall/repair/move/import real games, never sync-saves for real.
- Any writes of OUR files (store_cache, download-manager.json, config.json) only into a fixture:
  cp -r /tmp/heronite-fixture /tmp/heronite-fixture-epic (or copy JSON from ~/.config/heroic), and inside it make
  `legendaryConfig` a SYMLINK to the real ~/.config/heroic/legendaryConfig (so tokens stay single-copy); run the app
  with HERONITE_HEROIC_DIR=/tmp/heronite-fixture-epic. Make sure our code then never writes into legendaryConfig
  except through legendary itself.
- Progress parsing: unit-test against sample legendary output lines (from Heroic tests/source or legendary source).
- Add debug hooks: HERONITE_DEBUG_PAGE=downloads|epic already exists; add e.g. HERONITE_DEBUG_DIALOG=epic-install:<appName>
  and a fake progress mode (HERONITE_DEBUG_FAKE_DOWNLOAD=<appName>) to screenshot progress UI without downloading.
