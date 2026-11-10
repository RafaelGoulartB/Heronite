# Mods

Heronite manages mods per game, MO2-style: every mod lives in a folder of its
own and, on Linux, the game's folder is never written (Windows: see
[Windows](#windows)). When a game with mods on starts
from Heronite, `fuse-overlayfs` is mounted **over the game's own folder**: the
enabled mods are layers above the game's files (the last mod of the list wins
a file), and whatever the game writes while it runs lands in the game's
*overwrite* folder. When the game ends the folder is unmounted and the game is
vanilla again. Turning a mod off or on, reordering, removing: nothing is
reinstalled or copied, the next launch simply mounts other layers.

Mods are a part of the app a system may lack (`Feature::Mods`): a Flatpak
cannot mount, so there the manager, Settings → Mods and the links are not
offered and games start as they are (`services::mods::available`).

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
4. **Layout is fixed for the user** (`layout::auto`, a function of the
   archive's tree and the game folder):
   - A lone folder is unwrapped when what is inside is recognisably the mod
     (a folder of the game or of the game's table entry, `markers`, mod files)
     or it is named like a download; readmes and pictures beside it are
     ignored. Control's `data_pc` is never peeled; `Cool Mod/natives/...`
     becomes `natives/...` for the RE Engine games.
   - Bethesda: mods go into `Data`; a script extender's archive, ENB and
     ReShade (proxy DLLs, `enbseries`, their `.ini`) go to the game folder.
   - Games with a mods folder (Witcher 3 `Mods`): each top entry on its own: a
     folder the game has (or a marker) stays, the rest goes to `Mods/`.
   - Unreal: loose `.pak`/`.ucas`/`.utoc` go to `<X>/Content/Paks/~mods`
     (found in the game folder); proxy DLLs, `.asi`, launchers and their
     `.ini` go to the folder of the only executable when the game folder has
     none. Elden Ring and Dark Souls III use `Game`; Cyberpunk's loose
     `.archive` goes to `archive/pc/mod`.
   - Thunderstore: r2modman's BepInEx layout (a plugin without a version in
     its folder name); MelonLoader content (`MelonLoader/`, `Mods/`,
     `UserLibs/`, `version.dll` + `dobby.dll`) goes as it is to the game
     folder; Hades II (Hell2Modding) puts `d3d12.dll` and `ReturnOfModding/`
     next to the executable. A package with only a manifest (a modpack) is
     added with no files so its dependencies keep meaning.
   - Every path is matched case-insensitively, to the game's own folders and
     then to the mods already installed (the first spelling wins), so
     `data/textures` becomes `Data/Textures` and two mods never leave
     `Textures/` and `textures/` side by side: Linux is case-sensitive and
     Windows mods are not.
   After an install, `install::looks_misplaced` tells when none of a mod's
   top-level names fits the game.
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
8. **Limits the manager says out loud.** The manager is labelled
   "Experimental" and every game has a label for what to expect of its mods
   (see "What to expect" below). Games with anti-cheat (Elden Ring, Dark
   Souls III, any game the anti-cheat list knows) can ban the account for
   online play; a Linux build ignores DLL-based loaders. A mod whose files
   do not look like they belong in the game's folders
   (`heronite_mods::install::looks_misplaced`) ends its install with an amber
   job that stays until dismissed and gets a "Check files" pill; a click on
   the pill says why the game will probably not see the mod and offers its
   folder, its page, Reinstall and Remove. The flag is looked at again each
   time the game's list is checked, so it outlives the app. Without the
   game's folder nothing is installed.
9. **While the game runs** (the overlay is mounted) the mods' files are in
   use: removing, updating, reinstalling, replacing, emptying the overwrite
   folder and "Remove all mods" are refused with a notice.

## What to expect (`support`)

The overlay only reaches the game's own folder, so the manager says how far
that goes for each game: `Support::of(known game, kind, native)` gives a
level and a reason, shown as a label under the game's title (the reason under
the pointer) and, for every level but the first, as a card on the Installed
tab.

