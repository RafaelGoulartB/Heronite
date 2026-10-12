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
 ├─ heronite-images     decode/resize/blur pipeline and thumbnail cache (no GPUI)
 ├─ heronite-launch     launch plans, game runtime, playtime, logs   → data
 ├─ heronite-epic       legendary CLI, Epic library, download queue  → data, launch
 ├─ heronite-emulation  emulators, ROM scan, save states, Ludusavi   → data, launch
 ├─ heronite-metadata   IGDB/Steam/Lutris/HLTB metadata and artwork  → data
 ├─ heronite-media      screenshots: listing, thumbnails, capture,
 │                      lossless PNG → WebP compression              → data, images
 ├─ heronite-wine       Wine/Proton manager                          → data
 ├─ heronite-shortcuts  .desktop files and Steam shortcuts.vdf       → data
 └─ heronite-update     app updates: release check, signed download,
                        AppImage swap / package install, relaunch   → data
epic-login (bin)        WebKit window for the Epic login (keeps a web engine out of the app)
```

Library crates never depend on GPUI. Their functions are blocking; the app
calls them from `cx.background_spawn` or worker threads.

- **heronite-data:** the base. `paths` (`DataPaths`, `AppPaths`), `migrate`,
  `json_store` (electron-store read-modify-write: tab indent, atomic write,
  unknown keys kept), `temps` (atomic writes and the startup `.tmp` sweep),
  `proc` (`guard` and `Run`: helper processes die with the app), and one
  module per data file: `library`, `collection`, `sideload`, `game_config`,
  `settings`, `status`, `sessions`, `playtime`, `ui_state`, `appearance`,
  `language`.
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
  first time it runs in a Wine prefix.
- **heronite-epic:** `cmd` runs legendary. `manager` + `queue` are the
  download queue (`store/download-manager.json`). `ops` covers install,
  repair, uninstall and import. `moves` is the resumable install move with
  its journal. `eos_overlay` manages the EOS overlay.

## App (`app/src`)

| Module | Role |
|---|---|
| `main.rs` | logging, CLI, single instance, signals, the GPUI loop |
| `env.rs` | `HERONITE_*` switches (`flag`, `value`, `parse`) |
| `system/` | lifecycle `hub/` (window, quit, launch requests, tray events), `cli`, `single_instance`, `signals`, `tray`, `notify`, `shortcuts` |
| `services/` | long-lived entities shared by every screen: `downloads/` (the Epic service), `session` (post-game work), `compression` (background lossless compression of the captures), `update/` (app updates: checks, download, restart), `widgets` |
| `runtime.rs` | bridge between `heronite-launch` and the UI |
| `collection/` | the main screen (`CollectionView`): list, header, focus panel, stage, context menu, command palette, screenshots, and the glue to dialogs and services |
| `dialogs/` | one view per dialog; they talk back through `DialogEvent` |
| `pages/` | ☰ pages (Settings, Downloads, Epic, Wine Manager, Emulators); they talk back through `PageEvent` |
| `ui/` | building blocks: `controls`, `fields`, `dialog`/`drawer`, `dropdown`, `glass`, `scrollbar` |
| `theme/` | colour, font, layout and glass tokens, plus the runtime appearance |
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
