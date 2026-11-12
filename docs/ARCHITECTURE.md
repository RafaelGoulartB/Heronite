# Architecture

HeroniteRust is a GPUI rewrite of the Heronite launcher (a Heroic Games
Launcher fork). It is a separate app with its own data folder,
`~/.local/share/heronite/` (`DataPaths`; `HERONITE_DATA_DIR` overrides
it). The folder keeps Heroic's layout and electron-store formats, but nothing
in it is shared with Heroic or Heronite, and this app never reads or writes
Heroic's folders. UI state lives in `~/.config/heronite/` and caches in
`~/.cache/heronite/` (`AppPaths`).

On the first start, when the data folder does not exist yet, the UI state
and cache folders of the old name (`heronite-rs`) are renamed. Nothing is
copied from Heroic then: its games come in only when the user turns its
import on in Settings → Other launchers (`heronite_import::heroic`).
`heronite_data::migrate` can still copy a whole Heroic setup
(`~/.config/heroic`, `%APPDATA%\heroic` on Windows, rewriting the paths into
it, also where JSON doubles a Windows path's backslashes), but
`Feature::HeroicMigration` is off on every system.

`docs/CODE-STYLE.md` has the code rules; `docs/UI-CONVENTIONS.md` the look of
the UI.

## Crates

```
app (bin heronite-rs) ── GPUI UI, desktop integration
 ├─ heronite-i18n       interface language: tr!/trn! and the pt-BR, es and de strings (no GPUI)
 ├─ heronite-data       data folder files, paths, Heroic migration, safe writes, processes
 ├─ heronite-backup     backups of the data folder and app state:
 │                      incremental snapshots, retention, restore    → data
 ├─ heronite-images     decode/resize/blur pipeline and thumbnail cache (no GPUI);
 │                      `public`: what an extension causes to be fetched
 │                      reaches public addresses only
 ├─ heronite-sync       the cloud account: the library synced between PCs,
 │                      backups, game saves, screenshots (rclone)    → data, backup, metadata, launch
 ├─ heronite-launch     launch plans, game runtime, playtime, logs   → data
 ├─ heronite-epic       legendary CLI, Epic library, download queue  → data, launch
 ├─ heronite-gog        gogdl CLI + GOG web APIs: GOG account,
 │                      library, installs, updates, cloud saves      → data, launch, epic
 ├─ heronite-amazon     nile CLI: Amazon Games account, library,
 │                      installs, updates                            → data, launch, epic
 ├─ heronite-clients    store clients (Battle.net, EA app, Ubisoft
 │                      Connect, Rockstar Games Launcher), under Wine
 │                      on Linux, as they are on Windows:
 │                      setup, their games, sign-in, open / close     → data, launch, epic
 ├─ heronite-emulation  emulators, ROM scan, save states, Ludusavi   → data, launch
 ├─ heronite-xbox       the Microsoft Store and PC Game Pass games the
 │                      Xbox app installs (Windows packages): found
 │                      among the user's packages, kept in the
 │                      Collection                                   → data, platform
 ├─ heronite-import     games of Heroic, Lutris, Bottles, Faugus and Playnite
 │                      brought into the Collection                  → data
 ├─ heronite-installer  games from YAML install scripts              → data, launch, emulation
 ├─ heronite-display    monitors, their modes, HDR and VRR, for
 │                      per-game display settings (DisplayConfig;
 │                      kscreen-doctor, Mutter, wlr-randr, hyprctl),
 │                      read-only so far                             → platform
 ├─ heronite-metadata   registry of metadata sources (IGDB, Steam,
 │                      Lutris, LaunchBox, HLTB, ...), the merge,  
 │                      the review of unsure matches, artwork        → data, images
 ├─ heronite-metadata-ext metadata sources as extensions: manifest,
 │                      package (folder / zip), installed ones, the
 │                      QuickJS sandbox that runs their scripts      → data, metadata, images
 ├─ heronite-achievements achievements of Steam (the client's files),
 │                      GOG, Epic and RetroAchievements (ROM hashes
 │                      by the vendored rcheevos), and the local copy
 │                      that keeps them                → data, metadata, gog, epic, steam, launch
 ├─ heronite-media      screenshots: listing, thumbnails, capture,
 │                      lossless PNG → WebP compression, posters of
 │                      replays (ffmpeg)                             → data, images
 ├─ heronite-overlay    in-game overlay backend: controller input
 │                      (evdev, Guide button, grabs; XInput with
 │                      Back + Start on Windows), audio (pactl; Core
 │                      Audio on Windows), controller battery /
 │                      Bluetooth, replay buffer
 │                      (gpu-screen-recorder)                        → data, platform
 ├─ heronite-wine       Wine/Proton manager                          → data
 ├─ heronite-storage    disk space of games, prefixes, Proton builds,
 │                      Steam leftovers, caches; cleaning them up    → data, launch, wine
 ├─ heronite-shortcuts  .desktop files and Steam shortcuts.vdf,
 │                      icons of Windows programs                    → data
 ├─ heronite-steam      settings of Steam games (launch options,
 │                      Proton) via the client or its files         → data, launch, shortcuts
 ├─ heronite-sunshine   the games in Sunshine's application
 │                      list through its web API, for
 │                      Moonlight; only entries marked as Heronite's  → data, platform
 └─ heronite-update     app updates: release check, signed download,
                        AppImage swap / package install, relaunch   → data
epic-login (bin)        WebKit window for the Epic, GOG (`--store gog`) and Amazon
                        (`--store amazon --url ..`) logins (keeps a web engine out of the app)
```

Library crates never depend on GPUI. Their functions are blocking; the app
calls them from `cx.background_spawn` or worker threads.

The Collection's adaptive quick toolbar (`app/src/collection/toolbar`) measures its spare header lane and shapes
localized labels, then exposes a priority prefix of the existing View menu actions, hiding the filter chips those
controls already show. Its on/off preference (off by default) belongs to `appearance.json`, not `UiState` or saved view filters; applying an appearance profile also keeps this window choice.

`vendor/gpui-pre-linux` is GPUI's Linux platform, patched to release the GPU
device when the last window closes (the app lives on in the tray); see its
`PATCHES.md`. `vendor/gpui-pre-windows` is GPUI's Windows platform, patched so
a window the user closes is torn down while its handle is valid (upstream
leaked it and logged errors); see its `PATCHES.md`. Both open every window
through `vendor/gpui-zoom` (Heronite's own crate): the interface scale of
Settings → Appearance → Zoom, on top of the screen's. GPUI sees a screen that
many times denser and smaller, so the whole interface (all in `px`) resizes and
stays sharp (not the in-game overlay: pop-ups and layer-shell surfaces keep the
system's scale); `app/src/system/ui_scale.rs` picks the scale (the one chosen, else
`GDK_DPI_SCALE`, `QT_SCALE_FACTOR`, GNOME's text size on Wayland or Windows'
text size) and applies it.

- **heronite-data:** the base. `paths` (`DataPaths`, `AppPaths`), `migrate`,
  `json_store` (electron-store read-modify-write: tab indent, atomic write,
  unknown keys kept), `temps` (atomic writes and the startup `.tmp` sweep),
  `proc` (`guard` and `Run`: helper processes die with the app), and one
  module per data file: `library`, `collection`, `sideload`, `game_config`,
  `settings` (and `source_prefs`, the per-source part of its metadata),
  `status`, `status_history` (when each game's status changed, in
  `local_library/status-history.json`, recorded by the status setters), `sessions`, `playtime`, `ui_state`, `appearance`,
  `stats` (the Statistics page's numbers: periods cut into hours, the history
  indexed by time and by game so a period reads only its slice, `game` for a
  game's sheet, `journeys`, `backlog` (goal pace, weekly hours, running counts),
  `insights` (discoveries and milestones), `album` (moments, captures per session, capture hours) and `prefs` (its keys in `ui-state.json`)),
  `journal` (each game's notes in `local_library/journal.json`: the moment
  they are about, placed on the timeline's days by the screenshots' rule in
  `timeline`, an optional screenshot or replay, and "where I left off"),
  `console_theme` (console mode themes: the shareable JSON format with every option, read leniently so themes of
  other versions load; built-in `presets`; `store` for saved themes, export and import; `ConsolePrefs` in
  `console.json`), `console_art` (where console mode finds a game's logo),
  `cover_family` (covers per shape: `CoverFamily` Tall / Square / Wide, the
  family of each Cover aspect and console tile shape, and `CoverFamilies`:
  which families are in use (the desktop's aspect, the console theme's tiles;
  the tall one always) and which the user keeps besides, `keepCoverFamilies`;
  a record keeps the square and wide covers in `coverSquareUrl` /
  `coverWideUrl`, sourced as `coverSquare` / `coverWide`, `art.json` the same
  keys), `art` (each view's picture chain; `family_cover` / `pick_family`:
  a family's own art, else for Wide the background, else the tall cover
  cropped, with where it came from),
  `clients` (the store clients, under Wine on Linux and as they are on Windows: their ids, settings file, prefix and program, where they install on Windows, and
  `local_library/clients.json`: when each was synced and the games the user took out),
  `replay_prefs` (Settings → Replay, `replay.json` in the app state: the
  switch, the options every game uses and the games that have it on or off on their own),
  `language`, `install_size` (size on disk of a game its store gives none for: ROM files, a bounded and cached folder walk), `field_names` (the library fields manager: renames, merges and
  removals of genres, companies, series, themes, features and platforms, kept
  as rules in `local_library/names.json` that the metadata service and the
  library load apply too, plus the duplicate suggestions), `steam_owned` (the cached Steam library and its twins: a game
  owned on Steam that the Collection has from another source joins that
  entry instead of getting a Steam entry of its own), `smart_groups` (groups of games defined by rules, in
  `local_library/groups.json`: `all` / `any` / `not` over conditions on what the Collection knows of a game
  and how it is played (sessions: times played, average session, hours in the last 30 days, progress against
  HowLongToBeat),
  evaluated against the game and `Facts` (the `Organize` data, HowLongToBeat, the updates); rules and keys a newer
  version wrote are kept as read, and a condition on a value the game lacks never matches).
- **heronite-i18n:** English text is the key (`tr!("Play")`); Portuguese
  lives in `locales/pt-BR/*.json`, Spanish (neutral, `tú`) in
  `locales/es/*.json` and German (`du`) in `locales/de/*.json`, embedded at build time (one folder per language) and
  loaded the first time it is needed. The language follows the system unless one is
  picked in Settings → General (`language` in `appearance.json`); switching
  redraws the windows. `tests/coverage.rs` scans the workspace: every key
  needs a translation in every language and every translation a key.
- **heronite-launch:** `plan` turns a game plus settings into a `LaunchPlan`
  and does no I/O beyond reads. `runtime::Launcher` runs the plan through a
  `Spawner`, tracks the process group and emits `LaunchEvent`s. `record`
  writes sessions and playtime (with checkpoints while the game runs).
  `log_writer` writes the launcher and game logs. `diagnose` tells why a
  game closed right after starting, from its launch log (Proton games run
  with `WINEDEBUG=-all,err+module`, so the DLLs the loader cannot find are
  in it), the DLLs its executable imports (`diagnose::pe`), its prefix's
  winetricks components, its AreWeAntiCheatYet entry and a few facts about
  the computer (`vm.max_map_count`, the distribution, the GPU): missing
  runtimes, DirectX 12 / 9-11 under plain Wine without VKD3D / DXVK,
  anti-cheats, Vulkan, esync, missing libraries, crashes; each finding
  carries its fixes. An `actionable` finding (the cause is shown and a fix
  solves it) opens the dialog by itself after a failed start; the others
  (crashes the log does not explain, likely causes, anti-cheats that refuse
  Linux) only show in "Troubleshoot". It is pure; the app applies the
  fixes. `prepare` does the work
  around a launch (cloud saves, runtimes); `first_launch` sets up a game the
  first time it runs in a Wine prefix. `lsfg` finds lsfg-vk (Lossless
  Scaling frame generation), the Lossless Scaling DLL and lsfg-vk's profiles,
  and adds the frame generation variables to a launch. `reshade` installs
  ReShade into a Windows game's folder like ReShade's installer (the
  executable, its API and the DLL name from its PE headers and
  `Compatibility.ini`; the DLL, `d3dcompiler_47` and shader packages
  downloaded into `<data>/tools/reshade`), keeps what it wrote in
  `<data>/reshade/<game>.json` so switching, updating, removing and adopting a
  manual install only touch Heronite's files, and adds `<dll>=n,b` to
  `WINEDLLOVERRIDES` at launch while it is on (never failing the launch; a
  Steam game under Proton has it in its Steam launch options instead, and
  `reshade::steam` finds a Steam game's folder and tells its Windows build
  from its Linux one, which gets no ReShade). It
  warns about anti-cheats and refuses games ReShade's list bans. `gfxmods`
  holds what graphics mods share: the `GraphicsMod` trait and its list
  (ReShade is the first), the journal that undoes a failed write in a game's
  folder, `WINEDLLOVERRIDES` editing, the anti-cheat check, downloads and
  hashes; `gfxmods::apply` adds every mod that is on to a launch.
  `gfxmods::catalog` reads Heronite's catalog of mods per game
  (`data/graphics-mods.json`, fetched from its raw address at most daily,
  the copy built into the app as fallback, built by
  `scripts/graphics-mods-catalog.py` and changed only through reviewed pull
  requests) and matches a game by Steam app id, executable, title or engine.
  `gfxmods::renodx` puts RenoDX's HDR add-on beside a ReShade that is the
  build with add-on support (`reshade::manage::set_addon`; both are refused
  for games with anti-cheat). Heronite only places files and sets variables
  for them; it never injects into a game. `fps_limit` holds a
  game's one FPS limit and picks the one limiter that applies it (MangoHud,
  DXVK / VKD3D-Proton or gamescope; Heroic's separate limiters are read as
  a fallback). `live_fps` makes that limit changeable while the game runs when MangoHud or gamescope applies it
  (MangoHud reads it from a config file of ours in `$XDG_RUNTIME_DIR/heronite/mangohud`, rewritten by the in-game
  overlay; gamescope's is its `GAMESCOPE_FPS_LIMIT` atom, set by `heronite_overlay::gamescope`). `gpu` holds a game's graphics card settings (the card that draws it,
  NVIDIA Smooth Motion and Reflex, the DLSS / FSR 4 / XeSS upgrades of GE-Proton and
  Proton-CachyOS, Mesa's Anti-Lag and glthread, vsync and what DXVK tells DirectX games)
  as variables, and `gpu::detect` finds the cards (sysfs, `vulkaninfo`, `pci.ids`) and the
  driver layers they need. On Windows `gpu::preference` writes Windows' per-program GPU
  preference for the program a launch starts and puts it back after the game (what it wrote
  is kept in `local_library/gpu-preferences.json`; the registry is behind a trait), and
  `gpu::detect` asks DXGI (`heronite_platform::gpu_adapters`) which card each choice gets. `actions` turns a game's own actions (`heronite_data::game_actions`,
  `local_library/actions.json`) into something to open, spawn or launch as
  the game's session. `galaxy` stands in for GOG Galaxy's communication service, which the GOG
  Galaxy SDK in a game talks to on `127.0.0.1:9977` for achievements, stats and leaderboards: a GOG plan of a
  Windows build sets `PrepSpec::galaxy` (not in a sandbox without the network), and the runtime starts Comet
  (`store_tools::COMET`, downloaded into `tools/bin` the first time, never taken from `PATH`) right before the
  game with the GOG account the app gives (`GalaxyAccount`, unless Settings → Achievements turns it off; the
  tokens never reach a log), unless the port is taken (GOG Galaxy, another launcher's Comet). GOG games that
  run together share it (`runtime/galaxy.rs`: a thread of its own owns it, so on Linux it gets SIGINT if the
  app dies; its output goes to `logs/runners/comet.log`). Once the last of them is gone it gets Ctrl+C
  (`heronite_platform::process::interrupt`), so it uploads what is left, and is ended after 15 s. Nothing of it
  fails or holds up a launch for long: the account (30 s, gogdl ended past it), Comet's first download (60 s,
  then it finishes in the background for the next launch), each Wine command of the prefix setup (45 s) and
  the start (10 s) have limits, past which the game runs without Comet. A Wine prefix gets Comet's dummy `GalaxyCommunication` service the first time (copied to
  `ProgramData`, `sc create`, GOG Galaxy's registry path), as Heroic does; on Windows registering it needs the
  administrator, which Settings → Achievements asks for (it runs as LocalService, from a folder only
  administrators may change). `umu_ids` picks the
  `GAMEID` of a umu launch (protonfixes) from the user's id, the umu
  database (downloaded weekly), Heroic's cache, the metadata's Steam id or
  the title; `keep_awake` adds the logind lock the runtime holds while the
  game runs. `sandbox` (`heroniteSandbox` in the game's settings) runs
  everything of a game, and everything run in its prefix, through bubblewrap:
  `mounts` says what the sandbox shows (the prefix read-write, the game's
  folder read-only, the Wine build and `tools/` read-only, Winetricks'
  downloads under a throwaway overlay), `script` builds the
  `sh -c` program that creates the folders, looks up what depends on the
  session (Wayland / X11 / PipeWire sockets, the controllers plugged in,
  NVIDIA's devices) and starts `bwrap` with its own user, pid, UTS and
  network namespaces and a private home (`<data>/sandbox/<prefix or game>`,
  `DataPaths::sandbox_home_of_*`). The program goes after the wrappers that
  stay outside (gamescope, GameMode, MangoHud) and before umu / Proton / the
  game, so legendary gets it inside `--wrapper` too. Inside, the command runs
  under a small `sh` that waits for every process of the sandbox (its pid
  namespace ends with its first process). The pid namespace is what keeps the
  sandbox closed: without it `/proc/<pid>/root` of any process of the user
  shows the real files. Each `Exception` lets one thing through (network,
  writing the game's folder, the shared folders, every device). The plan
  builders, the Wine tools (`wine_tools`), the first-launch setup, game
  actions and script installs all go through it; a plan that cannot be
  sandboxed is `Unavailable`, never run outside (`build_plan_with_settings`
  checks every plan, so a new way to start a game that forgets the sandbox
  does not start). A game in the sandbox has no
  stop commands nor `wineserver --wait` (both would start a sandbox of their
  own): `runtime::sandboxed` follows the sandbox until it is gone and kills
  its first process when a stop runs out of patience. Programs alongside the
  game and Wine tools started while it plays join its sandbox with `nsenter`
  (`sandbox::join`, `into_running`), sharing its wineserver. umu does not
  run in the sandbox (`sandbox::without_umu`): its container builds a copy
  of its Steam runtime on every start, which a throwaway layer would hold
  in memory (about 700 MiB) and a writable one would let a game change for
  every game outside. Proton runs directly, with the Steam client's
  libraries (`~/.steam/sdk32`, `sdk64`) shown read-only for its
  `steam.exe`, and Linux games run without the Steam runtime. DXVK's and
  VKD3D-Proton's shader caches go in the private home (the game's folder is
  read-only). Host writes into a sandboxed prefix (DXVK's DLLs, markers)
  never follow links the sandbox could have planted. bubblewrap must create
  a user namespace (`--unshare-user`, checked once it works and then kept):
  the running sandbox is found by it, for at most `sandbox::FOUND_WITHIN`.
- **heronite-epic:** `cmd` runs legendary. `manager` + `queue` are the
  download queue (`store/download-manager.json`), shared by every store: an
  element's `params.runner` picks the job (legendary, gogdl or nile), and
  `progress` parses the reports of all three tools. `ops` covers install,
  repair, uninstall and import. `moves` is the resumable install move with
  its journal. `eos_overlay` manages the EOS overlay.
- **heronite-gog / heronite-amazon:** the same shape for GOG and Amazon
  Games. The tools (`gogdl`, `nile`) are found or downloaded into
  `<data>/tools/bin` by `heronite_launch::store_tools`. GOG's library comes
  from GOG's web APIs (token from gogdl's `gog_store/auth.json`) into
  `store_cache/gog_library.json`, installs are listed in
  `gog_store/installed.json`; Amazon's library and installs are nile's own
  files (`nile_config/nile/`), mirrored into `store_cache/nile_library.json`.
  Both libraries are read by the Collection like Epic's. Launching is in
  `heronite-launch` (`gog.rs`: the primary task of `goggame-<id>.info`, or
  `start.sh` for Linux builds; `nile.rs`: `fuel.json`), always through the
  Wine/Proton plan of local games (`galaxy.rs`: Comet, GOG Galaxy's service
  for the games' achievements, runs beside a GOG game's Windows build); the first launch in a prefix runs the
  store's installers (`first_launch::StoreSetup`, Amazon's `PostInstall`).

- **heronite-import:** the other launchers: Linux's, and Heroic and Playnite
  on Windows (`Launcher::here`). `detect` finds Heroic,
  Lutris, Bottles and Faugus (native or Flatpak; Heroic in `%APPDATA%` on
  Windows) and reads their libraries,
  read only: Lutris' `pga.db` (SQLite, compiled in through rusqlite's
  `bundled`) and the Wine prefix from each game's YAML, each bottle's
  `bottle.yml` (its programs; the bottle is their prefix) and Bottles'
  `library.yml` (covers), Faugus' `games.json`, Heroic's sideload library and
  install records. On Windows `playnite` finds Playnite through the program
  its `playnite://` link opens (portable or installed) and reads its
  `games.db` and `emulators.db` with `litedb`, a reader of LiteDB 4 files;
  while Playnite runs it holds them open (`Found::in_use`) and nothing is
  synced. Its games of the stores this app brings in itself are left out.
  `collection` writes the installed games of Lutris,
  Bottles, Faugus and Playnite as sideload entries whose `launchKind` is
  `launcher` (`heronite_data::launchers`): `launcherArgs` holds the command
  that asks the launcher to start them, the launcher's play time moves up
  when it is ahead, and a game gone from the launcher stays as not
  installed. `heroic` copies the games added by hand in Heroic (they start
  here) and adds its Epic, GOG and Amazon install records to this app's;
  each of those games gets Heroic's `GamesConfig` (its Wine prefix) while
  it has none here. Which launchers are imported lives in
  `local_library/launcher-imports.json`; the app's `services::launcher_imports`
  keeps them in sync. A `launcher` game's plan is a native process with a
  `Handoff`: when the launcher returns at once (Lutris already open passes
  the request to its window; Playnite's link goes through `explorer.exe`,
  whose exit code says nothing), the runtime follows the game's processes
  (`Procs::game_pids`: files under its folder, or under the folder its
  program runs from, a Playnite game's emulator; Windows programs in its
  prefix) until they are gone, and a stop ends them.
  `duplicates` pairs the games then in the Collection twice: added here by
  hand (pointing into the launcher's prefix or folder, or with the same
  name) and brought in from Lutris, Bottles, Faugus or Playnite. Settings → Other
  launchers merges a pair (`heronite_data::carry_over` moves play time, sessions,
  status and its history, notes, journal, rating, favourites, categories and
  custom fields onto the launcher's entry, then the other leaves the
  Collection), keeps only the launcher's, or keeps both
  (`keptApart` in `launcher-imports.json`). The folders and prefixes of the
  `launcher` games (`launchers::managed_paths`) are theirs: an uninstall or
  prefix removal of a game added here, Wine Manager's trash and the Storage
  page (in use, never a leftover) all leave them alone. The same goes for a
  store client's prefix and its games' folders (`managed_paths` returns
  them too, named after the client).

- **heronite-clients:** the Windows store clients: Battle.net, the EA app,
  Ubisoft Connect and the Rockstar Games Launcher
  (`heronite_data::clients::Client`), under Wine on Linux and as they are on
  Windows. `place::Place` is where a client lives (`Prefix` on Linux,
  `Machine` on Windows: this PC's drive, the user's folders wherever Windows
  keeps them, the real `HKLM`); the readers ask it for files and keys and
  never look at which one it is. On Windows `native` sets a client up with
  its official installer (elevated when its manifest asks), uses one already
  installed as it is, opens it detached, closes the programs running from
  its folder and tells whether it or a game runs, by name and by folder.
  On Linux each client lives in a Wine prefix of
  its own (`<defaultWinePrefixDir>/<client>`), set with its Wine build in
  `GamesConfig/heronite-client-<id>.json`, so the game settings panel edits
  it like a game's. `setup::install` downloads the official installer
  (`tools/redist/clients/`; the EA app's and Ubisoft Connect's are the ones
  Epic's EA / Ubisoft games use, in `tools/redist/legendary/`), creates the
  prefix, writes what makes the client behave under Wine (Battle.net
  without hardware acceleration, Ubisoft Connect without its overlay) and
  runs the installer silently; an installer often starts the client and umu
  keeps its command alive, so the setup ends once the client's program is
  there and no installer runs. `adopt` uses a client already installed in
  another prefix, `forget` drops its settings (never the prefix). The user
  signs in and installs games inside the client; Heronite reads, without
  Wine, what it installed and whether it is signed in: `reg` (Wine's
  `system.reg` / `user.reg`), `winpath` (`C:\` paths through `dosdevices`,
  ignoring case), and one module per client (`battlenet`: the Agent's
  `product.db` protobuf and `Battle.net.config`; `ea`: `installerdata.xml`
  of each game; `ubisoft`: its `Installs` keys and
  `ConnectSecureStorage.dat`; `rockstar`: its uninstall and `Rockstar
  Games` keys and the Social Club profiles). `collection::sync` writes the
  games as `sideload` entries of `launchKind` `client` (`source` the
  client's id, `launcherArgs` what the client is asked: `--exec="launch
  WoW"`, `origin2://game/launch?...`, `uplay://launch/<id>/0`,
  `-launchTitleInFolder "<folder>"`); the ones the user removed stay out
  (`local_library/clients.json`). `run` opens the client outside the app's
  process group (it stays open when Heronite quits) and closes it
  (`wineserver -k`). Their games start in `heronite-launch` (`clients`): the
  client's program with `launcherArgs`, in its prefix and with its
  settings, Proton's `run` verb and umu's container open to later requests
  (a request to a client already open joins it, `runinprefix`); the session
  is run by `runtime::client`, which follows the game (a program of the
  prefix running from the game's folder that is not one of the client's,
  `Procs::prefix_programs`), starts the clock when it shows up, never stops
  the client and gives up after ten minutes or when the client closed. On
  Windows the plan runs the client's program as it is with `launcherArgs`
  (a client already open takes the same command as a request) and
  `Procs::prefix_programs` lists every program of the PC, the game being the
  ones running from its folder.

- **heronite-steam:** Steam games keep launching through Steam
  (`steam://rungameid`), with Steam's Proton and prefix. Their game settings
  tabs edit what Steam stores per game instead: the compatibility tool and the
  launch options line. `options` maps the line to the panel's settings keys
  and back (lines with shell syntax are only edited as text). While Steam
  runs, changes go through its CEF debugging interface (`client`, via
  `heronite_shortcuts::steam_client`, whose port `debugger_addr` finds from
  what the `steamwebhelper` processes listen on (`heronite_platform::net`),
  their command line and Chromium's `DevToolsActivePort`).
  Steam opens that interface only if its flag file is there when it starts,
  so `prepare_access` writes it into every Steam install found (native,
  Flatpak, Snap, and the one running) as soon as Heronite sees Steam: a Steam
  opened after Heronite never needs a restart. `steamwebhelper`'s command
  line tells a Steam still starting (`LibraryAccess::Starting`) from one that
  started without the flag (`NeedsRestart`); when another program holds the
  port (Steam on Windows asks for 8080; a torrent client's web interface may
  have it), the answer is not a Chromium debugger and the socket table names
  that program (`PortTaken`), since a restart alone does not help then;
  with Steam closed, `files` edits `localconfig.vdf` and `config.vdf` in
  place (`vdf_edit` touches only the changed entry). `wrap` writes the script
  that runs the before/after scripts from the launch options. `library`
  reads the account's library from the client (Heronite never signs in to
  Steam: the QR sign-in of 1.5.0 and 1.6.0, a session of the mobile app's kind
  made from a PC, got accounts locked by Steam's security; `account` only
  ends a sign-in those versions saved in `local_library/steam-account.json`,
  at start, with `RevokeToken`, so it leaves the account's authorized
  devices, and deletes the file even when Steam cannot be reached) (games only: no tools,
  soundtracks, demos or test servers), caches it and adds the games the
  Collection does not have yet (a game hidden in Steam comes in hidden, read
  from the client or, for the account, from Steam's
  `userdata/<id>/config/cloudstorage` files); Steam's play time moves
  the Steam entries up when it is ahead. A Steam entry still "Not played"
  (the status the scan of the installed games gave it before the library was
  read; the scan now starts a game as "Played" when the cached library has
  play time for it) becomes "Played" once Steam has play time for it, unless
  the user ever chose its status (the status history has it): the move is an
  inferred default like the status at creation, so it is not recorded there.
  The rule is `heronite_data::status::promote_played`, which the Epic and GOG
  play time sync and the recording of a session use too, so any game with
  play time leaves "Not played".
- **heronite-xbox:** Windows only (`Feature::XboxStore`). The Microsoft
  Store and PC Game Pass games are Windows packages: `heronite_platform::packages`
  reads the user's packages from the registry the deployment service keeps
  (`AppModel\Repository\Packages` of `HKCU`: folder, display name, one key
  per application) and resolves `ms-resource:` names with
  `SHLoadIndirectString`. A package is a game when its folder has
  `MicrosoftGame.config` (GDK games, installed by the Xbox app under
  `XboxGames`) or `xboxservices.config` (Store games that sign in to Xbox);
  `files` reads those and `AppxManifest.xml` for the title and the
  application to start. `collection` keeps them as sideload entries of
  `launchKind` `xbox` (`heronite_data::xbox`; `launcherArgs` is the AUMID,
  `local_library/xbox.json` the games the user removed). `heronite_launch::xbox`
  starts them with `explorer.exe shell:AppsFolder\<AUMID>`, which returns at
  once with exit code 1 (`Handoff::exit_ignored`), and the session follows
  the programs running from the package's folder. Owned and Game Pass games
  are the same packages: only Microsoft's services know the licence, so
  both show as Xbox. `services::xbox_library` reads them after the start,
  every ten minutes and on request (the Xbox page, the refresh button).
- **heronite-installer:** installs a game from a YAML (or JSON) install
  script. `script` reads the file (one script, a list, or an API dump;
  repeated keys keep the last value), `steps` types the `installer` list,
  `vars` substitutes `$GAMEDIR`, `${VERSION}`, file ids and answers the way
  the scripts expect (unknown `$names` stay for the shell), `check` says
  before anything runs what blocks it (unsupported runner or step, missing
  archiver or emulator). `engine::install` fetches the `files` (downloads to
  `~/.cache/heronite/install-scripts/<slug>`, files the user picks, local
  paths), runs the steps (`archive` through the system archivers, `fsops`,
  `tasks`: Wine tasks through `heronite_launch::wine_tools`, each waiting for
  the prefix's wineserver) and `finish` adds a local game: `wine`, `linux`,
  `dosbox`, `scummvm`, `libretro` and `web` runners, `system.env` and DLL
  overrides as environment variables, `launch_configs` as the game's actions.
- **heronite-overlay:** the GPUI-free backend of the in-game overlay. On Windows,
  `input::windows` polls documented XInput through `heronite-platform::gamepad`,
  feeding the shared `input::core` state machine: Back+Start substitutes for Guide,
  with no exclusive grab (games can still see the buttons; the open panel takes
  the foreground, which most games respect). A controller is
  named by `input::ControllerId`: its XInput slot there, its evdev node on
  Linux. Controller battery is XInput's coarse estimate; Bluetooth disconnect
  is not offered.
  `audio::windows` adapts Core Audio (`heronite-platform::audio`): endpoint
  volume/mute, microphone mute and active per-app/game sessions, identified by
  stable session IDs rather than enumeration order. The default output and
  microphone change through the undocumented IPolicyConfig
  (`heronite-platform::audio::set_default_device`, every role). On Linux, `input` reads
  the controllers from `/dev/input/event*` on its own thread (only while a
  game launched by the app runs and the overlay is on) and grabs them
  (`EVIOCGRAB`) while Guide is held or the panel is open, so the game gets
  none of those presses; `input::core` is the pure state machine (Guide tap
  = open / close, Guide + RB (or the button chosen) = screenshot,
  Guide + ↑ / ↓ = volume, Guide + Y = a moment for the journal, Guide + X
  = the replay saved, held directions repeat). `audio` drives the default output, the microphone,
  the game's own streams and the other apps playing through `pactl`; `controller` reads the battery from
  sysfs and disconnects Bluetooth controllers. `replay` runs gpu-screen-recorder (6.0 or newer, the
  system's or the Flatpak) as a replay buffer: `info` reads its `--info`, `plan` builds the command
  (the monitor captured through KMS, the card's encoder only, NVENC / VA-API or Vulkan video, a
  constant bitrate, the buffer in RAM or on disk) and the memory estimate Settings shows (measured on
  an RTX 5070), `recorder` runs it on a thread of its own (a socket of its own each time, restarted
  when it dies, Vulkan that cannot start falls back to the usual encoder, ended with SIGTERM: a replay
  buffer saves nothing on exit), `ipc` saves through its `-ipc` socket, `target` finds the game's
  monitor (Hyprland) and `trim` cuts the panel's time off the end of a save (ffmpeg stream copy, idle
  priority).
  `Feature::Overlay` supports Linux and Windows; `Feature::Replay` is Linux-only.
  The replay service and every replay control use the latter independently,
  including when preferences were copied from Linux.
- **heronite-storage:** Settings → Storage. `inventory` lists the installed games (store size, Steam's
  `SizeOnDisk`, ROM files, else a walk), the Wine prefixes (in use while an installed game or the defaults
  use them; one inside a game's folder goes with the game), the Wine Manager builds (in use when the default,
  set on an installed game or picked in Steam's `config.vdf`) and the runtimes, Steam's `compatdata` /
  `shadercache` of apps without a manifest, the caches that rebuild themselves and the folders of the games
  folders nothing known points to (installed games, prefixes, ROM folders, emulators, the download queue, any
  path of the settings; nothing changed in the last two hours). `measure` walks one item (`du`-style blocks);
  `disks` maps paths to devices (`mountinfo`, `statvfs`); `remove` re-checks the path and trashes what may
  hold saves, deletes the rest.
- **heronite-backup:** Settings → Backups. `catalog` says what each file of the data folder and the app
  state is: a `Part` a backup holds (Collection, app settings, artwork, store sign-ins) or something left out
  because it is downloaded or rebuilt again (Wine builds, image caches, store caches, logs, temp files);
  unknown files are in. `snapshot` writes `heronite-YYYY-MM-DD_HHMMSS/{data,config,manifest.json}` under a
  `.partial` name, hard-linking the files unchanged since the previous backup (copies where links fail), with
  the folder `0700`; `prune` keeps the newest N (a folder without a manifest is not a backup: never listed,
  restored or removed). `run` is the schedule and "Back up now" (status in
  `settings.json` `backup`). `restore::prepare` checks every file against the manifest's SHA-256, backs up
  the current data and leaves `restore-pending.json` in the app state; `main` applies it first thing at the
  next start (`apply_pending`), replacing each chosen part whole and rewriting paths when the backup came
  from other folders. The app's `services::upkeep` runs the schedule (90 s after start, then hourly, never
  during a game) and the image cache cap (`imageCacheLimitMb`, LRU over `images-cache` and the thumbs).
- **heronite-sync:** Settings → Cloud, the same Collection on several PCs (a dual boot's Linux and
  Windows, other computers), whatever stores each is signed in to. Every PC writes only its own
  file, `devices/<id>/state.json` (`format`: everything it has to say, never a list of changes),
  where all of them reach it (`transport`: a folder, or a cloud account or server through
  `rclone`: Google Drive, Dropbox, OneDrive, Box, FTP, SMB, WebDAV or a remote of the user's own
  settings; the binary is found or downloaded into `tools/bin`, checked against the release's
  SHA-256 list, and its settings with the account's token are this app's `rclone.conf`, 0600, a
  backup's sign-ins). No two PCs write one file, so nothing a sync service copies can conflict.
  `merge` is pure: status, favourite, hidden, rating, notes and each custom category a game is in
  (`category:<name>`, game by game, so two PCs adding a game to two categories keep both) are last written wins by a stamp
  (a logical clock past every clock read from the others, then the time, so a dual boot's wrong
  hardware clock does no harm; a value seen for the first time is weak, so a new PC's defaults,
  like the `not-played` of a Steam import, never overwrite another's); sessions are a union (a PC
  publishes only its own; one taken in and deleted here does not come back); play time is the
  largest total (the stores' own time is in every PC's, so adding would count it twice), except
  the newest total made smaller by hand. Smart groups are last written wins group by group (a
  deletion is a stamped `null`, so another PC's copy does not bring it back), without `pinned`,
  which each PC keeps; which games a group holds is never synced, each PC works it out from its own
  Collection. A game another PC has and this one does not waits in that
  PC's file. `ledger` (`local_library/sync-ledger.json`) remembers what was published and taken
  in: the data files have no time of change per field, so a value that differs from it was
  changed here. `local` reads the Collection and writes through `heronite_data`, after copying the
  files it changes into `local_library/sync-before`. `prefs` is `sync.json` in the app state (the
  place, what goes there, this PC's id and name, the last results and the other PCs seen). A PC set
  to send only its play (`playOnly`, for a PC the user does not keep organised) publishes sessions
  and play time and none of its fields, and its own changes lose to any other PC's. `cloud` is what
  else the same sign-in takes, under `<root>/Backups/<PC>`, `Saves/<PC>` and `Screenshots` (the sync
  is `Sync/`): each backup as one zip of its Collection and app settings with a manifest of only
  them (sign-ins never leave the PC; the artwork goes next to it as files, sent once), Ludusavi's
  backup folder mirrored (it is a zip per game already; an empty folder never empties the copy),
  and the screenshots one file each with only the new ones sent (a WebP gains nothing in a zip),
  never a PNG waiting for its compression, replays only when asked. `list_backups` lists every PC's
  backups in the account (one listing, when the user asks) and `download_backup` brings one into the
  backup folder, where Settings → Backups restores it. `sent` (`cloud-sent.json` in the app state)
  is what each upload set out to send, recorded only once it went through, so Settings → Cloud tells
  all sent / something new / nothing sent without asking the account, and an upload stopped by
  closing the app shows as not sent and goes again at the next start. Every rclone call is throttled
  (`--tpslimit`, few transfers, no folder dates) and retried; an account still refusing requests
  (Google Drive's quota a minute, shared by every user of rclone's own Google app) gives
  `SyncError::RateLimited`, and the app's queue tries again later. Google Drive can sign in with a
  Google app of the user's own, or one the build carries (`HERONITE_GOOGLE_CLIENT_ID` and
  `HERONITE_GOOGLE_CLIENT_SECRET` at build time), whose quota is not shared. Each of
  the four is `Off`, `Manual` (only "Send now" / "Sync now") or `Auto` (`prefs::Mode`; every run says
  whether the user asked for it). `usage` measures, on this PC only (the account is never asked), what
  each kind would take in the cloud by game or part, by the uploads' own rules, and the
  screenshots' monthly growth, for the space bar and treemap of Settings → Cloud.
- **heronite-metadata:** the Collection's details and pictures. `sources` is
  the registry of sources: each has a `SourceDescriptor` (plain data, the
  shape an extension's manifest will carry: id, name, kind details /
  pictures / extras, the fields it provides, the games it applies to
  (`AppliesTo`), how it finds a game, what it needs, whether it uses the
  network, hidden or offered). Details sources implement `MetadataSource`
  (`lookup` of a `SourceGame` with a `SourceContext`: paths, HTTP,
  settings); thin adapters in `sources/builtin` wrap the provider modules
  (`igdb`, `steam`, `providers` for the store and Playnite, `lutris`),
  in the order candidates are merged. `launchbox` is the LaunchBox Games Database: no API, the
  whole database is one archive (`Metadata.zip`, ~105 MB, daily) that the
  user downloads from the source's row; `fetch` downloads it into the
  data folder's cache area (`store_cache/sources/launchbox/`, out of
  backups and sync; resumed with `Range` / `If-Range`, refreshed only when
  its ETag changed), `build` streams `Metadata.xml` and `Files.xml` out of
  the zip (`parse`, quick-xml, a record at a time) into `index` (SQLite:
  games, names and alternate names as `rom_names::match_key` keys by
  platform, LaunchBox's file → title table, the pictures used with packed
  file names, the overviews deflated 64 games a block; ~80 MB for all
  190,000 games, built in ~8 s with ~50 MB of memory, a lookup ~0.15 ms),
  built as `index.sqlite.part` and renamed over the old one only once it
  opens with games, so a failed or cancelled refresh keeps the old one; a
  failed refresh is recorded in `state.json` and the idle maintenance waits
  a day, two, four, then a week before the next (`DownloadState::retry_at`),
  the archive is kept for it unless it does not read (`BadArchive`), and a
  record whose text does not read is left out and counted. An index that
  no longer opens is not ready (the source is not asked) and is downloaded
  again whole at the next idle pass.
  A lookup of an emulated game stays on its platform's LaunchBox
  platforms (`platforms`): the dump's file in LaunchBox's file table,
  then the dump name cleaned, then the title (exact and alone on all of
  the platform's LaunchBox platforms → `ExactTitle`; a longer name or a
  shorter one, or two games, the Famicom Disk System's beside the NES's
  among them → `FuzzyTitle`);
  a PC game (the "Also for PC games" option) is looked up on Windows / DOS,
  by its Steam id first; a search by hand spans every platform and shows
  which: the names that are the query, then those starting with it, then
  those holding it (shortest first), digits a name and an id too. `map` fills the record (box art by the "Box art region" option,
  automatic from the interface language; the fan art background; the
  square cover; the 0..5 rating as a score from 5 votes; game modes from
  the players). The database has no picture sizes: while this PC keeps the
  square or the wide family, the box art's size is read from its first bytes
  (`LaunchBox::picture_size`, `Request::send_head`: the image server ignores
  `Range`, so the answer is dropped once `picture_size` reads the PNG header
  or the JPEG frame header; remembered by file name) and the box is also the
  cover of the family its shape says (`picture_size::family_of`, libretro's
  rule): a landscape box the wide cover (LaunchBox is in its default order,
  ahead of SteamGridDB), a CD case the square one, ahead of LaunchBox's own
  square art; the tall cover is the box whatever its shape. The Covers dialog
  asks `service/launchbox_art` for the game's pictures (`map::choices`: every
  region's box, those of the slot's shape first, with sizes). `platforms` is the table of the app's platform ids (the
  emulator catalog's and Playnite's), how the Collection names them and
  how each database a source reads names them (a column per database:
  LaunchBox's and libretro's folders), plus `resolve`, an emulated game's platform from
  its entry (the `platformId`, else the ROM's extension, the emulator's
  name, the ROM's folders, the record's platforms) and `rom_file`, the
  file its dump is named by (the folder above a boot file such as
  `EBOOT.BIN`). `rom_names` cleans No-Intro / Redump / TOSEC names (tags,
  "Legend of Zelda, The", discs, safe roman numerals). `SourceGame`
  carries `emulated`, `platform`, `rom_file` and `rom_hashes` (empty: the
  app keeps no ROM hash yet; a lookup never hashes a file). `libretro` is
  libretro-thumbnails (`thumbnails.libretro.com/<System>/Named_Boxarts/<Game>.png`,
  the box art RetroArch uses: no API, key, account or search, 512 px wide at
  most): a pictures source for the emulated games of the 43 platforms that
  have a system folder (the `platforms` column; PlayStation 3 / 4, Vita,
  Xbox 360 have under 70 pictures, arcade is named by MAME's descriptions,
  DOS and ScummVM by titles, so they have none). `libretro::names` makes
  the file names: the dump's own (extension off, the characters RetroArch scrubs, `& * / : ` " < > ? \ |`, as `_`),
  then without the tags after region and languages, with the region alone,
  the title (article last, `: ` as ` - `: the file's own, or the game's when
  the file has no tags) with each region of the interface language, the title
  alone (RetroArch's own tries are the file's name, the playlist label and the
  title before the first ` (`). A try is one `Range: bytes=0-31` GET: 404 for a
  missing file (404 or 410, remembered for an hour so another family's pass
  does not ask again), 206 with the PNG's header for a picture, whose size picks the
  cover family (`family_of`: width/height under 0.85 tall, under 1.33 square,
  else wide: Genesis and NES boxes are tall, PlayStation, Game Boy Advance and
  Dreamcast square, Super Nintendo and Nintendo 64 landscape); the dump's own
  name is asked on each folder the ROM's extension may be of, the others on
  the first, and a game costs 8 requests at most (`MAX_REQUESTS`). The dump's
  own name is `ExactTitle` (only a ROM's: a title alone is a guess), any
  other `FuzzyTitle`; any other status is unreachable for the game (a 429's
  `Retry-After` of 5 s at most is waited out once), and requests go 100 ms
  apart (`Libretro`, the client the source keeps). It has no background (its screenshots are 240 to 512 px wide).
  It leads for emulated games like LaunchBox (after it in each order). A descriptor
  may say a source needs a download (`needs: download`, `download`:
  its size and refresh days); `sources/download` is the contract
  (`DownloadState`, `DownloadProgress` with cancel, the trait's
  `download_state` / `download` / `remove_download`), Settings draws it
  for any such source, `service/source_settings` runs it
  (`download_source`, `refresh_due_source_downloads` for the idle
  maintenance: weekly, only a database the user downloaded, only while the
  source is on), and `gather` never asks a source before its download
  (`NotAsked::NeedsDownload`). A descriptor may widen its audience by a
  switch option (`alsoFor`) and name an audience it leads for
  (`leadsFor`, see `sources/order`: for an emulated game LaunchBox found
  surely, it goes ahead of the sources that found the game only by its
  title, in each field whose order is still the default one). A descriptor may say a source is asked only as a
  fallback (`Asked::AsFallback`; no built-in source is, the VNDB and
  Wikipedia extensions are) and name the kind of game it is
  about (`GameKind`: a name for Settings and the genre / theme / Steam tag
  names that signal it). `service/gather` asks the sources: a fallback
  source goes with the others when the game is identified with it (a
  pinned or stored match), when the request names it
  (`SourceRegistry::names_fallback`: only fallback details sources asked),
  or when the record or the game's cached Steam tags carry its signal;
  otherwise it is asked after them, only when no online details source
  found the game for sure or one of their candidates carries the signal,
  and never when they were all unreachable (the fetch is offline). A search
  by hand asks it like any searchable source. Not asked, it says why
  (`NotAsked::Fallback`); a job's fillable fields leave it out unless the
  job names it. A candidate may flag pictures as adult
  (`MetadataCandidate::adult_pictures`, `SourceHit::thumb_adult`; an
  extension per picture, as the VNDB extension does from its users'
  votes, or for all its pictures with `adult = true`): unless
  `settings.metadata.allowAdultCovers` is on, the gather,
  `SourceRegistry::lookup_id` / `search` and `review_entry` drop them, so
  the next source fills the picture and none is shown or downloaded; saved
  pictures stay. A source added after `fieldPriority` lists were saved (or
  later added to more fields: LaunchBox to the wide cover, after libretro) is
  in `heronite_data::settings::ADDED_SOURCES`: the old default list, without
  any of them, still reads as the default, and a list the user set gets it at its end (a
  pictures source too: libretro-thumbnails is in `ADDED_SOURCES` and in the
  covers' default orders, not in `SOURCE_IDS`). The default lists of the
  builds where VNDB was built in name `vndb`, last of the sources of then:
  `heronite_data::settings::DEFAULT_LISTED_EXTENSIONS` keeps such a list
  reading as the default, and the VNDB extension, once installed, joins
  its end like any extension (a list the user set keeps `vndb` where it
  is, and it answers there). The fallback rule's tests add a fallback
  source of their own (`SourceRegistry::for_tests_with_fallback`,
  `tests/common`'s `fallback`). SteamGridDB, web images, HowLongToBeat,
  Steam tags and ProtonDB are listed as descriptors only and still run on
  their own paths (`covers`, `web_images`, `hltb`, `steam_tags`,
  `protondb`), but SteamGridDB also has a lookup: a pictures source asked
  only for the cover families it is in the order of (the square and wide
  covers by default, only while kept), whose unsure match never sends a
  game to review (`fetch::decide`). A cover family is a field of its own
  (`coverSquare`, `coverWide`: `heronite_data::settings::ALL_FIELDS`, own
  source, lock, order, file `cover-square.*` / `cover-wide.*` beside
  `cover.*`); a fetch fills the kept families only, and
  `BulkArgs::download_covers` / `service/families` (`cover_family_job`, what a
  view that starts using a family queues, once while one runs or waits)
  fill one for the games with details, or found in no source, lacking it
  (a game never fetched is left to the idle fill, `counts::waits_for_details`:
  a fetch of pictures only neither marks a game as fetched nor as failed,
  and only a details source's answer counts as finding a game); `family_lack` lists those games (and whether a source
  that gives the family can be asked now) whatever the automatic download
  says, for the notices and the "Download <family> covers" rows (a source
  counts only when it is for some game: a Collection without emulated games
  is not offered libretro's covers). `service` asks the sources that apply to a game in parallel,
  marks a fetch offline when every network source asked was unreachable
  (what a fetch writes in each case, found, unsure, in no source with its
  sure pictures, or not answered, is the rule of `service/fetch`'s docs,
  decided with the registry the gather used),
  and `merge` combines the candidates field by field
  (`settings.metadata.fieldPriority`, whose defaults stay in
  `heronite_data::settings`; a registry test checks the two agree). Each
  candidate says how it was found (`MatchedBy`: the game's own record, an
  id, the Steam id, an exact or a fuzzy title). Settings → Metadata and the
  per-game dialog list the offered details sources from the registry.
  What the user sets per source is stored by `heronite_data::source_prefs`
  (`settings.metadata.sources`: on or off, options, fields switched off;
  `sourceOrder`; the older switches `steamTags`, `howLongToBeat`, the IGDB
  keys, `steamTagLimit` and the global `steamGridDbApiKey` stay where they
  were). In `sources`, `options` (a descriptor's options as data, read and
  written where each is kept), `order` (each field's order: the user's
  `fieldPriority` for that field, else the default list until a source is
  moved, then the user's order of the sources that fill it; sources off and
  fields switched off left out) and `rows` (status and rows for Settings);
  `service/source_settings` has the entry points. A source that is off is
  not asked: `gather`, the idle fill, the age ratings fill, and the side paths
  (`covers::steamgrid_key_if_on`, web images, ProtonDB in the app).
  Updating several games is a bulk job: `job` says what one asks for
  (`BulkArgs` with an `UpdateMode` — fill what is empty, refresh what
  sources gave, replace everything — chosen fields and sources; presets
  such as `BulkArgs::fill_missing`) and what it reports per game
  (`GameResult`: outcome, fields changed and their source). In `service`,
  `bulk` runs one job at a time with a FIFO queue (Stop, clear the queue)
  and keeps the idle fills apart from jobs (a fill gives way to a job);
  `job_run` maps the mode onto the merge's options and runs each game
  (Steam tags in one pass, 50 games per request); `undo` puts back what the
  last job wrote, part by part, leaving what changed after it, and the
  `metadata-art` pictures it replaced (copies in `local_library/metadata-undo`)
  and the review queue entries it changed;
  `counts` reads, without network, what a set of games lacks.
  An unsure match is not saved: `confidence` is the rule (sure candidates
  and the guesses that are the same game as one are used; guesses alone
  only when enough sources agree; else nothing is saved), applied by every
  automatic path (jobs, the idle fill) through `service/gather` (asking
  the sources) and `service/fetch` (the decision, the merge a preview
  proposes and the saves).
  `review` is the queue of the games sent to review with what each source
  proposed (`local_library/metadata-review.json`, safe to delete), and
  `service/review` its entry points (list, read, search the sources by
  name, and the answers: use a match, in no source, skip). `record` reads
  and writes what a record keeps beside its fields, in additive keys: the
  game of each source (`sourceMatches`; the IGDB id, Steam id and Lutris
  slug stay in their own keys), pinned or not; the locked fields
  (`lockedFields`, kept by every update); the user's "in no source"
  (`noMatch`). Searchable sources (`SourceDescriptor::searchable`: IGDB,
  Steam, Lutris) implement `search` and `lookup_id`, so a pinned game is
  looked up by its id. `service/attention` classifies a set of games from
  disk (waiting for review, no details, failed last time, no description,
  no cover; in no source apart; the games showing a cover family in use
  cropped are listed apart, per family, and are not a reason to need
  attention).

  Extensions are sources too (`heronite-metadata-ext`): the registry
  holds the built-in sources then the extensions, and the service swaps it
  whole when one is added or removed (`MetadataService::add_source` /
  `remove_source`, `sources()` returns the registry of the moment, a
  gather keeps the one it started with); a built-in id is never taken
  (`sources::is_reserved_id`). A descriptor with `extension`
  (`ExtensionInfo`: version, author, homepage, adult) is an extension's:
  it joins the end of each field order it fills (`order`), and its fields
  can be switched and its row moved whatever its kind. A source says
  whether it can be asked (`MetadataSource::health`: `LoadFailed`,
  `Failing`); one that is not ok is not asked (`NotAsked::Failing`) and
  shows `SourceStatus::LoadFailed` / `Failing` with `SourceRow::problem`;
  switching it on calls `reset_health`. Extras and pictures sources only
  follow the details ones in `fetch::decide`. `SourceGame` carries the
  record's year and `file_name` (the ROM's, else the install folder's
  name, never a path) beside the platform and ROM the emulated-game
  sources use; `http` answers keep their headers, a request may refuse
  redirects, a body over its cap is `BodyTooLarge`, and
  `Endpoints::origins` sends whole origins to a test server
  (`Http::route`).

  *Adding a built-in source* (what LaunchBox and libretro-thumbnails needed; no screen code; a
  source that needs neither a database nor the app's own state is better written as an extension):
  1. A provider module (`crates/metadata/src/<source>.rs`, or a folder
     when it has a local database: `launchbox/`): the client (base URLs in
     `http::Endpoints`, and their test prefixes in `tests/common`'s
     `Server::endpoints`; the API's limits; `http::APP_UA` when it asks
     clients to say who they are), and the mapping of an answer to a
     `CollectionGameMetadata` (text through `util::clip` / plain text,
     lists through `unique_names12`, dates as `YYYY[-MM[-DD]]`, only `http`
     picture URLs, a cover family only when the source has art of that
     shape: check its real sizes), with unit tests on recorded answers in
     `tests/fixtures/` (trimmed by hand from the real data, a few hundred
     KB at most) and a `HERONITE_LIVE` check in `tests/live.rs`.
  2. A descriptor in `sources/builtin`'s `BUILTIN`, in merge order: id,
     name, kind, `provides` (only fields it really fills), `applies_to`
     (and `also_for` when an option widens it), `finds_by`, `needs`,
     `network` (false for a local database: it never counts towards
     "offline"), `searchable`, `asked` and `game_kind` when it is a
     fallback for a kind of game, `about` (a `n_!` line for its open row,
     with "Heronite is not affiliated with …"), `options` (`n_!` labels,
     help and choice labels, defaults), `download` for a database it
     downloads, `leads_for` when it should go ahead of title-only matches
     for its audience. Options not in the legacy keys are stored under
     `sources.<id>.options` by themselves.
  3. An adapter implementing `MetadataSource` (`lookup` with an honest
     `MatchedBy`, and for a searchable source `lookup_id` and `search`,
     whose hits name the platform when the source has several), with
     `match_id` set, `adult_pictures` / `thumb_adult` when the source rates
     its pictures, and `download_state` / `download` / `remove_download`
     for a database; listed in `builtin::registry` (shared client state
     goes in `Shared`, and `SourceRegistry::for_tests`). A pictures source
     answers only for the fields asked of it (`SourceGame::fields`: the
     covers kept), picks the cover family from the picture itself when the
     source has pictures of several shapes, has no "Found as" row (only
     details sources are listed) and is not searchable.
  4. `heronite_data::settings`: the id in `SOURCE_IDS` (a details source;
     a pictures source is not), in `default_field_priority` for each field it
     fills, and at the end of `ADDED_SOURCES` (in the order sources were
     added: a list saved between two of them still reads as the default);
     the registry tests check the two agree. A pictures source also needs
     its place in the registry's order (`BUILTIN`: after SteamGridDB), its
     lookup in the registry (`has_lookup` is true for it), and a check that a
     source with a narrow audience does not make `family_askable` say yes
     for a Collection it is not for. Expect the tests that list the sources
     or a field's order (`sources/tests`, `order`, `rows`, `merge`, `settings`, `bulk_jobs`,
     `review`, `sources`, `cover_families`) to change on purpose.
  5. A monogram colour in `theme::source_colour` (or a logo in
     `pages/settings/metadata/labels::logo`), the de / es / pt-BR
     translations of its descriptor's strings (reusing the words already
     translated: regions, "Automatic"), and, when it adds an `AppliesTo`,
     `NotAsked` or status variant, its words in `labels` and `found_as`.
  6. Integration tests against the fixture server (`tests/common`): lookup,
     id, search, when it is asked and when it is not (never before a
     download), offline, and the debug states that show it in a capture
     (`dialogs/metadata/fake.rs`, `HERONITE_DEBUG_SOURCE_DOWNLOAD`).
- **heronite-metadata-ext:** metadata sources the community writes
  (`docs/EXTENSIONS.md` is the authors' reference). `manifest` parses and
  checks `source.toml` into the `SourceDescriptor` (contract version
  `API_VERSION`; unknown keys refused in `[network]`, left out with a
  warning elsewhere; `[applies]` gives `applies_to` — `pc` is
  `AppliesTo::Pc`, `platforms` are read through `platforms::by_name` into
  the app's platform ids — `asked` and `game_kind`; an extension never has
  a `download`, `also_for` or `leads_for`, so it never goes ahead of the
  sources already in a field's order); `package` reads a folder or a zip
  (only the expected files, no links, no path out, size caps; SHA-256);
  `store` keeps the
  installed ones in `<data>/extensions/metadata/<id>/` with `install.json`,
  replaced whole by renames (`.kept/<id>.json`: who made one removed with
  its settings kept, and its hosts); `engine` is the sandbox: QuickJS (`rquickjs`),
  the script compiled once to bytecode, each lookup in a fresh runtime on
  a thread of its own (memory, stack and time limits, no `eval`, no module
  loader; `arrays` wraps the generic `Array.prototype` methods against
  array-likes the interrupt handler cannot stop), whose caller waits at
  most each call's time and `GRACE`: past that the thread is left running
  (`Limit::Stuck`, the extension stopped at once, a few such threads at
  most per extension and in all), `ctx` the only host object (`request`:
  the manifest's hosts over https, checked again on every redirect, sent
  through `net::public_client`, which connects to public addresses only,
  the app's user agent, the manifest's rate, per-call request and byte
  budgets, every string copied out of the engine bounded first;
  `unreachable`: an error that ends the call as unreachable, not as a
  failure, for a script that sees its service busy or down; `html`:
  CSS selectors over `dom_query`, documents per call bounded); `record`
  checks and clamps what scripts return like the built-in mappings
  (a hit's title or its `altTitle`, a name in another script, is what the
  title rule compares), and
  marks the pictures it names (`heronite_images::public::marked`) so the
  images cache and the copy to `metadata-art` fetch them from public
  addresses only; `source::ScriptSource` is the
  `MetadataSource` (failures in a row stop it, kept in `install.json`),
  `BrokenExtension` one that could not be loaded; `service::ExtensionsExt`
  is what the app calls on its `MetadataService` (`load_extensions` at
  startup, `inspect_extension`, `install_extension`, `install_approved`
  — the package shown, by its SHA-256, its saved settings kept or not as
  the user chose when its author or its hosts changed —,
  `remove_extension`, `installed_extensions`). `index` reads a list of extensions
  (`index.json`, format 1, strictly: https only, sizes and ids checked; its
  `type` is `"metadata"` or absent, any other is refused);
  `lists` is the gallery's client: the app's list (`HERONITE_LIST`) and
  the user's (`settings.metadata.extensionLists`), fetched with the
  service's `Http`, the last good copy kept in
  `store_cache/extensions/metadata/`, `prepare` (both files downloaded and
  checked against the entry's SHA-256, then `inspect`, then the manifest
  compared with the entry) and `install_prepared` / `install_approved`
  (records the list in `install.json`), `updates` (never applied by itself) and `Origin`
  (file, the app's list, a list added, a list removed).
  `scripts/extensions-index.py` writes `extensions/metadata/index.json` from the
  folders; `tests/lists.rs` fails when it is out of date. The app's
  screens are `app/src/dialogs/extensions/` (gallery and installer) and
  Settings → Metadata's `extension.rs` / `more.rs`. `examples/run.rs` runs an extension against a
  title. The example extensions are in `extensions/metadata/` (Wikipedia, VNDB),
  each tested against recorded answers (`tests/examples.rs`,
  `tests/vndb.rs`).
- **heronite-achievements:** every game's achievements, whatever its store.
  `target` says which sources a Collection game has (its store, its Steam
  twin, RetroAchievements for a ROM). `sources/` reads them: `steam` the
  client's `appcache/stats` files (binary KeyValues, offline, any Steam
  root) plus the public global percentages; for games never played on this
  PC the Web API with an optional key, else the game's public list on Steam
  Community (`community`: the page's `valve-ssr-data` JSON, its
  `schema_v2` query, all locked). Steam's Achievement Sets come with the
  client's schema (`groups`, `groupid`) and the Community page (`groups`
  with `dlcappid`) as optional `groups` on the set and a `group` id on each
  achievement (`AchievementGroup`: a developer's collection inside one game,
  unlike `AchievementSet`, one game at one source); `sections()` splits a
  game by them (a flat list unless two known groups have achievements); `gog` `gameplay.gog.com` with gogdl's token,
  and `comet` the same achievements while the game runs, from the SQLite database Comet keeps per game and
  account (`<comet data>/gameplay/<client id>/<galaxy user id>/gameplay.db`, read-only), where an unlock lands
  seconds before GOG's servers have it;
  `epic` the store's GraphQL
  (`launcher.store.epicgames.com`, legendary's token, the namespace as
  sandbox, the launcher's User-Agent); `retro` RetroAchievements: the user
  signs in with the username and password once (`connect`, the client API
  `dorequest.php` emulators use: `login2` gives the token kept instead of the
  password), the ROM matched by its rcheevos hash (`vendor/rcheevos`, built
  by `build.rs`) against each console's games and accepted hashes
  (`systemgames`, no login, cached a week in `store_cache/ra_games_<id>.json`)
  or a game picked by hand, then the achievements and the user's unlocks
  through the client API with the token (`patch`, `unlocks`: no unlock
  times, so a new unlock gets the time it is noticed), or through the Web
  API when the user gave its optional key (`api`: the times of past unlocks
  too). `emulators` writes that login (and hardcore mode) into the settings
  files of RetroArch, DuckStation, PCSX2, PPSSPP, Dolphin and Flycast (native
  and Flatpak; only files that exist, never while the emulator runs, other
  lines untouched), so they unlock achievements without signing in there.
  `merge` folds a read into the local copy so it only
  grows (an unlock is never taken back, the earliest time wins); a read
  with groups replaces them (renames, reorders and moved achievements keep
  ids and history), a read without them keeps the known ones, and a DLC id
  the read does not give stays known;
  `store` keeps it in `local_library/achievements/<runner>/<app>.json` plus
  `index.json` (in backups, so a new PC starts with the history). Unknown
  fields in games, provider sets and achievements survive a read/write;
  refreshes retain unknown fields on matching sets and achievement IDs,
  with fresh values taking precedence;
  `accounts` the RetroAchievements account and Steam key
  (`local_library/achievement-accounts.json`, 0600, a backup's sign-ins);
  `prefs` Settings → Achievements (`achievements.json` in the app state);
  `sync::refresh` one game's sources at once, saying what is new.
- **heronite-wine:** the Wine Manager (`manager`: download, install, remove
  builds from `repos`) and `detect`, the Wine/Proton list the settings pick
  from. `default` keeps a global default that can run games: when none is
  set (a new setup) or it was removed, the newest Proton found becomes the
  default, a few seconds after start, after a Wine Manager install, and when
  a Windows game cannot start for lack of one.

## App (`app/src`)

| Module | Role |
|---|---|
| `main.rs` | logging, CLI, single instance, signals, the GPUI loop |
| `env.rs` | `HERONITE_*` switches (`flag`, `value`, `parse`) |
| `system/` | lifecycle `hub/` (window, quit, launch requests, tray events), `cli`, `single_instance`, `signals`, `tray`, `notify`, `shortcuts` |
| `services/` | long-lived entities shared by every screen: `downloads/` (the store service: Epic, GOG, Amazon Games), `session` (post-game work), `compression` (background lossless compression of the captures), `script_install` (the install from a script: worker thread, questions, progress), `update/` (app updates: checks, download, restart), `replay` (the replay buffer, Settings → Replay, off by default, on or off per game in Game settings → Performance: starts gpu-screen-recorder a few seconds after a game is running, again when the options change (one recorder at a time, never during a save) and ends it with the last game, a game that has it off, or when the overlay turns it off for the session; it owns `replay.json` and writes it in order; a save moves the file from the hidden `.heronite-replay` folder of the captures root into `<game folder>/replays/`, named like the captures), `proton_setup` (the newest GE-Proton downloaded on its own when an install dialog finds no Wine or Proton, through the app's one `WineManager`, which the Wine Manager page shares; it becomes the default Wine; it also keeps the installed `-latest` builds up to date (`autoUpdateLatestProton`, on by default): three minutes after the start and then every six hours it checks only the sources that have one, through `heronite_wine::checks` (the release feed first, an API call only when something was published), and downloads a new release in the background, never while a game runs. `HERONITE_WINE_ONLY_TOOLS=1` makes the detection see only the data folder's builds, to try it with a copy), `launcher_imports` (the games of Heroic, Lutris, Bottles and Faugus, Heroic and Playnite on Windows: which are on this PC, turning an import on or off, keeping the ones on in sync every quarter of an hour and on the refresh button of the ☰ menu; Heroic is turned on only in Settings → Other launchers, never from the Collection's welcome or the first-start dialog), `clients/` (the store clients, `heronite_clients`, under Wine on Linux and as they are on Windows: their setup, opening, closing and sign-in; it starts with the app and costs nothing while none is set up: it looks at them ten seconds after the start and every quarter of an hour, syncing the installed ones' games into the Collection; whether a client runs is asked every 3 s while its page is on screen and every 15 s while a client opened from here runs, and one that closes, or whose game ended, is synced; `HERONITE_DEBUG_CLIENT=<id>:<state>` shows a state without Wine, see `debug.rs`), `achievements` (each game's local copy read when the Collection shows it and refreshed when older than six hours, 20 s after a session ends, from the focus panel, the dialog and Settings → "Update now", one game at a time; new unlocks become a desktop notification (off by default); it also signs the emulators in to RetroAchievements: at start, on sign-in, when the options change and after a game for one that was open, and on sign-out clears only the emulators holding this account), `sync` (the cloud account, `heronite_sync`: the library sync 20 s after start, 5 s after each game and every ten minutes, never during a game, plus "Sync now"; it connects the account on a thread of its own while the browser sign-in waits, and a sync that changed the Collection makes it reload; `uploads` is one queue for the rest: a backup as it is made (`upkeep`), the saves after each game, the screenshots when the compression has nothing left, one upload at a time on a thread of its own, stopped by a game launch and resumed after it, tried again after 2, 5, 15 then 30 minutes when the account refuses requests, stopped when the app quits and resumed at the next start; `cloud_backups` lists, brings and restores the backups in the account), `steam_library` (reads the Steam library while Steam runs: every few seconds until the first read, then every ten minutes; opens and restarts Steam for the welcome, and tells "starting" from "needs a restart" by the `steamwebhelper` command line; at start it ends and deletes a Steam sign-in an earlier version saved), `widgets`, `mods/` (see `docs/MODS.md`: one entity with each game's mod list shared with the job threads (a change shows at once and is saved in the background, always from the latest list), the jobs (a thread each: download, unpack, FOMOD questions, commit; installs of a game take turns, three downloads at most), `nxm://` / `ror2mm://` links matched to an installed game (`links.rs`) or asked with a picker, the Nexus Mods account, `xdg-mime` handlers (`handlers.rs`), Steam launch options for DLL overrides (`steam.rs`), and the launch hook `ModsLaunch` (`launch.rs`, the `heronite_launch::GameMods` that mounts the mods around a session); Settings → Mods is `pages/settings/mods.rs`), `game_folders` (where a game's files are: shared by the Details panel and the mods) |
| `runtime.rs` | bridge between `heronite-launch` and the UI |
| `diagnosis/` | the launch diagnosis for the UI: `gather` reads what `heronite_launch::diagnose` needs for a game, `apply` applies a fix (winetricks components, game settings, the best Proton, ending a wineserver, `vm.max_map_count` through `pkexec`), `text` says it; the fixes applied this run are remembered so one that did not help is not offered as the answer again. The Collection runs it when a game ends within `FAILED_LAUNCH_WINDOW` and opens `dialogs/launch_problem`; without a window the cause goes into the notification |
| `overlay/` | the in-game overlay (off by default): `OverlayService` follows the launch events like `services::session`, runs `heronite_overlay::input` while a game runs, and opens the panel (`panel/`, its Notes tab in `notes.rs` with the compact on-screen keyboard, its Achievements tab in `achievements.rs` from `services::achievements` when the game has a set) and notices (`toast.rs`; achievement notices queue) through `window.rs`: Wayland layer-shell on Linux, native topmost tool windows on Windows (`window/windows.rs`, HWND placement/focus in `heronite-platform::overlay_window`). Windows notices do not activate and pass pointer input through; a keyboard panel returns focus only while it still owns foreground. The keyboard shortcut uses the existing capture `GlobalHotkey` (Windows: `RegisterHotKey`, Ctrl+Shift+G default). `labels.rs` has every text that names the controller chord (Guide on Linux, Back + Start on Windows), for Settings, its search, the palette, "Getting started" and the panel. `moment.rs` marks a moment in the game's journal (a capture and an empty note); Linux-only `Feature::Replay` gates `services::replay` and its controls independently of the overlay; `replay.rs` saves the replay and marks it as a moment (`clip` in the journal entry) |
| `collection/` | the main screen (`CollectionView`): list, header and its View menu (`view_menu`: filter search, views, sort, grouping), the Focus / Grid switch (`view_mode`) and its options menu (`view_options`), focus panel, grid view (`grid`, `grid_drawer`), wider list and drawer by dragging their edge (`resize`), Settings → Appearance on the Collection (`look`: background picture and slideshow, accent from the game, start view, Focus or Grid by the window's width), the picture with no game selected (`home`: yours, or a random game's art or screenshot, a new one every 20 seconds; also behind glass), covers by family (`family_art`: each view's picture of the family it draws, keeping the tall cover's and the background's texture ids), selecting several games (`selection`: Ctrl / Shift click, Ctrl A, the batch changes and their undo; `dock`: the bar that shows them; `metadata_menu`: the menu of its Metadata button and the selection's context menu; `metadata_pill`: the header pill of a bulk metadata update and of the idle fill's games to review; `metadata_review`: opens "Review matches" on the queue, a job's games or the selection, and the debug openers of the review and of the one-game "Metadata"), stage, context menu, command palette, screenshots, the game's Journal (`journal/`: the Timeline's other view, notes written in place), and the glue to dialogs and services; `welcome/` is the empty Collection of a new setup: what Heronite found on this PC and brings in by itself (Steam's library, with "Open Steam" or "Restart Steam" when that is what is missing; emulators and the usual ROM folders, set up once with `heronite_emulation::setup`), the other sources, the other launchers found with games (a click turns their import on), the stores, and a shelf of empty covers (`HERONITE_DEBUG_WELCOME=<steam>[:<emulation>]` shows a state); `launchers` reloads the Collection when another launcher's games come in; `onboarding` hosts "Getting started" (`dialogs/onboarding`: three cards, your games, your look and while you play, that open inside it or switch things on in place, plus tips; `art` draws its pictures without needing games: empty covers, a preview of the Collection in the current look and chosen view (the bare layout until there are games) and the overlay panel; the look view puts Focus and Grid first, over the preview), which opens by itself once per setup (on a new setup, once the first games arrive), from the bulb in the header until it is closed once, and from the command palette (`onboarding.json` in the app state) |
| `dialogs/` | one view per dialog; they talk back through `DialogEvent`. `metadata_update/` is "Update metadata" (three steps: games, what to update, results), `families` (a view that starts drawing a cover family queues its download) and, in `hub`, the `MetadataHub` that owns the metadata service, the bulk jobs' progress, their undo and the idle fill, read by the dialog, the Metadata menu, the header pill and Settings → Metadata; it also keeps the review queue the pill and the palette read. `metadata_review/` is "Review matches" (the game on the left, the games the sources proposed on the right, a search of every source by another name, and the answers: use a match, in no source, skip; `fake.rs` makes it up for the debug hooks). `metadata/` is the "Metadata" of one game: "Found as" (`found_as`, and `change` to pick another game of one source), a row per field (a cover row per kept family) with the sources to take it from and a lock (`fields`), and what it asks and saves (`actions`). `covers/` is "Covers", a game's picture of each cover family and its background (`slots` the left column, `found` the pictures LaunchBox (the game's boxes of every region, with their region and size), SteamGridDB and the web found for the picked slot, `actions` what it searches and saves, `words` how a family reads on every screen that names one, `fake` the debug searches); Game settings → Images, the one-game Metadata and the palette open it. `extensions/` is "Get more sources" and the installer of an extension, one dialog with two faces (`gallery` the entries a list offers and the picked one's panel, `confirm` the installer, shared by a file and a list's entry, `lists_manager` "Lists…", `permissions` the "What it is allowed to do" lines, `words` every error, origin and trust sentence, `actions` what reads, downloads, installs and adds lists off the UI thread, `state` what the hub keeps of the installed extensions and the lists, `fake` the debug lists); Settings → Metadata, the palette and the Collection's debug hooks open it through `HubEvent::OpenSources`. `metadata_update/family_notice.rs` is the notice under both Cover aspect pickers (offer, started, no source) and the hub's `download_family`; `metadata_update/source_downloads.rs` the hub's downloads of the sources' databases (start, cancel, remove, the progress Settings draws, the idle refresh) |
| `pages/` | ☰ pages (Settings, Downloads, Epic, GOG, Amazon Games, Steam, the store clients' `clients/`: Battle.net, EA app, Ubisoft Connect, Rockstar Games, Wine Manager, Emulators); they talk back through `PageEvent`. Settings → Metadata (`pages/settings/metadata/`) is drawn from the source registry; its `extension.rs` is what an extension's open row adds to a source's (the "Extension · 1.0.0" pill, the problem block, who made it, where it came from, where it connects, "Update from a file…", "Remove") and `more.rs` the dashed "Get more sources" row under the sources; its `shapes.rs` is the "Cover shapes" card (a row per cover family, the automatic download switch, the downloads of the families shown cropped) and its Console mode rows. `methods` is the layout of a store read in two ways (Steam, Xbox): a library strip, then "Method 1 · Local" (no login) and "Method 2 · Online · Optional" (the account) side by side |
| `ui/` | building blocks: `controls`, `fields`, `dialog`/`drawer`, `dropdown`, `glass`, `scrollbar`; `window_area` (`content_size` / `content_bounds`: the room the views have inside a client-side window frame; layouts size from it, never from `window.viewport_size()`) |
| `theme/` | colour, font, layout and glass tokens, plus the runtime appearance and the light or dark scheme |
| `images.rs` | UI side of the image pipeline (texture cache, eviction) |

### Patterns
- **One window, one root view.** `CollectionView` owns the open dialog, page,
  menus and palette, and draws them over the list. A big view is a
  directory: `mod.rs` holds the struct and `Render`, and sibling files add
  `impl` blocks (`actions.rs`, `load.rs`, `render_*.rs`, `debug.rs`).
- **Services are entities** kept in GPUI globals and created on first use. Views `observe` them or
  `subscribe` to their events. They own background work, so closing the
  window (to the tray) does not stop a download or a post-game task.
- **No I/O in `render`.** Loads run in the background and land in the entity
  with `this.update(..)`. Long-lived channels are drained by "pump" tasks
  (`collection/pumps.rs`).
- **Batch writes.** A change to many games is one write per file, never a
  write per game: `status::set_games_status`, `config_games::set_favourites`
  / `set_hidden`, `categories::set_many_in_category` / `restore_categories`
  and `heronite_metadata::edit::update_many_details` (tag deltas and the
  series; it returns the entries it replaced, which `restore_details` puts
  back). The Collection keeps what the games were before each batch change
  and undoes it with the same functions.
- **Debug hooks.** `HERONITE_DEBUG_*` variables open a page, dialog, menu or
  state at startup, so `scripts/shot.sh` can capture it without clicks. They
  live in each area's `debug.rs`, and every read goes through `crate::env`.
- **Safe shutdown.** `system/hub/quit.rs` is the single quit path (window,
  tray, signals): checkpoint playtime, pause the Epic queue, release the
  socket and the tray, then end the loop. Games are separate process groups
  and keep running. "Restart now" after an update goes through the same path
  (`restart_into`, refused while a game runs) and `main` starts the new
  AppImage once the socket is gone.
- **App updates.** `heronite_update` reads the latest GitHub release and
  checks every download against its SHA-256 in the release's `SHA256SUMS`,
  whose minisign signature (`SHA256SUMS.minisig`, key in
  `crates/update/src/lib.rs`, trusted comment `version:<X.Y.Z>`) it checks
  first; a release without the list (up to 1.0) is checked against each
  file's own `.minisig` (`release::Proof`). From an
  AppImage (`$APPIMAGE`) it downloads `Heronite-x86_64.AppImage` next to the
  running one and renames it over the old file: the running app keeps the old
  inode, and nothing is written into it. From our .deb / .rpm / Arch package
  (marker `/usr/share/heronite/package-format`, written by
  `scripts/packages.sh` from `packaging/nfpm.yaml`) it downloads the package
  of the same format and runs apt-get / dnf / pacman through `pkexec`. Any
  other build only checks when asked. The release workflow signs with the
  `MINISIGN_SECRET_KEY` secret and checks the result against the key in the
  code.

## Tests
- Unit tests sit next to the code. Integration tests are in
  `crates/*/tests`, and some need `HERONITE_FIXTURE=/tmp/heronite-fixture`
  (a copy of a data folder).
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` must
  both be clean.
- UI checks use `scripts/shot.sh` on a fixture copy (see "Verifying visually"
  in `docs/UI-CONVENTIONS.md`).
