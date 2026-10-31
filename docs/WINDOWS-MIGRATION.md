# Windows migration

What works on Windows, what is left, and what Windows does not need.

✅ works · 🚧 needs a real-world check · ⬜ to do · ➖ not planned

## ✅ Works

- App: window and its controls, tray, single instance, start with Windows, `heronite://` links, keep awake
- Collection: Steam (installed games and the whole library), games added by hand, play time
- Emulators and ROM folders
- Save backups (Ludusavi)
- Screenshots
- Desktop and Start menu shortcuts, Add to Steam
- Installer, portable zip, self-update, releases
- Restoring a backup made on Linux

## 🚧 Needs a real-world check

- Playing Steam games
- Epic Games, GOG and Amazon Games: login, downloads, first launch, cloud saves, the official launchers' games
- Settings → Storage: the drives, games, store tools, caches, leftover folders (to the Recycle Bin) and old downloads
- Launch extras: scripts before and after a game (`.bat`, `.cmd`, `.ps1`, programs), programs alongside it (started as they are, closed with the game), presets (variables and arguments; Windows has no wrappers)

## ⬜ To do

- In-game overlay and replay buffer
- Importing games of Battle.net, EA app, Ubisoft Connect, Rockstar and Xbox app
- GOG Galaxy services (`comet`)
- RetroAchievements sign-in of the emulators: `heronite_achievements::emulators` only knows their Linux config files (`~/.config`, Flatpak), not where they keep them on Windows (`%APPDATA%`, portable folders)
- Ludusavi registry entries: in a backup made on Windows, Ludusavi restores them itself, but they are missing from the restore preview and from the undo copy
- Screenshots of games in legacy exclusive fullscreen: Desktop Duplication loses the display while such a game holds it. HDR capture also still needs a check on a real HDR monitor
- Dark tray menus: the tray menu stays light in dark mode, because the fix needs an undocumented uxtheme call. A new tray icon also starts in Windows 11's hidden-icons flyout until the user pins it
- Authenticode signing: ready but off until there is a certificate (secrets `WINDOWS_SIGN_PFX_BASE64` and `WINDOWS_SIGN_PASSWORD`, or `WINDOWS_SIGN_COMMAND`). Until then SmartScreen warns on the first run, and the release notes say so
- Installer languages: the Inno Setup installer is English only, while the app also speaks pt-BR, es and de
- Steam tests: the Steam library and session tests (`TODO(windows)` in `heronite-launch`) only run on Linux. `SteamLocator` reads this machine's real Steam from the registry, so the fake Steam of a test is not seen. It first needs a way to leave the registry out

## ➖ Not planned

Windows runs Windows games directly, so these Linux parts are not needed:

- Wine, Proton, winetricks and store clients under Wine
- Sandbox (bubblewrap)
- Lutris, Bottles, Faugus and Heroic imports, and copying Heroic's setup
- GameMode, MangoHud, gamescope, wrapper commands and the Steam runtime (`Feature::Wrappers`)
- The "On Linux" row (ProtonDB)
- Steam's per-game launch options and Proton choice (little used on Windows; Steam has them)
- Troubleshoot (its checks are about Wine, Proton and Linux drivers)
- AppImage and Linux packages

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
