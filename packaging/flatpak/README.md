# Flatpak

`com.heronite.rs.yml` builds Heronite as a Flatpak on the GNOME runtime
(it has WebKitGTK 4.1 for the stores' login window), with the 32-bit
libraries Wine and Proton need (`org.freedesktop.Platform.Compat.i386` and
`GL32`), `vulkaninfo`, `7z` and `cabextract`. The build is offline:
`cargo-sources.json` lists every crate of `Cargo.lock` and is made by
[`scripts/flatpak.sh`](../../scripts/flatpak.sh), not kept in git.

## Build

```bash
scripts/flatpak.sh            # dist/Heronite-x86_64.flatpak
scripts/flatpak.sh --install  # and install it for this user
flatpak run com.heronite.rs
```

It needs `flatpak`, `python3` and `flatpak-builder` (or the
`org.flatpak.Builder` Flatpak); the GNOME SDK, Rust and LLVM extensions come
from Flathub. On Ubuntu, [`scripts/ci-flatpak-deps.sh`](../../scripts/ci-flatpak-deps.sh)
installs what CI uses. Every release attaches the bundle, and pull requests
that touch the Flatpak get it as a workflow artifact.

## What changes in the Flatpak

The app finds out it runs in a sandbox (`heronite_platform::flatpak`):

- Its data, settings and caches are in `~/.var/app/com.heronite.rs/`, apart
  from the AppImage's and the packages' `~/.local/share/heronite/`.
- Flatpak installs the updates: the in-app updater says so instead of
  installing them.
- The game sandbox (bubblewrap) cannot run inside Flatpak's sandbox: its
  settings are hidden and games run without it.
- Steam (starting and closing it) and Flatpak emulators start on the host
  through `flatpak-spawn --host`; Steam games open through `steam://` links.
- Game shortcuts, Steam shortcuts and the autostart entry start the app
  through the launcher Flatpak exports (`exports/bin/com.heronite.rs`), and
  go in the host's `~/.local/share/applications` and `~/.config/autostart`.
- MangoHud and gamescope are Flathub's Vulkan layers:
  `flatpak install flathub org.freedesktop.Platform.VulkanLayer.MangoHud//26.08 org.freedesktop.Platform.VulkanLayer.gamescope//26.08`.

Programs installed on the host itself (a native emulator, Ludusavi, the
system's Wine, gpu-screen-recorder) are not seen from inside the sandbox: use
their Flatpak or AppImage, or the AppImage of Heronite.

## Flathub

To publish on Flathub, a copy of the manifest goes to the
`flathub/com.heronite.rs` repository with these changes:

- the `heronite` module's `dir` source becomes the release's `git` source
  (`url`, `tag` and `commit`);
- `cargo-sources.json`, made by `scripts/flatpak.sh` from that tag's
  `Cargo.lock`, is committed next to it;
- the app id must be one Flathub can verify: a domain the project controls
  (`app.heronite.Heronite` for heronite.app) or `io.github.RafaelGoulartB.Heronite`.
