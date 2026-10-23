# Architecture

HeroniteRust is a GPUI rewrite of the Heronite launcher (a Heroic Games
Launcher fork). It is a separate app with its own data folder,
`~/.local/share/heronite/` (`DataPaths`; `HERONITE_DATA_DIR` overrides
it). The folder keeps Heroic's layout and electron-store formats, but nothing
in it is shared with Heroic or Heronite, and this app never reads or writes
Heroic's folders. UI state lives in `~/.config/heronite/` and caches in
`~/.cache/heronite/` (`AppPaths`).

On the first start, when the data folder does not exist yet,
`heronite_data::migrate` copies an existing Heroic setup into it once:
`~/.config/heroic` (minus Chromium's files), Heroic's game logs and the
legendary binary of the Heronite install (to `tools/bin/legendary`). Paths
into Heroic's folder are rewritten. Heroic's files are only read. The
UI state and cache folders of the old name (`heronite-rs`) are renamed.

`docs/CODE-STYLE.md` has the code rules; `docs/UI-CONVENTIONS.md` the look of
the UI.

## Crates

```
app (bin heronite-rs) ── GPUI UI, desktop integration
 ├─ heronite-i18n       interface language: tr!/trn! and the pt-BR strings (no GPUI)
 ├─ heronite-data       data folder files, paths, Heroic migration, safe writes, processes
 ├─ heronite-backup     backups of the data folder and app state:
 │                      incremental snapshots, retention, restore    → data
 ├─ heronite-images     decode/resize/blur pipeline and thumbnail cache (no GPUI)
 ├─ heronite-launch     launch plans, game runtime, playtime, logs   → data
 ├─ heronite-epic       legendary CLI, Epic library, download queue  → data, launch
 ├─ heronite-gog        gogdl CLI + GOG web APIs: GOG account,
 │                      library, installs, updates, cloud saves      → data, launch, epic
 ├─ heronite-amazon     nile CLI: Amazon Games account, library,
 │                      installs, updates                            → data, launch, epic
 ├─ heronite-emulation  emulators, ROM scan, save states, Ludusavi   → data, launch
 ├─ heronite-import     games of Heroic, Lutris, Bottles and Faugus
 │                      brought into the Collection                  → data
 ├─ heronite-installer  games from YAML install scripts              → data, launch, emulation
 ├─ heronite-metadata   IGDB/Steam/Lutris/HLTB metadata and artwork  → data
 ├─ heronite-media      screenshots: listing, thumbnails, capture,
 │                      lossless PNG → WebP compression              → data, images
 ├─ heronite-overlay    in-game overlay backend: controller input
 │                      (evdev, Guide button, grabs), pactl audio,
 │                      controller battery / Bluetooth, replay
 │                      buffer (gpu-screen-recorder)                 → data
 ├─ heronite-wine       Wine/Proton manager                          → data
 ├─ heronite-storage    disk space of games, prefixes, Proton builds,
 │                      Steam leftovers, caches; cleaning them up    → data, launch, wine
 ├─ heronite-shortcuts  .desktop files and Steam shortcuts.vdf       → data
 ├─ heronite-steam      settings of Steam games (launch options,
 │                      Proton) via the client or its files         → data, launch, shortcuts
 └─ heronite-update     app updates: release check, signed download,
                        AppImage swap / package install, relaunch   → data
epic-login (bin)        WebKit window for the Epic, GOG (`--store gog`) and Amazon
                        (`--store amazon --url ..`) logins (keeps a web engine out of the app)
```

Library crates never depend on GPUI. Their functions are blocking; the app
calls them from `cx.background_spawn` or worker threads.

- **heronite-data:** the base. `paths` (`DataPaths`, `AppPaths`), `migrate`,
  `json_store` (electron-store read-modify-write: tab indent, atomic write,
  unknown keys kept), `temps` (atomic writes and the startup `.tmp` sweep),
  `proc` (`guard` and `Run`: helper processes die with the app), and one
  module per data file: `library`, `collection`, `sideload`, `game_config`,
  `settings`, `status`, `status_history` (when each game's status changed, in
  `local_library/status-history.json`, recorded by the status setters), `sessions`, `playtime`, `ui_state`, `appearance`,
  `stats` (the Statistics page's numbers: periods cut into hours, the history
  indexed by time and by game so a period reads only its slice, `game` for a
  game's sheet, `journeys`, `backlog` (goal pace, weekly hours, running counts),
  `insights` (discoveries and milestones), `album` (moments, captures per session, capture hours) and `prefs` (its keys in `ui-state.json`)),
  `journal` (each game's notes in `local_library/journal.json`: the moment
  they are about, placed on the timeline's days by the screenshots' rule in
  `timeline`, an optional screenshot or replay, and "where I left off"),
  `replay_prefs` (Settings → Replay, `replay.json` in the app state: the
  switch, the options every game uses and the games that have it on or off on their own),
  `language`, `install_size` (size on disk of a game its store gives none for: ROM files, a bounded and cached folder walk), `field_names` (the library fields manager: renames, merges and
  removals of genres, companies, series, themes, features and platforms, kept
  as rules in `local_library/names.json` that the metadata service and the
  library load apply too, plus the duplicate suggestions), `steam_owned` (the cached Steam library and its twins: a game
  owned on Steam that the Collection has from another source joins that
  entry instead of getting a Steam entry of its own).
- **heronite-i18n:** English text is the key (`tr!("Play")`); Portuguese
  lives in `locales/pt-BR/*.json`, embedded at build time and loaded the
  first time it is needed. The language follows the system unless one is
  picked in Settings → General (`language` in `appearance.json`); switching
  redraws the windows. `tests/coverage.rs` scans the workspace: every key
  needs a translation and every translation a key.
- **heronite-launch:** `plan` turns a game plus settings into a `LaunchPlan`
  and does no I/O beyond reads. `runtime::Launcher` runs the plan through a
  `Spawner`, tracks the process group and emits `LaunchEvent`s. `record`
  writes sessions and playtime (with checkpoints while the game runs).
  `log_writer` writes the launcher and game logs. `prepare` does the work
  around a launch (cloud saves, runtimes); `first_launch` sets up a game the
  first time it runs in a Wine prefix. `lsfg` finds lsfg-vk (Lossless
  Scaling frame generation), the Lossless Scaling DLL and lsfg-vk's profiles,
  and adds the frame generation variables to a launch. `fps_limit` holds a
  game's one FPS limit and picks the one limiter that applies it (MangoHud,
  DXVK / VKD3D-Proton or gamescope; Heroic's separate limiters are read as
  a fallback). `live_fps` makes that limit changeable while the game runs when MangoHud or gamescope applies it
  (MangoHud reads it from a config file of ours in `$XDG_RUNTIME_DIR/heronite/mangohud`, rewritten by the in-game
  overlay; gamescope's is its `GAMESCOPE_FPS_LIMIT` atom, set by `heronite_overlay::gamescope`). `gpu` holds a game's graphics card settings (the card that draws it,
  NVIDIA Smooth Motion and Reflex, the DLSS / FSR 4 / XeSS upgrades of GE-Proton and
  Proton-CachyOS, Mesa's Anti-Lag and glthread, vsync and what DXVK tells DirectX games)
  as variables, and `gpu::detect` finds the cards (sysfs, `vulkaninfo`, `pci.ids`) and the
  driver layers they need. `actions` turns a game's own actions (`heronite_data::game_actions`,
  `local_library/actions.json`) into something to open, spawn or launch as
  the game's session. `umu_ids` picks the
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
  Wine/Proton plan of local games; the first launch in a prefix runs the
  store's installers (`first_launch::StoreSetup`, Amazon's `PostInstall`).

