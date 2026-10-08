# Agent brief: merge the Collection settings dialog into the global Settings page

Read docs/agents/common.md (rules + SAFETY), docs/agents/glass-common.md (design spec) and the "Glass design system"
section of docs/UI-CONVENTIONS.md. Never modify docs/design/.

## Context
- The header gear button (app/src/collection/header.rs ~:324, `open_settings`) opens the Collection settings DIALOG
  (app/src/dialogs/settings/: tabs Appearance, Screenshots, Metadata, Emulators, Backup, Saves).
- The ☰ Settings PAGE (app/src/pages/settings.rs + app/src/pages/settings/**) has sections General, Game defaults,
  Library, Downloads, Advanced, Logs, About, with `request_section` / `PendingSection` (used by the Ctrl K palette).
- The user wants ONE settings place: move the dialog's content into the Settings page, organised well, and remove
  the header gear button.

## What to implement
1. Settings page sections, with the left nav grouped under small captions (glass caption style):
   - **App**: General, Appearance, Advanced
   - **Games**: Game defaults, Downloads, Library (hidden games, favourites)
   - **Collection**: Metadata, Screenshots, Emulators*, Backups (Ludusavi saves + Heroic config backup as two cards
     of one section)
   - **Help**: Logs, About
   *Emulators: look at what the dialog's Emulators tab contains. If it duplicates the ☰ Emulators page, move whatever
   is unique into the Emulators page (app/src/pages/emulators.rs) and do not add a Settings section; if it is global
   emulator/ROM-scan preferences, keep it as a Settings section. Explain the choice in your report.
   Each section uses the page's existing glass layout (title + lead + captioned cards of rows). Reuse the tab code
   (move it from app/src/dialogs/settings/<tab>.rs into app/src/pages/settings/<section>.rs, adapting the state and
   save plumbing to the page); keep every setting and every write identical (same files, same keys, same format).
2. Remove the Collection settings dialog (`SettingsDialog`) and the header gear button. Redirect every caller to the
   page section instead (navigate to Settings + `request_section`): app/src/collection/metadata_ui.rs (Metadata),
   app/src/collection/emulation.rs, the debug hooks in app/src/collection/mod.rs (`HERONITE_DEBUG_DIALOG=settings`,
   the screenshots variant, `HERONITE_DEBUG_SETTINGS_TAB`) -> map them to `HERONITE_DEBUG_PAGE=settings:<section>`.
   Check the context menu and anything else that referenced the dialog (`grep -rn "dialogs::settings" app/src`).
3. Update the Ctrl K palette's Settings section list (app/src/collection/palette/sources.rs) with the new sections
   and their keywords (e.g. "screenshots", "igdb", "ludusavi", "backup", "blur").
4. Keep the header's other buttons unchanged. The Collection must look exactly the same otherwise.
5. Update docs/UI-CONVENTIONS.md (settings tabs paragraph) and PROGRESSO.md.

## Files you own
app/src/pages/settings.rs, app/src/pages/settings/**, app/src/dialogs/settings/** (to move/delete),
app/src/dialogs/mod.rs (module list), app/src/collection/{header,metadata_ui,emulation}.rs, the settings-related
debug hooks in app/src/collection/mod.rs, app/src/collection/palette/sources.rs, app/src/pages/emulators.rs (only if
the Emulators tab content moves there).

## Verification
shot.sh captures (fixture copy with ludusavi/screenshot capture neutralised and IGDB keys blanked) of every new
section and of the Collection header without the gear. Unit tests still pass; full suite at the end. Commit with
explicit pathspecs.
