# Windows migration

Heronite was written for Linux. This file tracks the port to Windows: how
the code is organised so both systems stay easy to maintain, what already
works there and what is left.

**First milestone:** the Collection works on Windows. It imports the
installed Steam games by itself, takes games added by hand and launches them,
and restores a backup made by Heronite on Linux.

Legend: ✅ done · 🚧 in progress · ⬜ to do · ➖ not planned for Windows
(Linux only by nature)

## How the code handles two systems

1. **One crate holds what differs between systems: `heronite-platform`.**
   Processes (alive, stop, groups, detached starts, a clock that survives a
   restart), files (permission bits, executables, file identity), the app's
   folders, opening a file or URL, and the Windows registry. Its API is the
   same on every system; `process/unix.rs` and `process/windows.rs` hold the
   two implementations. Other crates call it instead of `libc` or
   `std::os::unix`, so they need no `cfg`.
2. **Code that only makes sense on one system stays where it is, behind
   `cfg(target_os = "linux")`** (`/proc`, evdev, bubblewrap, AppImage). What
   the rest of the app needs from it on Windows is a small stub or a Windows
   implementation of the same trait next to it (for example
   `heronite_launch::procs`: the `Procs` trait, read from `/proc` on Linux
   and from a Toolhelp snapshot on Windows).
3. **`heronite_platform::features` says which parts of the app a system
   has.** Linux has all of them. The UI asks `supported(Feature::Wine)` before
   it shows a page, a menu entry or a setting, and services that could only
   fail do not start. A part turns on for Windows by moving it out of the
   `false` list in `features.rs` once it works there.
4. **Everything compiles everywhere.** Linux-only crates (Wine, the stores,
   the overlay) still build on Windows, they are just switched off by (3).
   This keeps `cargo check --workspace` meaningful on both systems and
   avoids a maze of `cfg` on imports.

### Rules for new code

- No `std::os::unix`, `libc::` or `windows_sys::` outside
  `heronite-platform` and modules already behind `cfg(target_os = "linux")`.
- Paths: build them with `Path::join`, never with `format!("{a}/{b}")`, and
  test "is absolute" with `Path::is_absolute`, not `starts_with('/')`.
- Opening a file or URL: `heronite_platform::shell::open`, never `xdg-open`.
- A new Linux-only part of the app gets a `Feature` and is hidden on Windows.
- `~` is the home folder only at the start of a path
  (`game_config::expand_tilde`): Windows' short names contain it
  (`C:\Users\RUNNER~1`).
- Replacing a file: `temps::write_atomic` (or `fs::rename_replacing`), which
  waits for Windows to let go of the target.
- Tests that run programs use what each system has (`sh -c` on Unix,
  `cmd /C` and `ping` on Windows) when the code they test runs on both, and
  are `#[cfg(unix)]` when they test a Linux-only part (Wine, bubblewrap,
  shell scripts as programs). Run them from PowerShell or `cmd`, not Git
  Bash, which adds Unix tools and `HOME`.
- `drop()` on a value that is a guard on one system only (empty on the
  other) fails clippy there: give the guard a method that says what it does
  (`release`).

## Folders on Windows

| | Linux | Windows |
|---|---|---|
| Data (`DataPaths`) | `~/.local/share/heronite` | `%APPDATA%\heronite\data` |
| Settings and UI state (`AppPaths::config`) | `~/.config/heronite` | `%APPDATA%\heronite\config` |
| Caches (`AppPaths::cache`) | `~/.cache/heronite` | `%LOCALAPPDATA%\heronite\cache` |

`HERONITE_DATA_DIR` works the same on both.

## Status

### Building

- ✅ `heronite-platform` crate (processes, files, folders, shell, registry, features)
- ✅ Every library crate builds on Windows
- ✅ The app (`heronite`) builds on Windows, no warnings
- ✅ Linux still type-checks (`cargo check --target x86_64-unknown-linux-gnu` from Windows, see "Checking Linux from Windows")
- ✅ `cargo test --workspace` on Windows: tests of Linux-only parts (Wine, shell scripts, symlinks, Linux paths) run only on Unix
  - ⬜ Steam library and session tests (`TODO(windows)` in `heronite-launch`): `SteamLocator` reads this machine's Steam from the registry, so they need a way to leave it out first
- ✅ `cargo clippy --workspace --all-targets` clean on Windows
- ✅ CI job on `windows-latest`: clippy and tests (`.github/workflows/ci.yml`)

### First milestone: the Collection

