# Windows migration

Heronite was written for Linux. This file tracks the port to Windows: how the
code keeps both systems easy to maintain, what already works on Windows, what
is left, and what Windows will not get because it only makes sense on Linux.

Legend: ✅ works on Windows · 🚧 written, waiting for a real-world check ·
⬜ to do · ➖ not planned for Windows

## At a glance

| Area | Windows |
|---|---|
| App, window, tray, start with Windows | ✅ |
| Collection, Steam (installed games and whole library), games added by hand | ✅ |
| Playing Steam games (session from Steam's log) | 🚧 |
| Emulators and ROM folders | ✅ |
| Epic Games, GOG, Amazon Games | 🚧 |
| Save backups (Ludusavi) | ✅ |
| Screenshots | ✅ |
| Desktop and Start menu shortcuts, Add to Steam | ✅ |
| Installer, portable zip, self-update | ✅ |
| Restoring a backup made on Linux | ✅ |
| In-game overlay and replay buffer | ⬜ |
| Launch extras (scripts, programs alongside, presets), Settings → Storage | ⬜ |
| Steam's per-game launch options | ⬜ |
| Wine / Proton, sandbox, Linux launchers' imports | ➖ |

## How the code handles two systems

1. **One crate holds what differs between systems: `heronite-platform`.**
   Processes (alive, stop, groups, detached starts, a clock that survives a
   restart), files (permission bits, executables, file identity, drives), the
   app's folders, opening a file or URL, the registry, shortcuts, the tray,
   keeping the computer awake, running a program as administrator. Its API is
   the same on every system; `process/unix.rs` and `process/windows.rs` hold
   the two implementations. Other crates call it instead of `libc` or
   `std::os::unix`, so they need no `cfg`.
2. **Code that only makes sense on one system stays where it is, behind
   `cfg(target_os = "linux")`** (`/proc`, evdev, bubblewrap, AppImage). What
   the rest of the app needs from it on Windows is a small stub or a Windows
   implementation of the same trait next to it (for example
   `heronite_launch::procs`: the `Procs` trait, read from `/proc` on Linux
   and from a Toolhelp snapshot on Windows).
3. **`heronite_platform::features` says which parts of the app a system
   has.** Linux has all of them. The UI asks `supported(Feature::…)` before
   it shows a page, a menu entry or a setting, and services that could only
   fail do not start. A part turns on for Windows in `built_in()` of
   `features.rs` once it works there. In debug builds,
   `HERONITE_FEATURES=+epic,-capture` turns parts on or off for a run
   (names from `Feature::name()`).
4. **Everything compiles everywhere.** Linux-only crates (Wine, the overlay)
   still build on Windows, they are just switched off by (3). This keeps
   `cargo check --workspace` meaningful on both systems and avoids a maze of
   `cfg` on imports.
5. **One rule says which builds run here:** `heronite_launch::host::run_mode`
   (on Windows: Windows builds natively, Linux and macOS builds not at all;
   on Linux: Linux builds natively, Windows builds through Wine). Games
   added by hand, emulators and the three stores use it.

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
- Running a helper program: `heronite_data::proc::Run`, which starts it
  without a console window and stops it the polite way first (SIGTERM on
  Linux, Ctrl+C to its own hidden console on Windows), then kills it.
- Tests that run programs use what each system has (`sh -c` on Unix,
  `cmd /C` and `ping` on Windows) when the code they test runs on both, and
  are `#[cfg(unix)]` when they test a Linux-only part (Wine, bubblewrap).
  Fake store tools: `heronite_data::testing::FakeTool`, which runs on every
  system. Run the tests from PowerShell or `cmd`, not Git Bash, which adds
  Unix tools and `HOME`.
- `drop()` on a value that is a guard on one system only (empty on the
  other) fails clippy there: give the guard a method that says what it does
  (`release`).
- Tests and scripts never send keys or clicks to the screen of a machine
  someone is using: the input goes to whatever window is in front.

## Folders on Windows

| | Linux | Windows | Windows, portable copy |
|---|---|---|---|
| Data (`DataPaths`) | `~/.local/share/heronite` | `%APPDATA%\heronite\data` | `<copy>\data` |
| Settings and UI state (`AppPaths::config`) | `~/.config/heronite` | `%APPDATA%\heronite\config` | `<copy>\config` |
| Caches (`AppPaths::cache`) | `~/.cache/heronite` | `%LOCALAPPDATA%\heronite\cache` | `<copy>\cache` |
| Store tools | `<data>/tools/bin` | `<data>\tools\bin\*.exe` | inside `<copy>\data` |
| New game installs | `~/Games/Heronite` | `%USERPROFILE%\Games\Heronite` | the same |

`HERONITE_DATA_DIR` works the same on both. A copy is portable when a
`portable.txt` lies next to its program (`dirs::portable_root`).

## ✅ Works on Windows

### The app

- Starts on Windows: window, quit, logs to its file, no console window in release builds
- Single instance: a loopback port and a key in the user's temp folder (`system/single_instance/transport.rs`); `--launch` from a shortcut is handed to the running copy
- Window controls in the app's style at the right of the top bars (minimize, maximize / restore, close); the empty part of a bar moves the window (`ui/window_controls.rs`)
- Tray icon with the Linux menu (recent games, Show / Hide, Quit); a left click shows or hides the window; "Keep in the tray when closed" and "Start minimized" apply; the icon comes back after Explorer restarts (`heronite_platform::tray`, `system/tray/windows.rs`)
  - A `WM_CLOSE` to the tray window (a plain `taskkill`) quits gracefully, the Windows counterpart of SIGTERM
- Start with Windows: a `Heronite` value in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`system/autostart.rs`); only the real data folder drives it
- `heronite://` links: registered for the user at every start (`HKCU\Software\Classes\heronite`, `system/protocol.rs`); not by a portable copy nor for a copy of the data folder
- Open folders, files and links (`shell::open`)
- System information (Logs, About): processor, memory, graphics cards and Windows version (`heronite_platform::system`)
- Keep the computer awake while a game runs: `power::KeepAwake` holds `SetThreadExecutionState` for the session
- What is not on Windows yet is hidden (`Feature`): ☰ pages, command palette, console mode menu, Settings sections and rows, Getting started, the empty Collection; the background jobs of those parts do not start
- No "On Linux" row (ProtonDB, anti-cheat) on a game, and ProtonDB is not asked

