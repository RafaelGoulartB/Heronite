# Third-party notices

Heronite is licensed under the [GNU General Public License v3.0](LICENSE). It includes or is derived from the third-party work listed below. The full license texts are in [`licenses/`](licenses/).

## Heroic Games Launcher

- **Project:** [Heroic Games Launcher](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher)
- **Copyright:** the Heroic Games Launcher contributors
- **License:** GPL-3.0 ([`LICENSE`](LICENSE))
- **Used in Heronite:** parts of the code are ported from Heroic. This includes the Epic Games integration through legendary, the Wine Manager, game settings, Steam shortcuts, and the data formats Heronite reads and writes.

## Fonts

| Font | Files | Copyright | License |
|---|---|---|---|
| [Cabin](https://github.com/impallari/Cabin) | `app/assets/fonts/Cabin-400.ttf`, `Cabin-600.ttf` | Copyright 2018 The Cabin Project Authors | SIL Open Font License 1.1 ([`licenses/OFL-1.1.txt`](licenses/OFL-1.1.txt)) |
| [Rubik](https://github.com/googlefonts/rubik) | `app/assets/fonts/Rubik-400.ttf`, `Rubik-600.ttf` | Copyright 2015 The Rubik Project Authors | SIL Open Font License 1.1 ([`licenses/OFL-1.1.txt`](licenses/OFL-1.1.txt)) |

## Icons

| Icons | Files | Copyright | License |
|---|---|---|---|
| [Material Icons](https://github.com/google/material-design-icons), through [`@mui/icons-material`](https://github.com/mui/material-ui/tree/master/packages/mui-icons-material) | every `app/assets/icons/*.svg` with a `0 0 24 24` view box, except `moon`, `sun`, `sunrise`, and `sunset`, which were drawn for Heronite (as were `play-circle`, `install-circle`, `stop-circle`, and `cursor`) | Google LLC (icons); Copyright (c) 2014 Call-Em-All (package) | Apache License 2.0 ([`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt)); MIT ([`licenses/MIT-mui-icons-material.txt`](licenses/MIT-mui-icons-material.txt)) |
| [Font Awesome Free](https://fontawesome.com) 6 | `app/assets/icons/fa-*.svg` | Copyright (c) 2024 Fonticons, Inc. | CC BY 4.0 ([`licenses/FontAwesome-Free.txt`](licenses/FontAwesome-Free.txt)) |

## Libraries in the AppImage

The AppImage carries these system libraries in `usr/lib`, taken unchanged from Ubuntu 22.04 (`scripts/appimage.sh`), because not every system has them. The .deb, .rpm and Arch packages use the system's copies instead. Each one is a separate shared library: replace it in an extracted AppImage to use another build.

| Library | Files | Copyright | License |
|---|---|---|---|
| [libxkbcommon](https://xkbcommon.org) | `libxkbcommon.so.0`, `libxkbcommon-x11.so.0` | The Open Group, Dan Nicholson, Silicon Graphics, Digital Equipment Corporation, and the other xkbcommon authors | MIT and similar permissive licenses ([`licenses/MIT-xkbcommon.txt`](licenses/MIT-xkbcommon.txt)) |
| [libxcb](https://xcb.freedesktop.org) | `libxcb-xkb.so.1` | Copyright (C) 2001-2006 Bart Massey, Jamey Sharp, and Josh Triplett | MIT ([`licenses/MIT-libxcb.txt`](licenses/MIT-libxcb.txt)) |
| libudev, from [systemd](https://github.com/systemd/systemd) | `libudev.so.1` | the systemd contributors | LGPL-2.1-or-later ([`licenses/LGPL-2.1.txt`](licenses/LGPL-2.1.txt)); source: the [`systemd` package of Ubuntu 22.04](https://launchpad.net/ubuntu/jammy/+source/systemd) |

## Rust crates

The Rust crates Heronite depends on are listed in [`Cargo.lock`](Cargo.lock). Each one is distributed under its own license, mostly MIT and/or Apache-2.0. To list them all with their license texts, run [`cargo about`](https://github.com/EmbarkStudios/cargo-about) or [`cargo license`](https://github.com/onur/cargo-license).

## External tools

Heronite runs these programs if they are installed. It does not include them:

- [legendary](https://github.com/derrod/legendary) (GPL-3.0) for Epic Games
- [Ludusavi](https://github.com/mtkennerly/ludusavi) (MIT) for save backups
- Wine/Proton builds downloaded by the Wine Manager ([GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom), [Proton-CachyOS](https://github.com/CachyOS/proton-cachyos)), under their own licenses