- ✅ App starts on Windows (window, quit, logs to its file; no console window in release builds)
  - ✅ Single instance: loopback port + key in the user's temp folder (`system/single_instance/transport.rs`)
  - ✅ Signals: none on Windows (`system/signals.rs`)
  - ✅ Tray: an icon in the notification area with the Linux menu (recent games, Show / Hide, Quit), a left click shows or hides the window, "Keep in the tray when closed" and "Start minimized" apply (`heronite_platform::tray`, `system/tray/windows.rs`; the menu is rebuilt when it opens, the icon comes back after Explorer restarts)
  - ✅ Window controls: minimize, maximize / restore and close in the app's style at the right of the top bars, and the empty part of a bar moves the window (`ui/window_controls.rs`; window control areas, so Windows does the work)
- ✅ Data, settings and cache folders in `%APPDATA%` / `%LOCALAPPDATA%`
- ✅ Steam: find the client (registry `SteamPath`, `steam.exe`), its libraries (Windows paths in `libraryfolders.vdf`) and the installed games
- ✅ Steam: installed games come into the Collection by themselves (`heronite_launch::install_state`, at start and on refresh)
- ✅ Steam: the whole library (owned games, installed or not, with Steam's play time) is read from the running client, as on Linux: the `steamwebhelper` command line (`process::command_line`) gives the debugging port and the Steam folder; Steam runs when its registry pid lives or a `steam.exe` runs. Checked on this PC: 34 games added besides the installed one
- 🚧 Steam: play through `steam://rungameid` (`shell::open`); the session follows Steam's `console_log.txt` like on Linux (to verify with a real game)
- ⬜ Steam's per-game settings (launch options, Proton) are off on Windows (`Feature::SteamGameSettings`)
- ✅ Games added by hand: a Windows program runs as it is, from its folder, with its arguments and environment variables (`plan::build::windows_plan`)
- ✅ Play time and sessions of native Windows games: the job object follows the game, the session is recorded when its last process ends (checked with `charmap.exe` through `--no-gui --launch`)
- ✅ Add game dialog: no Linux tab, no Wine section, no "Run installer first", no store note on Windows
- ✅ Game settings: a Windows game on Windows has only the Launch tab (`Feature::LaunchExtras`)
- ✅ A game's menu: no Troubleshoot on Windows (shortcuts and save backups: see below)
- ✅ Restore a backup made on Linux: paths into the backup's data, settings and home folders become this machine's, with `/` separators (valid JSON, Windows takes them). Checked end to end: a backup claiming `/home/rafael/...` folders restored at the app's start into a Windows data folder
- ✅ Open folders, files and links (`shell::open` instead of `xdg-open`)
- ✅ Hide on Windows what is not ported yet (`Feature`): ☰ pages, command palette, console mode menu, Settings sections and General's rows (desktop and console mode), Getting started, the empty Collection; store, client and launcher background jobs and the Wine/store tools updates do not start
- ✅ No copy of Heroic's folder at the first start on Windows (`Feature::HeroicMigration`)

### Later

- 🚧 Epic, GOG and Amazon Games, natively (on in Windows builds; to be checked with real accounts: login, a download with pause and resume, first launch with its installers, play time, cloud saves, move, uninstall)
  - One feature per store (`Feature::EpicStore`, `GogStore`, `AmazonStore`); the Downloads page and its settings show when any is on (`features::any_store`). `HERONITE_FEATURES=+epic,-gog` turns features on or off in debug builds
  - Tools: the Windows builds of legendary 0.21.1, gogdl v1.3.0 and nile v1.2.0 (standalone `.exe`, no Python), downloaded into `<data>\tools\bin` and run with no console window (`store_tools`); their config stays in the data folder
  - Login: `heronite-epic-login.exe`, a Win32 window with WebView2 (InPrivate, its data in the cache folder), shipped in the installer and the portable zip; without the WebView2 runtime it says so and the browser method stays
  - Stopping a download: Ctrl+C to the tool's own hidden console, then a kill after the grace time (`process::ask_to_stop`); legendary and gogdl keep their resume files, nile gets 2 s
  - Which builds run here: one rule, `host::run_mode` (Windows builds natively, Linux and macOS builds not at all); the install dialogs only offer builds this PC runs
  - Launch: the game itself, not the tool. Epic: `legendary launch --json` gives the command (sign-in arguments included) and Heronite starts it, so the session lasts as long as the game; GOG: the play task of `goggame-<id>.info`; Amazon: `fuel.json` with the variables nile sets. EA app and Ubisoft Connect games of Epic open through those apps (their play time is not tracked)
  - First launch (`native_setup`): prerequisites, GOG's redistributables and scripts, Amazon's `PostInstall` run once per game folder; installers that need it go through the administrator prompt (`shell::run_elevated`); a declined prompt is a warning and it asks again next time
  - Cloud saves: legendary and gogdl resolve the real Windows folders (Saved Games, AppData, Documents, OneDrive), no prefix
  - EOS Overlay: one switch for all games (the registry, no prefix)
  - Default install folder: `%USERPROFILE%\Games\Heronite`; moves across drives copy with progress and can be cancelled
  - Games other launchers installed, without downloading them again: Epic Games Launcher (`legendary import` one game at a time; not `egl-sync`, which links the games to the launcher), GOG Galaxy (its registry keys; Galaxy keeps updating, moving and uninstalling them), Amazon Games app (a copy of its install database, then `nile import`; the app keeps its files). After login and from a button on each store's page
  - ⬜ GOG's `comet` (Galaxy services: achievements and some multiplayer in games that use them)
- ⬜ Store clients (Battle.net, EA app, Ubisoft Connect, Rockstar) run natively
- ✅ Emulators and ROM folders (`Feature::Emulation`)
  - Finding emulators: one catalog with each emulator's Linux and Windows names; `PATH` (Scoop shims lead to the real program), Program Files, `%LOCALAPPDATA%\Programs`, `%LOCALAPPDATA%`, `%APPDATA%`, the system drive (`C:\RetroArch-Win64`), Scoop's `apps\<app>\current`, EmuDeck's `%APPDATA%\EmuDeck\Emulators` and the usual `Emulation` / `Emulators` / `Games` folders; only `.exe`, helpers (`retroarch_debug.exe`...) skipped; RetroArch cores are `.dll`
  - Launching: the emulator runs as it is, from its folder, with the profile's arguments and the ROM; emulators set up for Wine on Linux (a restored backup) run directly; Flatpak emulators show as not installed
  - ROM folders and scanning: Windows paths (both separators, drive roots, case ignored, CRLF playlists); the automatic setup looks in `ROMs`, `Emulation\roms`, `Games\ROMs`, `Documents\ROMs`... of the home folder and the system drive
  - Save states: PCSX2, DuckStation, PPSSPP, Dolphin, RetroArch, mGBA and RPCS3 folders on Windows (portable installs too)
  - ✅ ROM folders and emulators on every fixed or removable drive, never network ones (`fs::fixed_drives`)
  - ✅ Dolphin's user folder from the registry (`HKCU\Software\Dolphin Emulator\UserConfigPath`), after a portable copy's own folder as Dolphin does
  - ⬜ RetroAchievements sign-in of the emulators (`heronite_achievements::emulators` knows their Linux config files only)
  - ⬜ RetroArch from Steam and Microsoft Store builds of emulators
- ✅ Save backups with Ludusavi (`Feature::SaveBackups`): after play, by hand, the list of backups, restore and undo, checked end to end with the real `ludusavi.exe` 0.31
  - Finding it: the program set in Settings, `ludusavi.exe` on `PATH` (a Scoop shim leads to the real program), `%LOCALAPPDATA%\Programs\Ludusavi`, winget's `Links` and `Packages`, Scoop's `apps\ludusavi\current`, `%ProgramFiles%\Ludusavi`; nothing is added to `PATH` on Windows
  - Its config: `ludusavi config path`, else a portable Ludusavi's own folder, else `%APPDATA%\ludusavi\config.yaml`
  - Backups: no `--wine-prefix`; Windows paths in its output, backup folders compared ignoring case and slash direction
  - Restores: files go back where they were backed up from; only what moved is redirected (the game's folder in another Steam library, Steam's `userdata`, another Windows user's folder, the Proton prefix of a backup made on Linux); Linux paths, Wine registry files and drives this PC lacks are left out ("No place on this PC"); Steam's `RunningAppID` blocks a restore while the game runs
  - The Saves tab: Steam games and Windows programs added by hand
  - ⬜ Windows registry entries in Windows-made backups: Ludusavi restores them itself; they are not in the preview nor in the undo copy
- ✅ Screenshots (`Feature::Capture`): a `RegisterHotKey` shortcut only while a game runs (on its own thread; a key another program holds is reported), the game window's monitor captured with DXGI Desktop Duplication (GDI `BitBlt` when it is not available), saved, named and compressed like on Linux; an on-screen notice that never takes the focus and is left out of captures (`crates/media/src/capture/win/`)
  - HDR desktops: the capture is scaled to Windows' SDR white level and highlights are compressed into 8-bit sRGB (only unit-tested; no HDR monitor here)
  - ⬜ Legacy exclusive-fullscreen games (Desktop Duplication loses the display while they hold it)
  - ⬜ The controller's capture (through the overlay, `Feature::Overlay`)
- ✅ Tray icon (see the first milestone)
  - ⬜ Dark tray menus (needs an undocumented uxtheme call); new icons start in Windows 11's hidden-icons flyout until the user pins them
  - ✅ A `WM_CLOSE` to the tray window (plain `taskkill` while the app sits in the tray) quits gracefully, the Windows counterpart of SIGTERM
- ✅ Start with Windows: a `Heronite` value in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`system/autostart.rs`, `registry::set_string` / `delete_value`); only the real data folder drives it, `HERONITE_AUTOSTART_KEY` redirects it for tests
- ✅ Desktop and Start menu shortcuts (`Feature::DesktopShortcuts`): `.lnk` files written by the shell's own ShellLink object (`heronite_platform::shortcut`), starting `heronite-rs.exe --launch <runner> <appName>`; one API for both systems (`heronite_shortcuts::menu`), `.desktop` files on Linux. Desktop and `%APPDATA%\Microsoft\Windows\Start Menu\Programs` (`HERONITE_DESKTOP_DIR` / `HERONITE_APPLICATIONS_DIR` override them); the icon is the game's picture as an `.ico` (PNG inside), or the program's own
  - ✅ Add to Steam: Steam's folder from the registry, `steam.exe` counted as a running Steam; Steam must have its debugging interface on or be closed (Linux's `steam://addnonsteamgame` fallback has no Windows counterpart)
