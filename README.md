<h1 align="center">Heronite</h1>

<p align="center">
  <a href="https://www.linkedin.com/in/rafael-goulartb/">
    <img alt="Rafael Goulart" src="https://img.shields.io/badge/-Rafael%20Goulart-2B8CA8?style=flat&logo=Linkedin&logoColor=white" />
  </a>
  <a href="docs/ARCHITECTURE.md">
    <img alt="Documentation" src="https://img.shields.io/badge/documentation-yes-2B8CA8.svg" />
  </a>
  <a href="https://github.com/RafaelGoulartB/Heronite/graphs/commit-activity">
    <img alt="Maintenance" src="https://img.shields.io/badge/Maintained%3F-yes-2B8CA8.svg" />
  </a>
  <a href="LICENSE">
    <img alt="License: GPL v3" src="https://img.shields.io/badge/License-GPL_v3-2B8CA8.svg" />
  </a>
  <a href="https://github.com/RafaelGoulartB/Heronite/actions/workflows/ci.yml">
    <img alt="CI" src="https://img.shields.io/github/actions/workflow/status/RafaelGoulartB/Heronite/ci.yml?branch=main&label=CI&color=2B8CA8" />
  </a>
  <a href="https://github.com/RafaelGoulartB/Heronite/releases/latest">
    <img alt="Latest release" src="https://img.shields.io/github/v/release/RafaelGoulartB/Heronite?color=2B8CA8&label=release" />
  </a>
  <img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-2B8CA8.svg?logo=rust&logoColor=white" />
  <img alt="Platform: Linux" src="https://img.shields.io/badge/platform-Linux-2B8CA8.svg?logo=linux&logoColor=white" />
  <img alt="GitHub Pull Requests" src="https://img.shields.io/github/issues-pr/RafaelGoulartB/Heronite?color=2B8CA8" />
  <img alt="GitHub repository size" src="https://img.shields.io/github/repo-size/RafaelGoulartB/Heronite?color=2B8CA8" />
</p>

<p align="center">
  Every game you own, on Linux, in one fast native launcher.
</p>

https://github.com/user-attachments/assets/438452ac-dc6e-46fa-b443-9d85e94dca54

<p align="center">
  <a href="docs/ARCHITECTURE.md">Architecture</a> &middot;
  <a href="docs/UI-CONVENTIONS.md">UI conventions</a> &middot;
  <a href="docs/CODE-STYLE.md">Code style</a>
</p>

> [!NOTE]
> **Windows support is coming soon.** Heronite runs on Linux today, and a Windows version is on the way.

Heronite puts your Steam, Epic Games, GOG, Amazon Games, emulated, native Linux, and Windows games into one Collection. You can sort them by your own statuses, fill in their metadata and art, and launch them through Wine/Proton or an emulator. Each session is recorded, so you can see how much you played and which screenshots you took.

