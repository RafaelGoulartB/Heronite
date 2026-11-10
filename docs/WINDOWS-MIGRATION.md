# Windows migration

What works on Windows, what is left, and what Windows does not need.

✅ works · 🚧 needs a real-world check · ⬜ to do · ➖ not planned

## ✅ Works

- App: window and its controls, tray, single instance, start with Windows, `heronite://` links, keep awake
- Collection: Steam (installed games and the whole library), games added by hand, play time
- Other launchers: Playnite's installed games (Settings → Other launchers), started through Playnite; Heroic's games added by hand and its Epic, GOG and Amazon install records (`%APPDATA%\heroic`), once its import is turned on there. Nothing is copied from Heroic at the first start (`Feature::HeroicMigration` is off, on Linux too)
- Epic Games, GOG and Amazon Games through the Windows builds of legendary, gogdl and nile: login (WebView2 window), library, downloads, installs and moves, first launch, cloud saves, and the games of the official launchers (Epic Games Launcher, GOG Galaxy). Epic has one EOS Overlay switch instead of one per Wine prefix; Amazon's `.msi` installers run through `msiexec`
- GOG Galaxy services (`comet`): Comet runs beside a GOG game's Windows build, so its achievements, stats and leaderboards reach the GOG account without GOG Galaxy, and new unlocks show over the game (`heronite_launch::galaxy`). Settings → Achievements → GOG registers Comet's dummy `GalaxyCommunication` service as administrator (LocalService, in a folder only administrators may change) for the games that look for it. Not yet checked on a real Windows: the service's registration, and a standard user starting it (`sc start GalaxyCommunication`)
- Battle.net, the EA app, Ubisoft Connect and the Rockstar Games Launcher as they are (no Wine): set up with the official installer or found already installed, sign-in and games read from this PC (`heronite_clients::place`), opened and closed by Heronite (`heronite_clients::native`), games started through the client; the session follows the programs running from the game's folder
- Xbox (Windows only, `Feature::XboxStore`): Microsoft Store and PC Game Pass games the Xbox app installed, found among the user's packages (`heronite_platform::packages`, `heronite-xbox`) and started by their AUMID through `explorer.exe shell:AppsFolder\...`
- Settings → Storage: drives, games, store tools, caches, leftover folders (to the Recycle Bin) and old downloads
- Emulators and ROM folders
- RetroAchievements sign-in for RetroArch, including registered installed and portable copies
- Save backups (Ludusavi)
- Screenshots
- Desktop and Start menu shortcuts, Add to Steam
- Installer, portable zip, self-update, releases
- Installer languages: English, Brazilian Portuguese, Spanish and German; silent updates retain the selected language
- Steam's per-game launch options (Settings → Launch of a Steam game, `Feature::SteamGameSettings`): through the running client, or in `userdata\<account>\config\localconfig.vdf` while Steam is closed, as on Linux; the account is the registry's `ActiveProcess\ActiveUser`, else the latest login. No compatibility tool: Steam starts Windows games as they are
- Native launch extras: before/after scripts (`.bat`, `.cmd`, `.ps1`, programs), delayed companions and presets (variables and arguments); for Steam games they run around `steam://rungameid` (`heronite_launch::steam_extras`); real-process tests cover environment, working folders, normal exit, stop and cancellation before a companion starts
- Restoring a backup made on Linux
- Game settings → GPU: the card a game's program asks Windows for ("Power saving" or "High performance", the `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` value of Settings → Display → Graphics), written right before the game starts and put back after it only while it still holds Heronite's value (`heronite_launch::gpu::preference`). DXGI names the card behind each choice. Steam games, store clients, Playnite, Xbox games and emulators keep Windows' own choice; tests use a fake registry
- Steam tests: library and session fixtures run on Windows without reading the host's registry, Steam folders or process list; achievement fixtures are isolated too
- Tray menus follow the app's light/dark theme (including System and Custom) on Windows 10 1903+ and Windows 11. High contrast keeps Windows' native colours. Older Windows versions, or missing uxtheme exports, fall back to the native menu with a warning


## ⬜ To do

