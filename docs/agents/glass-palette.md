# Agent brief: command palette (Ctrl K)

Read docs/agents/common.md, then docs/agents/glass-common.md (the spec). Layout reference:
docs/design/propostas-v1-telas/o3-0.jpg (option 3 palette) — but drawn with the GLASS tokens (option 1), not
option 3's colours. Another agent ("foundation", docs/agents/glass-foundation.md) is building the glass primitives
(app/src/ui, theme.rs) in parallel: do not edit those files. Start with your own local styling that follows the spec;
when the foundation commits its helpers (glass_panel, kbd, caption...), switch to them (re-check `git log` now and
then).

## What to implement
- **Open/close**: Ctrl+K anywhere in the main window (also when a ☰ page is shown); Esc closes; clicking outside
  closes. While a dialog is open, Ctrl+K does nothing. Also expose `CollectionView::open_palette` so the ☰ menu can
  offer it.
- **Panel**: glass, 620px wide, centered, ~90px from the top. Input row (search icon, text, placeholder
  "Type a command, page or game…", Esc kbd). Results grouped with captions; each row: icon, title, muted subtitle,
  kbd shortcut on the right; selected row = accent tint + 2px accent bar on the left. Footer hints
  (↑↓ move, ↵ open, Tab filter by type). Max ~9 visible rows, scroll the rest.
- **Sources**:
  - Go to: Collection, Downloads, Wine Manager, Epic Games, Emulators, Settings (+ each Settings section:
    General, Game defaults, Library, Downloads, Advanced, Logs, About — open the page on that section).
  - Actions: Add game…, Refresh library (same as header refresh, incl. ROM scan), Scan ROMs, Check for Epic updates,
    Open logs folder, Quit. Reuse existing code paths/events (PageEvent, header actions, system quit) — do not
    duplicate logic.
  - Games: every library game by title (and alt names if cheap). Choosing a game selects/focuses it in the Collection.
    Secondary verbs typed or offered as sub-results for the top game match: "Play <game>", "<game> settings"
    (opens the game settings dialog), "Open <game> folder" if installed. Playing must go through the normal launch
    path (never launch anything in tests — use HERONITE_DEBUG_FAKE_LAUNCH).
  - Empty query: recent games (last played) + the Go to list.
- **Matching**: case/diacritic-insensitive fuzzy match (prefix and word-start boosts), stable ranking, fast for
  ~2000 games (no allocations per keystroke beyond the result list). Tab cycles a type filter (All / Pages /
  Actions / Games).
- **Shortcuts** shown in the palette must actually work globally: Ctrl+1..5 pages (Collection, Downloads, Wine
  Manager, Epic, Emulators), Ctrl+, Settings, Ctrl+N Add game, F5 refresh. Don't break existing keys (Esc handling,
  screenshot viewer keys, text inputs typing).

## Files you own
New: app/src/palette/** (or app/src/collection/palette.rs + submodules). Hooks: app/src/collection/mod.rs (key
handling + rendering the overlay — keep edits small, the foundation agent may touch other parts of this file;
re-read before editing), app/src/collection/nav.rs ONLY to add the menu entry after the foundation agent finished
with it (coordinate via git log; if it is not done when you finish, report the one call to add).

## Verification
Unit tests for matching/ranking and source building. shot.sh captures with a debug hook (e.g.
HERONITE_DEBUG_PALETTE="wi" opens the palette with that query) for: empty query, "wi", a game title. Full test
suite at the end. Commit with explicit pathspecs.
