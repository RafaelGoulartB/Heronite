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
  <img alt="Platform: Linux and Windows" src="https://img.shields.io/badge/platform-Linux%20%7C%20Windows-2B8CA8.svg" />
  <a href="https://github.com/RafaelGoulartB/Heronite/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/RafaelGoulartB/Heronite/total?color=2B8CA8&logo=github&label=downloads" />
  </a>
</p>

<p align="center">
  Every game you own, on Linux and Windows, in one fast native launcher.
</p>

<p align="center">
  <a href="https://heronite.app"><b>Website</b></a> ·
  <a href="https://github.com/RafaelGoulartB/Heronite/wiki"><b>Wiki</b></a> ·
  <a href="https://github.com/RafaelGoulartB/Heronite/releases/latest"><b>Download</b></a>
</p>

https://github.com/user-attachments/assets/438452ac-dc6e-46fa-b443-9d85e94dca54

Heronite brings your games from Steam, Epic Games, GOG, Amazon Games, Battle.net, EA app, Ubisoft Connect, and Rockstar Games, plus emulated, native Linux, and Windows games, into one Collection. It launches them natively, through Wine/Proton on Linux, or through an emulator, records every session, and [stays light on your PC](#light-on-your-pc) as a native Rust app (GPUI, GPU rendered).

## 📥 Download

<div align="center">

| Platform | Package | Download |
|:---|:---|:---|
| 🪟 Windows | Installer | [Heronite-Setup-x86_64.exe](https://github.com/RafaelGoulartB/Heronite/releases/latest/download/Heronite-Setup-x86_64.exe) |
| 🪟 Windows | Portable | [Heronite-Portable-x86_64.zip](https://github.com/RafaelGoulartB/Heronite/releases/latest/download/Heronite-Portable-x86_64.zip) |
| 🐧 Linux | AppImage (any distro) | [Heronite-x86_64.AppImage](https://github.com/RafaelGoulartB/Heronite/releases/latest/download/Heronite-x86_64.AppImage) |
| 🐧 Linux | Flatpak (any distro) | [Heronite-x86_64.flatpak](https://github.com/RafaelGoulartB/Heronite/releases/latest/download/Heronite-x86_64.flatpak) |
| 🐧 Arch / CachyOS | .pkg.tar.zst | [Latest release](https://github.com/RafaelGoulartB/Heronite/releases/latest) |
| 🐧 Debian / Ubuntu | .deb | [Latest release](https://github.com/RafaelGoulartB/Heronite/releases/latest) |
| 🐧 Fedora / openSUSE | .rpm | [Latest release](https://github.com/RafaelGoulartB/Heronite/releases/latest) |

</div>

<details>
<summary><b>How to install</b></summary>

| System | File | Install |
|---|---|---|
| Any distribution (**recommended**) | `Heronite-x86_64.AppImage` | `chmod +x Heronite-x86_64.AppImage && ./Heronite-x86_64.AppImage` |
| Any distribution, sandboxed | `Heronite-x86_64.flatpak` | `flatpak install --user ./Heronite-x86_64.flatpak`, then for Windows games `flatpak install --user flathub org.freedesktop.Platform.Compat.i386//26.08 org.freedesktop.Platform.GL32.default//26.08`; start it with `flatpak run app.heronite.Heronite` ([what differs](packaging/flatpak/README.md#what-changes-in-the-flatpak)) |
| Debian 12+, Ubuntu 22.04+, Mint, Pop!_OS | `heronite_<version>-1_amd64.deb` | `sudo apt install ./heronite_*_amd64.deb` |
| Fedora 36+, openSUSE | `heronite-<version>-1.x86_64.rpm` | `sudo dnf install ./heronite-*.x86_64.rpm` |
| Arch, Manjaro, EndeavourOS, CachyOS | `heronite-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U heronite-*-x86_64.pkg.tar.zst` |
| Windows 10 and 11 (**recommended**) | `Heronite-Setup-x86_64.exe` | Run it: it installs for your user, no administrator rights needed |
| Windows 10 and 11, portable | `Heronite-Portable-x86_64.zip` | Extract it anywhere and run `Heronite\heronite.exe`; the library, settings, and screenshots stay in that folder |

</details>

The Windows files are not code-signed yet, so SmartScreen may warn about them: choose **More info**, then **Run anyway**.

---

## Your library, organized

- **Collection:** a list with details or a grid of covers, grouped by status, store, genre, series, or year. Details show screenshots, How Long To Beat, achievements, and a play timeline.
- **Journal:** notes on your play days and screenshots.
- **Views:** filter, sort, and save presets like Playing now or Backlog. Search forgives typos.
- **Bulk edits:** change many games at once, with undo.
- **Command palette:** <kbd>Ctrl</kbd> <kbd>K</kbd> opens any page, action, or game.
- **Add any game:** programs, browser games, scripts, or ROM folders, with art from Steam, SteamGridDB, IGDB, and more.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/collection.webp"><img src="assets/screenshots/collection.webp" width="100%" alt="Collection in Focus view with ELDEN RING selected, showing screenshots, overview, details, and the play timeline over the game's art" /></a><br /><sub><b>Collection</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/command-palette.webp"><img src="assets/screenshots/command-palette.webp" width="100%" alt="Command palette searching for 'contr', listing matching games and the play, settings, and open folder actions for Control" /></a><br /><sub><b>Command palette</b></sub></td>
  </tr>
</table>

## Stores and launchers

- **Epic Games, GOG, and Amazon Games:** log in and install, update, repair, or move games from one download queue. On Windows, games from their official launchers come in without downloading again.
- **Steam:** your library comes in by itself, no login needed.
- **Battle.net, EA app, Ubisoft Connect, and Rockstar Games (Linux):** each store's app runs in its own Wine prefix, and its games show up in your Collection.
- **Other launchers (Linux):** import and sync games from Heroic, Lutris, Bottles, and Faugus.
- **Games you already have:** add one by hand, or a whole folder at once.

## Configure and play

- **Wine and Proton (Linux):** per-game version and prefix, DXVK, gamescope, FPS limit, DLSS/FSR upgrades, Winetricks, and Lossless Scaling frame generation.
- **Wine Manager (Linux):** download Proton-CachyOS, GE-Proton, and other builds, and manage prefixes.
- **Launching:** scripts before and after, programs alongside (like trainers), and presets to reuse.
- **Sandbox (Linux):** run untrusted games without network or access to your home folder.
- **Emulators:** detected by themselves or added with RetroArch cores, with ROM scanning and save states.
- **In-game overlay (beta, Linux):** the Guide button opens volume, FPS limit, screenshots, and notes over the game.
- **Replay (Linux):** save the last seconds of play with Guide + X, recorded by [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/).

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/game-settings.webp"><img src="assets/screenshots/game-settings.webp" width="100%" alt="Game settings for Control on the Compatibility tab, with the Proton version, prefix, Winecfg and Winetricks buttons, and the umu game fixes" /></a><br /><sub><b>Game settings</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/wine-manager.webp"><img src="assets/screenshots/wine-manager.webp" width="100%" alt="Wine Manager listing Proton-CachyOS builds with release dates, sizes, and install buttons" /></a><br /><sub><b>Wine Manager</b></sub></td>
  </tr>
</table>

## Look back at what you played

- **Statistics:** play time by week, month, or year, plus a yearly backlog goal.
- **Year in review:** your year as full-screen stories, with a card to share.
- **Sessions:** every launch is recorded, and screenshots are matched to the game.

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

A controller-first layout for the couch or a TV. Turn it on from the ☰ menu, the palette, or with `--console`.

- **Themes:** eight layouts (Shelves, Spotlight, Cinema, Grid, Wheel, and more), with your own covers, colours, fonts, and menus.
- **Share them:** seven built-in themes, or export yours as a `.heronite-theme.json` file.
- **Light when off:** nothing loads until you turn it on. It can open at startup or when a controller connects.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/console-home.webp"><img src="assets/screenshots/console-home.webp" width="100%" alt="Console home with SP Football Life 2026 large over its art and a Continue playing shelf of covers" /></a><br /><sub><b>Home</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/console-game.webp"><img src="assets/screenshots/console-game.webp" width="100%" alt="Console game page for ELDEN RING with Play, Status, Favourite, Game settings, Playtime, and Ludusavi backup rows next to its details" /></a><br /><sub><b>Game page</b></sub></td>
  </tr>
</table>

## Make it yours

- **Appearance:** dark or light, accent colour, fonts, density, and a background picture.
- **Language:** English, Portuguese (Brazil), Spanish, and German.
- **Saves and backups:** Ludusavi save backups after each session, and scheduled Collection backups.
- **Storage:** see what your games take on each disk and clean up leftovers.
- **Startup:** start with the system, and launch games from `heronite://` links.

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

A single native process drawn on the GPU, with no bundled browser.

| | Heronite |
|---|---|
| Memory while idle | **~190 MB** in a single process, **~95 MB** closed to the tray |
| CPU while idle | **~0.1%** (0.05 s of CPU time over 60 s) |
| Install size | **64 MB** (app + store login helper) |
| Start to library on screen | **~215 ms** median |

## Run from source

You need stable Rust. On Linux, also `clang` and `lld`, a Vulkan driver, and GTK/WebKitGTK; on Windows, the Visual Studio Build Tools ("Desktop development with C++"). Heronite downloads legendary, gogdl, and nile itself; [Ludusavi](https://github.com/mtkennerly/ludusavi) is optional.

```bash
# quick build (the login window is its own binary, so build it too)
cargo build --profile quick -p heronite-epic-login && cargo run --profile quick -p heronite

# release build, installed for the current user (Linux)
scripts/install.sh

# installer and portable zip in dist\ (Windows, needs Inno Setup 6)
powershell -ExecutionPolicy Bypass -File scripts\windows\package.ps1
```

## License

Copyright (C) 2026 [Rafael Goulart](https://www.linkedin.com/in/rafael-goulartb/)

Licensed under the [GNU General Public License v3.0](LICENSE). Third-party components keep their own licenses; see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Contact

Questions and ideas are welcome in [Discussions](https://github.com/RafaelGoulartB/Heronite/discussions), and bugs in [Issues](https://github.com/RafaelGoulartB/Heronite/issues). For anything else, write to [hello@heronite.app](mailto:hello@heronite.app).
