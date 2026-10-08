# Agent brief: glass design system foundation + ☰ menu

Read docs/agents/common.md, then docs/agents/glass-common.md (the spec). Another agent ("palette",
docs/agents/glass-palette.md) builds the Ctrl K command palette in parallel. Later agents will restyle each screen
using what you build, so build the shared pieces well and document them.

## What to implement
1. **Tokens** in app/src/theme.rs (glass fill/border/highlight/shadow, radii, card, caption, toggle, button variants,
   segmented tabs, inputs, nav pill, kbd). Keep old tokens used by the Collection.
2. **Glass backdrop**: a reusable way to paint the blurred art behind a glass panel (see "Blur" in the spec): a global
   or CollectionView-owned blurred art image (strong blur, reduced resolution, cached per selected game/art), plus a
   helper to draw it clipped under a panel and the fallback solid fill. Also the full-screen variant for big sheets.
   Measure memory before/after and report it.
3. **Primitives in app/src/ui** (restyle in place, keep signatures where possible):
   - `controls::checkbox` -> toggle switch row (label + optional description + switch on the right); `mini_checkbox`
     -> small switch; new `toggle(id, on, disabled)` and `toggle_row(id, label, description, on)`.
   - `controls::button` / `action_button` / `ButtonKind` -> pill buttons (primary / secondary / ghost / danger,
     sentence case — remove any uppercase transform); small size.
   - `controls::tabs` -> segmented pill tabs.
   - `controls::section_title`, `page_heading`, `help_text` -> new caption / title / lead styles.
   - New: `glass_panel` (surface with backdrop), `card` (group of rows with separators), `row(label, desc, control)`,
     `caption(text)`, `nav_pill(id, icon, label, active)`, `kicker(text)`, `kbd(text)`, `close_button(id)`.
   - `dialog::dialog` -> glass modal (radius 26, padding 24–28, title Rubik 22, round close button, backdrop
     darkening the Collection). Add a `drawer(...)` variant (right side, inset 10px, width param, radius 24, header
     slot, footer slot) that the game-settings agent will use.
   - `dropdown::{menu_panel, menu_item, menu_separator, dropdown}` -> glass popover (radius 14, item radius 10,
     hover rgba(255,255,255,.07), active accent tint). This restyles the Collection's context menu and header
     dropdowns automatically — check app/src/collection/context_menu.rs and header.rs still look right
     (context_menu.rs has its own panel styling: move it onto `menu_panel` if needed — that file is allowed).
   - `fields::*` (input_box, select_field, path_field, search_box, textarea, tags) -> new input look.
4. **☰ menu** (app/src/collection/nav.rs `nav_dropdown`, and header.rs only where needed): glass popover 330px wide,
   2-column tile grid: Collection, Downloads, Wine Manager, Epic Games, Emulators, Add game (dashed tile) with a
   one-line live meta each (games count; active downloads "1 active · 42%" or "Idle"; default Wine/Proton name;
   "Signed in"/"Not signed in"; emulator count) — only from state already in memory, no I/O in render. Bottom row:
   Settings with kbd hint "Ctrl ,". Active page tile highlighted. Keep the current behaviour (hover-open, navigation).
   Add the palette entry if the palette agent has landed (`Ctrl K` hint) — otherwise leave a TODO comment.
5. Update docs/UI-CONVENTIONS.md ("Glass design system" section) and PROGRESSO.md.

## Files you own
app/src/theme.rs, app/src/ui/**, app/src/collection/nav.rs, app/src/collection/header.rs (dropdown/menu parts only),
app/src/collection/context_menu.rs (styling only), app/src/collection/mod.rs + stage.rs (only for the blurred
backdrop plumbing — keep the stage rendering unchanged), docs/UI-CONVENTIONS.md.
Callers in dialogs/pages may need small fixes when you change a signature — allowed, keep them minimal.

## Verification
shot.sh captures (fixture): ☰ menu open (add a `HERONITE_DEBUG_DROPDOWN=nav` hook if missing), a dialog
(HERONITE_DEBUG_DIALOG=...), the Settings page (HERONITE_DEBUG_PAGE=settings), the context menu
(HERONITE_DEBUG_MENU), and the main Collection screen unchanged. Compare with docs/design/propostas-v1-telas/o1-*.jpg.
Full test suite at the end. Commit with explicit pathspecs.
