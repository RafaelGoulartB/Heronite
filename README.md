<h1 align="center">Heronite</h1>

<p align="center">
  <a href="https://github.com/RafaelGoulartB/Heronite">
    <img alt="GitHub stars" src="https://img.shields.io/github/stars/RafaelGoulartB/Heronite?style=social" />
  </a>
  <a href="LICENSE">
    <img alt="License: GPL v3" src="https://img.shields.io/badge/License-GPL_v3-2B8CA8.svg" />
  </a>
  <a href="https://github.com/RafaelGoulartB/Heronite/releases/latest">
    <img alt="Latest release" src="https://img.shields.io/github/v/release/RafaelGoulartB/Heronite?color=2B8CA8&label=release" />
  </a>
  <img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-2B8CA8.svg?logo=rust&logoColor=white" />
  <img alt="Platform: Linux" src="https://img.shields.io/badge/platform-Linux-2B8CA8.svg?logo=linux&logoColor=white" />
  <a href="https://github.com/RafaelGoulartB/Heronite/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/RafaelGoulartB/Heronite/total?color=2B8CA8&logo=github&label=downloads" />
  </a>
</p>

<p align="center">
  Every game you own, on Linux, in one fast native launcher.
</p>

https://github.com/user-attachments/assets/438452ac-dc6e-46fa-b443-9d85e94dca54

> [!NOTE]
> **Windows support is coming soon.** Heronite runs on Linux today, and a Windows version is on the way.

