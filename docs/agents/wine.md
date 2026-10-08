# Your brief: WINE MANAGER (agent "wine")

Goal: Heroic's Wine Manager: browse Wine/Proton builds from the same repositories Heroic uses, download/install them
with progress, cancel, remove, see installed ones; plus the detection of all usable Wine/Proton installations for the
"Wine version" selector of the game settings.

Heroic sources to port:
- src/backend/wine/manager/** (downloader/{main,utilities,constants}.ts, utils.ts, ipc_handler.ts): repositories
  (GE-Proton, Proton-CachyOS, Wine-Lutris, ... — check which are offered on Linux and which are default-enabled),
  GitHub release fetching, store/wine-downloader-info.json (cache; the real file exists in ~/.config/heroic/store —
  match its format exactly), download (+ checksum verification), extraction to ~/.config/heroic/tools/{wine,proton}/,
  "-latest" naming/rename semantics, removal, `downloadProtonToSteam` setting (install into Steam compatibilitytools.d).
- src/backend/utils/compatibility_layers.ts (getWineList / getLinuxWineSet / Steam compat tools, system wine, custom
  wine paths, Lutris dirs, tools dir, CrossOver/toolkit irrelevant on Linux), src/backend/utils.ts helpers it uses.
- frontend: src/frontend/screens/WineManager/** (layout reference only — use our style).
Existing Rust: crates/launch/src/{settings.rs,wine.rs} (WineVersion, umu), crates/data (paths, json_store).

Owned files:
- crates/wine/** (skeleton exists; KEEP `WineInstallation` and `list_installations(&HeroicPaths) -> Vec<WineInstallation>`
  signature — the settings agent uses it for the Wine version dropdown; you may add fields/functions).
  Suggested modules: repos.rs (repository list + GitHub fetch + cache), download.rs (streaming download with progress,
  cancel flag, checksum), extract.rs (tar.gz/tar.xz/tar.zst via `tar` + `flate2`/`xz2`/`zstd` crates, or spawn system
  `tar` like Heroic — your choice, but handle progress), detect.rs (list_installations).
- app/src/pages/wine_manager.rs (replace placeholder; keep `new(ctx, window, cx)` + EventEmitter<PageEvent>).
- app/Cargo.toml only to add deps if needed (heronite-wine is already a dependency).
Do not edit other app files except: app/src/pages/mod.rs if you need a tiny shared helper (prefer keeping helpers in
your file).

UI (pages/wine_manager.rs): repository tabs (or segmented control) like Heroic, list of versions (name, release date,
download size, installed size when installed), actions Install / Cancel (with progress bar, percent, speed) / Remove
(confirm), "Release notes" link (xdg-open), refresh button (re-fetch, respecting GitHub unauthenticated rate limit —
cache in wine-downloader-info.json like Heroic and only refresh on demand/when stale). Downloads continue when the user
navigates away from the page (state must live outside the view, e.g. in a GPUI Global or Arc<Mutex<..>> owned by the
page entity which persists — note CollectionView keeps page entities alive once created). Show errors as toasts via
PageEvent::Toast.

Testing constraints:
- GitHub API unauthenticated limit is 60 req/h — cache responses during development (save fixtures under
  crates/wine/tests/fixtures or /tmp) and don't loop requests.
- Real downloads/extractions: at most ONE real download of a small build to validate the pipeline, into a fixture
  (HERONITE_HEROIC_DIR=/tmp/heronite-fixture-wine, tools dir inside it), then delete it. NEVER touch the real
  ~/.config/heroic/tools or ~/.local/share/Steam/compatibilitytools.d.
- detection: test list_installations against the real machine read-only (it must find ~/.config/heroic/tools/proton/
  GE-Proton-latest and the Steam compat tool "Proton-CachyOS Latest" referenced in config.json) and with tempdir tests.
- Screenshot the page with HERONITE_DEBUG_PAGE=wine (+ a debug env to fake an in-progress download for the capture).