- Replay buffer (separate from the overlay; hidden on Windows)
- Mods (`docs/MODS.md`, hidden on Windows): open mod archives without 7-Zip or bsdtar on the PATH, retry putting the game's folder back while a file is locked, and make the cross-drive copy of a backed-up file safe to interrupt
- RetroAchievements sign-in for the remaining emulators (DuckStation, PCSX2, PPSSPP, Dolphin and Flycast): their Windows config locations and, where needed, token encryption still need support
- Ludusavi registry entries: in a backup made on Windows, Ludusavi restores them itself, but they are missing from the restore preview and from the undo copy
- Screenshots of games in legacy exclusive fullscreen: Desktop Duplication loses the display while such a game holds it. HDR capture also still needs a check on a real HDR monitor
- A new tray icon starts in Windows 11's hidden-icons flyout until the user pins it
- Appearance → "From the desktop" (accent and panel colour) reads only Linux desktops (`app/src/system/desktop_colours.rs`); on Windows it gets no colour and falls back. It should read Windows' accent colour and light/dark app mode

## 🚧 Console mode controllers

Console mode reads controllers through SDL 3, linked statically
(`heronite_platform::controllers`): Xbox, PlayStation (DualShock 4,
DualSense), Nintendo (Switch Pro, Joy-Con) and every controller with an SDL
mapping, over USB or Bluetooth, also after Steam switched a PlayStation or
Nintendo controller to its full report (Windows.Gaming.Input, used before,
then saw a controller that never moved). SDL is told not to switch
controllers to their full report itself (`SDL_JOYSTICK_ENHANCED_REPORTS=0`),
so games reading them through DirectInput keep working. Checks with each
controller over USB and Bluetooth, with Steam open and closed, are still
needed.

## 🚧 In-game overlay

Off by default, enabled in Settings → Overlay. It uses native topmost tool
windows over windowed and borderless-fullscreen games, on the foreground
window's monitor. Legacy exclusive fullscreen is not supported: Heronite
does not inject code into games.

- **Keyboard:** Ctrl+Shift+G by default (saved shortcuts are kept), registered
  only while a game runs, or while "Open it without a game too" is on. The
  existing `RegisterHotKey` backend reports conflicts in Settings. Win+G is
  normally owned by Xbox Game Bar. The panel requests focus however it was
  opened; closing it returns focus only if the panel still owns it. Windows
  refuses foreground requests made without user input (a controller read
  through XInput is none): the request is then repeated with the panel's
  thread input attached to the foreground window's thread for that call
  (`AttachThreadInput`). When Heronite's own main window has it (the
  shortcut pressed there), the panel is activated directly, as windows of
  the same thread may be, and checked again 250 ms later so the keys (Esc
  included) reach the panel. No input is simulated or sent to the game.
