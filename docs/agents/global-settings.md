# Your brief: GLOBAL SETTINGS PAGE + HIDDEN GAMES (agent "settings2")

User request: the "Settings" entry of the ☰ menu must be the app's GLOBAL configuration (today it opens the same
Collection settings dialog as the header gear). Put there "show/unhide hidden games" and the other global settings
that make sense. The header gear keeps opening the Collection settings dialog (Appearance, Screenshots, Metadata,
Emulators, Backup, Save backup).

Build:
1. `Page::Settings` (app/src/pages/settings.rs + submodules; register in pages/mod.rs and collection/nav.rs, replacing
   the current ☰ "Settings" item which opens the dialog; HERONITE_DEBUG_PAGE=settings[:section]). Layout: left
   section list + right content (or stacked sections), our style. Sections (port labels/help from Heroic's
   src/frontend/screens/Settings/sections/{GeneralSettings,GamesSettings,AdvancedSettings,LogSettings,SystemInfo} and
   components, en locale):
   - General: move here what is today in Collection settings → "General" (app/src/dialogs/settings/heroic.rs /
     app/src/dialogs/heroic_settings/general.rs: default install path, default wine prefix dir, Steam path, tray /
     exitToTray / startInTray / minimizeOnLaunch, shortcuts toggles, protocol launch hiding, etc.).
   - Game defaults: move the "Defaults" tab (Wine/Other/Advanced/Gamescope defaults bound to config.json) here.
   - Library: "Show hidden games" (toggle that makes hidden games appear in the Collection, greyed with a "Hidden"
     badge, like Heroic's library "Show hidden" filter — store the flag where it fits: our ui-state or config; if
     Heroic stores it, use the same key), a list of hidden games (config.json games.hidden — check the real
     structure) with Unhide per game and "Unhide all"; also favourites list management and "Max recent games"
     (maxRecentGames) and "Clear recent".
   - Downloads/Epic: maxWorkers, alternative legendary binary (altLegendaryBin), autoUpdateGames,
     checkForUpdatesOnStartup, downloadNoHttps, ignore game updates per Heroic semantics if global.
   - Advanced: open the config folder / logs folder, clear cache (Heroic ClearCache: which caches — mirror only the
     safe ones: store_cache library files are rebuilt by refresh; images cache), disable logs / verbose logs,
     experimental features toggles only if they affect our app.
   - About/System info: app version, data dirs, legendary version, Wine/Proton found count, GPU/driver info if cheap
     (read /proc + vulkan/nvidia-smi only if it doesn't spawn GUI).
   Remove the moved tabs (General, Defaults) from the Collection settings dialog
   (app/src/dialogs/settings/mod.rs Tab enum) and keep their code reused (don't duplicate components).
2. Hidden games in the Collection: when "Show hidden games" is on, hidden games are included in groups and search,
   rendered greyed with a small "Hidden" badge; the context-menu item reads "Unhide" for hidden games (check the
   existing "Hide" item in app/src/collection/context_menu.rs — keep your edit small; another agent edits its own Epic
   block there). Also a quick toggle in the header install-filter dropdown ("Show hidden") if it fits naturally.
   Data: crates/data collection building (crates/data/src/collection.rs / library.rs filter hidden games) — add a flag.
Owned: app/src/pages/settings*, app/src/dialogs/settings/**, app/src/dialogs/heroic_settings/general.rs (+ moving
code), crates/data (hidden filter, ui_state flag), app/src/collection/{header.rs,card.rs,data.rs,vm.rs} for the hidden
rendering/filter. Hook points: pages/mod.rs, collection/nav.rs, collection/mod.rs (small), context_menu.rs (small).
Another agent ("epic2") works in parallel on Epic prerequisites/modify install (crates/epic, crates/launch,
dialogs/epic_*, collection/epic.rs, pages/epic.rs, services/downloads.rs) — avoid those files.
Testing: fixture copies only (HERONITE_HEROIC_DIR=/tmp/heronite-fixture-settings2, copy /tmp/heronite-fixture);
verify config.json/ui-state writes keep format and unknown keys; captures via scripts/shot.sh (workspace 7) of every
section and of the Collection with hidden games shown.