### Collection and Steam

- Steam: the client (registry `SteamPath`, `steam.exe`), its libraries (`libraryfolders.vdf` with Windows paths) and its installed games come into the Collection by themselves, at start and on refresh
- Steam: the whole library (owned games, installed or not, with Steam's play time) from the running client, through its debugging port (`process::command_line` of `steamwebhelper`)
- Games added by hand: a Windows program runs as it is, from its folder, with its arguments and environment variables (`plan::build::windows_plan`)
- Play time and sessions: a job object follows the game; the session ends when its last process does
- Add game dialog and game settings: no Linux tab, no Wine section; a Windows game has only the Launch tab
- Restore a backup made on Linux: paths into the backup's data, settings and home folders become this machine's (checked end to end with a `/home/...` backup)

### Emulators and ROM folders (`Feature::Emulation`)

- Finding emulators: one catalog with each emulator's Linux and Windows names; `PATH` (Scoop shims lead to the real program), Program Files, `%LOCALAPPDATA%\Programs`, `%LOCALAPPDATA%`, `%APPDATA%`, the system drive (`C:\RetroArch-Win64`), Scoop, EmuDeck's `%APPDATA%\EmuDeck\Emulators` and the usual `Emulation` / `Emulators` / `Games` folders; RetroArch cores are `.dll`
- Launching: the emulator runs as it is, with the profile's arguments and the ROM; emulators set up for Wine on Linux (a restored backup) run directly; Flatpak emulators show as not installed
- ROM folders and scanning: Windows paths (both separators, drive roots, case ignored, CRLF playlists), every fixed or removable drive but never network ones (`fs::fixed_drives`); the automatic setup looks in `ROMs`, `Emulation\roms`, `Games\ROMs`, `Documents\ROMs`…
- Save states: PCSX2, DuckStation, PPSSPP, Dolphin (its user folder from the registry), RetroArch, mGBA and RPCS3, portable installs too

### Save backups with Ludusavi (`Feature::SaveBackups`)

Checked end to end with the real `ludusavi.exe` 0.31: backup after play and by hand, the list of backups, restore and undo.

- Finding it: the program set in Settings, `ludusavi.exe` on `PATH`, `%LOCALAPPDATA%\Programs\Ludusavi`, winget, Scoop, `%ProgramFiles%\Ludusavi`
- Its config: `ludusavi config path`, else a portable Ludusavi's own folder, else `%APPDATA%\ludusavi\config.yaml`
- Restores put files back where they were backed up from; only what moved is redirected (another Steam library, Steam's `userdata`, another Windows user's folder, the Proton prefix of a backup made on Linux); what has no place on this PC is left out; Steam's `RunningAppID` blocks a restore while the game runs
- The Saves tab: Steam games and Windows programs added by hand

### Screenshots (`Feature::Capture`)

- A `RegisterHotKey` shortcut, registered only while a game runs (a key another program holds is reported); the game does not receive that key meanwhile, and Settings says so
- The monitor of the game's window captured with DXGI Desktop Duplication (GDI `BitBlt` when it is not available), in real pixels at any scale, rotated monitors upright; HDR scaled to Windows' SDR white level and saved as 8-bit sRGB
- Saved, named and compressed like on Linux; an on-screen notice that never takes the focus and is left out of captures (`crates/media/src/capture/win/`)

### Shortcuts (`Feature::DesktopShortcuts`)

- Desktop and Start menu: `.lnk` files written by the shell's own ShellLink object (`heronite_platform::shortcut`), starting the program with `--launch <runner> <appName>`; the icon is the game's picture as an `.ico`, or the program's own. One API for both systems (`heronite_shortcuts::menu`; `.desktop` files on Linux)
- Add to Steam: Steam's folder from the registry, `steam.exe` counted as a running Steam

### Installer, portable copy and updates

- `Heronite-Setup-x86_64.exe` (Inno Setup, `packaging/windows/heronite.iss`): per user in `%LOCALAPPDATA%\Programs\Heronite`, no administrator rights, Start menu and optional desktop shortcut; the uninstaller removes the `Run` value and `heronite://`
- `Heronite-Portable-x86_64.zip`: `heronite.exe`, the login window, `portable.txt` and `LICENSE`; the copy keeps everything in its folder and writes nothing to the registry on its own
- The program has its icon and version information (`app/build.rs`)
- Releases: the release workflow builds both next to the Linux files; one job signs everything with minisign and publishes one release. Pull requests that touch the packaging build them as artifacts (`windows-packages.yml`)
- Self-update (`Feature::SelfUpdate`), files checked with minisign like the AppImage:
  - Installed copy (a `package-format` file holding `windows-installer`): the setup runs as the app quits (`/VERYSILENT /WAITPID=<pid>`) and starts the new version after "Restart" (`/RELAUNCH=1`)
  - Portable copy: the zip is unpacked in the copy's cache; each replaced file is renamed `<name>.old` and the new one moved in, all undone on a failure; the `.old` files go at the next start
  - A development build and a release without the Windows files only say a new version exists

### Building

- Every crate and the app build on Windows; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- CI runs clippy and the tests on `windows-latest` next to Linux (`.github/workflows/ci.yml`)
- Line endings: `.gitattributes` (`eol=lf`), Windows checkouts match Linux

## 🚧 Written, waiting for a real-world check

- **Playing Steam games:** `steam://rungameid` (`shell::open`); the session follows Steam's `console_log.txt` like on Linux. To check with a real game
- **Epic Games, GOG and Amazon Games, natively** (`Feature::EpicStore`, `GogStore`, `AmazonStore`, on in Windows builds). To check with real accounts: login, a download with pause and resume, the first launch with its installers, play time, cloud saves, move, uninstall, and games of the official launchers
  - Tools: the Windows builds of legendary 0.21.1, gogdl v1.3.0 and nile v1.2.0 (standalone `.exe`, no Python) in `<data>\tools\bin`, run with no console window (`store_tools`)
  - Login: `heronite-epic-login.exe`, a Win32 window with WebView2 (InPrivate, its data in the cache folder); without the WebView2 runtime it says so and the browser method stays
  - Downloads: the Downloads page and its settings show when any store is on (`features::any_store`); a pause sends Ctrl+C to the tool, then kills it after a grace time (legendary and gogdl keep their resume files, nile gets 2 s); moves across drives copy with progress and can be cancelled
  - Launch: Heronite starts the game itself, so the session lasts as long as the game. Epic: `legendary launch --json` gives the command, sign-in arguments included; GOG: the play task of `goggame-<id>.info`; Amazon: `fuel.json` with the variables nile sets. Epic games of the EA app or Ubisoft Connect open through those apps (their play time is not tracked)
  - First launch (`native_setup`): prerequisites, GOG's redistributables and scripts, Amazon's `PostInstall` run once per game folder; installers that need it go through the administrator prompt (`shell::run_elevated`); a declined prompt is a warning and it asks again next time
  - Cloud saves: legendary and gogdl resolve the real Windows folders (Saved Games, AppData, Documents, OneDrive)
  - EOS Overlay: one switch for all games (the registry, no prefix)
  - Games the official launchers installed, without downloading them again, after login and from a button on each store's page: Epic Games Launcher (`legendary import`, one game at a time; not `egl-sync`, which links the games to the launcher), GOG Galaxy (its registry keys; Galaxy keeps updating, moving and uninstalling them), Amazon Games app (a copy of its install database, then `nile import`; the app keeps its files)

## ⬜ Left to do

### Parts of the app

- **In-game overlay and replay buffer** (`Feature::Overlay`): needs a Windows capture and input stack; today `heronite_overlay::input` and `heronite_media::capture` have do-nothing stand-ins off Linux. The controller's screenshot goes through it too
- **Launch extras** (`Feature::LaunchExtras`): scripts before and after a game, programs alongside it, launch presets. The Linux wrappers (GameMode, MangoHud, gamescope) are not part of it (➖ below)
- **Settings → Storage** (`Feature::Storage`): what games and caches take on disk, and cleaning them up, without the Wine prefixes and Proton builds
- **Steam's per-game launch options** (`Feature::SteamGameSettings`): Steam on Windows has them too; the forced compatibility tool (Proton) is Linux's
- **Troubleshoot** in a game's menu: its checks are about Linux causes (Wine, Proton, Vulkan); a Windows version would look at other things
- **Games of other Windows launchers** (Battle.net, EA app, Ubisoft Connect, Rockstar Games Launcher, Xbox app): importing what they installed, the way Linux imports Lutris or Bottles (`Feature::LauncherImports`)
- **GOG's `comet`**: GOG Galaxy's services (achievements, some multiplayer) for games that use them

### Details of what already works

- Steam library and session tests (`TODO(windows)` in `heronite-launch`): `SteamLocator` reads this machine's Steam from the registry, so they need a way to leave it out first
- RetroAchievements sign-in of the emulators: `heronite_achievements::emulators` knows their Linux config files only
- RetroArch from Steam, and the Microsoft Store builds of emulators
- Ludusavi: Windows registry entries in Windows-made backups are restored by Ludusavi itself, but they are not in the preview nor in the undo copy
- Screenshots of legacy exclusive-fullscreen games (Desktop Duplication loses the display while they hold it); HDR checked on a real HDR monitor
- Dark tray menus (needs an undocumented uxtheme call); a new tray icon starts in Windows 11's hidden-icons flyout until the user pins it
- Authenticode signing: ready but off until there is a certificate (secrets `WINDOWS_SIGN_PFX_BASE64` / `WINDOWS_SIGN_PASSWORD`, or `WINDOWS_SIGN_COMMAND`); until then SmartScreen warns on the first run and the release notes say so
- The installer in other languages (English only)

## ➖ Not planned for Windows

These exist because Linux runs Windows games through compatibility layers, or
because of how Linux desktops work. Windows runs those games directly.

- **Wine and Proton**, the Wine Manager, winetricks, prefixes, DXVK and the known fixes (`Feature::Wine`)
- **Store clients under Wine**: Battle.net, the EA app, Ubisoft Connect and the Rockstar Games Launcher installed into a prefix by Heronite (`Feature::StoreClients`). On Windows people install them normally (importing their games is listed above)
- **Sandbox** (bubblewrap)
- **Linux launchers' imports**: Lutris, Bottles, Faugus and Heroic for Linux
- **Linux launch wrappers**: GameMode, MangoHud, gamescope, the Steam runtime, `lsfg`
- **Copying a Heroic setup at the first start** (`Feature::HeroicMigration`): Heroic for Windows keeps stores Heronite runs differently there, so a Windows setup starts empty
- **The "On Linux" row** of a game (ProtonDB tier, anti-cheat on Linux)
- **AppImage, `.deb`, `.rpm`, Arch packages and `.desktop` files**: Windows has its installer, portable zip and `.lnk` shortcuts

## Checking Linux from Windows

`rustup target add x86_64-unknown-linux-gnu`, then `cargo clippy --target
x86_64-unknown-linux-gnu -p <crate>` with `CC_x86_64_unknown_linux_gnu` and
`AR_x86_64_unknown_linux_gnu` set to the `clang.exe` and `llvm-ar.exe` of
Visual Studio's LLVM (and a `CARGO_TARGET_DIR` of its own). Crates that
compile C code with libc headers (`libsqlite3-sys`, `libwebp-sys`, GTK,
rcheevos) and the app cannot be checked this way; CI on Linux covers them.
