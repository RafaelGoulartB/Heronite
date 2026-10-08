# Agent brief: split "game artwork background" into Collection background and Interface background

Read docs/agents/common.md (rules + SAFETY), docs/agents/glass-common.md (design spec) and the "Glass design system"
section of docs/UI-CONVENTIONS.md. Never modify docs/design/.

## Context
Settings → Appearance (app/src/pages/settings/appearance.rs) has "Use the game artwork as page background" + "Blur".
Today that one setting drives both the Collection stage background (app/src/collection/stage.rs, `Variant::Stage`)
and, indirectly, the glass surfaces of the interface (app/src/ui/glass.rs uses the blurred art set by
`CollectionView::glass_image`; tokens `glass_fill` .84 / `glass_art_dim` .5 / page backdrop .55 in app/src/theme.rs).
Find out exactly where the existing values are stored (it is shared with Heronite — keep its key, format and meaning
for the Collection).

## What the user wants
Two independent groups in Appearance, each with its own on/off and its own sliders, and a live preview:
1. **Collection background** (the selected game's art behind the Collection list and game details only):
   - On/off (the existing setting — same key, same meaning, Heronite-compatible).
   - Blur (existing) and **Opacity** (how visible the art is; add if missing; default = current look).
2. **Interface background** (menus, ☰ menu, context menus, dropdowns, dialogs/modals, drawer, Settings and other
   pages, command palette):
   - Mode: **Transparent** (glass: blurred game art visible through the panels) or **Solid** (opaque panels in a solid
     colour, no art). Default Transparent with today's values.
   - Transparent: **Opacity** slider for the panels (e.g. 50–100 %, default 84 %, maps to `glass_fill`; scale the art
     dim consistently so 100 % = fully opaque) and **Blur** of the art behind panels.
   - Solid: panels use the solid glass colour (`glass_fill_solid`-like, fully opaque); optionally offer 3–4 preset
     solid tints (dark slate default) if cheap — not required.
   - It must work independently: Collection background OFF + Interface Transparent still shows the art behind
     panels (generate the glass blurred variant even when the stage art is disabled); Collection ON + Interface Solid
     shows art only in the Collection.
   These interface settings are specific to the Rust app: store them in the heronite-rs config dir (AppPaths, e.g. a
   small JSON next to ui-state.json, through json_store), NOT in ~/.config/heroic.
- Replace the hard-coded token values with runtime values (a Global read by theme/glass helpers); no I/O in render.
- Changes apply live (sliders preview immediately, saved debounced/on release).
- Memory: when Interface = Solid and Collection = off, do not keep/generate blurred art textures.

## Files you own
app/src/pages/settings/appearance.rs, app/src/theme.rs, app/src/ui/glass.rs (and small changes in app/src/ui/* if
needed), app/src/collection/{mod.rs (glass_image / backdrop plumbing only), stage.rs}, the settings model in
crates/data if the Collection opacity needs a field there (keep Heronite format: check how Heronite stores the
Collection appearance settings in ../HeroicGamesLauncher/src/**/local-library or similar and reuse its key if it
already has an opacity; otherwise store the new Collection opacity in the heronite-rs config too), app/src/pages/settings.rs
(palette keywords), docs/UI-CONVENTIONS.md, PROGRESSO.md.

## Verification
Unit tests for the settings (defaults, load/save, clamping). shot.sh captures (fixture copy, ludusavi/screenshot
capture neutralised) of: Appearance section; Collection with art ON + Settings page Transparent; art OFF + Transparent;
art ON + Solid; and a dialog in Solid. Debug env hooks to force the modes are fine. Check memory with /tmp/memcmp2.sh in
Solid+off vs default. Full test suite. Commit with explicit pathspecs.
