# UI conventions (app/)

The Collection screen (list, cards, header bar, focus panel, stage) must look
like Heronite's Collection: reference captures in `docs/reference/`, sources in
`../HeroicGamesLauncher/src/frontend/screens/LocalLibrary/**`. Everything that
floats over or replaces it (menus, dropdowns, context menus, dialogs, drawers,
pages, settings) follows the **Glass design system** below
(`docs/design/propostas-v1-telas/o1-*.jpg` are the visual target; never edit
`docs/design/`).

## Stack
- GPUI through `gpui-kit = 0.7.1` (`use gpui_kit::*; use gpui_kit::prelude::*;`).
  Sources: `~/.cargo/registry/src/*/gpui-pre-0.3.8`, `gpui-component-0.7.1`,
  `gpui-base-0.7.1`. Read them when unsure of an API.
- Reuse from gpui-component only behaviour-heavy widgets: `input::{Input, InputState}`
  (text fields), `slider::{Slider, SliderState}`. Style everything else by hand.

## Layout of the code
- `app/src/theme/` — tokens (`color`, `font`, `layout`, `glass`: nord-dark colours, fonts, status colours,
  `W_BOLD` = CSS 600/700, `W_500` = CSS 500, and the glass tokens: fills,
  borders, radii `R_*`, text colours, toggle / button / segmented / input
  values). Never hard-code a new colour twice; add it here.
- `app/src/ui/` — building blocks: `icon`, `tool_button`, `round_button`,
  `chip`, `scrollbar`, `dropdown::{dropdown, menu_panel, menu_item,
  menu_separator}`, `dialog::{dialog, drawer}`, `glass::*`,
  `controls::*` (see "Glass design system").
- `app/src/ui/fields/` — the ONE set of form helpers; import from
  `crate::ui::fields` only (never write a local label/input/select look):
  - inputs: `new_input` / `new_textarea` (state), `input_box(&input, disabled)`,
    `textarea_box`, `text_field(label, &input)`, `path_field` (clear/browse
    icon) + `choose_path` (file chooser), `search_box`, `on_commit`
    (blur/Enter commit, for debounced or deferred saves);
  - `labeled(label, control)` for any control under a label (an input next
    to a button, a select...), `field_label`, `paragraph`,
    `meta_text`, `panel`;
  - `select_field(id, label, value, disabled, open, options, cx, on_toggle,
    on_pick)`: label + 40px select + its open menu (the caller keeps the
    `open` flag and the picked value);
  - `TagField` (entity: chips + autocomplete, emits `TagEvent::Changed`).
  Inputs/selects/tags share `input_surface` (glass input: 40px, radius 12,
  `rgba(0,0,0,.25)` fill, 1px border, 14px body font; accent ring at 60% while
  focused).
- `app/src/collection/` — the main screen (`mod.rs` = `CollectionView`).
- `app/src/dialogs/<name>.rs` — one dialog per file. A dialog is an
  `Entity<V>` where `V: Render + EventEmitter<DialogEvent>`; build it with
  `DialogCtx` (paths + `Arc<Library>`). Open it from `CollectionView` with
  `self.open_dialog(cx.new(|cx| V::new(ctx, window, cx)), cx)`.
  Emit `DialogEvent::Changed` after writing data, `Close` to close.
- Settings: there is ONE place, the ☰ Settings page (`app/src/pages/settings/`, Ctrl ,). The left nav is grouped under captions
  (`Section::GROUPS`): App (General, Appearance, Advanced), Games (Game defaults, Downloads, Library),
  Collection (Metadata, Screenshots, Backups), Help (Logs, About). A section = title + lead (`Section::blurb`) +
  captioned cards of rows. The Collection sections live in `app/src/pages/settings/<section>.rs`
  (`appearance`, `metadata`, `screenshots`, `saves` + `backup` for Backups) with `State::new(&PageCtx, &EffectiveSettings, …)`
  and `render(&mut SettingsPage, …)`; save with `heronite_data::settings::*` and call `d.saved(result, cx)`
  (updates the page copy of `settings.json` and emits `PageEvent::LibraryChanged`). Work that spawns a process or
  reads folders (Ludusavi detection, screenshot cache size) starts in `SettingsPage::on_section_shown`, never at page
  creation. Open a section from code with `CollectionView::open_settings_section` (or `pages::settings::request_section`
  + navigate); add new sections to `Section` (label, icon, blurb, palette `keywords`, `parse`). Emulators and ROM
  folders are NOT a Settings section: they live on the ☰ Emulators page.

## Glass design system (menus, dialogs, pages, settings)

Spec: `docs/agents/glass-common.md`. Everything below keeps its old name/signature
where one existed, so a caller picks the new look automatically.

