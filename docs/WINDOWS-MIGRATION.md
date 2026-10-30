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
- ⬜ `cargo check --workspace` passes on Windows
- ⬜ `cargo test --workspace` passes on Windows (Unix-only tests behind `cfg(unix)`)
- ⬜ `cargo clippy --workspace --all-targets` clean on Windows
- ⬜ CI job on `windows-latest`

### First milestone: the Collection

- ⬜ App starts on Windows (window, single instance, quit)
- ⬜ Data, settings and cache folders in `%APPDATA%` / `%LOCALAPPDATA%`
- ⬜ Steam: find the client (registry), its libraries and the installed games
- ⬜ Steam: installed games come into the Collection by themselves
- ⬜ Steam: play, install and uninstall through `steam://` URLs
- ⬜ Games added by hand: pick a `.exe`, launch it natively (no Wine)
- ⬜ Play time and sessions of native Windows games (job object follows the game)
- ⬜ Restore a backup made on Linux (paths rewritten into Windows folders)
- ⬜ Open folders, files and links (`shell::open` instead of `xdg-open`)
- ⬜ Hide on Windows what is not ported yet (`Feature`)

### Later

- ⬜ Epic, GOG and Amazon Games (legendary, gogdl and nile have Windows builds)
- ⬜ Store clients (Battle.net, EA app, Ubisoft Connect, Rockstar) run natively
- ⬜ Emulators and ROM folders
- ⬜ Screenshots
- ⬜ Tray icon
- ⬜ Start with Windows (`Run` registry key)
- ⬜ Desktop and Start menu shortcuts (`.lnk`)
- ⬜ Steam per-game settings and library from the client
- ⬜ Self-update (installer or portable zip, signed)
- ⬜ Packaging (installer, signing)
- ➖ Wine / Proton, Wine Manager, winetricks
- ➖ Sandbox (bubblewrap)
- ➖ Lutris, Bottles, Faugus and Heroic (Linux) imports
- ⬜ Overlay and replay buffer (needs a different capture and input stack)
