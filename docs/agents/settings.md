# Your brief: GAME SETTINGS + GLOBAL SETTINGS (agent "settings")

Goal: Heroic's per-game settings for installed games (the screenshot the user showed: dialog "Control (Settings)" with
tabs WINE / OTHER / ADVANCED / CLOUD SAVES SYNC / GAMESCOPE / LEGACY) and Heroic's global defaults (General settings +
default game settings), in our app.

Heroic sources to port:
- src/frontend/screens/Settings/** (sections/GamesSettings, GeneralSettings, AdvancedSettings, SyncSaves, components/*
  — every component maps to one config key; read each to get labels, help texts, defaults and conditions such as
  "only for wine/proton", "only Linux", "only when Gamescope enabled"), src/frontend/screens/Settings/components/
  SettingsModal (the per-game modal), Tools (winecfg/winetricks/run exe buttons).
- src/backend/game_config.ts (per-game file GamesConfig/<appName>.json: format {"<appName>": {...}, "version": "v0",
  "explicit": true}, JSON.stringify(…, null, 2), defaults inherited from global config, winePrefix default
  <defaultWinePrefixDir>/<title>), src/backend/config.ts (global config.json "settings" defaults/keys),
  src/common/types.ts (AppSettings / GameSettings types), src/backend/tools/** (winetricks, winecfg, run exe in prefix:
  runWineCommand) and src/backend/launcher.ts (env for running wine tools).
Existing Rust: crates/launch/src/settings.rs (GameSettings reading — extend or add a writer), crates/launch/src/
{env.rs,wine.rs} (env builders for wine/proton/umu, reuse them for winecfg/winetricks/run-exe), crates/data
(json_store, paths, settings.rs for Collection settings), app/src/dialogs/game_settings.rs (OUR existing per-game
"Game settings" dialog with Images/Details/Launch tabs for Collection metadata), app/src/dialogs/settings/** (OUR global
settings dialog: Appearance/Screenshots/Metadata/Emulators/Backup/Saves tabs), app/src/dialogs/form.rs, app/src/ui/
controls.rs, app/src/ui/fields.rs.

Owned files:
- crates/data/src/game_config.rs (new) or crates/launch/src/settings.rs extensions: typed read/write of
  GamesConfig/<app>.json and config.json `settings` (only the keys you edit; preserve everything else; keep the exact
  file formatting Heroic uses for each file — check json_store supports 2-space indent for GamesConfig, add an option
  if needed without changing existing behaviour).
- app/src/dialogs/game_settings.rs (extend) and new files under app/src/dialogs/game_settings/ or
  app/src/dialogs/heroic_settings/ for the new tabs; app/src/dialogs/settings/** (add "General" and "Game defaults"
  tabs; the defaults tab reuses the same Wine/Other/Advanced/Gamescope components bound to config.json).
- app/src/dialogs/mod.rs (register modules).
Hook points allowed: app/src/collection/context_menu.rs — add ONE "Settings" item (icon Tune or Settings) that opens
the game settings dialog for that game (the epic agent also adds items there; keep your edit to a small block);
app/src/collection/mod.rs debug dialog match (`HERONITE_DEBUG_DIALOG=gamesettings:wine` etc.).

Design: merge into ONE "Game settings" dialog per game: existing tabs (Images, Details, Launch) + Heroic tabs
(Wine, Other, Advanced, Gamescope, Cloud saves) shown only when relevant: Wine tab only for games that run through
Wine/Proton (Epic/GOG/Windows sideload games on Linux); Cloud saves only for legendary games with cloud_save_enabled;
"Legacy" can be skipped or folded into Advanced. The dialog title like "Control (Settings)". Opening from the focus
panel's settings button and from the new context-menu item. Settings save automatically on change (like Heroic:
"Settings are saved automatically").
Wine tab: Wine version dropdown fed by `heronite_wine::list_installations(&heroic)` (another agent is implementing it
right now — it currently returns an empty Vec; build against it and also show the currently configured version even if
not in the list), "Valve Proton versions ignored"/showValveProton info, WinePrefix folder (+ folder picker: use
gpui `cx.prompt_for_paths` or our existing picker pattern if any), auto DXVK/VKD3D/DXVK-NVAPI, Esync, Fsync, Msync
(macOS only — skip), Wine-Wayland, WoW64, FSR hack, HDR..., buttons WINECFG / WINETRICKS / RUN EXE ON PREFIX (spawn
via launch crate env builders; implement but DO NOT run them for real — unit-test the command/env construction only).
Other tab: launcher args, wrappers table, env variables table, game mode, mangohud/show FPS, nvidia prime, steam
runtime, prefer system libs, BattlEye/EAC runtime, before/after launch scripts, alternative exe, language, offline
mode, etc. — whatever Heroic shows for that game type on Linux. Advanced: verbose logs, umu disable, etc.
Gamescope: all Gamescope options. Cloud saves: saves path (auto-detect button calling
`heronite_epic::saves::detect_path`), sync buttons calling `heronite_epic::saves::sync(heroic, app, path, mode)`
(stub now, implemented by the epic agent), off the UI thread, result toast.
Global settings dialog: "General" tab (default install path, default wine prefix dir, default Steam path, max workers,
alternative legendary binary, minimize on launch, etc. — the subset meaningful for our app) and "Game defaults" tab
(same Wine/Other/Advanced/Gamescope components editing config.json defaults).
Verify: unit tests for read/modify/write of GamesConfig and config.json against a fixture copy (formatting identical to
Heroic, unknown keys preserved, untouched keys byte-identical); screenshots via shot.sh with
HERONITE_HEROIC_DIR=/tmp/heronite-fixture-settings and HERONITE_DEBUG_SELECT=legendary_Calluna
HERONITE_DEBUG_DIALOG=gamesettings:wine (add such hooks) for each new tab, and for the settings General/Game defaults
tabs.
