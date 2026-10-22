# Mods

Heronite manages mods per game, MO2-style: every mod lives in a folder of its
own and the game's folder is never written. When a game with mods on starts
from Heronite, `fuse-overlayfs` is mounted **over the game's own folder**: the
enabled mods are layers above the game's files (the last mod of the list wins
a file), and whatever the game writes while it runs lands in the game's
*overwrite* folder. When the game ends the folder is unmounted and the game is
vanilla again. Turning a mod off or on, reordering, removing: nothing is
reinstalled or copied, the next launch simply mounts other layers.

Crate: `heronite-mods` (no GPUI, blocking functions). App: `services::mods`
(the service: jobs, links, account), `dialogs/mods/` (the manager), Settings →
Mods.

## User flow

1. **Find the mods of a game.** Every installed PC game has "Mods…" in its
   context menu, in the command palette and as a row of the Focus panel's
   facts card ("Mods: 12 on"). It opens the Mods manager (a drawer).
2. **Add mods**, three ways, all landing in the same list:
   - *Nexus Mods*: the "Mod manager download" button on nexusmods.com opens an
     `nxm://` link. Heronite registers itself for `nxm://` (Settings → Mods, or
     the manager's first-run card) and needs the user's personal API key
     (Settings → Mods → Nexus Mods account). The link names the game
     (`nxm://skyrimspecialedition/...`): the mod goes to the Collection game
     linked to that Nexus game, found by Steam app id / title, or asked once
     with a game picker (and remembered). Premium accounts can also install
     straight from the manager's Browse tab; free accounts get "Download on
     Nexus Mods" there, which opens the file page in the browser.
   - *Thunderstore*: the Browse tab searches the game's Thunderstore community
     in the app and installs with every dependency (BepInExPack included);
     "Install with mod manager" on thunderstore.io (`ror2mm://`) works too.
   - *A file*: "Add from file…" (zip, 7z, rar, tar, or a folder), or dropping
     an archive on the manager.
3. **Installers.** A FOMOD installer (`fomod/ModuleConfig.xml`) opens the
   installer wizard: steps, options with their pictures and descriptions,
   conditions between them. The choices are kept: "Reinstall" asks again with
   them preselected.
4. **Layout is fixed for the user.** Archives wrapped in extra folders are
   unwrapped; Bethesda mods go into `Data` (a script extender's archive goes to
   the game folder); Thunderstore packages follow r2modman's BepInEx layout;
   every path is matched case-insensitively to the game's own folders
   (`data/textures` becomes `Data/Textures` when the game has that), because
   Linux is case-sensitive and Windows mods are not.
5. **Manage.** The list is the load order (drag, or ↑ ↓): a switch per mod, a
   master switch "Play with mods", conflicts ("Overrides 2 mods", "3 files
   overridden by X", with the file list), updates ("Update 1.2 → 1.3"), the
   mod's page, its folder, rename, reinstall, remove. Bethesda games get a
   Plugins list (`.esp`, `.esm`, `.esl` of the enabled mods, each with a
   switch), written to the prefix's `Plugins.txt` at launch and put back after.
6. **Play.** Play as usual. "Applying mods…" shows while the overlay mounts.
   Mod loaders that work as proxy DLLs (BepInEx's `winhttp.dll`, MelonLoader's
   `version.dll`, ASI loaders' `dinput8.dll`, …) get their
   `WINEDLLOVERRIDES=<dll>=n,b` added to the launch automatically. Steam games
   launch through Steam: the manager offers to add the variable to the game's
   Steam launch options. A script extender or loader an enabled mod brings
   (SKSE, F4SE, NVSE, SFSE, OBSE, ModEngine 2, SMAPI) only works when the game
   starts through its executable: the manager says so and, for games Heronite
   starts, sets the game's alternative executable (`targetExe`) with an Undo;
   for Steam games it shows the launch-options line to paste. Plans of games
   with mods are built with a probe that also sees the mods' files
   (`services::mods::ModsProbe`), because the plan checks the executable
   before the overlay exists.
7. **What the game wrote.** Configs, logs and saves the game writes inside its
   folder while modded are in the *overwrite* folder (the last row of the
   list when it is not empty, deletions included): open it, turn it into a
   mod, or empty it. Turning "Play with mods" off keeps it (a notice says the
   game will not see it until mods are on again); "Remove all mods" deletes it
   and says so.
8. **Limits the manager says out loud.** Baldur's Gate 3's `.pak` mods live
   outside the game's folder (not applicable); games with anti-cheat (Elden
   Ring, Dark Souls III, any game the anti-cheat list knows) can ban the
   account for online play; a Linux build ignores DLL-based loaders. A mod
   whose files do not look like they belong in the game's folders gets a
   "Check files" pill and a notice after the install
   (`heronite_mods::install::looks_misplaced`). Without the game's folder
   nothing is installed.
9. **While the game runs** (the overlay is mounted) the mods' files are in
   use: removing, updating, reinstalling, replacing, emptying the overwrite
   folder and "Remove all mods" are refused with a notice.

## Storage

`<mods root>` = `<data>/mods` (Settings → Mods → Mods folder can move it).

```
<mods root>/
  nexus.json                    Nexus account: API key (0600), name, premium
  links-cache/                  Nexus game list, Thunderstore communities (7 days)
  <game key>/                   runner_appName
    mods.json                   ModList: order, switches, sources, choices
    mods/<mod id>/              the mod's files, relative to the GAME folder
    downloads/                  archives kept for reinstall / FOMOD
    overwrite/                  upperdir: what the game wrote while modded
    .work/                      fuse-overlayfs workdir
    .layers/<nnn>               short symlinks to the layers (mount options)
```

Backups (`heronite_backup::catalog`): `mods.json` and `overwrite/` are in the
Collection part, `nexus.json` in Logins; mod files, downloads and caches are
left out (`Left::Mods`: big, downloaded again).

## Mounting (`deploy`)

- `fuse-overlayfs` from `$PATH`, else `<data>/tools/bin/fuse-overlayfs`, else
  downloaded once from containers/fuse-overlayfs v1.18 (static build, SHA-256
  pinned). Needs `fusermount3` (or `fusermount`) and `/dev/fuse`.
- `fuse-overlayfs -o lowerdir=<.layers/NNN of the enabled mods, highest
  priority first>:<.layers/game>,upperdir=overwrite,workdir=.work,squash_to_uid=<uid>,squash_to_gid=<gid> <game folder>`.
  fuse-overlayfs opens its layers before mounting, so mounting over the game
  folder works and the game folder is its own bottom layer.
- Unmount with `fusermount3 -u`, retried for a few seconds, then `-uz` (lazy).
- Before mounting, a mount already there (a crash) is unmounted; at app start
  every stale mount of ours whose game is not running is unmounted.
- The sandbox (`heronite_launch::sandbox`) binds the game folder read-only:
  it binds the mount, so sandboxed games see their mods too.

## Launch hook

`heronite_launch::GameMods` (set with `Launcher::set_game_mods`): on the
session thread, `apply(app_name, runner)` mounts and returns the extra
environment and a guard. A process plan applies inside `run_process`, last:
after the prefix, the first-launch store setup and fixes and the before
script, right before the game spawns, so whatever the preparation writes
into the game's folder lands in the folder and not in *overwrite*. Steam
applies before the URI is opened. The guard is dropped when the session ends
(`Plugins.txt` and the game's `*Custom.ini` restored, then unmount). A
failure aborts the launch with the reason ("Mods could not be applied: …")
rather than starting the game without the mods its saves expect; so does an
enabled mod whose folder is missing.

`apply` also sets, in the prefix and with a crash-safe backup like
`Plugins.txt` (`file_swap`): for Fallout 4 and Starfield the
`[Archive] bInvalidateOlderFiles=1` and `sResourceDataDirsFinal=` keys of
`Documents/My Games/<game>/<game>Custom.ini` (the games ignore loose files
without them). Plugins and that file are prepared before the mount, so old
Bethesda games' file times are computed from the real game folder.

A game picked up again after the app restarted (`Launcher::adopt`) has no
guard: when it ends, `GameMods::release` unmounts the overlay of ours that is
still on its folder (`deploy::unmount_ours`).

## Service (`services::mods`)

- Lists are shared with the job threads and written in the background from the
  latest list; a change shows at once. A job builds a mod from a copy of the
  list and merges the result into the shared one, so toggles made meanwhile
  are kept.
- A job is one thread: download (three at a time over all games), then the
  game's install turn (one at a time per game, held while a FOMOD installer
  waits for the user), unpack, wizard, commit. A scripted or unreadable
  installer installs the files as they are, with a notice.
- A link finds the game by the games' own links, then the known table, then
  the title; one match starts the job and asks the UI to open that manager
  (`OpenManager`), else `PickGame`. Links read before the window is up wait in
  the service.
- At start: unfinished installs are swept and stale overlays unmounted (not
  when the start is a game launch: its own mount is being made).
- Settings → Mods: the account, who opens the links, fuse-overlayfs and FUSE.