| Level | Label | Games |
|---|---|---|
| Expected | Should work | Known games whose mods are files in the game's folder, with the loader the manager sets up: the BepInEx games of Thunderstore, Unreal `~mods`, Control, Monster Hunter: World, Sekiro, Kingdom Come, Bannerlord, RimWorld… |
| Steps | Needs extra steps | Bethesda games (script extender loader, plugin order by hand), Morrowind and Oblivion Remastered (the game's own plugin list), Cyberpunk 2077 (REDmod), The Witcher 3 (Script Merger), the RE Engine games (REFramework), Elden Ring and Dark Souls III (ModEngine 2), Stardew Valley (SMAPI's installer) |
| Untested | Untested game | Every game that is not in the table |
| Unsupported | Most mods won't work | Baldur's Gate 3, Dragon Age: Origins, Divinity: Original Sin 2 (mods outside the game's folder), Mass Effect Legendary Edition (its own mod manager), and the Linux build of a BepInEx game |

The level of a known game is its `KnownGame::fit`. It comes from how the game
loads mods, not from a test of the game: move a game up or down as it is
tried.

## Storage

`<mods root>` = `<data>/mods`.

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

Backups (`heronite_backup::catalog`) leave the whole `<mods root>` out
(`Left::Mods`): the mod files and their downloads are big, what a game wrote
while modded can be as big, and a list restored without its files would stop
the game from starting. The Nexus Mods key is typed again after a restore.

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

## Windows

Hidden for now (`Feature::Mods` is off on Windows; a debug build turns it on
with `HERONITE_FEATURES=+mods`): mod archives still need 7-Zip, `unzip` or
`bsdtar` on the PATH, which a stock Windows lacks, and putting the game's
folder back has no retry when a file is still locked.

Windows has no overlay file system, and Heronite does not hook a game's file
calls (MO2's usvfs). So while the game runs, the enabled mods' files are **in
its folder**, and they come out again when it ends (`journal_deploy`):

- **Link, else copy.** Each file is a hard link to the mod's copy when the
  mods root and the game are on the same volume, and a copy otherwise
  (another drive, FAT, a share). The first failed link makes the rest of
  that deployment copy. Text files (`.ini .cfg .json .xml .txt .toml .yaml
  .yml .log`) and files under 64 KiB are always copied, so a game that
  rewrites one never writes into the mod through a link.
- **The game's own files** that a mod replaces are moved to
  `<mods root>/<game key>/.backup/<path>` and put back afterwards.
- **Journal first.** `deploy.json` (the plan, then the size and time of each
  placed file) and `deploy.snapshot` (the game folder's list) are written
  before the game folder is touched. `restore` is idempotent: after a crash at
  any step it puts the folder back. It runs when the session ends (the guard),
  when an adopted game ends (`GameMods::release`), at start for every game
  that is not running (`deployment::cleanup_stale`), and before the next
  deployment. A failed restore keeps the journal for the next try.
- **What the game wrote** goes to the overwrite folder, as on Linux: files
  that were not in the snapshot, and copies of ours whose size or time
  changed. The overwrite folder is the top layer of the next deployment. Not
  caught: changes to the game's own files no mod replaced (they stay, as
  without mods), game files the game deleted, and in-place writes into a
  linked file.
- While a journal exists the game counts as running with mods
  (`deployment::in_use`): removing, updating and emptying are refused.
- **Proxy DLLs** need nothing: Windows loads the DLL in the game folder before
  System32's, so there is no `WINEDLLOVERRIDES`, and nothing is offered for
  Steam's launch options. A script extender's Steam line is
  `"<game folder>\skse64_loader.exe" %command%`.
- **Bethesda**: `Plugins.txt` in `%LOCALAPPDATA%\<game>`, the `*Custom.ini`
  keys in `<Documents>\My Games\<game>`. Documents comes from the known
  folder, so OneDrive redirection is followed.
- **Links**: `nxm://` and `ror2mm://` under `HKCU\Software\Classes`, for this
  user, only when Settings → Mods is asked to.
- Not used: symbolic links (they need Developer Mode or admin, and some games
  and anti-cheat refuse reparse points) and the Projected File System (an
  optional feature that is never turned on from here). Settings → Mods shows
  whether either is available.

Storage adds, per game: `deploy.json`, `deploy.snapshot` and `.backup/`, only
while mods are in the game folder.