- ✅ Packaging (`scripts/windows/package.ps1`): `Heronite-Setup-x86_64.exe` (Inno Setup, `packaging/windows/heronite.iss`, per user in `%LOCALAPPDATA%\Programs\Heronite`, no administrator rights; Start menu and optional desktop shortcut; the uninstaller removes the `Run` value and `heronite://`) and `Heronite-Portable-x86_64.zip`; the program has its icon and version information (`app/build.rs`)
  - ✅ Releases: the release workflow builds them next to the Linux files, and one job signs everything with minisign and publishes one release; pull requests that touch the packaging build them as artifacts (`windows-packages.yml`)
  - ⬜ Authenticode signing: ready but off until there is a certificate (secrets `WINDOWS_SIGN_PFX_BASE64` / `WINDOWS_SIGN_PASSWORD`, or `WINDOWS_SIGN_COMMAND`); until then SmartScreen warns on the first run and the release notes say so
  - ⬜ The installer in other languages (English only)
- ✅ Portable copy: a `portable.txt` next to `heronite.exe` (the program is `heronite-rs.exe` only in the installed copy) (`dirs::PORTABLE_MARKER`) keeps the data, settings and cache in `data`, `config` and `cache` of that folder (`dirs::portable_root`), and the copy writes nothing to the registry on its own (no `heronite://`; Start with Windows only if the user turns it on)
- ✅ Self-update (`Feature::SelfUpdate`), files checked with minisign like the AppImage:
  - Installed copy (a `package-format` file holding `windows-installer` next to the program): the setup is downloaded and runs as the app quits (`/VERYSILENT /WAITPID=<pid>`), and starts the new version when the update came from "Restart" (`/RELAUNCH=1`); log in the cache folder (`update-setup.log`)
  - Portable copy: the zip is unpacked in the copy's cache (nothing outside `Heronite/`, no links), each replaced file is renamed `<name>.old` (a running program can be renamed) and the new one moved in, all undone on a failure; the `.old` files go at the next start
  - A development build (neither file) and a release without the Windows files only say a new version exists