- **heronite-import:** the other Linux launchers. `detect` finds Heroic,
  Lutris, Bottles and Faugus (native or Flatpak) and reads their libraries,
  read only: Lutris' `pga.db` (SQLite, compiled in through rusqlite's
  `bundled`) and the Wine prefix from each game's YAML, each bottle's
  `bottle.yml` (its programs; the bottle is their prefix) and Bottles'
  `library.yml` (covers), Faugus' `games.json`, Heroic's sideload library and
  install records. `collection` writes the installed games of Lutris,
  Bottles and Faugus as sideload entries whose `launchKind` is
  `launcher` (`heronite_data::launchers`): `launcherArgs` holds the command
  that asks the launcher to start them, the launcher's play time moves up
  when it is ahead, and a game gone from the launcher stays as not
  installed. `heroic` copies the games added by hand in Heroic (they start
  here, with their `GamesConfig`) and adds its Epic, GOG and Amazon install
  records to this app's. Which launchers are imported lives in
  `local_library/launcher-imports.json`; the app's `services::launcher_imports`
  keeps them in sync. A `launcher` game's plan is a native process with a
  `Handoff`: when the launcher returns at once (Lutris already open passes
  the request to its window), the runtime follows the game's processes
  (`Procs::game_pids`: files under its folder, Windows programs in its
  prefix) until they are gone, and a stop ends them.
  `duplicates` pairs the games then in the Collection twice: added here by
  hand (pointing into the launcher's prefix or folder, or with the same
  name) and brought in from Lutris, Bottles or Faugus. Settings → Other
  launchers merges a pair (`heronite_data::carry_over` moves play time, sessions,
  status and its history, notes, journal, rating, favourites, categories and
  custom fields onto the launcher's entry, then the other leaves the
  Collection), keeps only the launcher's, or keeps both
  (`keptApart` in `launcher-imports.json`). The folders and prefixes of the
  `launcher` games (`launchers::managed_paths`) are theirs: an uninstall or
  prefix removal of a game added here, Wine Manager's trash and the Storage
  page (in use, never a leftover) all leave them alone.

