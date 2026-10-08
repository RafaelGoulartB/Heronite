# Glass redesign — shared spec (all glass agents)

Read docs/agents/common.md first (rules + SAFETY). Then this file.

The user chose **option 1 · Vidro** of the design proposals, plus the **command palette (Ctrl K)** of option 3.
References (look at them, they are the target):
- docs/design/propostas-v1-telas/o1-0.jpg (☰ menu), o1-1.jpg (global Settings), o1-2.jpg (game settings drawer),
  o1-kit.jpg (components), o3-0.jpg (palette layout — but drawn with the glass tokens below).
- Exact CSS of the mockups: docs/design/fonte/style.css, section "OPTION 1 — Vidro" (and "OPTION 3" for the palette).
- NEVER modify or delete anything under docs/design/ (user wants it as reference).

## Scope
Change menus, submenus, dropdowns, context menus, modals/dialogs, pages (☰ pages) and settings.
**Do NOT change the Collection itself**: the game list/cards, the header bar layout, the focus panel (game details,
screenshots, play button, sessions), the stage background. Popovers/menus opened FROM the Collection (context menu,
sort/filter dropdowns, ☰ menu) DO get the glass look.
The goal is to stop looking like Heroic: no UPPERCASE outlined buttons, no underlined uppercase tabs, no square
checkboxes, no flat grey modal.

## Tokens (put them in app/src/theme.rs, never hard-code twice)
- Accent: existing `accent()` #a5dceb. Text on accent: #0c1a20.
- Glass surface fill rgba(22,27,36,.62) drawn OVER a blurred copy of the art (see "Blur" below); when there is no art,
  use rgba(22,27,36,.94). Border 1px rgba(255,255,255,.11). Top inner highlight 1px rgba(255,255,255,.08).
  Shadow 0 30px 80px rgba(0,0,0,.45).
- Radii: popover/menu 20 (small dropdowns/context menus 14), sheet/modal 26, drawer 24, card 16, tile 14, input 12,
  menu item 10, pill 999.
- Card (group of rows): fill rgba(255,255,255,.05), border 1px rgba(255,255,255,.07), radius 16, padding 4px 16px.
  Rows: padding 11px 0, separator 1px rgba(255,255,255,.06) between rows; label 14px regular #e8edf4;
  description 12px #8c97a8 under the label; control on the right.
- Section caption above a card: 12px #9aa6b6, letter-spacing .04em, sentence case, margin 14px 6px 6px.
- Toggle switch (replaces checkboxes): 40×24 pill; off rgba(255,255,255,.16), on = accent; knob 18px white circle
  with small shadow, 3px inset; animate knob (150ms) unless HERONITE_NO_ANIMATIONS. Disabled = 40% opacity.
- Buttons — sentence case, Cabin 13px, pill height 36, padding 0 16:
  secondary fill rgba(255,255,255,.09) + border rgba(255,255,255,.10), text #eef2f7;
  primary fill accent, text #0c1a20 weight 600, no border; ghost: no fill, text #9aa6b6;
  danger: fill rgba(191,97,106,.20), text #f2a5ad. Hover: +.05 white on fill. Disabled 45% opacity.
  Small variant height 30.
- Segmented tabs (replace underlined tabs): container pill rgba(0,0,0,.28), padding 4, gap 2; item padding 6×12,
  radius 999, 13px #b5bfcc; active item fill rgba(255,255,255,.14), text white. Scroll horizontally if too wide.
- Inputs/selects: height 40, radius 12, fill rgba(0,0,0,.25), border 1px rgba(255,255,255,.08), text 14px #e8edf4
  (body font, not Rubik 16 any more); placeholder #6f7a8a; leading icon #9aa6b6; trailing text action in accent
  (e.g. "Change"); focus = border accent at 60%. Select = same box + chevron; its option list = glass popover.
- Side navigation (settings sections): pill items padding 9×12, radius 999, 14px #c3cbd8, icon 16;
  active fill rgba(165,220,235,.16), text white, icon accent.
- Titles: page/sheet title Rubik 600 26px; drawer/dialog title Rubik 600 22px; kicker above a title 11px accent,
  UPPERCASE, letter-spacing .08em (the only uppercase text); lead text 13px #9aa6b6.
- Close button: 32px circle rgba(255,255,255,.08), X icon 16px, hover .14.
- Backdrop behind modals/drawers: the Collection stays visible, darkened by rgba(0,0,0,.55)
  (sheets that cover most of the screen: blurred art instead, like o1-1.jpg).
- kbd hint chip: mono-ish 11px, padding 1×6, radius 5, fill rgba(255,255,255,.08), border rgba(255,255,255,.12).

## Blur (GPUI has no backdrop-filter)
The stage already renders the selected game's art pre-blurred on the CPU (`app/src/collection/stage.rs`,
`Variant::Stage { blur_px, saturation }` in app/src/collection/mod.rs ~:770). The glass look = draw that
blurred art (a stronger blur, e.g. 40px, at reduced resolution to keep memory low — the project goal is low memory)
behind the glass panel, clipped to the panel's rounded bounds, then the translucent fill on top. Cache it like the
stage variant; never blur per frame. If the art is not ready, fall back to the solid fill. Measure memory before/after
(scripts/shot.sh prints PSS; compare `Pss_Anon` with /tmp/memcmp2.sh <binary>) and keep the increase small (< ~10 MB).

## Process
- Keep public APIs of app/src/ui stable where possible so callers restyle automatically (e.g. `checkbox()` renders a
  toggle row; `button(kind)` maps old kinds to the new variants; `tabs()` renders segmented pills; `dialog()` renders
  the glass modal). Add new helpers instead of breaking signatures; if you must change a signature, fix all callers in
  the same commit.
- Verify with scripts/shot.sh (fixture dir, HERONITE_DEBUG_PAGE / HERONITE_DEBUG_DIALOG / HERONITE_DEBUG_DROPDOWN /
  HERONITE_DEBUG_MENU hooks; add hooks if needed) and compare against the reference JPGs. Also capture the main
  Collection screen once to prove it did not change.
- Update docs/UI-CONVENTIONS.md: replace the Heroic-look guidance for dialogs/menus with a "Glass design system"
  section listing the helpers (keep the Collection guidance).