Heronite puts your Steam, Epic Games, GOG, Amazon Games, emulated, native Linux, and Windows games into one Collection, launches them through Wine/Proton or an emulator, and records every session. It is a native Rust app (GPUI, GPU rendered) that [stays light on your PC](#light-on-your-pc).

## Your library, organized

- **Collection:** every game in a list with a details panel (Focus) or a grid of covers, switching by the window's width. Group by status, store, genre, series, year and more; drag the panels wider. The details panel shows screenshots, How Long To Beat, ProtonDB and anti-cheat status, achievements, related games, and a play timeline.
- **Journal:** notes on the days you played and on your screenshots, plus where you left off.
- **View menu:** filter, sort, and save the view as a preset (Playing now, Backlog, Finished, and Favourites built in). Search forgives accents and typos.
- **Bulk edits:** change status, categories, and other fields of many games at once, with undo. A fields manager renames and merges genres, companies, and series.
- **Command palette:** <kbd>Ctrl</kbd> <kbd>K</kbd> jumps to any page, runs any action, or finds and plays a game.
- **Add any game:** native Linux programs, Windows programs, browser games, install scripts, or ROMs from a folder. Art comes from Steam, SteamGridDB, IGDB, Lutris, the web, or your own screenshots.
- **Getting started:** on first run, Heronite finds your Steam library, emulators, and ROM folders, and a checklist walks you through the rest.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/collection.webp"><img src="assets/screenshots/collection.webp" width="100%" alt="Collection in Focus view with ELDEN RING selected, showing screenshots, overview, details, and the play timeline over the game's art" /></a><br /><sub><b>Collection</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/command-palette.webp"><img src="assets/screenshots/command-palette.webp" width="100%" alt="Command palette searching for 'contr', listing matching games and the play, settings, and open folder actions for Control" /></a><br /><sub><b>Command palette</b></sub></td>
  </tr>
</table>

## Install, configure, and play

- **Stores:** Steam (no login needed), Epic Games, GOG, and Amazon Games, with EA app and Ubisoft Connect games started through their apps. One download queue to install, update, repair, move, import, and uninstall; it pauses while offline and retries failed downloads.
- **Wine and Proton:** per-game Wine version, prefix, DXVK-NVAPI, Esync/Fsync, gamescope, FPS limit, GPU choice, DLSS/FSR/XeSS upgrades, Winetricks, protonfixes via umu, and Lossless Scaling frame generation (lsfg-vk).
- **Launching:** see each step of a launch and cancel it, run scripts before and after the game or programs alongside it, and add your own actions next to Play.
- **Sandbox:** run a game you do not trust, and its installer, through bubblewrap: no network, no access to your home folder, only its prefix and its folder. Winetricks, Run EXE and the programs alongside the game run inside it too. When a game needs more, let one thing through (the Internet, writing to its folder, your shared folders, every device), each one with what it risks.
- **Wine Manager:** download and update Proton-CachyOS, GE-Proton, DW-Proton, UMU-Proton, Proton-EM, and Proton-Sarek, and manage Wine prefixes.
- **Emulators:** detect installed emulators or add your own with RetroArch cores, scan ROM folders, and continue from save states.
- **In-game overlay (beta):** press the controller's Guide button (or a shortcut) for a panel over the game with volume, microphone, FPS limit, screenshots, and journal notes. Guide + Y marks a moment to write about later.
- **Replay (off by default):** keep the last seconds of the game (30 unless changed) and save them with Guide + X (or a keyboard shortcut of your own) as a moment in its journal, recorded by [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/) on the graphics card's encoder. Settings shows the memory it takes and how to spare it (Vulkan video, the buffer on disk, a smaller video); each game can have it on or off on its own.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/game-settings.webp"><img src="assets/screenshots/game-settings.webp" width="100%" alt="Game settings for Control on the Compatibility tab, with the Proton version, prefix, Winecfg and Winetricks buttons, and the umu game fixes" /></a><br /><sub><b>Game settings</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/wine-manager.webp"><img src="assets/screenshots/wine-manager.webp" width="100%" alt="Wine Manager listing Proton-CachyOS builds with release dates, sizes, and install buttons" /></a><br /><sub><b>Wine Manager</b></sub></td>
  </tr>
</table>

## Look back at what you played

- **Statistics:** play time by week, month, year, or all time, in five tabs: **Overview** (charts, top games, records), **Journeys** (each game as a lane in time), **Backlog** (a yearly goal of games to beat and a plan for the rest), **Discoveries** (the period told in sentences, with milestones), and **Album** (your screenshots as moments). Any game opens a sheet with its own numbers.
- **Year in review:** your year told as full-screen stories, with a summary card to share.
- **Sessions:** every launch is recorded, in-game screenshots are matched to the game, and sessions can be edited by hand.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/statistics.webp"><img src="assets/screenshots/statistics.webp" width="100%" alt="Statistics Overview for October 2026 with total time played, cards for the other tabs, and a day-by-day chart per game" /></a><br /><sub><b>Overview</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/year-in-review.webp"><img src="assets/screenshots/year-in-review.webp" width="100%" alt="Year in review intro story: Your 2026 in games, with days, games, and sessions next to a wall of covers" /></a><br /><sub><b>Year in review</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/stats-backlog.webp"><img src="assets/screenshots/stats-backlog.webp" width="100%" alt="Backlog tab for 2026 with 9 games beaten, the yearly goal ring, and the beaten games along the year" /></a><br /><sub><b>Backlog</b></sub></td>
  </tr>
</table>

## Console mode

A controller-first layout for the couch or a TV that covers the whole app, with an on-screen keyboard and a Power menu to suspend or shut down. Turn it on from the palette, the ☰ menu, or with `--console`.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/console-home.webp"><img src="assets/screenshots/console-home.webp" width="100%" alt="Console home with SP Football Life 2026 large over its art and a Continue playing shelf of covers" /></a><br /><sub><b>Home</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/console-game.webp"><img src="assets/screenshots/console-game.webp" width="100%" alt="Console game page for ELDEN RING with Play, Status, Favourite, Game settings, Playtime, and Ludusavi backup rows next to its details" /></a><br /><sub><b>Game page</b></sub></td>
  </tr>
</table>

## Make it yours

- **Appearance:** a dark or light theme (or the desktop's, or one in colours you pick), profiles, an accent colour (or the game's), high contrast, fonts, density, cover style, a background picture or slideshow behind the Collection, and which sections the details panel shows.
- **Language:** English and Portuguese (Brazil), switched live.
- **Saves and backups:** back up saves with Ludusavi after each session and restore them into any prefix. Scheduled, incremental backups of your Collection, play history, settings, and art.
- **Storage:** see what your games take on each disk and clean up leftovers in one click.
- **Startup:** start with the system, launch games via `heronite://launch` links or `--launch`.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/appearance.webp"><img src="assets/screenshots/appearance.webp" width="100%" alt="Appearance settings with profiles, accent colours, font and density, and a live preview of the Collection background" /></a><br /><sub><b>Appearance</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/storage.webp"><img src="assets/screenshots/storage.webp" width="100%" alt="Storage settings with disk usage bars, space to free up, and a treemap of installed games by size" /></a><br /><sub><b>Storage</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/backups.webp"><img src="assets/screenshots/backups.webp" width="100%" alt="Backups settings with the backup folder, schedule, what goes in a backup, and the last backup" /></a><br /><sub><b>Backups</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/screenshots.webp"><img src="assets/screenshots/screenshots.webp" width="100%" alt="Screenshots settings with the capture folder and space used, lossless compression, the in-game capture shortcut, and controller capture" /></a><br /><sub><b>Screenshots</b></sub></td>
  </tr>
</table>

## Light on your PC

A single native process that draws its UI on the GPU, with no bundled browser engine. When idle, it does no work.

| | Heronite |
|---|---|
| Memory while idle | **~195 MB** in a single process |
| CPU while idle | **0.0%** (no CPU time used over 60 s) |
| Install size | **47 MB** (app + store login helper) |
| Start to library on screen | **~335 ms** median |

## Your data

Everything stays on your machine, in Heroic-compatible files under `~/.local/share/heronite/` (override with `HERONITE_DATA_DIR`). On first start, Heronite copies an existing Heroic setup once, without ever writing to Heroic's folders. For IGDB metadata, add your Twitch client ID and secret in **Settings > Metadata**.

## Download

Every [release](https://github.com/RafaelGoulartB/Heronite/releases/latest) has an AppImage and packages for Debian/Ubuntu, Fedora/openSUSE, and Arch:

| System | File | Install |
|---|---|---|
| Any distribution (**recommended**) | `Heronite-x86_64.AppImage` | `chmod +x Heronite-x86_64.AppImage && ./Heronite-x86_64.AppImage` |
| Debian 12+, Ubuntu 22.04+, Mint, Pop!_OS | `heronite_<version>-1_amd64.deb` | `sudo apt install ./heronite_*_amd64.deb` |
| Fedora 36+, openSUSE | `heronite-<version>-1.x86_64.rpm` | `sudo dnf install ./heronite-*.x86_64.rpm` |
| Arch, Manjaro, EndeavourOS, CachyOS | `heronite-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U heronite-*-x86_64.pkg.tar.zst` |

Heronite updates itself: a card offers new versions, and every download is checked against its signing key. Keep the AppImage in a writable folder (like `~/Applications`) so it can update in place.

## Run from source

You need a stable Rust toolchain, `clang` and `lld`, a Vulkan-capable GPU driver, and GTK/WebKitGTK. Heronite downloads legendary, gogdl, and nile on its own; [Ludusavi](https://github.com/mtkennerly/ludusavi) is optional for save backups.

```bash
# quick build (the login window is its own binary, so build it too)
cargo build --profile quick -p heronite-epic-login && cargo run --profile quick -p heronite

# release build, installed for the current user
scripts/install.sh
```

## Development

A Cargo workspace: `app/` is the GPUI binary, `crates/` holds the GPUI-free libraries, and `heronite-epic-login` is the separate store login window.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

See [ARCHITECTURE.md](docs/ARCHITECTURE.md), [CODE-STYLE.md](docs/CODE-STYLE.md), and [UI-CONVENTIONS.md](docs/UI-CONVENTIONS.md). To release, merge a pull request labelled `release:patch`, `release:minor`, or `release:major`. Issues and pull requests are welcome.

## License

Copyright (C) 2026 [Rafael Goulart](https://www.linkedin.com/in/rafael-goulartb/)

Licensed under the [GNU General Public License v3.0](LICENSE). Heronite builds on the [Heroic Games Launcher](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher), also GPL-3.0. Third-party components keep their own licenses; see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