- **heronite-steam:** Steam games keep launching through Steam
  (`steam://rungameid`), with Steam's Proton and prefix. Their game settings
  tabs edit what Steam stores per game instead: the compatibility tool and the
  launch options line. `options` maps the line to the panel's settings keys
  and back (lines with shell syntax are only edited as text). While Steam
  runs, changes go through its CEF debugging interface (`client`, via
  `heronite_shortcuts::steam_client`, whose port `debugger_addr` finds).
  Steam opens that interface only if its flag file is there when it starts,
  so `prepare_access` writes it into every Steam install found (native,
  Flatpak, Snap, and the one running) as soon as Heronite sees Steam: a Steam
  opened after Heronite never needs a restart. `steamwebhelper`'s command
  line tells a Steam still starting (`LibraryAccess::Starting`) from one that
  started without the flag (`NeedsRestart`);
  with Steam closed, `files` edits `localconfig.vdf` and `config.vdf` in
  place (`vdf_edit` touches only the changed entry). `wrap` writes the script
  that runs the before/after scripts from the launch options. `library`
  reads the account's library from the client (games only: no tools,
  soundtracks, demos, test servers or games hidden in Steam), caches it and
  adds the games the Collection does not have yet; Steam's play time moves
  the Steam entries up when it is ahead.
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
- **heronite-overlay:** the backend of the in-game overlay. `input` reads
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
| `services/` | long-lived entities shared by every screen: `downloads/` (the store service: Epic, GOG, Amazon Games), `session` (post-game work), `compression` (background lossless compression of the captures), `script_install` (the install from a script: worker thread, questions, progress), `update/` (app updates: checks, download, restart), `replay` (the replay buffer, Settings → Replay, off by default, on or off per game in Game settings → Performance: starts gpu-screen-recorder a few seconds after a game is running, again when the options change (one recorder at a time, never during a save) and ends it with the last game, a game that has it off, or when the overlay turns it off for the session; it owns `replay.json` and writes it in order; a save moves the file from the hidden `.heronite-replay` folder of the captures root into `<game folder>/replays/`, named like the captures), `launcher_imports` (the games of Heroic, Lutris, Bottles and Faugus: which are on this PC, turning an import on or off, keeping the ones on in sync every quarter of an hour and on the refresh button), `steam_library` (reads the Steam library while Steam runs: every few seconds until the first read, then every ten minutes; opens and restarts Steam for the welcome, and tells "starting" from "needs a restart" by the `steamwebhelper` command line), `widgets` |
| `runtime.rs` | bridge between `heronite-launch` and the UI |
| `overlay/` | the in-game overlay (beta, off by default): `OverlayService` follows the launch events like `services::session`, runs `heronite_overlay::input` while a game runs, and opens the panel (`panel/`, its Notes tab in `notes.rs` with the compact on-screen keyboard) and notices (`toast.rs`) as layer-shell windows (`window.rs`); `moment.rs` marks a moment in the game's journal (a capture and an empty note); `replay.rs` saves the replay (Guide + X, the panel's "Save replay") and marks it as a moment (`clip` in the journal entry) |
| `collection/` | the main screen (`CollectionView`): list, header and its View menu (`view_menu`: filter search, views, sort, grouping), focus panel, grid view (`grid`, `grid_drawer`), wider list and drawer by dragging their edge (`resize`), Settings → Appearance on the Collection (`look`: background picture and slideshow, accent from the game, start view, Focus or Grid by the window's width), selecting several games (`selection`: Ctrl / Shift click, Ctrl A, the batch changes and their undo; `dock`: the bar that shows them), stage, context menu, command palette, screenshots, the game's Journal (`journal/`: the Timeline's other view, notes written in place), and the glue to dialogs and services; `welcome/` is the empty Collection of a new setup: what Heronite found on this PC and brings in by itself (Steam's library, with "Open Steam" or "Restart Steam" when that is what is missing; emulators and the usual ROM folders, set up once with `heronite_emulation::setup`), the other sources, the other launchers found with games (a click turns their import on), the stores, and a shelf of empty covers (`HERONITE_DEBUG_WELCOME=<steam>[:<emulation>]` shows a state); `launchers` reloads the Collection when another launcher's games come in; `onboarding` hosts "Getting started" (`dialogs/onboarding`: three cards, your games, your look and while you play, that open inside it or switch things on in place, plus tips; `art` draws its pictures without needing games: empty covers, a preview of the Collection in the current look and chosen view (the bare layout until there are games) and the overlay panel; the look view puts Focus and Grid first, over the preview), which opens by itself once per setup (on a new setup, once the first games arrive) and from the bulb in the header (`onboarding.json` in the app state) |
| `dialogs/` | one view per dialog; they talk back through `DialogEvent` |
| `pages/` | ☰ pages (Settings, Downloads, Epic, GOG, Amazon Games, Steam, Wine Manager, Emulators); they talk back through `PageEvent` |
| `ui/` | building blocks: `controls`, `fields`, `dialog`/`drawer`, `dropdown`, `glass`, `scrollbar` |
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
  checks every download against its minisign signature (key in
  `crates/update/src/lib.rs`, trusted comment `version:<X.Y.Z>`). From an
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