**Backdrop.** GPUI has no backdrop-filter. `CollectionView::glass_image`
(collection/mod.rs) asks the image pipeline for `Variant::Glass` of the selected
game's art (blur 30px, saturate 1.5, ~1/5 of the window, cached in the
`ImageStore` like the stage; costs ~+1 MB PSS) and publishes it every frame with
`ui::glass::set_backdrop`. Glass surfaces paint it aligned to the window and
clipped to their rounded bounds (`Window::paint_image` with a sub-rect), under a
dark dim (`theme::glass_art_dim`) and the translucent fill
(`rgba(22,27,36,.84)` by default). No art (no game selected, Solid) or still
loading: nearly opaque fill (`.94`). Never blur per frame; never request another
blur: reuse this one.

**Interface vs Collection appearance** (Settings → Appearance, two independent
groups). *Collection background* = the stage art behind the list/details: on/off,
blur, darkening, colour, edge shading in Heronite's `local_library/settings.json`
(`background`), plus Opacity (`collectionOpacity`, default 100). *Interface
background* = every glass surface (menus, ☰, context menus, dropdowns, dialogs,
drawer, pages, palette): `interfaceMode` `transparent` (default) | `solid`,
`interfaceOpacity` 50-100 (default 84, maps to `glass_fill`; the art dim rises
from .5 to 1 above 84 so 100 is fully opaque), `interfaceBlur` 0-60 in steps of 5
(default 30), `interfaceSolidTint` slate (default) | graphite | midnight | plum.
These live in this app's `appearance.json` (`AppPaths::appearance()`, next to
`ui-state.json`; `heronite_data::appearance::Appearance`), NOT in ~/.config/heroic.
At runtime `theme::appearance()/set_appearance()` (a process-wide value) feeds
`theme::{glass_fill, glass_fill_solid, glass_art_dim, page_backdrop,
glass_blur_px, glass_shows_art}`; never hard-code those numbers. The glass
backdrop is generated whenever `glass_shows_art()` (Transparent and < 100%),
independent of the Collection art; in Solid or at 100% nothing is generated and
`ImageStore::forget_prefix("g:")` frees the cached texture (same for the stage
texture "s:" when the Collection art is off). Pages sit on `theme::page_backdrop()`
(opaque page colour when Solid). Sliders call `set_appearance` live and save on
release (off the UI thread).

**Surfaces** (`ui::glass`): `glass_panel(radius)` (big shadow; popovers 20, sheets
26, drawers 24), `glass_menu_surface(radius)` (menus/dropdown lists, radius 14). Children added after the constructor paint above the layers. Children
are not clipped to the rounded corners: give edge-to-edge children their own radius.

**Menus** (`ui::dropdown`): `menu_panel(min_w)`, `menu_item(id, active)` (radius 10,
14px, hover .07, active = accent tint), `menu_separator()`, `dropdown(..)`. The
Collection's context menu (`collection/context_menu.rs::item`/`paper`) uses the
same look; other context-menu rows must call `context_menu::item`.

**Dialogs** (`ui::dialog`): `dialog(DialogProps, body, footer, on_close)` = glass
modal (radius 26, title Rubik 22, round close button, Collection darkened by
`rgba(0,0,0,.55)`); `drawer(DrawerProps { id, width }, header, body, footer,
on_close)` = right-hand sheet inset 10px, radius 24, with fixed header/footer
slots (build the header with `kicker`, `dialog_title`, `close_button`).

**Controls** (`ui::controls`):
- `toggle(id, on, disabled)` 40x24 switch (spring-animated knob unless
  `HERONITE_NO_ANIMATIONS`), `toggle_row(id, label, desc, on)`, legacy
  `checkbox(id, checked, label)` (= toggle row) and `mini_checkbox` (30x18 switch).
  The caller adds `.on_click`.
- `button(id, kind, label)` / `button_sm` / `action_button(.., enabled, on_click)`:
  pill buttons, sentence case. `ButtonKind::Glass` = secondary, `Primary` = accent
  fill, `Ghost` = no fill, `Danger` = red tint. Disabled = 45%.
- `tabs(id, items, active, on_select)` = segmented pills (scrolls horizontally).
- `card(rows)` / `card_box()` + `row(label, desc, control)` (11px padding,
  separators between rows), `caption(text)`, `page_heading`
  (Rubik 26), `dialog_title` (Rubik 22), `help_text` (lead), `kicker` (the only
  uppercase text), `kbd("Ctrl K")`, `close_button(id)`, `nav_pill(id, icon, label, active)`.
- Fields (`ui::fields`): `path_field` shows a folder icon and "Clear"/"Browse" in accent;
  `select_field`'s list is a scrollable glass menu; labels are 12px muted.

**Game settings drawer** (`dialogs/game_settings/`): the first `drawer` caller. Tabs are
segmented pills in the header slot; each tab is a stack of `caption` + `card`. The Heroic
panels (`dialogs/heroic_settings`) build their cards with `parts::rows()` (`.child(toggle_row)`,
`.field(input)`, `.field_help(input, notes)`, `.card()`); they assume no container, so Settings →
Game defaults can host them too.