- **Controller:** an XInput controller's Back+Start acts as Guide when both
  go down within 300 ms (a Back held for a game's map stays Back). Tap the
  combination to open/close; hold it with the configured screenshot button,
  a direction for volume, or Y to mark a moment. Real Guide is not exposed
  by documented XInput and can be claimed by Game Bar or Steam. There is no
  controller grab on Windows: games still receive Back+Start. While the
  panel is open it owns the foreground, so games that ignore the controller
  in the background (SDL's default, Unity, Unreal: most of them) do not see
  the navigation; games that poll XInput regardless still do. Non-XInput controllers need an XInput mapping here (console
  mode reads them through SDL). Battery levels
  are coarse estimates; transport and Bluetooth disconnect are unavailable.
- **Panel:** Home, Screenshots, Notes (including the on-screen keyboard),
  Audio and Controller reuse the Linux views. Screenshot capture uses the
  existing Windows backend.
- **Audio:** Core Audio supplies system volume/mute, microphone mute and
  active app/game audio sessions. The output and the microphone are switched
  from the panel (Y on Home's mixer, the Output and Microphone rows on Audio)
  through IPolicyConfig: undocumented, but what Windows' sound settings use
  and unchanged since Vista (EarTrumpet and SoundSwitch rely on it). It sets
  the device for every role (console, multimedia, communications).
- **Notices:** non-activating, pointer-transparent windows; they do not
  request foreground. Native placement accounts for monitor origin and DPI.
- **Unavailable:** replay recording, gamescope/MangoHud live FPS controls
  and Bluetooth controller disconnect. Replay has its own Linux-only
  `Feature::Replay`, so copied Linux settings cannot start a recorder.

Real-game, physical shortcut/controller, mixed-DPI/HDR and elevated-game
checks are still needed. Background debug hooks cannot prove keyboard
focus handoff because Windows can refuse it without a real hotkey event.
The debug hooks also support native Windows windows:
`HERONITE_DEBUG_OVERLAY=layer[:<app_name>]`; `window` keeps the normal
debug window, and `HERONITE_DEBUG_OVERLAY_MONITOR=<x>,<y>` targets the
monitor holding that desktop point. Never simulate keyboard, mouse or controller input on the
machine: use `HERONITE_DEBUG_OVERLAY_PAD` for in-app navigation.

## ➖ Not planned

Windows runs Windows games directly, so these Linux parts are not needed:

- Wine, Proton, winetricks and store clients under Wine (on Windows they run as they are)
- Sandbox (bubblewrap)
- Lutris, Bottles and Faugus imports (Linux launchers; Windows imports Heroic and Playnite)
- GameMode, MangoHud, gamescope, wrapper commands and the Steam runtime (`Feature::Wrappers`)
- Lossless Scaling through lsfg-vk (a Linux Vulkan layer; on Windows the Lossless Scaling app does it itself)
- The "On Linux" row (ProtonDB)
- Troubleshoot (its checks are about Wine, Proton and Linux drivers)
- AppImage and Linux packages

## RetroArch sign-in

Add the native or portable executable on the Emulators page and open RetroArch
once so it creates its config. Enable emulator sign-in in Settings → Achievements.
Heronite follows RetroArch's Windows search order: `retroarch.cfg` beside each
executable, then `%APPDATA%\retroarch.cfg`. Executables found on `PATH` and the
APPDATA fallback also work without a registered installation; a Scoop shim is
followed to the app's folder, and a Chocolatey shim (whose target cannot be
read) is skipped. Unopened configs are never created, and a running RetroArch
(including a renamed registered executable) is left alone; after a sign-out it
is cleared once the game ends.

Profile `-c` / `--config` paths are written as well as the default config (the
Emulators page opens RetroArch without arguments). Absolute paths, paths
relative to the emulator's working folder and `{EmulatorDir}` are supported;
RetroArch opens the `-c` value as given, so its `:\` prefix does not apply
there. A relative config with a ROM working folder, a drive-relative path
(`C:x.cfg`, often an unquoted `C:\x.cfg`) or a game-dependent config template
is reported in Settings and skipped, rather than editing a guessed file; the
other configs are still written. Signing out clears only configs that still
hold Heronite's account.
DuckStation and the other emulators are intentionally not enabled on Windows by
this change. Real account authentication still needs a check inside RetroArch.

## Rules for code on both systems

- System-specific code lives in `heronite-platform`; Linux-only parts get a `Feature` and are hidden on Windows (`features.rs`; `HERONITE_FEATURES=+epic` in debug builds)
- Paths with `Path::join` and `Path::is_absolute`; files and links opened with `shell::open`
- Files replaced with `temps::write_atomic`; helper programs run with `proc::Run`
- Tests use each system's own tools (`FakeTool` for the stores) and run from PowerShell, not Git Bash
- Never send keys or clicks to the screen of a machine in use

## Folders

| | Linux | Windows | Portable |
|---|---|---|---|
| Data | `~/.local/share/heronite` | `%APPDATA%\heronite\data` | `<folder>\data` |
| Settings | `~/.config/heronite` | `%APPDATA%\heronite\config` | `<folder>\config` |
| Cache | `~/.cache/heronite` | `%LOCALAPPDATA%\heronite\cache` | `<folder>\cache` |

## Checking Linux from Windows

`cargo clippy --target x86_64-unknown-linux-gnu -p <crate>` with
`CC_x86_64_unknown_linux_gnu` and `AR_x86_64_unknown_linux_gnu` pointing to
Visual Studio's LLVM. Crates with C code (sqlite, webp, GTK) and the app are
checked by Linux CI.
