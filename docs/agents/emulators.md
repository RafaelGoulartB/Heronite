# Your brief: EMULATOR MANAGEMENT (agent "emulators")

User request (translated): "Improve emulator management: it must be possible to create several emulators, to scan
(detect) emulators, and the refresh must automatically work with the configured emulators."
Current state: emulators live in ~/.config/heroic/local_library/emulators.json and ROM folders ("scanners") in
local_library/rom-scanners.json (Heronite format — keep it byte-compatible; real example: one custom "PCSX2" AppImage
emulator at /data/Games/Emulation/PS2/emulator/pcsx2-v2.8.2.AppImage with a "custom" profile, platformName "Other",
args "{ImagePath}", extensions iso/cue/chd/zip, and one scanner for /data/Games/Emulation/PS2/games with
autoScanOnRefresh true). Backend: crates/emulation (catalog.rs: 36 emulator definitions with profiles/platforms;
detect.rs: PATH/flatpak detection; store.rs: upsert/remove emulators & scanners; scan.rs/import.rs: ROM scan preview,
import, run_auto_rom_scanners; jobs.rs: idle job 90 s after start). UI today: Collection settings → Emulators tab
(app/src/dialogs/settings/emulators.rs: "Detect on this PC", "Scan saved folders", "Add emulated games…") and the
"Add emulated games" dialog (app/src/dialogs/scan_roms.rs). Heronite reference UI:
HeroicGamesLauncher/src/frontend/screens/LocalLibrary/Collection/{EmulatorManager.tsx,ScanRomsDialog.tsx} and backend
src/local-library/emulation/**.

Build:
1. A new "Emulators" page in the ☰ menu (add `Page::Emulators` in app/src/pages/mod.rs + nav.rs; page file
   app/src/pages/emulators.rs; HERONITE_DEBUG_PAGE=emulators). It is THE emulator manager:
   - List of configured emulators (cards/rows): name, platforms of its profiles, executable + launch kind
     (native/flatpak/wine), health (executable exists / flatpak installed), number of games using it, its ROM folders.
   - Add emulator: from the catalog (pick a definition → its profiles/platforms prefilled; executable found by
     detection or chosen with a file picker) or Custom (name, executable, launch kind, flatpak id, install dir).
     Several emulators allowed, including several of the same kind.
   - Edit emulator: all fields, and its profiles (add/edit/remove profile: name, platform, arguments with the
     placeholders the launch code supports — check crates/launch/src/emulators.rs, e.g. {ImagePath} — extensions,
     RetroArch core when relevant). Duplicate. Delete (confirm; follow Heronite's remove semantics for games and
     scanners using it — check removeUserEmulator in the Heronite backend).
   - Detect emulators: improve detection beyond PATH/flatpak: AppImages and binaries in common places
     (~/Applications, ~/.local/bin, ~/Games, /opt, and the folders of already configured emulators) matched against
     the catalog (names like pcsx2*.AppImage, duckstation*, ppsspp*, rpcs3*, dolphin*, cemu*, ryujinx*/yuzu forks,
     retroarch, etc.), plus "Scan a folder for emulators…" where the user picks a folder (e.g. /data/Games/Emulation)
     and we search it recursively (bounded depth, skip huge game dirs/ROM extensions). Show results with checkboxes,
     already-configured ones marked; add the selected ones with the catalog profiles.
   - ROM folders per emulator (several per emulator): directory, profile, include subfolders, merge discs,
     auto-scan on refresh; actions Scan now (opens the existing scan preview/import dialog prefilled, or imports
     directly with a summary), Edit, Remove. Plus "Add ROM folder" and "Scan all folders now" with a result summary.
2. Auto refresh: the Collection refresh button (app/src/collection/header.rs `hdr-refresh` → CollectionView::reload)
   must first run `run_auto_rom_scanners` off the UI thread (spinner already exists), then reload the library, and
   toast "Added N new game(s)" when N>0. Also run it once at startup (keep/adjust the 90 s idle job — do it shortly
   after the library is on screen, off the UI thread, respecting HERONITE_DISABLE_IDLE_JOBS) and when a scanner is
   saved with auto-scan on. Mirror Heronite (check when it runs runAutoRomScanners) and also remove games whose ROM
   disappeared ONLY if Heronite does that (check; don't invent destructive behaviour).
3. Collection settings → Emulators tab: replace its content with a short summary + "Open emulator manager" button
   that navigates to the page (PageEvent::Navigate or a CollectionView hook), so there is one place to manage.
4. Fix in the "Add emulated games" dialog: the platform/profile select shows nothing useful when the profile's
   platform is "Other" — show the profile name; keep the dialog working as the scan/import step.
Owned: app/src/pages/emulators.rs (+ submodules), crates/emulation/** (detection, store additions), app/src/dialogs/
scan_roms.rs, app/src/dialogs/settings/emulators.rs, new dialogs app/src/dialogs/emulator_edit.rs etc. Hook points:
pages/mod.rs + collection/nav.rs (page registration), collection/mod.rs reload path + debug arms (small),
collection/header.rs refresh only if needed, collection/emulation.rs (existing idle-job glue).
Safety: tests against fixture copies only (HERONITE_HEROIC_DIR=/tmp/heronite-fixture-emu — copy /tmp/heronite-fixture,
which mirrors ~/.config/heroic JSON). Detection may READ the real filesystem (it's read-only), but never launch an
emulator or a game (the previous agent accidentally launched a PS2 game — never run --launch or press play; never
execute any detected binary, not even with --version, unless you are sure it can't open a window — prefer not to).
GUI only via scripts/shot.sh (workspace 7). Verify each UI state with captures and add debug hooks for dialogs/states.
