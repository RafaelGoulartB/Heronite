# Your brief: EPIC PREREQUISITES + MODIFY INSTALL (agent "epic2")

Context: Epic support exists (crates/epic, app/src/services/downloads.rs, app/src/collection/epic.rs,
app/src/dialogs/epic_*.rs, pages/epic.rs, pages/downloads.rs) and the launch pipeline has Wine/Proton preparation
(crates/launch/src/prepare.rs, tools.rs, runtimes.rs, dxvk.rs, legendary.rs; app/src/runtime.rs). Read PROGRESSO.md
sections "Epic" and the launch one first.

Implement, porting Heroic exactly (HeroicGamesLauncher/src/backend/...):
1. Epic prerequisites before the first launch of a game in its prefix — Heroic's `legendarySetup`
   (src/backend/storeManagers/legendary/setup.ts) and whatever it calls: Epic redistributables / prerequisites
   declared by the game (legendary's install info `prereq_info` / `legendary install --... ` / redist path
   tools/redist/legendary — see constants.ts epicRedistPath), the per-prefix marker Heroic writes so it runs once
   (e.g. "installed_games" file in the prefix), EOS overlay (src/backend/storeManagers/legendary/eos_overlay/**:
   install/update/enable/disable per prefix via `legendary eos-overlay ...`) with UI: on the Epic page an "EOS Overlay"
   section (install/update/remove, and enable/disable for a game's prefix) like Heroic, and a per-game toggle where
   Heroic puts it (game settings → check where; our game settings dialog is app/src/dialogs/heroic_settings/**, add a
   small block there if needed).
2. known_fixes (src/backend/known_fixes or launcher.ts installFixes / runWinetricks; the file
   ~/.config/heroic/known_fixes.json / fixes/ dir exists): apply the per-game fixes Heroic applies (winetricks verbs,
   env variables are already handled in crates/launch/src/legendary.rs known_fixes_env — check), once per prefix with
   Heroic's marker, as part of the launch preparation (status toasts "Installing fixes…"). Also check whether Heroic
   downloads/updates known_fixes.json (and from where); mirror it (into the shared file, same format).
3. Modify install (Heroic: src/frontend/screens/Game/ModifyInstallModal/** + backend games.ts modify / install with
   new SDL tags / DLC selection): a dialog to change installed DLCs and selective-download components (SDL) of an
   installed Epic game; applying it queues the right legendary operations (install/uninstall DLC, `legendary install
   --repair`/re-install with new --install-tag set, exactly as Heroic) through the existing download queue.
   Entry point: context-menu item "Modify installation" for installed Epic games that have DLCs or SDL options
   (keep it in your existing Epic menu block in app/src/collection/epic.rs).
Testing: fake `legendary` scripts and fake prefixes (tempdirs) only; never run real legendary write commands, real
winetricks, wine or a game; read-only legendary queries against the REAL legendary config are allowed (never on a copy
— token rotation). Fixture data dir for app runs. Screens via scripts/shot.sh (workspace 7) with debug hooks.
Owned: crates/epic/**, crates/launch/** (prepare/legendary parts), app/src/dialogs/epic_*.rs (+ new
epic_modify.rs), app/src/collection/epic.rs, app/src/pages/epic.rs, app/src/services/downloads.rs.
Another agent ("settings2") works in parallel on a global Settings page, hidden games and app/src/dialogs/settings/**;
avoid those files. Hook points: app/src/dialogs/mod.rs (register), app/src/dialogs/heroic_settings/** only a small
block for the EOS toggle if Heroic has it per game, app/src/collection/mod.rs debug arms.