**Pages** (`pages/mod.rs`, ☰ pages other than Settings): `sheet(id, title, lead, actions, max_w, window, close, column)`
= glass sheet inset 12px under the slim header (title Rubik 26 + lead, optional header actions, round close =
`page_close(cx)` in the sheet's top-right corner, which navigates back to the Collection, then a scrolling body). Title
and body share one column of at most `max_w`, **centred**: never leave content stuck to the left of a wide window.
Use the width instead of a narrow strip: pick the layout from `content_width(window)` — `columns(wide, [a, b])` puts
blocks side by side (stacked when narrow), a `grid().grid_cols(n)` holds repeated cards (Emulators), and a fixed side
column + the main list suits browsers (Wine Manager). Check new pages at ~1000px and on an ultrawide. Fill the column
with `caption` + `card`/`card_box` + `row`. List helpers: `list_item(leading, content, trailing)` (card row with cover/icon),
`icon_action(id, icon, enabled)` (round 32px list button), `status_pill(text, colour)` / `tag_pill(text)`,
`caption_with(text, action)` (caption + action on the right), `empty_row(text)`. Progress bars: `services::widgets::progress_bar`.
Confirmations inside a page are inline rows of a card (text + ghost "Cancel" + danger button), not modals.
Statistics (`pages/stats.rs`, Ctrl 6) only reads: the Collection hands it the loaded library
(`StatsPage::set_library`, on navigate and after each reload) and the numbers come from `heronite_data::stats`
(sessions cut into hours; sessions under 2 min are failed launches and ignored). Charts are plain divs: bars stacked
in `theme::chart_series(i)` for the top 5 games of the period (`Stats::series`), the rest `theme::chart_other()`; a
game keeps its colour in every block of the page. Hovering a bar / heatmap cell updates the line above it (no tooltips).
`HERONITE_DEBUG_PAGE=stats:week|month|year|all` opens it on a span.

**Do not** use uppercase labels, outlined buttons, underlined tabs, square
checkboxes or flat grey modals in new UI. Colours come from `theme` (`text_primary`,
`text_muted`, `text_dim`, `accent_tint`, ...).

**Verifying**: captures with the app on a hidden workspace only show what was
drawn before the compositor stopped sending frames, so late images (art, covers)
are missing. Take a first capture to wake the window, wait a second, then take
the real one (a modified copy of `scripts/shot.sh` doing `grim -T` twice).
`HERONITE_DEBUG_DROPDOWN=nav|install|sort|status|facets`, `HERONITE_DEBUG_MENU=<key>`,
`HERONITE_DEBUG_DIALOG=...`, `HERONITE_DEBUG_PAGE=...`. Copy
`~/.config/heroic/images-cache` into the fixture or no art appears at all.

## Rules
- No I/O on the UI thread except tiny reads; use `cx.background_spawn` and
  update the entity when done. Never block render.
- Shared files (`~/.config/heroic`) are written only through
  `heronite_data::json_store` or the backend crates (`crates/*`), which keep
  the electron-store format Heronite reads.
- Images: request through `CollectionView::cover` / `ImageStore` (worker
  threads, LRU of GPU textures). For one-off rasters use
  `heronite_images::*` in `background_spawn` and wrap with
  `RenderImage::new(smallvec![image::Frame::new(decoded.into_image_buffer())])`.
- Icons: SVGs in `app/assets/icons` (MUI/FontAwesome names). Add new ones by
  extracting from `../HeroicGamesLauncher/node_modules/@mui/icons-material/esm/<Name>.js`
  and register them in `icons.rs`.
- Text is English (Heronite's `en` locale strings in
  `../HeroicGamesLauncher/public/locales/en/translation.json`).

## Verifying visually (mandatory for UI work)
- Build: `cargo build -p heronite`.
- `scripts/shot.sh /tmp/x.png [wait_s]` opens the app **on Hyprland
  workspace 2 silently**, captures the window with `grim -T` (works
  off-screen), prints PSS and closes it. Never open windows anywhere else,
  never switch workspaces, never move/click the real mouse.
- Debug hooks (env, via `SHOT_ENV="..."`): `HERONITE_DEBUG_SELECT=<runner_appName>`,
  `HERONITE_DEBUG_MENU=<key>` (context menu), `HERONITE_DEBUG_DROPDOWN=install|sort|status|facets`,
  `HERONITE_DEBUG_DIALOG=settings|statuses` (extend the match in
  `CollectionView::render` for new dialogs; `settings:<section>` opens that Settings section).
- Main Collection screen: compare with the reference PNG (crop the same region with PIL, view both). Dialogs/menus/settings: one sanity check is enough — they need to work and be consistent, not pixel-identical (user decision).
- Anything that WRITES data must be exercised only against a copy:
  `HERONITE_HEROIC_DIR=/tmp/heronite-fixture-<you> scripts/shot.sh …`
  (copy `/tmp/heronite-fixture`, which mirrors `~/.config/heroic` JSON).
- Never launch games, emulators, Steam or Ludusavi backups for real.