- ➖ Wine / Proton, Wine Manager, winetricks
- ➖ Sandbox (bubblewrap)
- ➖ Lutris, Bottles, Faugus and Heroic (Linux) imports
- ⬜ Overlay and replay buffer (needs a different capture and input stack); today `heronite_overlay::input` and `heronite_media::capture` have do-nothing stand-ins off Linux
- ➖ Copy an existing Heroic for Windows setup at the first start (`Feature::HeroicMigration` stays off: a fresh Windows setup starts empty)
- ✅ System information (Logs, About): processor, memory, graphics cards and Windows version from the registry and `GlobalMemoryStatusEx` (`heronite_platform::system`)
- ✅ `heronite://` links: registered for the user at every start (`HKCU\Software\Classes\heronite`, `system/protocol.rs`); only the real data folder does it
- ✅ Keep the computer awake while a game runs (`heroniteKeepAwake`): `power::KeepAwake` holds `SetThreadExecutionState` for the session (`ProcessPlan::hold_awake`)
- ✅ Line endings: `.gitattributes` (`eol=lf`), Windows checkouts match Linux

## Checking Linux from Windows

`rustup target add x86_64-unknown-linux-gnu`, then `cargo check --target
x86_64-unknown-linux-gnu -p <crate>` with `CC_x86_64_unknown_linux_gnu` set to
the `clang.exe` of Visual Studio's LLVM. Crates that compile C code with
libc headers (`libsqlite3-sys`, `libwebp-sys`, rcheevos) cannot be checked
this way; CI on Linux covers them.
