# Your brief: LAUNCH FROM OUTSIDE, SHORTCUTS, TRAY (agent "system")

1. Launch a game without the UI: `heronite-rs --launch <runner>_<appName>` (or `--launch <runner> <appName>`) and
   Heroic protocol URLs as argument: `heroic://launch/<runner>/<appName>`, `heroic://launch?appName=..&runner=..`
   (check src/backend/protocol.ts for all forms Heroic accepts) plus our own scheme `heronite-rs://` with the same
   paths. Behaviour like Heroic: if the app is already running, forward the request to it (single instance via a unix
   socket in $XDG_RUNTIME_DIR, name derived from the data dir so a fixture instance never talks to the user's instance);
   otherwise start, launch the game, and (Heroic's hideWindowOnProtocolLaunch / start minimized semantics) don't show
   the main window if configured — then stay in the tray while the game runs and exit when it ends if no window was
   ever shown (check Heroic's behaviour and mirror it sensibly). Also support `--no-gui`-like semantics if Heroic has.
   IMPORTANT: the user has their own heronite-rs instance running right now. Single-instance must be disabled when
   HERONITE_NO_SINGLE_INSTANCE=1, and you must set that (and HERONITE_NO_TRAY=1) in scripts/shot.sh's default env so
   screenshot runs of all agents never forward to / interfere with the user's instance. Use a fixture data dir for
   every manual test (HERONITE_HEROIC_DIR=/tmp/heronite-fixture-system → different socket name).
   Never actually launch a game: test the CLI parsing/forwarding with a debug hook (e.g. HERONITE_DEBUG_FAKE_LAUNCH=1
   that logs instead of launching) and unit tests.
2. Shortcuts (Heroic src/backend/shortcuts/**): desktop (~/Desktop or XDG desktop dir) and application-menu
   (~/.local/share/applications) .desktop files per game with icon (Heroic downloads/extracts an icon — mirror it; the
   shared images cache ~/.config/heroic/icons exists), Exec pointing at OUR binary (`heronite-rs --launch ...`); and
   "Add to Steam" as a non-Steam game (shortcuts/nonesteamgame: binary shortcuts.vdf read/modify/write for every Steam
   user, artwork into grid/ folder, Steam must not be running or the change is lost — mirror Heroic's checks).
   UI: context-menu items "Add shortcuts"/"Remove shortcuts" and "Add to Steam"/"Remove from Steam" (one small block in
   app/src/collection/context_menu.rs), and honour config.json addDesktopShortcuts/addStartMenuShortcuts/
   addSteamShortcuts when an Epic install finishes (hook in app/src/services/downloads.rs, small) or a sideload game is
   added (another agent "addgame" is building that dialog — expose a function `shortcuts::on_game_added(...)` they or I
   can call; coordinate by documenting it). Add these three toggles to the General settings tab if not present
   (app/src/dialogs/settings/heroic.rs or wherever General lives — small edit).
   SAFETY: never write the real ~/.local/share/Steam/userdata/*/config/shortcuts.vdf, the real Desktop or the real
   ~/.local/share/applications in tests — make all roots injectable (env overrides like HERONITE_DESKTOP_DIR,
   HERONITE_APPLICATIONS_DIR, HERONITE_STEAM_DIR) and test in /tmp. Unit-test the VDF binary format round trip
   (byte-identical re-serialization of a real shortcuts.vdf copied to /tmp — read-only copy).
3. Tray icon (StatusNotifierItem, Linux): use the `ksni` crate (pure Rust D-Bus; check the version compatible with
   our async setup — it may need tokio or its blocking API). Menu: recent games (Heroic's recent list:
   config.json "games.recent" / store — check where Heronite keeps it; launching via the same path as --launch),
   Show/Hide window, Quit. Heroic settings: exitToTray (closing the window keeps the app in the tray), startInTray,
   noTrayIcon, minimizeOnLaunch, darkTrayIcon — support the ones that make sense; add them to the General tab.
   GPUI: closing the window should, with exitToTray, drop the window (freeing GPU memory) and keep the app alive;
   "Show" recreates it (refactor app/src/main.rs window creation into a function). Make sure the app quits when it
   should (no zombie process).
   Testing the tray: at most a couple of brief runs (the icon appears in the user's real bar while the app runs; that is
   acceptable but keep it short); everything else with HERONITE_NO_TRAY=1.
Owned: app/src/main.rs, new app/src/system/** (cli.rs, single_instance.rs, tray.rs, shortcuts UI glue), new crate
crates/shortcuts (desktop files, Steam VDF) or modules in an existing crate, scripts/shot.sh (env defaults only),
scripts/install.sh (register `x-scheme-handler/heronite-rs` MimeType in our desktop file; do NOT claim heroic://).
Hook points: app/src/collection/mod.rs (handling forwarded launch requests / window show — keep small; the
"collection" agent also edits this file), context_menu.rs one block, services/downloads.rs one hook, settings General
tab small additions.
