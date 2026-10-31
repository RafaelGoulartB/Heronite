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

## ⬜ To do

- In-game overlay and replay buffer
- Launch extras: scripts before and after a game, programs alongside it, presets
- Settings → Storage
- Troubleshoot for Windows
- Importing games of Battle.net, EA app, Ubisoft Connect, Rockstar and Xbox app
- GOG Galaxy services (`comet`)
- Smaller: RetroAchievements sign-in of emulators, Ludusavi registry entries, exclusive-fullscreen screenshots, dark tray menus, Authenticode signing, installer languages, Steam tests

## ➖ Not planned

Windows runs Windows games directly, so these Linux parts are not needed:

- Wine, Proton, winetricks and store clients under Wine
- Sandbox (bubblewrap)
- Lutris, Bottles, Faugus and Heroic imports, and copying Heroic's setup
- GameMode, MangoHud, gamescope
- The "On Linux" row (ProtonDB)
- Steam's per-game launch options and Proton choice (little used on Windows; Steam has them)
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
