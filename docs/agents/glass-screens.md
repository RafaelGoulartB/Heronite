# Glass redesign — phase 2: apply the design system to every screen

Read docs/agents/common.md, docs/agents/glass-common.md (spec) and the "Glass design system" section of
docs/UI-CONVENTIONS.md (the helpers the foundation agent built — USE THEM, do not restyle by hand; if a helper is
missing, add it to app/src/ui in a small, generic way and list it in your report). Targets:
docs/design/propostas-v1-telas/o1-*.jpg. Never modify docs/design/. Three agents run in parallel, one per section
below; edit only the files of your section (plus generic additions to app/src/ui announced in your report —
re-read ui files before editing, others may add helpers too).

Common rules for all three:
- Replace remaining Heroic-isms in your files: uppercase labels/buttons, outlined buttons, underlined tabs, square
  checkboxes, hand-written grey panels, ad-hoc colours (use theme tokens).
- Group related settings into captioned cards of rows (label + description left, control right), like o1-1.jpg.
- Don't change behaviour or data writes. Keep every existing control reachable. Keep debug hooks working.
- Verify each screen with scripts/shot.sh against a fixture copy (neutralise ludusavi/screenshot capture first) and
  look at the PNGs. Check long content scrolls, narrow window (e.g. 1100px) still works.
- Full test suite at the end; commit with explicit pathspecs; add a short PROGRESSO.md note.

## Section A — "glass-settings": global Settings + Collection settings
Files: app/src/pages/settings.rs, app/src/pages/settings/**, app/src/dialogs/settings/** (the Collection settings
dialog opened from the header gear: appearance, backup, emulators, metadata, saves, screenshots).
- ☰ Settings page = glass sheet floating over the blurred art (inset like o1-1.jpg), left column 230px with the title
  (gear icon + "Settings") and nav pills (General, Game defaults, Library, Downloads, Advanced, Logs, About...),
  content on the right with page title + lead + captioned cards. Round close button returns to the Collection.
- The Collection settings dialog: same visual language (glass modal, nav pills or segmented tabs, cards of rows).
- "Game defaults" reuses the heroic_settings panels (owned by section B) — coordinate: only wrap them, don't edit them.

## Section B — "glass-game": game settings drawer
Files: app/src/dialogs/game_settings.rs, app/src/dialogs/heroic_settings/**, app/src/dialogs/{art_images,
web_image,screenshot_art,metadata,metadata_wizard,launch_state}.rs.
- The per-game settings ("<Game> (Settings)", tabs Images / Details / Wine / Other / Advanced / Cloud saves /
  Gamescope / Launch...) becomes the right-side **drawer** (ui drawer helper): header with the game cover (52×70,
  radius 10), kicker "Game settings", title, meta line (store · Installed · size, when known); segmented tabs below;
  scrollable body of captioned cards; footer with "Use defaults" (ghost, where a reset exists) and "Done" (primary).
  The Collection stays visible (dimmed) on the left, like o1-2.jpg.
- heroic_settings panels are also used inside Settings → Game defaults (section A wraps them): keep them
  container-agnostic (no drawer assumptions inside the panels).
- Images tab: cover/background previews in cards with pill buttons (Change / Search web / Screenshot / Reset).

## Section C — "glass-pages": ☰ pages and all other dialogs
Files: app/src/pages/{downloads,emulators,epic,wine_manager}.rs, app/src/pages/mod.rs (page chrome helpers only),
and every dialog not owned by A or B: app/src/dialogs/{add_game/**, emulator_confirm, emulator_detect,
emulator_edit, epic_confirm, epic_import, epic_install, epic_modify, log_viewer, ludusavi_backup, manage_statuses,
pick_rom, playnite_import, playnite_merge, playtime, quit_confirm, remove_game, rom_folder, scan_roms}.rs.
- Pages: glass sheet over blurred art like Settings (title + lead, content in cards, lists as card rows with pill
  actions, progress bars rounded with accent). The slim page header from collection/nav.rs stays (foundation owns it).
- Dialogs: glass modal; confirms are compact (title, text, pill buttons right-aligned, primary/danger as fits).
