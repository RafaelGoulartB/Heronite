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
- ✅ A game's menu: no Ludusavi, Troubleshoot or shortcuts on Windows
- ✅ Restore a backup made on Linux: paths into the backup's data, settings and home folders become this machine's, with `/` separators (valid JSON, Windows takes them). Checked end to end: a backup claiming `/home/rafael/...` folders restored at the app's start into a Windows data folder
- ✅ Open folders, files and links (`shell::open` instead of `xdg-open`)
- ✅ Hide on Windows what is not ported yet (`Feature`): ☰ pages, command palette, console mode menu, Settings sections and General's rows (desktop and console mode), Getting started, the empty Collection; store, client and launcher background jobs and the Wine/store tools updates do not start
- ✅ No copy of Heroic's folder at the first start on Windows (`Feature::HeroicMigration`)

### Later

- ⬜ Epic, GOG and Amazon Games (legendary, gogdl and nile have Windows builds)
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
- ⬜ Save backups with Ludusavi (`Feature::SaveBackups`, split from emulation): `ludusavi.exe`, the `PATH` separator of `SystemRunner` (`;`), its config in `%APPDATA%\ludusavi`, restore plans without Wine prefixes
- ⬜ Screenshots
- ✅ Tray icon (see the first milestone)
  - ⬜ Dark tray menus (needs an undocumented uxtheme call); new icons start in Windows 11's hidden-icons flyout until the user pins them
  - ✅ A `WM_CLOSE` to the tray window (plain `taskkill` while the app sits in the tray) quits gracefully, the Windows counterpart of SIGTERM
- ✅ Start with Windows: a `Heronite` value in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`system/autostart.rs`, `registry::set_string` / `delete_value`); only the real data folder drives it, `HERONITE_AUTOSTART_KEY` redirects it for tests
- ⬜ Desktop and Start menu shortcuts (`.lnk`)
- ⬜ Self-update (installer or portable zip, signed)
- ⬜ Packaging (installer, signing)
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
