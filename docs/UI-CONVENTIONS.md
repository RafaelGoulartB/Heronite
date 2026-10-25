# UI conventions (app/)

The Collection screen (list, cards, header bar, focus panel, stage) keeps the
look of the Electron Heronite Collection
([`src/frontend/screens/LocalLibrary`](https://github.com/RafaelGoulartB/HeroicGamesLauncher/tree/master/src/frontend/screens/LocalLibrary)).
Everything that floats over or replaces it (menus, dropdowns, context menus,
dialogs, drawers, pages, settings) follows the **Glass design system** below.

## Stack
- GPUI through `gpui-kit = 0.7.1` (`use gpui_kit::*; use gpui_kit::prelude::*;`).
  Sources: `~/.cargo/registry/src/*/gpui-pre-0.3.8`, `gpui-component-0.7.1`,
  `gpui-base-0.7.1`. Read them when unsure of an API.
- Reuse from gpui-component only behaviour-heavy widgets: `input::{Input, InputState}`
  (text fields), `slider::{Slider, SliderState}`. Style everything else by hand.

## Layout of the code
- `app/src/theme/` — tokens (`color`, `font`, `layout`, `glass`, `scheme`: nord-dark colours and their light
  counterparts, fonts, status colours, `W_BOLD` = CSS 600/700, `W_500` = CSS 500, and the glass tokens: fills,
  borders, radii `R_*`, text colours, toggle / button / segmented / input
  values; light or dark, see "Theme"). Never hard-code a new colour twice; add it here.
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
  (`Section::GROUPS`): App (General, Appearance, Advanced), Games (Game defaults, Downloads, Library, Other launchers, Storage, Overlay),
  Collection (Metadata, Screenshots, Backups), Help (Logs, About). A section = title + lead (`Section::blurb`) +
  captioned cards of rows. The Collection sections live in `app/src/pages/settings/<section>.rs`
  (`appearance`, `metadata`, `screenshots`, `saves` + `backup/` for Backups: settings, what goes in, then the restore list
  with each backup's parts and an inline "Restore and restart" confirmation; `HERONITE_DEBUG_SETTINGS_ACTION=backup-now|backup-open:<n>|backup-confirm:<n>`) with `State::new(&PageCtx, &EffectiveSettings, …)`
  and `render(&mut SettingsPage, …)`; save with `heronite_data::settings::*` and call `d.saved(result, cx)`
  (updates the page copy of `settings.json` and emits `PageEvent::LibraryChanged`). Work that spawns a process or
  reads folders (Ludusavi detection, screenshot cache size) starts in `SettingsPage::on_section_shown`, never at page
  creation. Open a section from code with `CollectionView::open_settings_section` (or `pages::settings::request_section`
  + navigate); add new sections to `Section` (label, icon, blurb, palette `keywords`, `parse`). Emulators and ROM
  folders are NOT a Settings section: they live on the ☰ Emulators page.
  Other launchers (`launchers.rs`) is a card per launcher (Heroic, Lutris, Bottles, Faugus): whether it is on this PC
  and how many games it has, a switch that brings its games in and keeps them in sync, "Bring in now", and
  "Remove them" for the games of one turned off. The same switch is offered where games come from: a tile per
  launcher on the empty Collection and a row in "Getting started" → Your games, shown only for the launchers
  found with games and not imported yet.
- Command palette (Ctrl K, `app/src/collection/palette/`): it reads the app's own lists, so a new feature reaches it
  without palette code. A game action goes in `collection/game_actions.rs` (`GameAction`, listed by
  `CollectionView::game_actions`, run by `run_game_action`): the card's context menu draws the same list. A page action
  goes in the page's `palette.rs` (`pages::commands::PaletteCommands`: list it, even before the page exists, from the
  services or `CommandCtx`; run it; say when the page has a dialog open with `page_modal`). Something that cannot run
  now is listed with `disabled(Some(reason))`, not left out.

## Glass design system (menus, dialogs, pages, settings)

Everything below keeps its old name/signature where one existed, so a caller
picks the new look automatically.

**Backdrop.** GPUI has no backdrop-filter. `CollectionView::glass_image`
(collection/mod.rs) asks the image pipeline for `Variant::Glass` of the selected
game's art (blur 30px, saturate 1.5, ~1/5 of the window, cached in the
`ImageStore` like the stage; costs ~+1 MB PSS) and publishes it every frame with
`ui::glass::set_backdrop`. Glass surfaces paint it aligned to the window and
clipped to their rounded bounds (`Window::paint_image` with a sub-rect), under a
dark dim (`theme::glass_art_dim`) and the translucent fill
(`rgba(22,27,36,.90)` by default). No art (no game selected, Solid) or still
loading: nearly opaque fill (`.94`). Never blur per frame; never request another
blur: reuse this one.

**Interface vs Collection appearance** (Settings → Appearance, two independent
groups). *Collection background* = the stage art behind the list/details: on/off,
blur, darkening, colour, edge shading in Heronite's `local_library/settings.json`
(`background`), plus Opacity (`collectionOpacity`, default 100). Its art is
`Variant::Backdrop`: the stage with its highlights rolled off (`BACKDROP_KNEE` /
`BACKDROP_CAP`) so white text keeps 4.5:1 over white art; the scrims keep a
floor under the details (no lighter dip in the middle) and the focus panels
(`focus::panel`) are dense enough for .52 labels. Text that sits on the art
with no panel stays at .66 or more. *Interface
background* = every glass surface (menus, ☰, context menus, dropdowns, dialogs,
drawer, pages, palette): `interfaceMode` `transparent` (default) | `solid`,
`interfaceOpacity` 50-100 (default 90, maps to `glass_fill`; the art dim rises
from .5 to 1 above 84 so 100 is fully opaque), `interfaceBlur` 0-60 in steps of 5
(default 30), `interfaceSolidTint` slate (default) | graphite | midnight | forest | plum | mocha for the dark theme
and `interfaceSolidTintLight` paper (default) | frost | mint | lilac | blush | sand (pastels, readable and told apart:
`tints_are_readable_and_distinct`) for the light one (`theme::solid_tint()` gives the
current theme's). They sit in the Colour card under the theme, as Panels with its opacity and blur or its
Panel colour indented under it; Panel colour shows the twelve as dots, the dark ones above the light ones, dark and light together, plus a dot for a colour picked
by hand (`interfaceSolidColour` / `interfaceSolidColourLight`, through `own_panel`: it goes to the side its brightness
belongs to and is moved until that side's secondary text reads on it); picking a light colour turns the theme light
and a dark one dark, System keeps one per side. **From the desktop** (`interfaceSolidDesktop`) takes the desktop's
window colour instead and the System theme with it. `theme::panel_rgb()` is the panels' colour wherever it comes from.
The desktop's colours (`system::desktop_colours`, kept in `theme::desktop_colours()`): the accent from the XDG
settings portal (GNOME 47+, KDE Plasma 6, COSMIC), else KDE's `kdeglobals` or GNOME's `accent-color`; the window
colour from KDE's colour scheme or libadwaita's on GNOME. Read on a thread, again when the portal says the appearance
changed; their dots show only while the desktop has them.
These live in this app's `appearance.json` (`AppPaths::appearance()`, next to
`ui-state.json`; `heronite_data::appearance::Appearance`), NOT in the data folder.
At runtime `theme::appearance()/set_appearance()` (a process-wide value) feeds
`theme::{glass_fill, glass_fill_solid, glass_art_dim, page_backdrop,
glass_blur_px, glass_shows_art}`; never hard-code those numbers. The glass
backdrop is generated whenever `glass_shows_art()` (Transparent and < 100%),
independent of the Collection art; in Solid or at 100% nothing is generated and
`ImageStore::forget_prefix("g:")` frees the cached texture (same for the stage
texture "s:" when the Collection art is off). Pages sit on `theme::page_backdrop()`
(opaque page colour when Solid). Sliders call `set_appearance` live and save on
release (off the UI thread).

**Personal look** (Settings → Appearance, `appearance.json`, `heronite_data::appearance::{choices, profiles}`; page in
`pages/settings/appearance/`: `look.rs` for the rows, `profiles.rs` for the tiles). Every value defaults to today's
look and applies at once (`look::set_look` → `set_appearance`, saved off the UI thread, `PageEvent::AppearanceChanged`
repaints the Collection). Read them through the theme, never from the file: `theme::accent()` and the `accent_*` tints
follow **Accent colour** (Ice `#a5dceb` default, Sky, Mint, Amber, Coral, Rose, Lavender, "From the game": the vivid average of
the selected cover, measured on an 8×12 copy in `collection/look.rs`, or one of your own from the colour picker:
`accentColour`, `Appearance::accent_own`, drawn through `theme::own_accent_shade`, lightened on the dark theme and
darkened on the light one until it reads, or "From the desktop"); never write the accent's RGB at a call site
(`theme::accent_alpha(a)`). **High contrast** raises `text_muted`, `text_dim`, borders, separators and
`theme::contrast_a(a)` for faint text over art. **Font** (`theme::font_body()` / `font_heading()`: Cabin + Rubik,
Rubik, or `.SystemUIFont`). **Density** Compact halves the list and grid gaps (`list_metrics`). Covers: **Corners**
(radius 2 / 6 / 12), **Under the cover** (playtime or the sort value, playtime, last played, status with its colour,
title, nothing; `data::chosen_caption`), **Favourite star**, **Under the pointer** (nothing, lift 4px, glow with an
accent ring). **Picture** behind the Collection (`look::stage_source`: the game's art, one of its screenshots, its
cover blurred, the Solid tint, or a picture copied to `AppPaths::custom_background()`), with a slideshow of the
screenshots every 20 s. **Animations**: Full, Reduced (`theme::fades()` only: the stage cross-fade and covers fading
in) or Off (`theme::animations()` is false; `HERONITE_NO_ANIMATIONS` forces Off). **Open the Collection in** (last
used, Focus, Grid; applied once at start), **Reset widths** (`PageEvent::ResetPanelWidths`), **Game sections** (order
and visibility of Screenshots, Overview, Details, Timeline, Related in `focus::body`; in two columns Details stays on
the side, How Long To Beat and the extra panel travel with it). **Profiles**: four built-in looks (Standard = today,
Cinema, Minimal, High contrast) and the user's (`profiles` in `appearance.json`), "Save as profile", export / import
(`heroniteAppearance` marker). A profile holds everything in `appearance.json`, not the background sliders of
`settings.json`. Console mode lists the same choices except the sections' order and "Your own picture".

**Theme** (Settings → Appearance → Colour, `theme` in `appearance.json`: `dark` default | `light` | `system` |
`custom`; `theme/scheme.rs`). System follows the desktop's style: `CollectionView` reads `window.appearance()` (GPUI gets it
from the XDG portal's `color-scheme`) and observes it, so the app switches as the desktop does. The theme stays out
of profiles, like the switch by the window's width (`Appearance::with_window_choices_of`). Every token has a dark
and a light value picked by `theme::is_light()`; the light text, accent (`Accent::rgb_light`, a cover's accent
through `readable_on_light`) and status colours keep 4.5:1 on every light tint, cards included (tested). So:
- interface ink is `theme::ink_a(a)` (white on dark, `rgb(20,26,36)` on light), wells `recess_a`, shadows
  `shadow_a`, bright text `text_bright()`; `white_a` / `black_a` / `white()` only for what sits on a picture
  (cover overlays, the "+N" of a strip, the treemap's tiles) or on a dark-only screen;
- a colour that is text or an icon and comes from data (status, score, a group's colour) goes through
  `theme::readable(c)`; dots and bars keep it as is;
- the Collection's scrims are `stage_ink_a`: on the light theme the art is faded, never under
  `LIGHT_FADE_FLOOR` (`collection/stage.rs`) so dark text keeps 4.5:1 over black art; Play and the active Focus /
  Grid half are `play_*` / `solid_pill` (a dark pill on the light theme).

Console mode, the in-game overlay, Statistics (and Year in review) and the screenshots gallery and viewer stay dark:
the root view says so at the start of its render (`theme::set_dark_only`; GPUI draws one window at a time and the
root renders first, so the whole window reads it). A picture of another theme (the Theme cards, Getting started's
overlay preview) is built inside `theme::drawn_as(light, ..)`; its colours are read while building, so no canvas
in it may read one later. The gpui-component widgets get the theme again when it changes
(`CollectionView::sync_look` → `apply_component_theme`). Solid shows the current theme's four tints, both rows with
System. The theme is Dark until the user picks another, never the desktop's by itself; **Reset theme** under the
Colour card goes back to Dark, glass panels, Ice and no high contrast, and keeps the custom theme's colours. On the light theme the Collection background's Darkening is called Fading.

**Custom theme** (`heronite_data::appearance::custom`, editor in `pages/settings/appearance/custom.rs`): six
colours (`Role`: page, panels, raised parts, text, secondary text, accent) in `customPalette`, light or dark as the
panels are. While it is drawn (`theme::custom_palette()`, `Scheme::Custom`) every token derives from them: surfaces
and text as picked, `ink_a` in the text colour, status colours through `readable_on` its backgrounds, the accent
from the palette (the Accent row hides). The editor shows one colour at a time (the colour picker, suggestions), the contrast of the text colours on the panels with "Make it readable" below 4.5:1, and the
themes saved by name (`customThemes`, export / import with the `heroniteTheme` marker). Like the theme, the custom
colours stay out of profiles. Light colours come from `LIGHT_PAPER` / `LIGHT_PAGE` / `LIGHT_LIFT` in
`theme/color.rs` (keep `SolidTint::Paper` equal to `LIGHT_PAPER`).

**Colour picker** (`ui::colour_picker::ColourPicker`, an entity): a saturation / brightness square and a hue strip to
click or drag on (drags go through `on_drag_move`, so they follow the pointer outside), a hex field; it emits
`PickerEvent::Change` while dragging and `Commit` at the end or when a hex is typed, and `set` moves it without an event.

**Surfaces** (`ui::glass`): `glass_panel(radius)` (big shadow; popovers 20, sheets
26, drawers 24), `glass_menu_surface(radius)` (menus/dropdown lists, radius 14). Children added after the constructor paint above the layers. Children
are not clipped to the rounded corners: give edge-to-edge children their own radius.

**Collection header** (`collection/header.rs`, the one place the main screen departs from Heronite, by user
decision): faint, no bar fill. Over the list column: ☰ (`tool_button`, radius 10, icon `theme::tool_icon()`), the
search pill (`search_fill`, accent ring while focused, clear button) and the View button (`Icon::Tune`, accent while
anything is filtered). Its menu (`collection/view_menu/`) starts with a search box: the active filters sit in it as
tokens (click or Backspace removes one) and typing lists matching values of every kind (Only, Status, Category,
Store, Genre, Developer, Publisher, Series, Platform) with counts, four per kind until `kind:` narrows it; ↑ ↓ and
Enter pick, and an X beside the first kind's title empties the box (back to the menu). Below, when nothing is typed (every control has the cards' faint fill and border, `render::control`): Presets (a select showing the preset on screen, else "Custom", its list drawn like the "More" lists; a preset applies
everything the menu sets: filters, sort, grouping, hidden games; four built-in ones are added to the user's list once
and are then ordinary presets; under them "Save as preset…" while none matches and "Edit presets…", which lists them
with move, store current state, rename and delete, plus "Restore default presets"; stored as `savedViews`), Show
(segmented with counts), Group by (None | Status | Store | More), Sort (segmented + direction), Only (Favourites, Update, Offline,
Hidden tiles) Browse (one chip per kind, fills `kind:`) and Manage ("Statuses…" and "Library fields…", the fields manager
`dialogs/manage_fields`, also in the palette); the footer has the count and Reset all. In a small window (under 900px tall or 1200px wide) the panel is compact (`render::Density`): 360px instead of 404, tighter gaps, segments (no icons), Only tiles and Browse pills. Several
values of a kind add up (any of them); kinds combine. While its box has the keyboard the menu stays open until Esc or
a click outside. Each active filter also appears as a removable `filter_chip` after the button. Right: in the grid the cover size slider and the "Aa" titles switch, the
Focus / Grid switch (`grid.rs`), bulk metadata, refresh, close focus (Focus view only). Header menus open on click or
after a 140ms hover (`OPEN_DELAY`) and close 180ms after the pointer leaves. The ☰ menu (`nav.rs`) is a list:
pages grouped (Collection, Downloads, Statistics | Stores, Wine Manager, Emulators), then Add game, Console mode,
then Settings and Command palette with their keys; each row = icon square + title + meta on the right. "Stores" is one
row for Epic Games, GOG and Amazon Games: each store keeps its own page, titled "Stores", and the three switch with the
tabs of `pages::stores` (LB / RB in console mode); the row (and the console sidebar's) opens the store shown last.

**Focus panel** (`collection/focus/`, the second place that departs from Heronite, by user decision): a fixed left column
(`focus_left.rs`: big cover, Play, status selector + favourite / metadata / settings, the journal's "Where I left off"
card only once the player wrote one (click: the Journal; also in the grid drawer), star rating, facts card with score
(a green / amber / red badge from 75 / 50, with its source, IGDB or Steam, unless typed by hand), library, save backup and Linux) next to the scrolling details; no footer. The title + meta line sit above both, fixed and
aligned with the cover's left edge, so the cover, "Screenshots" and "Details" start on one line; the gap left column → main
equals main → side (`COL_GAP`). `focus::layout(panel_w)` picks the widths: narrow = everything scrolls in one column with the
left block as a row under the title; otherwise left column + main/side (stacked when tight), centred as a group in wide panels. The main column holds Screenshots, Overview, **Timeline** (`timeline.rs`: days of play with
the screenshots of their sessions, thumbnails only; a bar per day of the last 30 in the title when the column has room,
hovering one says that day there; a faint "5 days without playing" line between days 3+ days apart) and Related (one
group at a time picked with pills in the title: Same series first when there is one, Same developer, Similar = 2+
genres / themes in common; 100px covers with the time played, else the status, under the title, on one row that
scrolls sideways: ‹ › in the title, a swipe, or the mouse wheel while the row can still move, after which the page
scrolls; `HERONITE_DEBUG_RELATED=<group>`). Details: Last Played
is relative under 60 days; Platforms lists the PC ones first; every chip row keeps to one line (widths measured
with the font), the rest behind a "+N" chip that shows them all. Facts that are not in the Library load in `game_facts.rs`.
Clickable facts: "Library" sets the Store filter to the game's own store (replacing any other store), "Save backup" backs up now, "On Linux" opens the ProtonDB page, the Details folder rows open the file manager;
rows that act get `focus_left::clickable` (pointer + faint highlight). Folder rows are resolved on disk, never shown from
imported fields (`open_links::game_folders`): Steam games → the folder of their `appmanifest`; emulated games → "Emulator
Folder" + "ROM Folder"; others → the stored path only when it exists (Playnite's Windows paths are dropped). Under the folder row goes the size: the store's (Epic, GOG), else measured once the focus settles (`folder_size.rs`): Steam's `SizeOnDisk`, an emulated game's ROM files (with the tracks a `.cue` / `.gdi` / `.m3u` names) under "ROM Folder", an AppImage's file, else a walk of the folder (never home, system folders or a folder holding another game's; cached 3 days in `install-sizes.json`; stopped when the focus moves). ProtonDB needs a
Steam id: the game's, the metadata's, else a strict Steam search by title (`covers::steam_app_id_exact`, same name or name +
edition words, cached in `~/.cache/heronite/protondb/titles.json`); emulated games have no "On Linux" row.

**Grid view** (`collection/grid.rs`, `grid_drawer.rs`, by user decision): the header's Focus / Grid switch (also in the
palette) shows the covers across the whole window instead of the list next to the focus panel. Same groups, sticky
headers, cards, context menu and stage; the cards share each row's width (`grid_columns`: at least `gridCoverWidth`,
110-240, set by the header slider; covers are requested in 16px steps). The selected game opens in a glass drawer on the
right (460px at most, inset 10px, the grid narrows beside it): sharp hero art on top with a close button, the cover
reaching into it next to the title and meta line, then the focus panel's own pieces (Play, status row, rating, facts
card), three tiles (Played, Last played, Sessions) and the details stacked in one column (`focus::stacked_details`),
ending in "Open game page" (back to the Focus view). Keys while nothing floats over it: arrows move the selection
(the drawer follows), Enter plays an installed game, Esc closes the drawer (the selection stays; selecting a game opens
it again). The Focus list takes the same arrows and Enter (play an installed game). `ui-state.json` keeps `gridView`, `gridCoverWidth` and `gridTitles`. Console mode is not affected.

**Wider panels** (`collection/resize.rs`): the Focus list column and the grid's drawer grow when their inner edge is
dragged (the cursor turns to col-resize over it; nothing else is drawn). Their default width for the window is the narrowest they get;
dragging back to it, or a double click on the edge, returns to the default. A wider Focus list fits more columns of the
same covers; the focus panel keeps at least 560px and the grid 320px. `ui-state.json` keeps `listColumnWidth` and
`gridDrawerWidth` (`null` = default); Settings → Appearance → Layout → "Reset widths" clears both.

**Selecting several games** (`collection/selection.rs`, `dock.rs`, by user decision: proposal 2 "Dock" of
`propostas/selecao-v1`). Ctrl click ticks a game (with the focused one when nothing is selected yet), Shift click adds
the cards from the last one clicked, Ctrl A selects every card on screen, and the card context menu's last row,
"Select multiple" ("Add to selection" while selecting), starts it with the mouse. No tick shows on hover: only while
games are selected do the cards get a round tick on the top-right (filled = selected, the others dimmed, no play bar),
a plain click ticks or unticks, group headers get "Select group", the grid's drawer steps aside, and the dock floats at
the bottom: × (or Esc) ends it, the count and "Select all N", Status ▾ and Categories ▾ (popovers above the dock),
Favourite, Hide, Metadata (the metadata wizard on the selection) and Edit (F3). Right-clicking a selected card opens
the same actions as a menu. Categories have three states (round tick: all, dash: some, ring: none); each click moves
some → all → none → as it was (Playnite's assign / remove / leave alone), the next step shows under the pointer. Every
batch change writes off the UI thread and its toast offers Undo (also Ctrl Z, 20 steps). F3 opens
`dialogs/batch_edit.rs`: a drawer with the series and the tag fields of all the selected games; a tag only some have is
dashed with "3/7" (click: give it to all), × takes one out of all, typing + Enter adds one to all; nothing is written
until "Apply". Debug: `HERONITE_DEBUG_MULTI=<n>`, `HERONITE_DEBUG_DOCK=status|categories`,
`HERONITE_DEBUG_DIALOG=batchedit`, `HERONITE_DEBUG_BATCH=status:<id>,cat:<name>,fav,hide,undo` (data copy only).

**Menus** (`ui::dropdown`): `menu_panel(min_w)`, `menu_item(id, active)` (radius 10,
14px, hover .07, active = accent tint), `menu_separator()`, `dropdown(..)`. The
Collection's context menu (`collection/context_menu.rs::item`/`paper`) uses the
same look and groups a game's rows with separators (`menu_place`): Play (and the other ways to play or install);
Status, Categories, favourite, hide, recent; Settings, the folder, "Saves ›" (back up now and the game's automatic backup switch, which keeps the flyout open); "Manage installation ›" (the store's verify,
modify, move, relocate, EOS in a flyout; a single one stays a plain row); shortcuts; uninstall / remove; View log; Select multiple. A new menu action gets a
place there; rare ones (store page, metadata, playtime, cloud sync) stay in the palette only (`palette_only`); other context-menu rows must call `context_menu::item`.

**Dialogs** (`ui::dialog`): `dialog(DialogProps, body, footer, on_close)` = glass
modal (radius 26, title Rubik 22, round close button, Collection darkened by
`rgba(0,0,0,.55)`); `dialog_scrolled(.., &ScrollHandle, ..)` is the same with a draggable scrollbar on a long body; `drawer(DrawerProps { id, width }, header, body, footer,
on_close)` = right-hand sheet inset 10px, radius 24, with fixed header/footer
slots (build the header with `kicker`, `dialog_title`, `close_button`).

**Add game** (`dialogs/add_game/`, by user decision): the cover and background previews on the left; a click on one
opens its menu (Find art while it is empty, Search the web, Choose an image file, Type an address, which shows an
address field under the previews until it loses focus, and Remove the image). On the right: the store note (Epic, GOG,
Amazon; "Got it" hides it for good, `ui-state.json` `dismissedNotes`), Title with Find art inside it, Platform tabs,
then how the game starts (Windows: the executable, with one quiet line under it: "Not installed yet?" and the links
"Run installer first" · "Use an install script" (accent text and a small icon, no fill; tooltips say what they do);
Linux: the script one only), the Wine / Proton card (open; closed, the version and the prefix show as pills; the
prefix is named after the title) and a small "Run in a sandbox" switch whose tooltip (`ui::tooltip`) says what it is
for. The footer says what is still missing and keeps "Add game" disabled until then. "Use an install script" shows the
"Install from script" faces in place of the form (`ScriptInstallDialog::inside_add_game`: title "Add game", a back
arrow, `ui::dialog::dialog_scrolled_with_back`; Esc goes back too, the install keeps going); the ☰ menu has no row of
its own for scripts (the palette still opens them directly). `HERONITE_DEBUG_DIALOG=add-game:<states>` (joined with
`+`): `windows`, `linux`, `command`, `browser`, `shared`, `wine`, `wine-open`, `art-menu`, `address`, `no-note`,
`script`, `picker`, `art=<cover url>[,<background url>]`.

**Launch problem** (`dialogs/launch_problem/`, `crate::diagnosis`): when a game ends on its own within 30 s and the
diagnosis knows the cause and a fix for it, the dialog opens by itself (a toast with "How to fix" while another dialog
is open). Anything else (a crash the log does not explain, a likely cause, an anti-cheat that refuses Linux) never
interrupts: a bad exit keeps the plain "closed right after starting" toast, with "Details" when something was found, and
"Troubleshoot" shows every finding of the last session, however long the game ran. The first line says how the game
ended (how the last session ended, from its log, in "Troubleshoot"); the main finding is a card (a tinted badge, red
with a lock when nothing can be done, the headline, one or two sentences, "What the log says" with the log lines in a
mono box, a command to copy for packages) whose first fix is the Primary button: it applies the fix and starts the game
again, closing the dialog with a toast. A fix tried before this run is no longer Primary and says so. Other findings are
rows under "Also found". The footer has View log, ProtonDB and Close. A running fix shows a busy bar and its last output
lines (winetricks can be cancelled); a failed one its error. "Troubleshoot" (context menu, under View log, always there;
palette) opens it for the last session. `HERONITE_DEBUG_DIALOG=launch-problem:<log>[@<code>]` diagnoses the selected
game as if it had just ended with that log (`launch-problem-toast:` for the toast, `troubleshoot` for the on-demand
check) and `HERONITE_DEBUG_LAUNCH_FIX=working|failed` shows a fix's progress or failure; nothing is applied.

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
  separators between rows), `with_sub_rows(parent, rows)` (the rows that only
  matter because of `parent`, shown while it is on or for one of its values,
  indented under it along a line, as one row of the card), `caption(text)`, `page_heading`
  (Rubik 26), `dialog_title` (Rubik 22), `help_text` (lead), `kicker` (the only
  uppercase text), `kbd("Ctrl K")`, `close_button(id)`, `nav_pill(id, icon, label, active)`.
- Fields (`ui::fields`): `path_field` shows a folder icon and "Clear"/"Browse" in accent;
  `select_field`'s list is a scrollable glass menu; labels are 12px muted.

**Game settings drawer** (`dialogs/game_settings/`): the first `drawer` caller. Tabs are
segmented pills in the header slot; each tab is a stack of `caption` + `card`. The tabs are the same for
every game, by what the user wants to change: Game (images, details, the time played and, in the same card, the "Sessions" row:
their sum, Recalculate, and when expanded the sessions 3 per page to fix, delete or add by hand, `sessions.rs`; `HERONITE_DEBUG_SESSION=open|page:<n>|new|invalid|edit:<row>|delete:<row>|recalc`;
the card menu has no "Edit playtime"), Launch (Play, the actions,
the panel's launch settings), Compatibility (Wine / Proton), Performance (overlays, FPS limit, frame
generation, image, system) and Saves (Ludusavi backups + cloud saves); a tab only goes away when the
game has nothing for it. Sandbox shows only for a game whose settings have the sandbox on (set when it
was added or installed); turning it off there keeps the tab until the drawer closes. Descriptions are one short line saying the effect; paths and rare warnings
only show when they matter. The Heroic
panels (`dialogs/settings_panel`) build their cards with `parts::rows()` (`.child(toggle_row)`,
`.field(input)`, `.field_help(input, notes)`, `.card()`); they assume no container, so Settings →
Game defaults can host them too. A setting that differs from the game defaults and is technical (its
`Group` is made with `t(..)`: Wine version, DXVK/VKD3D, sync, FPS limit, gamescope, GPU tweaks...; not plain switches
or text boxes) gets a bare ↺ on the right (dim, accent under the pointer): before the switch of a switch row, at the
end of a field's label line, after the caption of a card that is one setting or holds an unlabelled field (Sync); and the dialog a
"Reset this tab" button next to Done while the tab has changes (`settings_panel/changes.rs`;
console: a dot after the label, Y resets, a "Reset this tab" row). A new setting is one entry of its
`GROUPS`, keyed by the row, input or setting id. The Launch tab ends with "How the game starts"
(`launch_preview.rs`): the command and variables of a launch built from the settings on screen, grouped
by the setting each comes from (`HERONITE_DEBUG_PREVIEW=1` opens it).

**Settings → Storage** (`pages/settings/storage/`, `heronite_storage`): what the games take on disk, read only while the
section is on screen (listed when shown, then the sizes no store or manifest gives are walked one by one and cached in
`install-sizes.json`; showing another section stops the walk). A bar per disk (one device = one disk, btrfs subvolumes
included) in the group colours (`group_color`), hover names a part; the "Free up space" card (big number = the ticked
choices, a split bar, the choices as switches, each with a chevron that opens what it does and every item it takes,
with its paths, "To the trash" / "Deleted" and size; then an inline confirmation); the games as a squarified treemap of their
wide art (`treemap.rs`, the biggest 12 and a "+n" tile; an amber dot = not played for half a year) over a list sorted by
size or least recently played; then one list per kind (Wine prefixes, Proton and runtimes, Steam leftovers, caches,
leftover folders), unused rows first with an amber "Unused" pill and bar. Prefixes, Steam's Proton prefixes and leftover
folders go to the trash, caches and Wine Manager builds are deleted; a game's remove button sends `PageEvent::Uninstall`
(the Collection runs its own uninstall over the page). Console: rows, A twice removes. `HERONITE_DEBUG_PAGE=settings:storage`
(`HERONITE_DEBUG_SWEEPS_OPEN=1` opens every choice).

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
Statistics (`pages/stats/`, Ctrl 6) has five tabs over one period (`tabs.rs`): Overview (hero, month by month,
top games, habits, records, every day, genres, plus a card per other tab and each month's beaten games), Journeys
(`journeys.rs`: a lane per game in time, games going at once), Backlog (`backlog.rs`: the yearly goal, beaten and
abandoned games on the year's line, the goal's open slots taking planned games (`plans.rs`: a slot opens a
picker of the queue, the games in progress or any game by name; plans in `ui-state.json` `statsPlans`), How Long To Beat, the Collection by status, the queue, the shelf, the pile) and
Discoveries (`discoveries.rs`: the period's facts as sentences, pinned or hidden by kind, milestones, memories) and Album
(`album.rs`, `album_panels.rs`: the screenshots of the period, a chip per game filtering the tab, moments, captures per
hour played, capture hours against play hours, a photo per month, captures per session). Any capture opens in the viewer
(`viewer.rs`: ← → through its list, Esc closes), from the Album, the game sheet and the month cards.
A tab that cannot show a span (Journeys of a week) shows the year around it and dims that span. Once the hero scrolls
away, the tabs and the period switch stay pinned in a bar at the top of the page (`desktop.rs`). Every cover and game
name opens the game sheet (`sheet.rs`, a drawer; Esc closes it). The last tab, the yearly goals and the pinned and
hidden discoveries live in `ui-state.json` (`heronite_data::stats::StatsPrefs`). The page only reads: the Collection hands it the loaded library
(`StatsPage::set_library`, on navigate and after each reload) and the numbers come from `heronite_data::stats`
(sessions cut into hours; sessions under 2 min are failed launches and ignored). Charts are plain divs (the ones of
hundreds of cells are painted, see below): bars stacked
in `theme::chart_series(i)` for the top 5 games of the period (`Stats::series`), the rest `theme::chart_other()`; a
game keeps its colour in every block of the page. Hovering a bar / heatmap cell updates the line above it (no tooltips).
`HERONITE_DEBUG_PAGE=stats:week|month|year|all` opens it on a span; `HERONITE_DEBUG_STATS_TAB=overview|journeys|backlog|discoveries|album`,
`HERONITE_DEBUG_STATS_SHEET=<n>` (the sheet of the period's game n) `HERONITE_DEBUG_STATS_OPEN=<n>` (lane n, or the Album's moment n, open) and `HERONITE_DEBUG_STATS_PHOTO=<n>` (the Album's
capture n in the viewer). `HERONITE_DEBUG_STATS_PLAN=<n>` opens the planned-game picker on the Backlog's slot n and
`HERONITE_DEBUG_STATS_PLANNED=<n>` plans the queue's first n games (in memory). `HERONITE_DEBUG_STATS_SCALE=<n>` plays every session n times (a history n times bigger),
and `RUST_LOG=debug` logs how long each period took to compute.

Statistics performance: the page renders again on every scroll frame and mouse move, so render code only reads.
- A period is computed off the UI thread in one job (`compute.rs`, `Job` → `Snapshot`); the page shows the last result
  until the next is back. The history is indexed by time and by game (`History::pieces_in`, `sessions_of`), so a period
  reads only its own slice. Per-game status answers (slug, beaten and abandoned days, counts, the pile) come from
  `facts.rs`, built once per library.
- Anything that grows with the library is never worked out in a `render_*` function: no `compute`, no pass over every
  game, session or screenshot, no copy of a list. Put it in the `Snapshot` or the facts.
- Charts of hundreds of cells (calendar, hour grid, day bars, lane dots) are painted as one element (`marks.rs`; the
  cell under the cursor comes from its rectangle). The month strip builds only the cards in view.
- Lists are capped: Journeys lanes by 40 with "Show more", "Show all" games at 100, the Backlog's covers at the latest 60.
- `crates/data/tests/stats_scale.rs` times every period function on 3,000 games and ~180,000 sessions
  (`cargo test --release -p heronite-data --test stats_scale -- --ignored --nocapture`).

**Console mode** (`app/src/console/`, shell in `collection/console/`): an extra controller-first TV mode
entered from ☰ › Console mode or the palette,
remembered across restarts (`ui-state.json` `consoleMode`). The desktop look must not change: every page and dialog
keeps its desktop render and, when `crate::console::active()`, returns its console layout instead
(`render_console`, usually in a child module `console.rs`) and implements `console::PadTarget` (`on_pad` + `hints`).
- Input: `console::input` reads controllers with gilrs on a thread that only runs in console mode (presses only count
  while the window is focused); the keyboard maps to the same `Pad`s (arrows, Enter = A, Esc = B, X, Y, Q/E = LB/RB, M = menu).
- Hosting: pages register their hook in `nav.rs` (`pages.hooks`); dialogs are found by `console::registry::hook_for`
  (add new console-capable dialogs there). Overlays owned by a view use `console::Layer` (drawn by the owner; for a
  view embedded in a smaller area wrap it with `kit::window_layer`).
- Building blocks: `console::kit` (sizes through `u()`, scaled to the window height; `row_base`, `toggle_pill`, `choice`,
  `tabs`, `glyph`, `hint_bar`, `modal`), `console::list` (`Row`s + `ListNav`: one focused row, values changed with ◀ ▶,
  `Outcome` tells the owner what happened; `simple_dialog` for confirmations), `console::sheet::Sheet`,
  `console::osk::Osk` (on-screen keyboard; never use text inputs in console layouts), `console::browser::Browser`
  (folder / file picker; the portal file chooser cannot be driven by a controller).
- Motion (`console::motion`): scroll areas that follow the focus use a `Glide` (`go(Goal::Reveal{..})` / `Start` /
  `by(px)`, its `driver()` as the last child of the area) instead of `scroll_to_item`, so they glide on a spring;
  `row_base` focus pills pop in on every press; covers lift with `motion::spring`; one value that drives many
  elements (a whole row) is a `motion::Follow` its owner steps each frame; screens, pages, dialogs, the menu
  and panels (`kit::panel_in`, `kit::scrim`) fade / slide in with `motion::enter` (new id = replay). Only moving
  things request frames; `HERONITE_NO_ANIMATIONS=1` jumps instead. Off-screen covers are empty boxes of the same size.
- Layout: top bar = the theme's tabs (`kit::tabs`; `interface.tabBar` may move them to a bar along the bottom, show only
  the current one as a centred title, or show the names alone) + `kit::clock` (pages keep their own title with `kit::wordmark`), content padded `u(46.)`, lists about `u(900.)`
  wide; the shell draws the hint bar from `hints()` (it adds Back / Menu). B goes back; Start opens the menu.
- **Console themes** (`heronite_data::console_theme`, shareable JSON; `console.json` holds the one in use and the
  machine options, `console-themes/` the saved ones): the shell (`collection/console/mod.rs`) reads
  `console::look::theme()` every frame and never caches it. Tabs (`TabSpec`: name, layout, source) dispatch to
  `collection/console/layouts/<layout>.rs` (`render_<l>`, `<l>_pad`, `<l>_hints`, a `Nav` in `TabNav`, a `SCRIM`);
  covers come from `tile.rs` (shape, size, corners, focus style, badges and "New", store strip, name below or in a
  bar), the background from `backdrop.rs`, names / logos / facts (a line or chips and genre tags) and the stats panel
  from `hero.rs` (logos: `art.rs`), shelves and tab games from `sources.rs`; game pages (`game/`: classic, hero, tabs,
  sheet) and the ☰ menu (`sidebar.rs`: sidebar, grid, dock, rail — the rail stays on screen and insets the content)
  follow the theme too. A light `colors.background` (`ColourOptions::is_light`, `look::light()`) makes the console
  light: dark text and focus, white panels, the art washed out; surfaces use the scheme's ink, never a bare white.
  The built-in themes (`presets.rs`), in the order they are listed: Minimal (what a new install uses,
  `presets::DEFAULT`, and what Reset goes back to), Classic (the base the others start from and what missing keys of a
  theme file read as), and Cinema and Pulse, modelled on
  other launchers' fullscreen themes and consoles' home screens. Classic's screens: Shelves count a shelf's games
  before `shelfLimit` cuts it and end a cut one with a "View all" tile (A opens the first Grid tab with the filters
  that show them, `grid::Nav::show_like`), LT / RT page along a shelf, the focused game's text slides in from the side
  the focus came from (with "Last played", a long name shrinks) over a soft shade of its own; the Grid without a panel
  puts the game's facts beside its name over a denser scrim (`Scrim::Library`); the Classic game page has the time
  played, last played, time to beat and score above the overview and the facts not shown elsewhere; new art settles
  in from a touch closer on these screens (`Scrim::Classic` / `Library`, not with Reduced motion).
  Minimal's Carousel: covers spaced evenly (a fixed gap at every step of the turn), the far ones smaller, darker and
  rising a little, each over its real reflection (`heronite_images::Variant::Reflection`, `cr:` textures) on the
  blurred cover glowing through a vignette (`Scrim::Carousel`); a thin ring a little way out marks the focus; under it
  the name (long ones shrink), the facts and the place in the list (letter, a line with a gliding mark, "n of N"),
  sliding in from the side the focus came from, each line of a fixed height so the covers never move. LT / RT jump by letter (the letter flashes over the covers), LB / RB
  change the filter when the theme has one tab (the focused game stays when it passes it), the covers fan out from the
  middle on first show and on a new filter; A opens the Sheet game page over the carousel: a window floating
  along the left edge (inset, round corners) over a dim of the whole window, sliding in from the left
  (`render_console_game_layer`, drawn by the shell above the top bar).
  Pulse's Spotlight (`layouts/spotlight/`), a console's home: a row of square icons along the top, the focused one at
  the left edge, larger, with a thin ring a little way out and its name beside it; the next icons after the name, the
  ones before it sliding off to the left. Every icon's place and size and the names come from one number, the row's
  position gliding on a spring (`motion::Follow`, stepped each frame with a `FrameClock`), so a held direction glides
  and the room for a longer name opens smoothly (an arriving name is revealed as its room opens, the leaving one fades
  with its icon). Below, the logo (or the name) with a wide Play and More, then when it was last played; the cards
  (playtime, How Long To Beat with a bar of how far along the main story, journal) peek in at the bottom, and going
  down to them slides the whole screen up, the row fading and the art darkening from the bottom. Buttons and cards
  ease into the focus colour. The art is shaded for the row, the logo and the cards (`Scrim::Home`) and settles in
  like Classic's. With `Hints::Auto` the hint bar floats over the bottom and fades in and out
  (`layouts::hints_float`) instead of taking room, so nothing moves when it comes and goes; the cards that peek step
  back while it shows.
  Y on any tab opens the Filters panel along the right edge (`collection/console/filters.rs`): the search field first
  and focused (A opens the keyboard, a physical keyboard types into it), Show with counts, Sort, Favourites only,
  Status and Category; X resets them. They edit the filters of the tab's grid, else the theme's first Grid tab (the
  first change switches to it), else a results grid over the tab (`ConsoleUi::results`, B leaves it), so every theme has them.
  Colours go through `theme::console_*` / `text_primary` / `text_muted`, which read the theme only while console
  mode is on. Change the theme only with `console::themes::update` (installs it live, saves off the UI thread);
  Settings → Console mode and ☰ → Customize share their rows (`console::theme_rows`). On the desktop, Settings → Console
  mode (`pages/settings/console_mode/`) has three views: Home (the themes as cards with a picture each, the user's
  with a delete, "Create a new theme" last; the theme in use with its live picture, All options, save, undo and the
  "Quick changes" beside it; the startup options), the editor (name, author, save, undo, export under the live
  picture, then the Customize panel's groups as tabs; options no tab uses are hidden and counted) and New theme
  (start from a theme or the one in use, a name). Clicking a theme while the one in use has changes asks first. The
  pictures (`preview/`) draw a theme's first tab's layout, covers, background, tab bar, clock, hint bar, colours and
  font on a 640 x 360 canvas with the covers and art of the games played last (the library Storage gets from the
  Collection; tinted blocks without it); their textures are freed when the section leaves the screen.
  `HERONITE_DEBUG_CONSOLE_SETTINGS=editor[:<group>]|create|pending:<theme id>` opens a view. A new option = a field in
  `ConsoleTheme` (with a default and `#[serde(default)]`, so old and new theme files keep loading), its rows in
  `theme_rows`, and its use. Nothing console-only may run, read files or allocate while console mode is off;
  no "HERONITE" wordmark in console screens (the theme's `interface.label` may add a text of its own).
- Debug: `HERONITE_DEBUG_CONSOLE=1` (or `=game:<key>` for a game page, `=menu`, `=customize`, `=filters`) starts in console mode
  without saving it; `HERONITE_DEBUG_CONSOLE_PRESET=<theme id>` / `HERONITE_DEBUG_CONSOLE_LAYOUT=<layout>` start with
  that theme / every tab in that layout (not saved); `HERONITE_DEBUG_IDLE_SECS=<n>` shows the idle screen sooner;
  `HERONITE_NO_GAMEPAD=1` ignores controllers (test copies must not follow the one in use; `shot.sh` and `idle-cost.sh` set it); `HERONITE_DEBUG_PAD=down,a,wait:500,b` plays presses (`up down left right a b x y lb rb lt rt start select sup sdown`).

**Do not** use uppercase labels, outlined buttons, underlined tabs, square
checkboxes or flat grey modals in new UI. Colours come from `theme` (`text_primary`,
`text_muted`, `text_dim`, `accent_tint`, `ink_a`, ...) and must read on both themes (see "Theme").

**Verifying**: captures with the app on a hidden workspace only show what was
drawn before the compositor stopped sending frames, so late images (art, covers)
are missing. Take a first capture to wake the window, wait a second, then take
the real one (a modified copy of `scripts/shot.sh` doing `grim -T` twice).
`HERONITE_DEBUG_DROPDOWN=nav|view|view:<text>`, `HERONITE_DEBUG_MENU=<key>`,
`HERONITE_DEBUG_DIALOG=...`, `HERONITE_DEBUG_PAGE=...`. Copy
`~/.local/share/heronite/images-cache` into the fixture or no art appears at all.

**In-game overlay** (`app/src/overlay/`, Settings → Overlay; beta, off by default, options in `overlay.json` of the app
state): during a game launched by the app, the controller's Guide button opens a panel over the game (by user decision:
proposal 1 "Guide lateral, estilo Xbox"). It is a Wayland layer-shell surface on the overlay layer (no keyboard focus, so
the game keeps it), 440 design px wide along the left or right edge, in console mode's look scaled to the display
(`kit::set_scale_value`; 1440 px tall → 1.16), from the top, with a fixed size (`window::PANEL_H`, the tallest tab: Home with its
recent screenshots; capped at the display's height) so it never changes size while it loads or between tabs: every tab
fills it, the Screenshots grid and the Controller list scroll inside it, and the mixer holds its place while the audio is
read: LB / RB tabs on top (icon over label: Home,
Screenshots, Audio, Controller), then game title, session and total time, clock and battery, the tab's `console::list`
rows and a compact hint bar. Volume is a mixer block (by user decision: option D) on Home and Audio: bars for System,
the game's own streams and each other app playing (a browser tab, a music player: `pactl` streams not paused and not
event sounds, one bar per app name; 2 on Home, which then drops its recent captures, 6 on Audio), that are rows of the
list (↑ ↓), ◀ ▶ change the focused one, A mutes it, Y switches the output shown under them. Audio adds the output, the
microphone (the default source; the outputs' monitors are left out) and its mute. Home's "FPS limit" row only shows when the game's limiter can change while it runs
(`heronite_launch::live_fps`: MangoHud through a config file of ours it watches, gamescope through its
`GAMESCOPE_FPS_LIMIT` atom set with `xprop`; DXVK / VKD3D cannot); the value lasts for the session. Steam games are
started by Steam, so the launch cannot move MangoHud's limit into a file of ours: they have no live limit (editing the
user's `MangoHud.conf` or their Steam launch options was ruled out). Quitting the game asks
first (focus on Cancel). Guide + RB, Guide + ↑ / ↓ and Guide + X (the replay, Settings → Replay; Home and Screenshots
show "Save replay" while it records, and Screenshots a switch that turns it off until the game ends) show a notice in
the top corner opposite the panel. The
screenshot's button is a choice (RB unless changed; not ↑ / ↓ nor the triggers, which many controllers report as axes),
shown in Settings → Overlay and in Settings → Screenshots ("With the controller"). A keyboard shortcut (Super + G unless
changed, `heronite_media::capture::GlobalHotkey`: a temporary consuming Hyprland bind that sends SIGUSR1, else the
GlobalShortcuts portal) opens and closes it too; it is registered only while a game launched by the app runs and removed
on quit. Opened that way the surface takes the keyboard (`KeyboardInteractivity::Exclusive`, so the game gets no keys
until it closes), console mode's keys drive it (`console::input::pad_from_key`) and the hints show keys
(`kit::key_glyph`: Enter, Esc, Q / E). "Open it without a game too" (`withoutGame`, off by default) keeps the input thread and the
shortcut running while the app runs; the panel then has no Screenshots tab, the header says "No game running", the mixer
has only System, and Home ends with "Open Heronite".
The panel takes controller presses from the service (`OverlayPanel::on_pad`); it never reads `OverlayService`
(which is usually updating when it calls in) and reaches it through deferred `with_service`. GNOME and X11 cannot show
it (no layer-shell): Settings says so. Debug: `HERONITE_DEBUG_OVERLAY=window[:<app_name>]` draws the panel in a normal
window (`com.heronite.overlay`) as if the game ran (Hyprland ignores its size requests there; audio changes are skipped
under any `HERONITE_DEBUG_OVERLAY`), `=layer` the real surface; `HERONITE_DEBUG_OVERLAY_PAD=rb,down,a`
plays presses; `HERONITE_DEBUG_OVERLAY_NOTICE=volume|shot|replay` adds a notice (`com.heronite.overlay-notice`);
`HERONITE_DEBUG_OVERLAY_FPS=<limit>` shows the FPS limit row (nothing applied); `HERONITE_DEBUG_OVERLAY=window:none` opens it with no game;
`HERONITE_DEBUG_OVERLAY_KEYBOARD=1` opens
it as the keyboard shortcut does, `=shortcut` registers the real shortcut instead; `HERONITE_DEBUG_OVERLAY_DISPLAY=<px>` stands in for the display height (800 for a Steam Deck, 2160 for 4K), and
the panel's size follows it.

## Rules
- No I/O on the UI thread except tiny reads; use `cx.background_spawn` and
  update the entity when done. Never block render.
- Data folder files (`~/.local/share/heronite`) are written only through
  `heronite_data::json_store` or the backend crates (`crates/*`), which keep
  the electron-store format.
- Images: request through `CollectionView::cover` / `ImageStore` (worker
  threads, LRU of GPU textures). For one-off rasters use
  `heronite_images::*` in `background_spawn` and wrap with
  `RenderImage::new(smallvec![image::Frame::new(decoded.into_image_buffer())])`.
- Icons: SVGs in `app/assets/icons` (MUI/FontAwesome names). Add new ones by
  taking the path of `<Name>` from the
  [`@mui/icons-material`](https://github.com/mui/material-ui/tree/master/packages/mui-icons-material)
  package, register them in `icons.rs` and list the source in
  `THIRD-PARTY-NOTICES.md` when it is a new one.
- Text is written in English (Heronite's `en` locale strings in
  [`public/locales/en/translation.json`](https://github.com/RafaelGoulartB/HeroicGamesLauncher/blob/master/public/locales/en/translation.json))
  and every user-visible string goes through `heronite_i18n`: `tr!("Play")`,
  `tr!("{n} of {total}", n, total)`, `trn!(count, "{n} game", "{n} games")`,
  `n_!` for constant tables. Never build a sentence from fragments
  (word order differs); never translate ids, keys or values that are saved or
  parsed. Add the Portuguese to `crates/i18n/locales/pt-BR/<area>.json`, the
  Spanish to `crates/i18n/locales/es/<area>.json` and the German to
  `crates/i18n/locales/de/<area>.json`;
  `cargo test -p heronite-i18n` fails on a missing or unused key in any.
  Community terms stay in English in every language (Wine, Proton, DXVK,
  shaders, frame generation, overlay, runner, launcher, preset, backlog,
  replay, sandbox, logs,
  store and tool names). Portuguese and Spanish run
  ~25% longer than English and German up to ~35%, with long compound words
  that cannot break: leave room in buttons and pills, and let a label that can
  meet a long word truncate (`truncate()` on a `min_w_0()` box) instead of
  pushing its neighbours out.

## Achievements
- The focus panel's side column (and the Grid drawer) has the Achievements section (`collection/achievements.rs`),
  a game section of its own (Settings → Appearance → Game sections; under Details by default, kept in the
  side column next to Details in the two-column layout). It shows whenever a source gave a list, even with
  nothing unlocked:
  count and bar, the three latest unlocks, the two closest locked ones (progress when the source counts),
  "View all" (`dialogs/achievements.rs`) and "Updated … ago" with a refresh. A game at 100% turns the panel
  gold (`theme::achievement_gold`). For a ROM it asks for the RetroAchievements account or says the file
  matches no set ("Pick the game…" opens the dialog's search).
- The Timeline puts each unlock on its day by the screenshots' rule: a chip ("3 achievements") and the icons
  under the day (`collection/timeline_achievements.rs`); it never adds days.
- Settings → Achievements → RetroAchievements signs in with the username and password (`pages/settings/achievements/retro.rs`;
  the password field is emptied at once, only the token is kept), then shows the account's points, "Sign the
  emulators in too" with each emulator's state under it, hardcore mode and the optional Web API key.
- Rarity colours come from `theme::rarity_tier` / `rarity_colour` (ultra rare < 5 %, rare < 15 %, uncommon
  < 40 %); icons, bars, pills and dates from `ui::achievements`.
- `scripts/shot.sh` on a hidden workspace paints only the frames it captures: textures that arrive late (an
  achievement icon, the focus cover) show after a few more captures.

## Verifying visually (mandatory for UI work)
- Build: `cargo build -p heronite`.
- `scripts/shot.sh /tmp/x.png [wait_s]` opens the app **on Hyprland
  workspace 2 silently**, captures the window with `grim -T` (works
  off-screen), prints PSS and closes it. Never open windows anywhere else,
  never switch workspaces, never move/click the real mouse.
- Debug hooks (env, via `SHOT_ENV="..."`): `HERONITE_DEBUG_SELECT=<runner_appName>`,
  `HERONITE_DEBUG_MENU=<key>` (context menu), `HERONITE_DEBUG_DROPDOWN=view|view:<text>|nav`,
  `HERONITE_DEBUG_DIALOG=settings|statuses|gamesettings:<tab>|onboarding[:<step>]` (extend the match in
  `CollectionView::render` for new dialogs; `settings:<section>` opens that Settings section,
  `onboarding:<step>` "Getting started" where that step lives: `steam`, `stores` or `emulators` open its games view, `appearance` its look view, any other step the three cards).
  `scripts/shot.sh` sets `HERONITE_NO_ONBOARDING=1`: "Getting started" does not open by itself on a new state folder.
- Main Collection screen: compare with the reference PNG (crop the same region with PIL, view both). Dialogs/menus/settings: one sanity check is enough — they need to work and be consistent, not pixel-identical (user decision).
- Anything that WRITES data must be exercised only against a copy:
  `HERONITE_DATA_DIR=/tmp/heronite-fixture-<you> scripts/shot.sh …`
  (copy `/tmp/heronite-fixture`, which mirrors the data folder's JSON).
- Never launch games, emulators, Steam or Ludusavi backups for real.