Heronite is a native Rust app (GPUI, GPU rendered) that [stays light on memory, CPU, and disk](#light-on-your-pc). On Linux it builds on Wine/Proton, gamescope, Ludusavi, the system tray, and desktop and Steam shortcuts.

## Your library, organized

- **Collection:** one screen for every game, as a list with a details panel or as a grid of covers with the selected game in a side drawer. Covers are grouped by status (nine to start with, such as Playing, Plan to Play, Beaten and Abandoned, or your own) or by category, store, platform, genre, developer, series, year, play time, or size on disk, and the selected game's art fills the background. The details panel shows screenshots, How Long To Beat times, ProtonDB tier and anti-cheat status, achievements, your rating, the last save backup, the game's folders and their size, related games, and a timeline of every day you played.
- **View menu:** type to find any filter (store, status, category, genre, developer, series, theme, feature, or platform), each with its number of games. Sort by title, last played, play time, date added, times played, size, release date, or score, in either direction. Save the whole menu as a preset: Playing now, Backlog, Finished, and Favourites come built in. Search ignores accents and forgives small typos.
- **Change many games at once:** select several games (<kbd>Ctrl</kbd> or <kbd>Shift</kbd> click, <kbd>Ctrl</kbd> <kbd>A</kbd>) and change their status, categories, favourite, or hidden state together, with undo, or edit their series, genres, developers, and other fields in one go. The library fields manager renames, merges, or removes genres, companies, series, and the other names, points out names that look alike, and applies the same rules to metadata downloaded later.
- **Command palette:** press <kbd>Ctrl</kbd> <kbd>K</kbd> to jump to any page or settings section, run any action, or find a game and play it, open its settings or its folder. Press <kbd>→</kbd> on a game to see everything its right-click menu offers (status, favourites, uninstall, shortcuts, Epic tools…). The palette also covers the pages (pause downloads, install the latest Proton, sign in to Epic, add an emulator), the Collection's layout, grouping, sort, and filters, and appearance toggles. With a game selected, its actions come first, and items you used recently rise to the top. Pages also have direct shortcuts (<kbd>Ctrl</kbd> <kbd>1</kbd> to <kbd>6</kbd>, <kbd>Ctrl</kbd> <kbd>,</kbd>, <kbd>Ctrl</kbd> <kbd>N</kbd>), and <kbd>F11</kbd> switches fullscreen.
- **Add any game:** add a native Linux program (an executable or a command), a Windows program (run through Wine/Proton), or a browser game. ROMs come in by scanning a folder (see [Emulators](#install-configure-and-play)). Covers and backgrounds can come from Steam, SteamGridDB, IGDB, Lutris, a web image search, a file or link, or one of your own screenshots. Any game can also get its own actions next to Play: another program, a command, or a link.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/collection.webp"><img src="assets/screenshots/collection.webp" width="100%" alt="Collection with games grouped by status and ELDEN RING selected, showing screenshots, overview, How Long To Beat times, details, and the play timeline over the game's art" /></a><br /><sub><b>Collection</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/command-palette.webp"><img src="assets/screenshots/command-palette.webp" width="100%" alt="Command palette searching for 'contr', listing matching games and the play, settings, and open folder actions for Control" /></a><br /><sub><b>Command palette</b></sub></td>
  </tr>
</table>

## Install, configure, and play

- **Steam:** your whole Steam library appears in the Collection, installed or not. Heronite reads it from the running Steam client, so it needs no login or API key. Install, play, and uninstall go through Steam, and Steam's play time and last played date are kept in sync. A game you also have from another source gets Steam as a second way to play instead of a second card. Heronite can also change a Steam game's launch options and Proton version, and add any other game to Steam.
- **Epic Games:** log in, install, update, repair, move, import, and uninstall games, manage DLC, and sync cloud saves. Pick how a game starts (its launch options or a playable DLC), and manage the EOS Overlay. Games sold through the EA app or Ubisoft Connect install and start through those apps. Downloads run through a queue you can reorder, which waits while you are offline and resumes when the connection is back. If you move or delete a game's folder by hand (from any store), Heronite asks for the new folder or lets you mark the game as not installed.
- **GOG:** log in, and your GOG games appear in the Collection. Install them (Windows or native Linux builds, with the language and DLCs you choose), update, repair, move, import, and uninstall them, and sync cloud saves. Downloads share the same queue.
- **Amazon Games:** log in, and the games of your Amazon account (Prime Gaming included) appear in the Collection. Install, update, verify, move, import, and uninstall them through the same queue.
- **Wine and Proton:** each Windows game gets Heroic's settings (Wine version and prefix, DXVK-NVAPI, Esync/Fsync, gamescope, launch options, environment variables you can paste as text), in tabs by what you want to change. The GPU tab picks the graphics card and turns on DLSS, FSR 4, or XeSS upgrades. Each game gets one FPS limit, with the limiter you choose (MangoHud, DXVK/VKD3D-Proton, or gamescope). A Winetricks page lists what is installed and what can be, and a preview shows the exact command a launch will run. Each changed setting can be reset on its own.
- **Fixes and comfort:** Proton gets each game's protonfixes through its umu ID, found by itself (or set by hand, with suggestions from the umu database). The anti-cheat list stays current. Save environment variables, wrappers and arguments as presets to apply to other games, and switch controllers to SDL when a game does not see them right. The computer stays awake while a game runs (useful with a controller), unless you turn that off for the game.
- **Frame generation:** turn on Lossless Scaling frame generation (through lsfg-vk) for any game, Steam games included, with its multiplier, flow scale, performance mode and HDR, or one of lsfg-vk's own profiles.
- **Wine Manager:** downloads and updates Proton from six sources: Proton-CachyOS, GE-Proton, DW-Proton, UMU-Proton, Proton-EM and Proton-Sarek. It only asks GitHub for a source's list when that source has something new, and it can make the builds available to Steam too. It also lists your Wine prefixes and the games that use each, and creates, duplicates, or removes them. Uninstalling a game keeps a prefix that other games still use.
- **Emulators:** detect installed emulators (PATH, Flatpak, AppImages, and folders such as `~/Applications`, `~/Games`, or `/opt`) or add your own with profiles and RetroArch cores, and open any of them without a game. Scan ROM folders to import games, with multi-disc games merged into one entry, and continue from a save state. An emulated game can become its PC version (a native port, say) and keep its play time, status, and art.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/game-settings.webp"><img src="assets/screenshots/game-settings.webp" width="100%" alt="Game settings drawer for Control on the Wine tab, with the Proton version, prefix, DXVK-NVAPI, Esync, Fsync, and Winecfg and Winetricks buttons" /></a><br /><sub><b>Game settings</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/wine-manager.webp"><img src="assets/screenshots/wine-manager.webp" width="100%" alt="Wine Manager listing Proton-CachyOS builds with release dates, sizes, and install buttons" /></a><br /><sub><b>Wine Manager</b></sub></td>
  </tr>
</table>

## Look back at what you played

- **Statistics:** see your play time for a week, a month, a year, or all time. The page shows a day-by-day chart, your top games, when you usually play, your records, and the genres you play most.
- **Year in review:** your year told as seven full-screen stories: the big number, your number one game, the top five, your habits, your records, and a summary card you can copy.
- **Sessions and screenshots:** each launch is recorded as a session. Screenshots taken in game with a shortcut are matched to the game and its timeline, and can be compressed to lossless WebP in the background.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/statistics.webp"><img src="assets/screenshots/statistics.webp" width="100%" alt="Statistics for October 2026 with total time played, a day-by-day chart per game, and the top eight games" /></a><br /><sub><b>Statistics</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/year-in-review.webp"><img src="assets/screenshots/year-in-review.webp" width="100%" alt="Year in review intro story: Your 2026 in games, with days, games, and sessions next to a wall of covers" /></a><br /><sub><b>Year in review</b></sub></td>
  </tr>
</table>

## Console mode

A controller-first layout for the couch or a TV. It covers the whole app: the library, game pages, downloads, the Wine Manager, Epic, GOG, Amazon Games, emulators, statistics, and settings. You can use a gamepad or the keyboard, and an on-screen keyboard is available for text fields. Turn it on from the palette or the ☰ menu (**Console mode**), or start Heronite with `--console`; it remembers the mode on the next start, and **Desktop mode** in its menu goes back. The desktop layout does not change. Its menu (<kbd>Start</kbd>) also has **Power**, to suspend, restart or shut down the computer without leaving the couch.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/console-home.webp"><img src="assets/screenshots/console-home.webp" width="100%" alt="Console home with ELDEN RING large over its art and a Continue playing shelf of covers" /></a><br /><sub><b>Home</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/console-game.webp"><img src="assets/screenshots/console-game.webp" width="100%" alt="Console game page for ELDEN RING with Play, Status, Favourite, Game settings, Playtime, and Ludusavi backup rows next to its details" /></a><br /><sub><b>Game page</b></sub></td>
  </tr>
</table>

## Make it yours

- **Appearance:** set the opacity, blur, darkening, and colour intensity of the game art behind the Collection. Panels can be glass or solid, and covers can use the Steam (2:3), IGDB (3:4), GOG (11:15), or other aspect ratios.
- **Screenshots:** set the capture folder and the in-game shortcut, compress captures without quality loss, and limit the preview cache.
- **Backups:** back up saves with Ludusavi after each game closes (or manually from the game's menu), restore any backup into the game's current Wine/Proton prefix from its settings (even one made in another Steam library or on Windows, with undo), and schedule copies of the whole data folder.
- **Startup:** start Heronite with the system, minimized if you like. `heronite://launch` links and `--launch` start a game straight away, and `--fullscreen` opens the window fullscreen.

<table>
  <tr>
    <td width="50%" align="center"><a href="assets/screenshots/appearance.webp"><img src="assets/screenshots/appearance.webp" width="100%" alt="Appearance settings with a live preview of the Collection background, its sliders, glass panel options, and cover aspect" /></a><br /><sub><b>Appearance</b></sub></td>
    <td width="50%" align="center"><a href="assets/screenshots/screenshots.webp"><img src="assets/screenshots/screenshots.webp" width="100%" alt="Screenshots settings with the capture folder and space used, lossless compression, the in-game capture shortcut, and the preview cache" /></a><br /><sub><b>Screenshots</b></sub></td>
  </tr>
</table>

## Light on your PC

A game launcher should get out of the way when the game starts. Heronite is a single native process that draws its UI on the GPU, with no bundled browser engine. A web engine is only used by the store login window (Epic, GOG, Amazon), which is a separate small program started when you log in. Slow work like scanning, metadata, image decoding, and compression runs on background threads. When the app sits idle, it does no work.

| | Heronite |
|---|---|
| Memory while idle | **~195 MB** in a single process |
| CPU while idle | **0.0%** (no CPU time used over 60 s) |
| Install size | **47 MB** (app + store login helper) |
| Start to library on screen | **~335 ms** median |


## Your data

Everything stays on your machine, in Heroic-compatible files: library, Collection, statuses, sessions, playtime, and settings live in `~/.local/share/heronite/` (override it with `HERONITE_DATA_DIR`). UI state goes in `~/.config/heronite/` and caches in `~/.cache/heronite/`; with another data folder, they go in a `<folder>.heronite/` next to it instead.

On first start, if that folder does not exist yet, Heronite copies an existing Heroic setup into it **once**. It only reads Heroic's files and never writes to Heroic's folders, so both apps can stay installed.

Online features need connectivity: metadata and art (IGDB, Steam, SteamGridDB, Lutris, How Long To Beat, ProtonDB), the umu database and the anti-cheat list (AreWeAntiCheatYet), Epic Games, GOG, Amazon Games, Wine/Proton downloads, and the app's update check (Heronite asks GitHub for the latest release at start and every 6 hours; switch it off in **Settings > About**). For IGDB, add your own Twitch client ID and secret in **Settings > Metadata**, and your SteamGridDB API key there to get its covers.

## Download

Every [release](https://github.com/RafaelGoulartB/Heronite/releases/latest) has an AppImage that runs anywhere, and packages for Debian/Ubuntu, Fedora/openSUSE, and Arch. Pick one:

| System | File | Install |
|---|---|---|
| Any distribution (**recommended**) | `Heronite-x86_64.AppImage` | `chmod +x Heronite-x86_64.AppImage && ./Heronite-x86_64.AppImage` |
| Debian 12+, Ubuntu 22.04+, Mint, Pop!_OS | `heronite_<version>-1_amd64.deb` | `sudo apt install ./heronite_*_amd64.deb` |
| Fedora 36+, openSUSE | `heronite-<version>-1.x86_64.rpm` | `sudo dnf install ./heronite-*.x86_64.rpm` |
| Arch, Manjaro, EndeavourOS, CachyOS | `heronite-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U heronite-*-x86_64.pkg.tar.zst` |

**Updates.** When a new version is out, a card offers it, or you can check in **Settings > About**. Every download is checked against Heronite's signing key before anything is installed.

- **Packages:** Heronite downloads the new package and installs it with apt, dnf, or pacman, which asks for your password. If no password prompt can open, Heronite shows the command to run in a terminal. The packages are not in any distribution repository, so `apt upgrade` and similar commands do not update them.
- **AppImage:** the update is downloaded next to the AppImage and swapped in place, so menu entries and game shortcuts keep working. Keep the AppImage in a folder you can write to (for example `~/Applications`). You can also let it install updates automatically. Tools that use AppImageUpdate (Gear Lever, AppImageLauncher) can update it too, downloading only the changed parts.

Either way, the new version runs after **Restart now** or the next time you open Heronite.

## Run from source

You need a stable Rust toolchain, `clang` and `lld` (used as the linker, see `.cargo/config.toml`), a Vulkan-capable GPU driver, and GTK/WebKitGTK for the store login window. Optional tools: [legendary](https://github.com/derrod/legendary) for Epic Games, [gogdl](https://github.com/Heroic-Games-Launcher/heroic-gogdl) for GOG (needs `python3`), [nile](https://github.com/imLinguin/nile) for Amazon Games, and [Ludusavi](https://github.com/mtkennerly/ludusavi) for save backups. Heronite downloads legendary, gogdl and nile into its data folder when they are missing.

```bash
# fast optimised build for trying the app (the login window is a binary of
# its own: build it too, or the app runs an old one)
cargo build --profile quick -p heronite-epic-login && cargo run --profile quick -p heronite

# build in release mode and install for the current user
# (~/.local/opt/heronite-rs, a launcher in ~/.local/bin and a .desktop entry)
scripts/install.sh
```

## Development

The app is a Cargo workspace. `app/` is the GPUI binary (`heronite-rs`): the Collection screen, dialogs, ☰ pages, and console mode. The `crates/` folder holds the GPUI-free libraries: data files and Heroic migration, launching and playtime, Epic (legendary), GOG (gogdl), Amazon Games (nile), emulation, Steam library and settings, metadata, screenshots and images, Wine/Proton, shortcuts, app updates, and translations. A separate small binary (`heronite-epic-login`) handles the store login window, which keeps a web engine out of the main app.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --release -p heronite -p heronite-epic-login
```

`HERONITE_DEBUG_*` environment variables open a page, dialog, or menu at startup, for screenshots and tests without clicking. See [ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crates and patterns, [CODE-STYLE.md](docs/CODE-STYLE.md) for the code rules, and [UI-CONVENTIONS.md](docs/UI-CONVENTIONS.md) for the look of the UI. [CI](.github/workflows/ci.yml) checks the formatting (`rustfmt.toml`) and runs clippy and the tests on every push and pull request. To release, merge a pull request labelled `release:patch`, `release:minor`, or `release:major`, or run the [release workflow](.github/workflows/release.yml) from the Actions tab. It bumps the version in `Cargo.toml` (`scripts/bump-version.py`), tags it, builds the AppImage (`scripts/appimage.sh`) and the .deb, .rpm, and Arch packages (`scripts/packages.sh`, nfpm), installs the packages and runs the AppImage on clean containers (`scripts/package-smoke.sh`), signs everything (`scripts/sign-release.sh`, with the minisign secret key in the `MINISIGN_SECRET_KEY` repository secret), and publishes a GitHub release. Issues and pull requests are welcome.

## License

Copyright (C) 2026 [Rafael Goulart](https://www.linkedin.com/in/rafael-goulartb/)

This program is free software: you can redistribute it and/or modify it under the terms of the [GNU General Public License v3.0](LICENSE) as published by the Free Software Foundation. Heronite builds on the [Heroic Games Launcher](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher), which is also licensed under the GPL-3.0. Fonts, icons, and other third-party components keep their own licenses; see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
