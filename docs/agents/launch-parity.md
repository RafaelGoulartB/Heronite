# Your brief: LAUNCH PARITY for Wine/Proton + Epic games (agent "launch")

Context: the Epic, Wine Manager and Heroic game-settings features were just added (see PROGRESSO.md sections
"Wine Manager", game settings, Epic). Settings are now editable, but several things Heroic does AROUND a launch are
missing in our launch flow. Port them so an Epic game launched from our app behaves like in Heroic.

Heroic sources: src/backend/launcher.ts (prepareLaunch, prepareWineLaunch ~l.813-1010, launch flow around
autoSyncSaves l.128-160 and l.340, runWineCommand), src/backend/tools/index.ts (DXVK.getLatest, DXVK.installRemove,
Winetricks), src/backend/wine/runtimes/** (EAC/BattlEye runtime download), src/backend/storeManagers/legendary/games.ts
(launch: skipVersionCheck, update check before launch), src/backend/main.ts l.180-200 + l.830-860 (startup tool
download and toggling DXVK from settings).
Our code: crates/launch/** (plan.rs, legendary.rs, wine.rs, env.rs, runtime.rs — the launch pipeline), app/src/runtime.rs,
crates/epic (saves::sync_flag, update detection), app/src/collection/epic.rs + app/src/dialogs/epic_confirm.rs ("Game
Needs Update" dialog with "Play anyway"), crates/data/src/game_config.rs.

Owned files: crates/launch/** (new modules welcome: e.g. prepare.rs, tools.rs, runtimes.rs), app/src/runtime.rs.
Hook points allowed (minimal): app/src/collection/epic.rs and app/src/dialogs/epic_confirm.rs for wiring "Play anyway"
to skip the version check; app/src/collection/mod.rs only for a status toast/flow call if strictly needed.

Implement:
1. Cloud saves around a launch (legendary games): if `autoSyncSaves` and savesPath set (and online), sync before the
   game starts (Heroic's pre-launch `--skip-upload`-style behaviour — check the exact flags) and after it exits
   (upload), using heronite_epic::saves::sync_flag. Surface errors through the existing LaunchEvent/toast mechanism.
2. "Play anyway" for games with pending updates: pass legendary `--skip-version-check` (LaunchOptions already has a
   field per plan.rs:182 — wire it end to end). Also port Heroic's pre-launch update check prompt if not already done
   by the epic agent (check collection/epic.rs first; don't duplicate).
3. prepareWineLaunch parity for Wine/Proton games launched from our app: prefix creation when missing (Heroic's logic
   per wine type; proton/umu creates its own), DXVK / DXVK-NVAPI / VKD3D install-or-remove in the prefix for
   wine-type versions according to autoInstallDxvk/autoInstallDxvkNvapi/autoInstallVkd3d (DXVK.installRemove: dll
   copies + overrides + backup/restore), downloading the latest DXVK/VKD3D/DXVK-NVAPI into <heroic>/tools/ when missing
   (DXVK.getLatest — same folder names Heronite uses: tools/dxvk/latest_dxvk symlink etc. — read the real
   ~/.config/heroic/tools layout READ-ONLY to match it), EAC/BattlEye runtime download into tools/runtimes when the
   setting is on and the runtime is missing, umu runtime presence. Run prep on the background launch thread, with
   LaunchEvent status updates ("Preparing prefix…", "Downloading DXVK…").
Testing: unit tests with tempdirs and fake binaries; real downloads only into a fixture
(HERONITE_HEROIC_DIR=/tmp/heronite-fixture-launch), never the real tools dir or real prefixes; never launch a game,
never run wine/proton/umu for real (inject a fake runner/Probe/spawner like the existing tests do).
Reply as in common.md and append a short Portuguese section to PROGRESSO.md.
