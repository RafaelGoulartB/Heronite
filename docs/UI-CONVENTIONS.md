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
  (`Section::GROUPS`): App (General, Appearance, Console mode), Games (Game defaults, Downloads, Other launchers),
  Collection (Library, Metadata, Achievements), While playing (Overlay, Screenshots, Replay), Data (Storage, Saves,
  Backups), System (Advanced, Logs, About). A section = title + lead (`Section::blurb`) +
  captioned cards of rows. The Collection sections live in `app/src/pages/settings/<section>.rs`
  (`appearance`, `metadata/`, `screenshots`, `saves` + `backup/` for Backups: settings, what goes in, then the restore list
  with each backup's parts and an inline "Restore and restart" confirmation; `HERONITE_DEBUG_SETTINGS_ACTION=backup-now|backup-open:<n>|backup-confirm:<n>`) with `State::new(&PageCtx, &EffectiveSettings, …)`
  and `render(&mut SettingsPage, …)`; save with `heronite_data::settings::*` and call `d.saved(result, cx)`
  (updates the page copy of `settings.json` and emits `PageEvent::LibraryChanged`). Work that spawns a process or
  reads folders (Ludusavi detection, screenshot cache size) starts in `SettingsPage::on_section_shown`, never at page
  creation. Open a section from code with `CollectionView::open_settings_section` (or `pages::settings::request_section`
  + navigate); add new sections to `Section` (label, icon, blurb, palette `keywords`, `parse`). The search above the
  sections (Ctrl F, `pages/settings/search.rs`) also finds the settings listed in its `INDEX` (the English labels of
  rows, field labels and captions): list a new setting there; picking it opens its section (and Game defaults tab),
  scrolls to it and flashes it (`ui::find`, drawn by `controls::row` / `caption` and `fields::field_label`). Its keys
  (`nav_keys.rs`) are the Collection search's: ↓ to the list, ↑ ↓ Home End move, ↑ on the first item back to the
  search, Enter / Space open, a letter types in the search; Esc empties the search, then leaves it, then the page. Emulators and ROM
  folders are NOT a Settings section: they live on the ☰ Emulators page.
  Other launchers (`launchers.rs`) is a card per launcher (Heroic, Lutris, Bottles, Faugus on Linux; Playnite on
  Windows, `Launcher::here`): whether it is on this PC and how many games it has, a switch that brings its games in and
  keeps them in sync, "Bring in now", and "Remove them" for the games of one turned off. On Linux the same switch is
  offered where games come from: a tile per launcher on the empty Collection and a row in "Getting started" → Your
  games, shown only for the launchers found with games and not imported yet. On Windows Playnite's games come in only
  from this section (`launcher_imports::OFFERED_ELSEWHERE`); while Playnite runs it holds its library open, and the
  card asks to close it.
  Metadata (`metadata/`, drawn from the metadata service's source registry: a new source needs no code here) opens with
  a summary card ("{n} of {total} games have details" with a progress bar, counted off the UI thread; "Update metadata…"
  opens the "Update metadata" dialog on the games on screen, or on the results of the update that runs). When games need
  attention, a muted line under the count says how many and why ("23 need attention: 9 without a match, 4 without a
  cover": without a match = waiting for review, no details at all or the last try found no game; the reasons with no game
  are left out; read with the count, `MetadataHub::attention`; a cover cropped to a shape in use is not one of them, it is
what the Cover aspect asks for and the Cover shapes card counts it) and "Review 23" sits before "Update metadata…": it opens
  "Review matches" on the games waiting for review, or, when none waits, "Update metadata" on the games that need
  attention (`summary.rs`). Below, the sources are rows in three captioned cards by kind: Game details (IGDB, Steam,
  Store, Lutris, LaunchBox, and the extensions of that kind), Pictures (SteamGridDB, From the web) and Extras
  (HowLongToBeat, Steam tags, ProtonDB); a kind with no source is left out. A closed row shows a drag handle (details sources only), the source's tile (its logo, else two
  letters in its colour), its name over what it gives ("Text · Scores · Covers"), the games it is for ("All games",
  "Games found on Steam"…, or the kind of game it is about: "Visual novels"), its status (a dot and a word: Off, On, Connected, Add a key, Could not connect, Sign in,
  Needs a download, Not on this system, By hand only; an extension also Could not be loaded or Stopped after errors, in
  red), its switch and a chevron. A source on but waiting for a key shows
  its switch grey; one not on this system has its switch locked. A click on the row opens it in place (several can be
  open); the switch does not. A source that is asked from a database it downloads (LaunchBox) shows "Needs a download"
  with its switch grey until it has it; its open row puts the database under the line about it, the same for any such
  source (`download.rs`): before, "Download the database (104.43 MiB)" (primary, with the download icon) and "Nothing is
  downloaded before you ask. The database stays on this PC and is refreshed every week while this source is on."; while
  it comes, a 6px accent bar (420px at most), "Downloading… 43.87 MiB of 104.45 MiB" (then "Preparing the database…
  63%") and "Cancel"; after, a green tick with "Refreshed 2 days ago · 76.74 MiB on this PC", "Refresh now" and
  "Remove the database"; a failed attempt adds "The last download failed: …" in red. Turning the source off keeps the
  database. An open row has a line about the source (the descriptor's `about`), when it is asked
  for a source asked only as a fallback ("Asked when the other sources do not find a game, or when they say it is one
  of these: Visual novels. Pick its match for a game to always ask it."), its options as the source's descriptor lists
  them (text inputs, secrets masked, selects, switches, each with its help or link), "Test" where the source has one
  (it saves what was typed first, runs off the UI thread and shows what came back beside the button: "ProtonDB answered." or the error) and
  "Fills": a chip per field the source provides. A chip of a details source can be clicked to stop taking that field
  from it (struck through); the other sources' chips are plain, because only the merge honours the switch. Dragging a
  details row onto another moves it, and the order is what the merge asks first (the hint "Drag to change the order"
  sits by the caption). Under them, "Cover shapes" ("A shape in use is always kept", `shapes.rs`) is a card with a row
  per cover family: its outline, the name and what it serves (Tall "2:3, 3:4, 11:15 and 5:7 share one picture", Square
  1:1, Wide 16:9), who draws it ("Grid and Focus covers · Game page and lists · Console mode (Minimal)", or "No view
  uses it"), a count with a bar ("153 with their own"; for a Square or Wide in use "2 with their own · 147 cropped",
  the bar accent then amber; "Not kept" for a family nobody draws and the user does not keep), an "In use" pill and a
  switch. The switch keeps the family (`keepCoverFamilies`: fetches and updates fill it, the Covers dialog and the
  Metadata rows list it); it is locked on while a view draws the family, and the Tall row is always so. Then a switch,
  "Download a shape as soon as a view starts using it" ("Picking a square aspect, or a Console mode theme with square or
  wide tiles, fetches those covers by itself", `fillCoverFamilies`, off by default), and, for each family in use that
  some game shows cropped, a line "147 games show a cropped square cover" with "Download square covers" (the whole
  Collection, whatever the switch says; dimmed with "No source that gives square covers is on." when none can be asked;
  the header pill follows the job). Under the cards, "When details arrive" holds the switches that belong to no source: fill
  missing metadata in the background, copy covers to disk, "Allow adult covers" (off by default: covers and backgrounds a
  source marks as suggestive or explicit (an extension can flag its pictures) are never shown or downloaded and the
  next source gives the picture; pictures already saved stay; `allowAdultCovers`), show age ratings, and the rating board shown first. Last,
  "Order for one field (advanced)" is a collapsed block with a line per field and its sources as chips, first to last,
  the first one accented: dragging a chip onto another of the same field puts it there for that field only, and the line
  then offers "Reset" (back to following the list above), else it says "List order". A source that is off, or whose
  field is switched off, is not in the line. Console mode lists the same without dragging or chips: per kind a header,
  a switch per source (status and games in its description) followed by its options (text opens the on-screen
  keyboard), "Place in the list" for details sources and "Test"; then "When details arrive", then "Source asked first
  for each field", one choice per field between "List order" and the details sources that fill it; before "When details
  arrive" it has the same Cover shapes: a header, a switch per family (its aspects and who draws it in the description,
  locked while in use), the automatic download and a row per "Download <family> covers". The Settings search
  finds the sources, their options, "Get more sources" and "Order for one field (advanced)" and opens the row that holds them.
  **Extensions** (sources the community writes, `docs/EXTENSIONS.md`; `extension.rs`, `more.rs`) are rows among the
  others, in the card of their kind, drawn from the same descriptor. After its name a row has a small grey pill,
  "Extension · 1.2.0" ("Extension" when the version is not known); a details extension has the drag handle and takes part
  in the order like a built-in one, and its fields feed the merge. Its open row adds, under "Fills": who made it ("Made by
  okabe") and where it came from ("From the Heronite list", "From lists.example.org", "From lists.example.org, a list you
  removed", or "Installed from a file: nobody checked this source"; amber unless it came from the app's list), "Connects
  to: api.example.org, img.example.org" (every host it may contact, pictures included; "Connects to nothing on the
  internet" when it has none; left out when its manifest could not be read), its homepage as a link with the address
  shown, and, when the list it came from offers a newer version, "Version 1.1.0 is on its list." with "See the update…"
  (opens the gallery with that entry picked); an extension installed from a file is never flagged. Last, "Update from a
  file…" (the file chooser, then the installer, which words an update, the same version or an older one) and "Remove",
  which turns the row's buttons into a question in place: "Remove Wikipedia? Its files are deleted and it is no longer
  asked. What you set for it stays, in case you install it again." with Cancel and a red Remove ("Removing…" while it
  runs; the row closes, and a failure is written beside the buttons). Removing keeps what the user set for the source
  (on or off, options, its place in the order) and forgets nothing, so installing it again finds it as it was.
  An extension that cannot be asked shows a block first in its open row, red tint and a warning icon: "Could not be
  loaded" (the row's status) says "Its files could not be loaded, so it is never asked. Install a version that works,
  or remove it." and has no fields to list; "Stopped after errors" says "It failed too many times in a row, so it is not
  asked for now. It is asked again once you try again or install an update." with "Try again", which switches it on
  again; both add "The last error: …" in small muted text. An extension that was installed or updated opens its row,
  scrolls to it and flashes it once the page lists it.
  Under the sources, a dashed row, "Get more sources" with a plus tile, opens the gallery (`more.rs`); once a copy of a
  list was read it adds how many sources the lists offer that are not installed: "· 2 more from the Heronite list",
  "· 5 more from lists.example.org" (one list), "· 5 more from your lists" (several), "· Nothing new on your lists"; no
  copy, no count. "Install from a file…" at its right goes straight to the file chooser and the installer. Console mode
  shows one muted row, "Get more sources" with "On the desktop" and "Sources made by the community are installed from
  Settings → Metadata on the desktop." (the file chooser and the permissions need the desktop).
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
title, nothing; `data::chosen_caption`), **Favourite star**, **Store icon** (`coverStores`: the logos of the
game's stores in a dark chip at the bottom-left (a ROM's emulator icon instead, `emulators::icon`), first in the row of the category tags, above the strip; each store
once, `+N` past three; with it on, two sub-toggles: "Show every store a game is in" (`coverStoresAll`, the copies'
stores too) and "Hide while grouped by store" (`coverStoresHideGrouped`, off by default); `store_badges`), **Under the pointer** (nothing, lift 4px, glow with an
accent ring). **Zoom** (50 to 300%, `system::ui_scale`; Ctrl + / Ctrl − anywhere, Ctrl 0 back to the system's): the
whole interface, on top of the screen's scale; "Follow the system" takes `GDK_DPI_SCALE`, `QT_SCALE_FACTOR`, GNOME's
text size (Wayland) or Windows' text size. Its slider applies when let go. A window choice like the layout: profiles
keep it. **Picture** behind the Collection (`look::stage_source`: the game's art, one of its screenshots, its
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
with move, store current state, rename and delete, plus "Restore default presets"; stored as `savedViews`) and,
beside it on the same row, the Smart groups select (`blocks.rs`: the picked group, "{n} groups" or "None"; its list ticks
groups, pins and edits them, then "New smart group…" and "Save filters as smart group…" while filters are on). Then two
blocks, each a bold word and a line: **Filter** (what shows: Show, segmented with counts, with "Include hidden" in its
title; Only, one pill per switch: Favourites, With an update, Run offline; Filter by, pills for Genre, Store, Status and
Category that fill `kind:`, and "More" with the other kinds and "Ages up to", whose row shows once picked or while it
filters) and **Organize** (how: Sort, segmented + direction, and Group by, None | Status | Store | More). The footer
has the count, "Manage ▾" (Statuses…, Manage categories…, Library fields… — the fields manager `dialogs/manage_fields`,
also in the palette —, Edit presets…, New smart group…, opening above it) and Reset all. In a small window (under 900px tall or 1200px wide) the panel is compact (`render::Density`): 360px instead of 404, tighter gaps, segments (no icons) and pills. Several
values of a kind add up (any of them); kinds combine. While its box has the keyboard the menu stays open until Esc or
a click outside. Each active filter also appears as a removable `filter_chip` after the button.
The spare middle lane can also show the quick toolbar (`collection/toolbar/`): a track drawn like the Focus / Grid
switch (`ink_a(.06)` pill, 3px padding, 26px segments 2px apart, 12.5px labels and 15px icons at `ink_a(.68)`,
brighter on hover), in priority order Sort with its direction, Show (All / Installed / Not installed), Favourites,
Group by, then saved Presets, using the same labels and actions as the View menu, which always keeps the complete
controls and preset management. A control at its default is a 30px icon; a value in use adds its label and a
chevron (a preset shows its colour dot and name; the grouping, with a stacked-groups icon, reads "by Status" with
"by" dimmed, so it is not taken for a filter), and a filter in use (Show, Favourites) is the accent pill
(`accent_tint`). The selects open on hover like the header's other menus (no tooltip, the chevron turns while
open; the Sort list has each field's icon, Presets their dots); the direction and Favourites have tooltips. While
a control is shown its filter chip is not (Show and Favourites), so nothing appears twice; the chip
returns when the toolbar is off or the control does not fit. It is off by default: hovering the View button
slides out a 24px arrow joined to it like a split button (one faint fill, out in about 130ms, back in about 300ms; it stays out while the View menu is open; the search
keeps its width, the lane after it moves) that turns it on (→) or off (←), the same preference as Settings → Appearance →
Layout → **Quick toolbar controls** (`quickToolbarControls` in `appearance.json`, independent of filters and
presets). Visibility is a priority prefix of shaped localized segment widths within the measured flex lane, after
reserving the remaining filter chips, 48px for dragging and the arrow's 24px, so hovering never hides a control
(`toolbar/layout.rs`, `fit`). The lane starts 6px after the View button, as between the left controls. Search, window controls,
Focus / Grid and activity pills keep their space. Measurement requests one redraw only when the lane changes,
never writes settings, and closes a menu whose trigger no longer fits. Long preset names truncate with their full
name in a tooltip.
**Smart groups** (`collection/smart_groups.rs`, editor `dialogs/smart_group/`, data `heronite_data::smart_groups`):
groups of games defined by rules that keep themselves current. Nothing of them shows on the Collection until the user
makes one, by user decision: the way in is the View menu's Smart groups select, beside Presets (its list: tick to
filter, several add up like categories; pin; pencil to edit; "New smart group…"; "Save filters as smart group…", which
starts from the filters on screen), also "Manage ▾" → "New smart group…". The menu's search finds them (`group:`) and a
picked group is a filter chip like the others. A pinned group is a segment of its own track in the header lane, after
the quick controls (or first, while they are off), fitted by the same `layout::fit` prefix and hiding its chip while
shown; a click shows only its games, a second click every game, a right click opens the editor. The editor: name,
colour (`theme::GROUP_COLOURS` or the accent), "Games that match all / any of these conditions", rows of field
(its menu searchable, under The game, How you play it — played, time played, in the last 30 days, times played, average
session, last and first played, progress to beat it — Details and Library fields), comparison and value (a list field ticks several values, a number has its unit), "Except games that match any of
these", the games it holds now (counted after each change, not each frame) and the pin; a rule nested deeper than
that is kept and only the name, colour and pin change; a status or category the rules name that this PC does not have
(made on another PC that syncs) is listed in a warning; "Delete group" is the red button with the bin. The groups and
the custom categories sync with the Collection (no switch of their own) and are in the backups' Collection part. In console mode the groups are reached only from the Filters
panel (Y): its Smart group row picks any group (the theme's tabs stay as they are). `HERONITE_DEBUG_DIALOG=smartgroup|smartgroup:<id>|smartgroup-filters`,
`HERONITE_DEBUG_SMARTGROUP_MENU=field[:<text>]` (the first row's field menu open, searched).
Right: the Focus / Grid switch (`view_mode.rs`: a solid pill slides on a spring under the active mode, which
carries a chevron; a click on it opens that mode's options and a click on the other mode switches to it) and bulk
metadata. The options (`view_options.rs`, 300px, scrolling when the window is short): in Focus the list as covers or
rows, the covers' size (Small fits one more per row, Large one less; `focusCoverSize`) and the game sections shown;
in Grid the cover size, what goes under the covers (Nothing, Title, Info, Title and info; `gridLabel`, Info being
Appearance's "Under the cover" line) and "Open details on select" (`gridOpenOnSelect`; off, a double click or Enter
opens the drawer); then, in both, the covers' look, the same settings as Appearance (Cover aspect, where Banner
draws each game's wide cover, else its background, else its tall cover cropped, with the notice below it, Corners, Density, Under the pointer, Favourite star, Store icon); then "More in Appearance" and
"Reset to defaults" (the mode's options and the covers' look; dimmed when all are at their default). Header menus
open on click or after a 140ms hover (`OPEN_DELAY`) and close 180ms after the pointer leaves; the view options open
on click only and close on a click outside, Esc or a change of mode. The ☰ menu (`nav.rs`) is a list:
pages grouped (Collection, Downloads, Statistics | Stores, Emulators, Wine Manager), then Add game and Add from folder, Console mode,
then Settings and Command palette with their keys; each row = icon square + title + meta on the right. The Collection
row shows a refresh button after its count while hovered (spinning, and always shown, while the library reloads). "Stores" is one
row for Epic Games, GOG, Amazon Games, Steam and the store clients under Wine (Battle.net, EA app, Ubisoft Connect,
Rockstar Games): each store keeps its own page, laid out by `pages::stores::sheet` (on `pages::side_nav`): on the left, as in Settings, the
list of the stores in two groups (Stores: Epic Games, GOG, Amazon Games, Steam; Store apps: the clients under Wine), each
with its icon, a line on where it stands (its games, "Not logged in", "Not set up", "Restart once"...) and a dot (green
signed in / set up, amber something to do, accent working, ring not used), then a dashed "Other launchers" link at the
bottom that opens Settings → Other launchers; on the right the store's name, its lead and its page (LB / RB switch
stores in console mode, whose pages keep their tabs). The list reads the services and repaints only when what it shows
changes (`stores::watch`); the row (and the console sidebar's) opens the store shown last. A client's page
(`pages/clients/`, one `ClientPage<K>` for the four) is a card with the client's state and its main action (Set up, the
setup's step with Cancel, Open, Close), its prefix and Wine with "Wine settings" (`dialogs/client_settings`: the game
settings panel bound to the client's own settings), the sign-in the client's files show, on Linux its Shortcuts (a
switch for the Desktop and one for the application menu: `heronite-client-<id>.desktop` running `--open-client <id>`,
with the icon read from the client's `.exe`; `services/clients/shortcuts.rs`, `HERONITE_DEBUG_CLIENT=<id>:shortcuts`),
the games it installed (Show selects one in the Collection) and an About card with the anti-cheats that keep some of
its games off Linux.
The ☰ Emulators page (`pages/emulators/`) has the same layout (`side_nav`): the list holds "All emulators" (counts, what
needs attention with its fix, every ROM folder, "Scan all folders now"), then each emulator with its icon
(`emulators::icon`: Dolphin's and RetroArch's logos, an icon drawn for each other emulator of the catalog, a gamepad
for a custom one), a line ("10 games · 1 folder", "No ROM folders", "Executable not found") and a dot (red when it does
not start), a search field from 8 emulators on, and "+ Add emulator" (a menu: Add emulator…, Detect emulators…, Scan a
folder for emulators…); a dashed link at the bottom opens the stores. An emulator's page: Open, Edit and ⋯ (Duplicate,
Remove…) in the header, a red banner when it does not start, its ROM folders (Scan now, Review…, ⋯ with Edit and
Remove…) on the left, its games, platforms and profiles on the right. With no emulator, the page offers the three ways
to add one.

**Downloads page** (`pages/downloads/`): no title (the slim header names it), a lead that sums the queue up
("Downloading X · 2 more in the queue · 20 GiB left · all done around 21:39"). The running element sits on a
stage (`stage.rs`) over its wide art (`InstallParams::art`, faded from the panel colour): tall cover, status pill,
percent, bar, download / disk speed, time left and the clock time it ends, a chart of the last reports' speeds
(`Downloads::speed_samples`, `widgets::sparkline`), open folder, stop (the keep / delete question opens under the
stage) and a round pause / resume. Paused and offline say why nothing moves. With nothing running the stage is a slim
"Nothing is downloading" card, or for 30 minutes the game that just installed with Play. Then the updates banner
("Update all"), and side by side (`lists.rs`) the waiting elements, which a drag moves (`move_in_queue_to`), each
with the time it starts, and the history with All / Done / Did not finish, Play for installs, Retry or Download
again, and the disk space hint for a full drive. Under them the free space of the queue's folder and what the queue
needs. An empty queue and history show what the page is for. The Collection header has a downloads pill
(`collection/downloads_pill.rs`): ring, title, percent and speed, its popover the queue with pause / resume; once
the queue empties it announces what finished (ready to play with Play, or failed) until Downloads is opened. The ☰
row shows the percent and the queue's length; a waiting game's card says its place ("Queued · #2").
`HERONITE_DEBUG_FAKE_DOWNLOAD=<appName>` (a fake queue), `HERONITE_DEBUG_FAKE_PAUSED=1` (paused),
`HERONITE_DEBUG_DOWNLOADS_STOP=<appName>` (the stop question), `HERONITE_DEBUG_DROPDOWN=downloads` (the popover).

**Focus panel** (`collection/focus/`, the second place that departs from Heronite, by user decision): a fixed left column
(`focus_left.rs`: big cover, Play, status selector + favourite / metadata / settings, the journal's "Where I left off"
card only once the player wrote one (click: the Journal; also in the grid drawer), star rating, facts card with score
(a green / amber / red badge from 75 / 50, with its source, IGDB or Steam, unless typed by hand), library, save backup and Linux) next to the scrolling details; no footer. The title + meta line sit above both, fixed and
aligned with the cover's left edge, so the cover, "Screenshots" and "Details" start on one line; the gap left column → main
equals main → side (`COL_GAP`). `focus::layout(panel_w)` picks the widths: narrow = everything scrolls in one column with the
left block as a row under the title; otherwise left column + main/side (stacked when tight), centred as a group in wide panels. The main column holds Screenshots, Overview, **Timeline** (`timeline.rs`: days of play with
the screenshots of their sessions, thumbnails only; the replays saved that day come first on a row of their own, as
larger posters (`replays.rs`: a frame of the video with a replay-yellow ring, `theme::replay`, the time, the length,
the note written about it; a click plays it), the screenshots step down to 96×54 under them, the day's dot becomes a
yellow ring and "N replays" opens the Journal, where each replay is a card with its poster, its words, Watch and Open
folder; a replay nothing was written about is not counted as a note. Posters are made by ffmpeg on one idle-priority
worker of their own, only for what is on screen, and kept in the screenshot preview cache; a bar per day of the last 30 in the title when the column has room,
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
110-240, set by the slider in the Grid mode's options; covers are requested in 16px steps). The selected game opens in a glass drawer on the
right (480px at most, inset 10px, the grid narrows beside it): sharp hero art on top with a close button, the cover
reaching into it next to the title and meta line, then the focus panel's own pieces (Play, status row, rating, facts
card), three tiles (Played, Last played, Sessions) and the details stacked in one column (`focus::stacked_details`),
ending in "Open game page" (back to the Focus view). Keys while nothing floats over it: arrows move the selection
(the drawer follows), Enter plays an installed game, Esc closes the drawer (the selection stays; selecting a game opens
it again). The Focus list takes the same arrows and Enter (play an installed game). `ui-state.json` keeps `gridView`, `gridCoverWidth`, `gridLabel` (read from the older `gridTitles` when missing) and `gridOpenOnSelect`. Console mode is not affected.

**Focus list as rows** (`collection/list_row.rs`, by user decision): in the Focus view the Focus mode's options in the
header (also in the palette) turn the list into one game per row, 44px tall: a 26px
cover thumbnail, the title (a star on a favourite with badges on, the hidden icon), the card's caption line on the right
(dropped under 220px of row) and the play button under the pointer. Same groups, sticky headers (on a denser shade:
the rows' titles would read through the label), selection (accent tint and border), selecting several games (the tick
before the thumbnail), context menu, keys and Epic progress (label over a bar, or the "Update" pill, instead of the
caption). The list keeps its width, dragged or not, and takes one column. `ui-state.json` keeps `focusList`.

**Wider panels** (`collection/resize.rs`): the Focus list column and the grid's drawer grow when their inner edge is
dragged (the cursor turns to col-resize over it; nothing else is drawn). Their default width for the window is the narrowest the list gets;
the drawer can also go 5% under it, to the nearest 10px (480 → 460). Dragging back to the default (within 6px), or a double click on the edge, returns to the default. A wider Focus list fits more columns of the
same covers; the focus panel keeps at least 560px and the grid 320px. `ui-state.json` keeps `listColumnWidth` and
`gridDrawerWidth` (`null` = default); Settings → Appearance → Layout → "Reset widths" clears both.

**Selecting several games** (`collection/selection.rs`, `dock.rs`, by user decision: proposal 2 "Dock" of
`propostas/selecao-v1`). Ctrl click ticks a game (with the focused one when nothing is selected yet), Shift click adds
the cards from the last one clicked, Ctrl A selects every card on screen, and the card context menu's last row,
"Select multiple" ("Add to selection" while selecting), starts it with the mouse. No tick shows on hover: only while
games are selected do the cards get a round tick on the top-right (filled = selected, the others dimmed, no play bar),
a plain click ticks or unticks, group headers get "Select group", the grid's drawer steps aside, and the dock floats at
the bottom: × (or Esc) ends it, the count and "Select all N", Status ▾ and Categories ▾ (popovers above the dock),
Favourite, Hide, Metadata ▾ (a popover above the dock, see below) and Edit (F3). Right-clicking a selected card opens
the same actions as a menu, its Metadata row a flyout with the popover's rows. Categories have three states (round tick: all, dash: some, ring: none); each click moves
some → all → none → as it was (Playnite's assign / remove / leave alone), the next step shows under the pointer. Every
batch change writes off the UI thread and its toast offers Undo (also Ctrl Z, 20 steps). F3 opens
`dialogs/batch_edit.rs`: a drawer with the series and the tag fields of all the selected games; a tag only some have is
dashed with "3/7" (click: give it to all), × takes one out of all, typing + Enter adds one to all; nothing is written
until "Apply". Debug: `HERONITE_DEBUG_MULTI=<n>`, `HERONITE_DEBUG_DOCK=status|categories|metadata`,
`HERONITE_DEBUG_DIALOG=batchedit`, `HERONITE_DEBUG_BATCH=status:<id>,cat:<name>,fav,hide,undo` (data copy only),
`HERONITE_DEBUG_MENU_METADATA=1` (with `HERONITE_DEBUG_MENU` on a selected game: the batch menu's Metadata flyout).

The **Metadata popover** (`collection/metadata_menu.rs`, 300px, above the dock's Metadata button; the dock's button
toggles it and Esc closes it) opens with the selection's count ("12 games") and has three rows, an icon and a title over a
hint, with one more for each cover family some of the selection show cropped (below). "Fill what is missing" (accent icon) starts an update of the selected games at once and closes the popover: every
empty field of theirs, extras included, from every source that is on (`BulkArgs::fill_missing`); no dialog opens, the
header pill follows it. Its hint says what it would write ("34 fields in 12 games"), "Counting…" while that is read off
the UI thread (once per selection, when the menu first opens), or "Nothing is missing" with the row dimmed and
disabled. Enter does it while the popover is open (the dock's rows show the key; the context menu's do not).
"Fix matches…" (hint "Check which game each one is", or "3 of these wait for review" when some of the selection are in
the review queue) opens "Review matches" on the selection: the games waiting for review first, then the others, whose
proposals are asked from the sources live (`fix_selection_matches`). "More options…" (hint "Pick the games, fields and
sources") opens "Update metadata" on the selection. The selection's context menu has the same rows in its
"Metadata" flyout (hover or click, like Status and Categories).
"Download square covers" (or wide; a row after "Fill what is missing" for each family a view draws, other than the
tall one, that some of the selection show cropped, `GameCounts::cropped`, read with the counts) has the hint "12 show a
crop on the desktop", "…in Console mode" or "…on the desktop and in Console mode", by who draws the family. It
queues the download of that family for the selected games that lack one (`MetadataHub::download_family`; whatever
the automatic download switch says) and closes the popover, and the header pill follows the job. "Fill what is
missing" also fills the kept families.

**Update metadata** (`dialogs/metadata_update/`: `mod.rs` the dialog and its stepper, `games.rs`, `what.rs` and
`results.rs` the three steps, `words.rs` how an outcome reads, shared with the pill, `hub.rs` the owner of the jobs,
`fake.rs` the debug jobs): one dialog, 760px, to update the details and pictures of several games. It opens from the
Metadata popover's "More options…" (on the selection), from the palette's "Update metadata…" (Actions; on the selection
when games are selected, else on the games on screen) and from Settings → Metadata's "Update metadata…" (on the games on
screen); opened while an update runs, or from the pill's "See all", it goes straight to step 3. A stepper of three
numbered pills (Games, What to update, Update) heads it: the active one is tinted and any other is clicked when it can be
shown (What to update needs games, Update a job). Esc closes a question first (below), then the dialog. Hiding the dialog
never stops a job.
- **1 · Games** (`games.rs`): a radio card per set, its count on the right: "Selected games" ("The games ticked in the
  Collection", only when it was opened from a selection, and then the default), "Games on screen" ("What the list shows
  now", or "…now, with its filters: “zelda”, Platform: PlayStation 2" when a search or a filter is on; the default
  otherwise), "Whole Collection" and "Games that need attention" ("Waiting for review, or missing details, a description or a cover",
  counted off the UI thread, with a chip per reason that has games under it: "To review 5", "No details 14", "Failed last
  time 3", "No description 8", "No cover 4"; a cover cropped to a shape in use is not counted here nor in the
  Settings summary: it is what the Cover aspect asks for, and the dock's "Download <family> covers" and the Cover
  shapes card deal with it; the Settings summary's "Review" opens the dialog on this card). A strip of the first ten
  covers and "N games" sits under the cards. Footer: the count, Cancel, Next (off with no game).
- **2 · What to update** (`what.rs`): "How", three cards in a row, one picked (Fill what is empty is the default), each
  with its consequence: Fill what is empty ("Nothing you have changes"), Refresh ("What you edited by hand stays"),
  Replace everything ("Your edits too"). A card of fields in three columns, Details (its ticks in two columns), Pictures (a tick per kept cover
  family, "Cover · Tall", "Cover · Square", "Cover · Wide", then "Background"; a family that is not kept has none and no job
  fills it) and Extras, a round tick per field (all on to start) and an "All" link per column that turns the column off, or on again.
  "From": a chip per source the job may ask, with its mark (`labels::mark`), in the order of Settings → Metadata; a click
  leaves one out (struck through), and a source with nothing for the chosen games (counted without network, off the UI
  thread) is dimmed, cannot be clicked and is named in the line beside "From" ("In your order. Click one to leave it out
  of this update. Nothing for these games: Lutris."). The footer sums the choice up ("7 games · empty fields only ·
  15 fields · 5 sources"; "keeps hand edits", "replaces hand edits") beside Back and "Update 7 games", which is off
  without a game, a field or a source. While another update runs the summary reads "It starts after the update running
  now": the job waits its turn. Replace everything asks once: the footer turns into a warning line ("This replaces what
  you edited by hand in these 7 games.") with Cancel and a red "Replace in 7 games"; choosing another mode or Esc
  closes it, and once confirmed it is not asked again by that dialog.
- **3 · Update** (`results.rs`): the line "3 of 7 games" (after it ended, "Updated 5 of 7 games"), a progress bar
  (accent; amber when games need a look; green otherwise) and the pills of the results: "All 7" and one per outcome
  that has games, with its count (Updated, To review, No change, Not found, Failed, Not done), which filter the list. While it
  runs a tinted row shows the game being fetched ("Fetching…", "Stopping after this game…" after Stop). The list is a
  `uniform_list` of 48px rows, so only the rows in view are built (a job of 2,000 games stays cheap): the cover, the
  title, the fields that changed as chips with their source's mark (three, then "+2"), or why nothing did ("Nothing was
  missing", "The sources had nothing new", "No source found this game", "A source did not answer: try again later", "Could not be
  saved" with the error in a tooltip, "Already had details", "No source is for this game", "The update stopped before
  it", "You said no source has this game"; for a game sent to review, why: "Found by a similar name only", "The sources
  found different games", "You asked to check it"), and the outcome's dot and word. A game sent to review has a
  "Review" button where the word would be: it opens "Review matches" on the job's games sent to review, at that game. While it runs the footer says "The update keeps going when this is hidden"
  beside Stop and Hide; Stop finishes the game in hand and leaves the rest as "Not done". Once it ended: "Done in 2 min 05 s ·
  31 fields updated" ("filled" for a Fill; "Stopped after 4 of 7 games · …" after a Stop), "Undo this update" (only when
  the job changed something and can still be undone), "Review 3…" (when it sent games to review: "Review matches" on
  them), "Update again" (back to step 2) and Done. Undo asks first, in the
  footer: "Put back what this update changed in 5 games? Changes made since stay." with Cancel and a red Undo; it works
  off the UI thread ("Undoing…"), then the line reads "Undone: 5 games put back" and a toast says "Update undone: 5
  games put back". A job that
  waits behind the running one shows "Waiting for the update running now" and "It starts after N more games.", with
  "Clear the queue" (back to step 2) and Hide; the running job's step 3 lists "N more updates waiting" and "Clear the
  queue" too. With no job at all, step 3 says "No update has run yet."
One job runs at a time and the rest queue in order; Undo takes back the last job only (`MetadataHub::can_undo`).

**Metadata pill** (`collection/metadata_pill.rs`, `Menu::Metadata`, modelled on the downloads pill): in the Collection
header's right cluster, between the Focus / Grid switch and the window controls, like the downloads pill a hover or a
click opens its popover. It exists for a bulk update (the Metadata popover's fill, or the dialog's) and, in amber, for
the idle fill when it queued games for review (below). Running: a ring with the percent, "Metadata", the percent and "+2" when updates wait. Ended: a full ring
with "Metadata ·" and a short result, in amber with a warning glyph and an amber border when games need a look ("3 to review" when the update sent games to
review, whatever else happened; else "3 not found" when all of them were not found, "3 failed" when none was, "3 need a
look" otherwise), or green with a
check ("12 updated", or "nothing new"). It stays after the end until its popover is opened or the dialog showed the
results; then it goes, and the next update brings it back. The popover (360px) has the title ("Updating 7 games",
"Updated 5 of 7 games"), a line under it ("3 done · now: Hades", "Stopping after this game…", "Done in 2 min 05 s · 31 fields
updated", "Undone: 5 games put back"), a round Stop while it runs, the progress bar and three results as in step 3
(the last ones while it runs; the games that need a look, those to review included, when the update ended with some;
else the last ones). Below: "N more updates waiting" when any (replaced by "3 to review" with an amber dot while games
wait for review), Undo (as in the dialog but without the question), "See all", which opens the dialog on the results,
and "Review", which opens "Review matches" on the games the update sent to review. A one-click fill (the Metadata popover's) also says how it ended with a toast: "Updated 12
games" with Undo (while the update can still be undone), "3 games were not found" when nothing was updated and some
games were not found or failed, or "Nothing new for these games". The dialog's updates show no toast,
only the pill. The idle fill (`MetadataHub::idle_review`, "fill missing metadata in the background") never opens a
dialog: when it queued games for review, the pill reads "Metadata · 4 to review" in amber, with no job behind it, until
the review or the pill's popover opens (then it goes; the games stay in the queue). That popover (360px) says "4 games to
review" and "The sources were not sure which game these are.", lists the first three (cover, title, why: "Found by a
similar name only"…) and has "Review" for the whole queue. Debug: `HERONITE_DEBUG_DIALOG=wizard[:<step>[:<state>]]` opens "Update metadata" (on the selection when
`HERONITE_DEBUG_MULTI` made one) at step `1`, `2`, `2:replace` (Replace everything with its question), `3` or `3:undo`
(the undo's question); `HERONITE_DEBUG_WIZARD_LIMIT=<n>` starts a real update of the first n games on screen (data copy
only); `HERONITE_DEBUG_METADATA_JOB=running|done|clean|review|idle-review[+queued]` makes every bulk job up (no
network, nothing written: held at ~60% with a game "being fetched", ended with games not found and failed, ended clean,
or ended with games sent to review; `idle-review` starts no job and fills the review queue the hub shows with the first
games on screen, as the idle fill would; `+queued` adds one waiting) and starts one over the games on screen, and
"Review matches" opened meanwhile is made up too; `HERONITE_DEBUG_DROPDOWN=metadata` opens the pill's popover.

**Review matches** (`dialogs/metadata_review/`: `mod.rs` the dialog, its keys and its end state, `game.rs` the strip and
the game on the left, `choices.rs` the cards and the search, `actions.rs` what reads, searches and saves off the UI
thread, `fake.rs` the debug review; opened by `collection/metadata_review.rs`): one dialog, 900px, where the user says
which game of the sources a game of the Collection is when the sources were not sure. A *match* is the game of a source
(IGDB, Steam…) that a game of the Collection was identified as. It opens from the header pill's "Review" (the whole
queue), the palette's "Review matches" (Actions; dimmed with "No game waits for review" while the queue is empty),
Settings → Metadata's "Review 23" (the queue), the "Review" buttons of an update's results and of the pill's popover
(that update's games sent to review, at the game clicked) and "Fix matches…" (the selection). Opening it counts as seeing
what the pill announced. It goes through the games one at a time:
- **The strip** on top: up to 14 covers around the game shown, the current one ringed in the accent; a game already
  matched or put "in no source" is dimmed, a skipped one is not; a click goes to that game (not while something saves);
  "2 of 7" sits at its right.
- **The game** (left, 220px): its cover (a placeholder when it has none), title, where it comes from ("PlayStation 2 ·
  DuckStation", "Steam") and "Why you are asked" with one sentence ("Only one source found a game for this name, and its
  name is not quite the same.", "The sources found different games for this name.", "You asked to check which game
  this is.", "The sources were not sure which game this is.").
- **The choices** (right, "Which one is it?"): a radio card per game the sources proposed (thumbnail, name, "2011 ·
  PC, Mac", with its name in another script after the year when the source gives one: "2009 · シュタインズ・ゲート ·
  Windows"; never an adult thumbnail unless adult covers are allowed), the marks of the sources that proposed it with their names as tooltips and "3 sources agree" or "1 source
  found it"), the one most sources agree on first and picked at start (ties keep the sources' order); a click, or ↑ ↓,
  picks another. A green pill ("Hash matches", "Steam id matches", "Id matches") says that a source found that game by
  ROM hash, Steam id or id, so the match is certain; a match by name has none. A game that was queued opens with what the
  sources proposed when it was queued ("Reading what the sources proposed…", no network); one that was not ("Fix
  matches…" on a game that waits for nothing) asks the sources live ("Asking the sources about this game…"). Offline
  shows the reason in place of the cards.
- **The search** under the cards ("Search another name in every source"): Enter asks every source for that name (empty:
  the game's title) and the cards become what they found, sorted and picked as above; a line under the field in amber
  says what is missing ("Nothing found for “zelda”.", "Could not reach IGDB.", "Needs a key: SteamGridDB.", "No source
  that can search by name is on.").
- **The footer**: the keys ("↑ ↓ pick · Enter use · S skip"; only "S skip" when nothing was found), Skip, "In no
  source" and "Use this match". Skip leaves the game in the queue. "In no source" says that no source has this game:
  automatic updates stop asking about it until it is fetched by hand or its title changes (its results say "You said no
  source has this game"). "Use this match" (needs a pick) looks the picked game up in each source that proposed it,
  merges and saves it pinned, so later updates use the same game ("Saving…"); a failure (offline, a source that could
  not read the game) shows in red under the search and the game stays. The keys do not act while the search field has
  the keyboard (Enter searches there). After an answer, the dialog goes to the next game not answered yet.
- **A game with nothing found**: the cards give way to "No source found a game for this name. Search another name, or say
  it is in no source."; "In no source" becomes the primary button and "Use this match" goes.
- **The end**: a green check, "All reviewed" and what was done ("4 matched · 1 in no source · 2 skipped") with Done
  (Enter closes). Opened with no game: "No game waits for review", "Games the sources are not sure about wait here.".

*When a game is sent to review* (`crates/metadata/src/confidence.rs`; every automatic path obeys it, the per-game dialog
saves what the user picked): a match found by the game's own record, an id, the Steam id, the ROM hash or the exact name
is sure and saved; a match by a similar name is saved only when at least two sources found the same game (or when it is
the same game as a sure one), while a lone similar-name match, or sources that found different games, is not saved: the
game waits in the review queue (`local_library/metadata-review.json`) and counts as needing attention.
`HERONITE_DEBUG_DIALOG=review[:<state>]` opens a made-up "Review matches" on the selection, else the first three games on
screen (nothing asked or written): `review` (four, two and one choices in turn; the first game has a proposal of a made-up
extension, `novels`, with its original title), `review:search` (after a search by
another name, one source unreachable), `review:empty` (one game, nothing found), `review:done` (the end state),
`review:busy` (asking the sources), `review:saving`, `review:error` (offline).

**Metadata of one game** (`dialogs/metadata/`: `mod.rs` the dialog, its head and footer, `found_as.rs` the left column,
`change.rs` "Change", `fields.rs` the field rows, `actions.rs` what it asks, picks and saves, `fake.rs` the debug
preview): one dialog, 1060px, with two columns 470px high that scroll inside, titled "Metadata". It opens from the
context menu's and the palette's "Metadata" (a game action) and from Game settings → Game → Details' "Fetch metadata", which opens
it over the drawer (Apply then updates the drawer's details). It asks every source when it opens ("Asking every
source…"), and again after a match is pinned and after "Search every source again". The head has the cover, the title and
"Steam · last updated 3 weeks ago".
- **Found as** (left, 310px): a row per details source that was asked: its mark, its name and the game it matched ("Hades
  · 2020 · by its Steam id"; after the title and year, how it found it: "by its id", "by its Steam id", "by ROM hash",
  "by name" or "by a similar name"), or "Not found" or "Could not be reached". "Change" (on a found match) and "Search" (on
  a source that found nothing) are on sources that can be searched. A pinned match, one the user chose, has a pin with the
  tooltip "Pinned: every update uses this game". A match the merge ignored because it is another game than the others
  found is amber and reads "(left out: another game)" (not while the game waits for review, where nothing is merged).
  A source asked only as a fallback (an extension such as VNDB or Wikipedia) that the others made unnecessary keeps
  its row, "Not asked: the others found it", with "Search". The other sources not asked fold into one muted line per
  reason under the rows: "Not for this game: Lutris", "Off: …", "Needs a key: …", "Not asked: …". Above the rows: the offline line, the IGDB key hint with
  "Open Settings → Metadata" (closes the dialog), and, for a game that waits for review, "The sources are not sure which
  game this is: nothing is picked for you. Pick the values, or change a match." The refresh button below ("Search every
  source again") asks again.
- **Change** replaces the column: a back arrow, "Which game of IGDB is it?", the hits of that source alone for the game's
  title (a search field, "Search this source", Enter searches again; a source may read a typed id or address as that
  entry, as the VNDB extension does with v17; each hit's year, its name in another script and platforms under it), the
  one in use checked. A click pins that game and asks every source again; when the match is pinned, "Automatic" ("Let IGDB find the game by itself") comes first and
  unpins it. A line says why there are no hits ("Nothing found for “zelda”.", "This source is off.", "This source needs a
  key: add it in Settings → Metadata.", "Could not reach this source. Check your connection and try again."). Esc closes
  Change, then an open select, then the dialog.
- **The fields** (right): a header (Field, What will be saved, Source), then a row per field that has a value now or
  from some source, in the usual order, with a cover row per kept family after the tall one, "Cover · Tall", "Cover ·
  Square" and "Cover · Wide" (the tall one always, the others even when no source has a picture of that shape), and
  "Choose covers…" under the tall row's name, which opens "Covers" over this dialog (Esc closes it first; a picture set
  there reaches the drawer behind): the field's name, the value that will be saved (two lines at most, "Empty" when
  none; a cover or a background as a small picture), where it comes from, and the lock. Sources with the same value are
  one choice ("IGDB + Lutris"), and so is the game's value ("· current"); with two choices or more they are segments on
  an `ink_a(.06)` track, written out (the source names, "Keep current" when only the game has it), the picked one in
  the accent. A click picks it; the segment under the pointer shows its value in the middle without picking. With one
  choice there is nothing to pick: the sources' marks and a line ("Only IGDB", "IGDB, Lutris · same as now", "No
  source has it"). Rows grow when the segments wrap. With nothing picked, a field takes what the merge would save (the
  order of Settings → Metadata). The lock (amber when
  locked, tooltips "Lock this field" and "Locked: later updates leave this field as it is. Click to unlock.") keeps
  the field as it is in every later update; a field already locked in the record has its segments dimmed and keeps the
  game's value until it is unlocked. While the sources answer: "The fields appear when the sources answered."; with no
  source finding the game and no details: "No source found this game, and it has no details yet. Search a source by
  another name, on the left."
- **Footer**: "A locked field stays as it is in every later update.", "Everything from…" (a select of the sources that found
  the game: picks that source for every field it has, except the locked ones), Close and Apply, which saves the picks,
  pins and locks of what is on screen without asking the sources again ("Saving…") and closes; with no source having
  answered it saves only the locks.
`HERONITE_DEBUG_DIALOG=metadata:<state>` opens the selected game's "Metadata" over a made-up preview: `fake` (a pinned
match, a match left out, a locked field, a field picked from another source, a fallback source not asked), `fallback` (as
`fake`, with that source found by name; `novels`, a made-up extension id, shows by its id and cannot be searched),
`emulated` (an emulated game: Steam not for it, LaunchBox found by the dump's name, its line with the platform: a source of emulated games says which platform it matched on; libretro-thumbnails gives box art and, a
pictures source, has no row there), `change` (the same with "Change" open),
`none` (no source found the game), `offline`, `nokey` (no IGDB key), `covers` (as `fake`, scrolled to the cover rows);
plain `metadata` asks the sources for real.
`HERONITE_DEBUG_DIALOG=covers[:<state>]` opens the selected game's "Covers" over made-up searches (pictures the
Collection already has; nothing asked or written): no state opens on the family the desktop draws, `tall`, `square`,
`wide`, `background` on that slot, `none` (nothing found), `busy` (searching), `saving` (a picture being
downloaded), `error` (its download failed). `HERONITE_DEBUG_ASPECT_NOTICE=offer[:wide]|started[:wide]` shows the
Cover aspect notice of the square (or wide) family over the real counts, with the View options' Covers section open
(with `HERONITE_DEBUG_DROPDOWN=display`) and in Settings → Appearance; `started` needs
`HERONITE_DEBUG_METADATA_JOB=running`, which makes the download up.


**Menus** (`ui::dropdown`): `menu_panel(min_w)`, `menu_item(id, active)` (radius 10,
14px, hover .07, active = accent tint), `menu_separator()`, `dropdown(..)`. The
Collection's context menu (`collection/context_menu.rs::item`/`paper`) uses the
same look and groups a game's rows with separators (`menu_place`): Play (and the other ways to play or install);
Status, Categories, favourite, hide, recent; Settings, Edit game, Metadata (the game's Metadata dialog), the folder, "Saves ›" (back up now and the game's automatic backup switch, which keeps the flyout open); "Manage installation ›" (the store's verify,
modify, move, relocate, EOS in a flyout; a single one stays a plain row); shortcuts; uninstall / remove; View log; Select multiple. A new menu action gets a
place there; rare ones (store page, covers, playtime, cloud sync) stay in the palette only (`palette_only`); other context-menu rows must call `context_menu::item`.

**Dialogs** (`ui::dialog`): `dialog(DialogProps, body, footer, on_close)` = glass
modal (radius 26, title Rubik 22, round close button, Collection darkened by
`rgba(0,0,0,.55)`); `dialog_scrolled(.., &ScrollHandle, ..)` is the same with a draggable scrollbar on a long body; `drawer(DrawerProps { id, width }, header, body, footer,
on_close)` = right-hand sheet inset 10px, radius 24, with fixed header/footer
slots (build the header with `kicker`, `dialog_title`, `close_button`).

**Covers per shape** (`heronite_data::cover_family`, `collection/family_art.rs`, `dialogs/covers/words.rs` for every sentence
that names a shape): a game has a cover per shape, in three families: Tall (2:3, 3:4, 11:15 and 5:7 share one picture),
Square (1:1) and Wide (16:9). Each view asks for the family it draws: the grid and Focus cards the Cover aspect's (Steam,
IGDB, GOG and DVD Tall, Square Square, Banner Wide), the game page, lists and drawer always the tall one, and a Console
mode theme the shape of its tiles (a theme with the Xbox layout all three). A game with no picture of that shape does
not go without: a wide view takes its background, else its tall cover, and a square one its tall cover, cropped to the
shape ("cropped" in every screen, an amber dot or bar). A family is *in use* when some view draws it and *kept* when
it is in use or the user keeps it anyway (`keepCoverFamilies`); only the kept families are stored, so only they have a
field in a fetch, a tick in "Update metadata", a row in the one-game Metadata and a slot in "Covers", and a family
in use is always kept. A cropped cover is a choice of the aspect, not a defect: it is counted per family
(`Attention::cropped`, `GameCounts::cropped`, `MetadataService::family_lack`) and is not part of "needs attention".

**Covers** (`dialogs/covers/`: `mod.rs` the dialog and its footer, `slots.rs` the left column, `found.rs` the right one,
`actions.rs` what it searches and saves, `words.rs` how a shape reads, `fake.rs` the debug searches): one dialog,
1040px at most, titled "Covers · Hades", over the pictures of one game: a slot per family, Tall, Square and Wide, and the
background. It opens from Game settings → Images' "All covers…" (over the drawer), the one-game Metadata's "Choose
covers…" (over it) and the palette's "Covers…" (a game action, Actions, palette only; on the selected game), on the family
the desktop draws. Left, "This game's pictures": a card per slot with the picture in its true shape
(a wide one at most 84px across), the name, a muted line and a dot. The line says where the picture comes from ("IGDB",
"SteamGridDB", "From the store", "Your own picture"), or "Cropped from the tall cover", "Cropped from the background", "No
picture", then "used on the desktop", "used in Console mode" or both ("kept, not shown now" when no view draws the
family); the dot is green for a picture of its own, amber for a crop and grey for none. A family that is not kept reads
"Not kept on this PC" with a grey dot and "Keep this shape" under it, which keeps it at once ("Saving…") and picks its
slot. A click on a card picks the slot. Right: for the picked slot, "Square pictures found · 9 from 2 sources" (or
"Backgrounds found") and a search field (the game's title; Enter searches), then the picture the slot shows now first,
labelled "{source} · now" or "Cropped · now", and what SteamGridDB (first) and the web found for that shape, each with its
source's mark and size ("1024×1024"), 15 at a time with "Load more"; a search already made is not asked again on the way
back, "Searching…" shows while it runs, "No pictures found. Try another search." when nothing came, and a failure in red
under the grid. A click picks a picture (a double click uses it). A slot that is not kept shows, in place of all this,
"Square covers are not kept", why ("No view on this PC draws square covers, so updates do not download them. Keep the shape
to give games a square picture anyway; a view that starts using it keeps it too.") and "Keep this shape". Footer: "Each
shape keeps its own picture. A view asks for the shape it draws.", "Choose a file…" and "From a screenshot…" (icon buttons,
for the picked slot), "Reset" (back to what the sources gave; only when the slot has a picture of the user's), Close and
the primary "Use for Square" (Tall, Wide, Background; "Downloading…" while it saves), which needs a picture picked. Every
change is saved at once and the Collection behind shows it; Esc closes the screenshot picker first.

**Get more sources** (`dialogs/extensions/`: `mod.rs` the dialog and its two faces, `gallery.rs` the entries and the picked
one's panel, `confirm.rs` the installer, `lists_manager.rs` "Lists…", `permissions.rs` "What it is allowed to do",
`words.rs` every error, origin and trust sentence and how old a copy is, `actions.rs` what runs off the UI thread,
`state.rs` what the hub keeps (`MetadataHub::extensions`), `fake.rs` the debug lists): one dialog, titled "Get more
sources", to install the sources the community writes. It opens from Settings → Metadata's dashed row (and "See the
update…"), from the palette's "Get more sources" (Actions: "Install metadata sources made by the community") and, for a
file, from "Install from a file…" and "Update from a file…" (`HubEvent::OpenSources`; the hub emits it so the Collection
opens the dialog from any page). Opening the gallery reads the lists from the network; the file's installer reads the
package off the UI thread. Esc goes back from the installer to the gallery, closes "Lists…", then closes the dialog.
- **The gallery** (1020px at most): a line, "Sources made by the community. Once installed, a source sits in your lists
  like any other.", then, in amber, one line per list that could not be read now but has an older copy ("The Heronite
  list: There is no list at this address. Showing the copy read before."). A search field ("Search sources": every word
  must be in the name, author, id or description) and the filters: "All 7", "Game details", "Pictures", "Extras" and,
  only when some installed extension has a newer version in its list, "Updates 1" (accent tint). Below, on the left, the
  entries, one card each (each id once, the first list that offers it): its mark, name, "okabe · 1.4.2" (and, for a list
  the user added, "· lists.example.org"), the description in two lines, a kind pill, "Adult content" in amber when it is,
  what it fills ("Text · Covers") and a small button by its state: "Install", "Update to 1.1.0", "Installed" (disabled; an
  extension installed from a file counts as installed, never as an update) or "Needs a newer Heronite" (disabled); a
  click on the card picks it, a click on the button starts the install. On the right (340px), the picked entry (the first
  shown when none is picked): its mark, name, "okabe · version 1.4.2", pills (kind, the games it is for, "Adult content",
  "1.0.0 installed" in accent for an update), the description, "What it is allowed to do" (below) and "Fills", then,
  pinned under it so the button stays in view, the trust line, the primary button (the download icon, or update) with
  "Homepage" at its right, and the reason in red when the last try failed.
  - **What it is allowed to do** (`permissions.rs`; the same lines in the panel and in the installer, built from the
    entry or the manifest, each one something the sandbox enforces and nothing more): "Connects to api.example.org and
    img.example.org, and nowhere else" (or "Connects to nothing on the internet"), "Takes pictures from … too" when its
    pictures come from other hosts, "Reads from your games: title, platform, year, file or install folder name (never a path), store and
    ids", "No access to your files, your accounts or other sources' keys", in amber "Its pictures are hidden until “Allow
    adult covers” is on" for an adult source, and "Asks you for: API key (needed), Region" when it has options.
  - **The trust line** says only what Heronite checked, with a tick when the Heronite list (at its own address) is the
    origin and a warning, in amber, otherwise (`words::origin`, `words::guarantee`); the app's own list set to another
    address is worded as a list the user added. "From the Heronite list": "Its files are checked against
    the Heronite list (SHA-256) before anything is installed." A list the user added ("From lists.example.org"; "…, a
    list you removed" when it was removed since): "Its files are checked against this list (SHA-256), but Heronite does
    not know who keeps the list. Install only what you trust." A file: "Installed from a file: nobody checked this
    source", "Install it only if you trust where the file comes from." Nobody reviews what a list names: the check is
    that the files are the ones it names.
  - **The footer**: on the left where the copy shown comes from and how old it is ("From the Heronite list · read
    today", "From 2 lists · read yesterday", "· read 3 days ago · could not be read now" when the list was not
    reachable and the copy kept is shown; "Reading the lists…" meanwhile), then "Refresh" (reads every list again; dimmed
    meanwhile), "Install from a file…", "Lists…" and Done.
  - **Lists…** takes the place of the entries, with "Back to the sources" and "Lists in use": a row per list, the app's
    own first ("The Heronite list", the tick, no Remove) then the ones added (their host, a link icon, Remove), each
    with its address and how it stands ("3 sources · read today", the sentence of why it could not be read, or "Not read
    yet"). "Add a list by its address" is a field (https://; Enter or Add) with a red reason when it is refused ("Only
    https addresses can be added.", "There is no list at this address.", "This list is in use already."…). A line with a
    warning icon closes it: "Anyone can publish a list. Heronite checks that a source's files are the ones its list
    names, not what the source does: add only lists from people you trust. Removing a list keeps the sources installed
    from it." Lists are kept in `settings.metadata.extensionLists`.
  - **Empty states**: "Reading the lists…" while no copy exists yet; with no copy and no list read, a Wi-Fi-off icon,
    "Could not read the list", why in a sentence (no connection, 404, too large, not a list, a newer format, mistakes
    with the engine's words under it), "You can still install a source from a file." and "Try again" (the app's own
    list answers 404 until it is published on the main branch); a list with no entry, an inbox icon, "No sources here
    yet" and "The lists in use offer no source. Sources show here once someone adds them to a list."; and "No source
    matches." when the search or the filter leaves nothing.
- **The installer** (640px, from the gallery with a back arrow, from a file alone), the one place every extension goes
  through: its title is "Install Wikipedia", "Update Wikipedia 1.0.0 → 1.1.0", "Install Wikipedia 1.0.0 again" (the same
  version is installed) or "Go back to Wikipedia 1.0.0" (an older one), "Install a source" while a file is read and
  "This source cannot be installed" when it does not read. Pressing a gallery entry's button first downloads its two
  files ("Downloading…" on the button; every other button is dimmed meanwhile) and checks them against the list's SHA-256
  and sizes, then reads the package as a file is read, and refuses one that is not what the entry says (another id,
  version or kind, adult content it does not declare, a field or a host it does not name; "It is not what the list says
  it is (its id, version, kind, fields, hosts or adult content differ), so nothing was installed."): nothing is written
  until Install. The body shows the mark, the name, "okabe ·
  version 1.4.2", in accent what replacing does ("It replaces version 1.0.0. What you set for it (on or off, its
  settings, its place) stays.", "Version 1.0.0 is installed already: this puts its files back. What you set for it
  stays.", "Version 2.0.0 is installed: this replaces it with an older one. What you set for it stays."), pills (kind,
  the games it is for, "Adult content"), the description and when it is asked, "What it is allowed to do", "Fills", the
  trust box (a tick on a quiet tint, or a warning on an amber one), "Its source.toml has keys this Heronite does not
  know; they are left out." when it has any, and "SHA-256 …" in small text. The footer is Cancel (Close when it did not
  read) and the primary button: "Install", "Update", "Install again" or "Install the older version" ("Installing…"
  and dimmed meanwhile). An error reads in plain words in red with the engine's own words under it in
  small muted text, for the extension's author ("This is not an extension. Choose its .zip, or the source.toml in its
  folder.", "The package has no source.js.", "Its source.toml has a mistake, so it cannot be installed.", "It was made
  for a newer Heronite. Update Heronite to install it.", "Its id is the id of a source Heronite has, so it cannot be
  installed.", "A file downloaded is not the one the list names, so nothing was installed.", "It could not be saved in
  the data folder."…; every `ExtensionError` has its sentence in `words::error`, and a test fails for a new one); a file
  that does not read adds "Nothing was installed. Choose another file, or ask the source's author for one that works."
  and "Choose another file…". A file chosen inside an extension's folder (`source.toml`, `source.js`, `README.md`)
  stands for the folder. After Install the dialog closes, the extension is on, and Settings → Metadata (on the page the
  user is on, else the next time it shows) opens its row and scrolls to it.

**Game settings → Images** (`dialogs/game_settings/images.rs`): the Cover block is the cover of the family the desktop
draws, "Custom image · Square" or "Default image · Square" (just "Custom image" or "Default image" for the tall one), its
preview in that shape, "Shown on the game card", the line "Square: cropped · Wide: not kept" (the other families, each
"own picture", "cropped" or "not kept") with "All covers…", which opens "Covers" over the drawer, then the picture's
actions (Change, Search web, Reset), which act on that family. The background block is as it was;
"Use defaults" resets every picture set by hand.

**The notice under a Cover aspect picker** (`dialogs/metadata_update/family_notice.rs`, under Settings → Appearance's
Cover aspect and in the View options' Covers section, the same one in both, kept by the hub): picking an aspect whose
family some games have no picture of (Square or Banner) counts them off the UI thread and says so under the picker.
With the automatic download off: "238 games have no square cover. They show their tall
cover cropped until one is downloaded." (for Wide "…show their background or tall cover cropped…") with "Download
square covers" and "Keep the crop", which dismisses it; when no source that gives the shape is on, the same sentence
plus "No source that gives square covers is on." with "Open Settings → Metadata" in place of the download. With the
automatic download on, the download starts by itself and the notice says "Downloading square covers for 238 games" with
Stop and Hide (while another update runs: "… It starts after the update running now.", Hide only). It goes when
dismissed, when the aspect goes back to a tall one, when the job it announced ends (the pill says how it went) and
when no game lacks the shape. Console mode says the same in the line under the theme rows (`ThemeState::notice`, in Settings → Console mode and
☰ → Customize; `theme_rows/host.rs`), when the theme starts drawing a family the one before did not: the lack and "Download them
in Settings → Metadata.", or "Downloading …". The downloads by hand (the notice's, Settings', the dock's) are
`BulkArgs::download_covers` jobs for the games lacking the family and not locked, run like any update: one at a time,
with the pill.

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

**Add games from a folder** (`dialogs/scan_folder/`, ☰ › Add from folder and the palette): the folder field (Browse
searches the chosen folder at once; a typed path when the field is left), "Scan folder" / "Scan again", then the games
found: one row each with a switch, the title (the pencil edits it in place; Enter or leaving the field keeps it), the
program that starts it relative to the folder (a chevron when there are others: a menu lists them with their platform)
and a Windows / Linux pill. A game whose program or folder is already in the Collection is dimmed and locked ("Already in
Collection"); one with the same name only starts unselected. On Linux, "Give each Windows game its own Wine prefix"
(on: a prefix named after the title, as in Add game; off: the default prefix) shows while a Windows game is selected.
"Add {n} to Collection" writes them in one go and their covers are fetched afterwards. `scan::scan` walks up to six
levels (links to folders are not followed; redistributables, anti-cheat installers, runtimes and Windows' folders are
skipped), groups programs by the game's folder (past `bin` / `Win64` folders; one folder takes in the ones with
programs close inside it, unless that is many: a folder of games) and puts first the program named after the folder,
near its top, big, and native on Linux (a `start.sh` at the top wins). `HERONITE_DEBUG_DIALOG=scan-folder[:<folder>[#<states>]]`
(joined with `+`): `programs`, `rename`, `none`, `import`.

**Launch problem** (`dialogs/launch_problem/`, `crate::diagnosis`): when a game ends on its own within 30 s with an
error (a non-zero exit code or a signal; an exit code 0 opens nothing) and the diagnosis knows the cause and a fix for
it, the dialog opens by itself (a toast with "How to fix" while another dialog is open). Anything else (a crash the log
does not explain, a likely cause, an anti-cheat that refuses Linux) never interrupts: a bad exit keeps the plain "closed
right after starting" toast, with "Details" when something was found, and "Troubleshoot" shows every finding of the last
session, however long the game ran. The first line says how the game ended (how the last session ended, from its log, in
"Troubleshoot"); the main finding is a card (a tinted badge, red with a lock when nothing can be done, the headline, one
or two sentences, "What the log says" with the log lines in a mono box, a command to copy for packages) whose first fix
is the Primary button: it applies the fix and starts the game again, closing the dialog with a toast. A fix tried before
this run is no longer Primary and says so. No fix is applied while the game runs (it asks to close the game first); one
that ends after the game started says it takes effect at the next start. Other findings are rows under "Also found". The
footer has View log, ProtonDB and Close. A running fix shows a busy bar and its last output lines (winetricks can be
cancelled); a failed one its error. "Troubleshoot" (context menu, under View log, always there; palette) opens it for
the last session. `HERONITE_DEBUG_DIALOG=launch-problem:<log>[@<code>]` diagnoses the selected game as if it had just
ended with that log (`launch-problem-toast:` for the toast, `troubleshoot[:<log>]` for the on-demand check) and
`HERONITE_DEBUG_LAUNCH_FIX=working|failed` shows a fix's progress or failure; nothing is applied.

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
the card menu has no "Edit playtime"), Actions (one card: Play, then "In the menu next to Play" with the
user's actions; each is a one-line summary that opens its editor in place, one at a time, with its own Cancel / Save while
the other rows dim; Play opens only for local games, others show a lock; actions reorder by dragging, an empty menu
offers shortcuts (Mod manager, Settings tool...); `HERONITE_DEBUG_PLAY_EDIT=1`, `HERONITE_DEBUG_ACTION_EDIT=new|<id>`),
Launch (the panel's launch settings), Compatibility (Wine / Proton), Performance (overlays, FPS limit, frame
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

**Mods manager** (`dialogs/mods/`, `docs/MODS.md`): a 720px `drawer` for one game, opened by the game action "Mods" (context
menu in the group of ReShade, palette; installed PC games, not emulated, not console mode), by the "Mods" row of the facts card
("12 · 10 on", only once the game has mods) and by the toast's "Open" of a finished install. Header: the "Mods" line (the puzzle icon, the kicker
and an amber "Experimental" pill; no icon tile beside the game's name), game title, the game's support label ("Should work" green, "Needs extra steps" amber, "Untested game"
neutral, "Most mods won't work" red; `heronite_mods::support`) before a one-line status, the "Play with mods" switch; both
labels say why under the pointer (`ui::tooltip_titled`: a title over the sentence). Tabs Installed, Browse, Game, and Plugins
for Bethesda games. Installed: notices as tinted cards (`render_jobs::tinted`: game folder not found, FUSE problem with "Open
Settings → Mods", game running, what to expect of the game's mods for every level but "Should work", Steam launch options,
the `nxm://` handler once a Nexus account is connected; the account itself is only asked for in Settings → Mods), this game's jobs (progress bar, Cancel / Dismiss; a mod installed where the game will
not see it is an amber job that stays until dismissed), then the load order in
one card: handle (drag a row onto another; Move up / Move down in its menu for the keyboard), switch, picture or initial, name,
version, author, source pill, "Check files" (amber; click says why the game will not see the mod, with Open folder, Open mod
page, Reinstall and Remove), conflict badge ("Overrides 2" accent, "Overridden by 1" amber; click lists the mods and file
counts), update pill, "⋯" menu (drawn at the dialog's root with `anchored`, never inside the drawer), inline Rename and Remove
question; then the Overwrite card (Open, Turn into a mod, Empty with inline question). The empty state offers Browse and Add
from file. Archives or folders dropped on the drawer are installed. Browse searches Nexus Mods and Thunderstore off the UI thread
(350ms debounce, sort pills, "Load more"; Premium installs, free accounts get "Download on Nexus Mods" with a one-line hint);
Game holds the links (Nexus game, Thunderstore community, "Mods go into"), the mods folder and "Remove all mods". The FOMOD
wizard (`dialogs/mod_installer/`) is a modal: step name and "Step 2 of 4", groups as radio rows (pick one) or switches (pick
any), "Recommended" / "Required" / "Not usable" pills, the hovered option's picture and description on the right, footer Cancel,
Install without the installer, Back, Next / Install. `dialogs/mod_link_game.rs` asks which game a mod link is for. The wizard
and the picker replace the manager and give it back when they close (`collection/mods.rs`). Debug:
`HERONITE_DEBUG_DIALOG=mods:<game key>[:installed|browse|game|plugins]`, `mod-installer[:<step>]` (folder with
`fomod/ModuleConfig.xml` in `HERONITE_DEBUG_FOMOD`), `mod-link`; `HERONITE_DEBUG_MODS_FIXTURE=1|nodir,steam,account,premium,fuse,jobs,misplaced,warned`
shows the `mods.json` of the game with invented conflicts, overwrite and jobs while the service has not loaded it;
`HERONITE_DEBUG_MODS=menu:<n>,expand:<n>,check:<n>,rename:<n>,remove:<n>,overwrite-name,overwrite-clear,forget,running,thunderstore,tip:experimental|support`.

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
blocks side by side (stacked when narrow), a `grid().grid_cols(n)` holds repeated cards, and a fixed side
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
- Input: `console::input` reads controllers on a thread that only runs in console mode (presses only count while the
  window is focused): gilrs on Linux, SDL on Windows (`heronite_platform::controllers`, one thread shared by every
  listener); both feed the same `Translator` (repeats, stick and trigger thresholds). The keyboard maps to the same
  `Pad`s (arrows, Enter = A, Esc = B, X, Y, Q/E = LB/RB, M = menu).
- Mouse (`console::pointer`): it presses too, so no screen handles it apart. Each window that takes presses registers a
  router (`pointer::set_router`: the shell's `console_pad`, the overlay panel's `on_pad`); a click sends the presses that
  bring the focus to what was clicked, then A: a row of a `console::list` is used at once (`ListNav::click`; the ‹ › of
  a value press ◀ ▶), a cover takes the focus on the first click and opens on the second (`pointer::focus_or_activate`).
  Hints, tabs and LB / RB glyphs press their button (`kit::hint_item`, `kit::tabs`). The wheel moves the focus one
  press per notch (← → on Carousel and Spotlight; on a game page with long text it scrolls the text, as the right
  stick does), the right and back buttons are B (`pointer::layer`, before the scroll areas). A hint bar faded out
  (`Hints::Auto`) takes no clicks (`kit::hint_bar_with`). Row clicks are worked out when clicked, from one pass over
  the rows (`list::Clicks`), so long lists stay cheap. Focus a press cannot reach in a few steps is set directly (`console_click`, `cover_click`: the menu's items,
  the Grid and Dashboard tiles). Anything drawn over a screen (dialogs, `Layer`s, menus, panels, keyboards) occludes
  it, so a click never reaches a screen that does not take the presses; a click on the dim around a modal, menu or
  sheet is B. A list beside the owner's own focus (Settings' content) takes clicks through `render_entered` (→ first).
  Hovering moves the focus (once a frame, `pointer::take_hover`) where nothing moves under the pointer: list rows,
  the ☰ menu, the Filters panel, Settings' sections, Grid and Dashboard tiles, the overlay's rows (not the covers of
  rows that glide). A controller press hides the cursor until the mouse moves or a key is pressed
  (`pointer::hide_cursor` / `show_cursor`, patched into `vendor/gpui-pre-linux` and `vendor/gpui-pre-windows`). In
  the overlay, a click on the output under the mixer switches to the next one (Y).
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
  machine options, `console-themes/` the saved ones, each with its picture as `<id>.background-<hash>.<ext>`; an
  export carries the picture inside as a `data:` URL and leaves out a local path it does not use, an import writes it
  out again; a value of the wrong type reads as that field's default, an id is kept to `[a-z0-9-]` and a saved
  theme's id is its file name): the shell (`collection/console/mod.rs`) reads
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
  theme file read as), and Cinema, Pulse, Arcade (Wheel), Hub (Dashboard, the rail) and Living room (Series X),
  modelled on other launchers' fullscreen themes and consoles' home screens. Classic's screens: Shelves count a shelf's games
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
  Status, Category and Smart group (every group, by name; the grid's Show chip then reads "<group> · <show>"); X resets them. They edit the filters of the tab's grid, else the theme's first Grid tab (the
  first change switches to it), else a results grid over the tab (`ConsoleUi::results`, B leaves it), so every theme has them.
  Colours go through `theme::console_*` / `text_primary` / `text_muted`, which read the theme only while console
  mode is on. Change the theme only with `console::themes::update` (installs it live, saves off the UI thread);
  Settings → Console mode and ☰ → Customize share their rows (`console::theme_rows`). On the desktop, Settings → Console
  mode (`pages/settings/console_mode/`) has three views: Home (the themes on one row that scrolls sideways, however many
  there are: ‹ › and the filters All / Built-in / Yours in its title, a swipe, or the mouse wheel while the row can
  still move, like Related; about three cards in view, the next one cut at the edge, and under the row a dot per page
  of cards, the current one a longer accent pill, a click goes to that page (`Pages`); an arrow or a dot slides the row
  there (`Slide`, 280 ms ease-out, frames asked for only while it runs; the wheel or a swipe cancels it; a jump with
  Animations Reduced or Off) and the pill flows from dot to dot with the row's position; it brings the theme in use into
  view when shown or filtered; each card a picture, the name and two lines, the rest in a tooltip, Edit / Make a copy /
  Export / Delete over the picture under the pointer; "New theme" and "Import…" (its tooltip says a file can be
  dropped on the themes) in the title; a theme file chosen with Import… or dropped on
  the themes is shown first, `import.rs`, with its picture, author, version and how it stands with the saved ones
  (`store::compare`: the same, another version of one, new) and is only added or used when asked; the theme in use
  with its live picture, All options, save, undo and the "Quick changes" beside it, each with its ↺; the startup
  options), the editor (`editor.rs`; in a window wider than 1040px of content a studio filling the page's height, the
  page itself not scrolling: the Customize panel's groups as tabs with a dot while they hold changes and a search
  across them on the left, scrolling; the live picture with chips for each tab, the game page and the menu (it turns
  to the screen an option changes, `follow`) and the details (name, author, version, description) on the right; Undo /
  Redo (`ThemeState::undo`, a drag is one step), Undo all changes, Export and Save in the header; every option a ↺
  while it differs from the theme it started from, `theme_rows::{differs, reset_command}`; options no tab uses are
  hidden and counted; colours in `colours.rs`: the ready-made `COLOUR_SETS`, a swatch per colour opening the colour
  picker, back to the built-in colour, a warning with "Make it readable" below 4.5:1, the focus colour as dots with a
  picker of its own) and New theme (start from a theme or the one in use, a name, an author, a description). Clicking
  a theme while the one in use has changes asks first; an import never replaces unsaved changes (on the TV it is only
  added to the list). The
  pictures (`preview/`) draw a screen of a theme (`Screen`: a tab, the game page in its style, or the open ☰ menu;
  the rail's column and inset included): the layout, covers (shape, size, corners, focus, badges, greys, store strip,
  names where the layout shows them), background, tab bar, clock, hint bar, colours and
  font on a 640 x 360 canvas with the covers and art of the games played last (the library Storage gets from the
  Collection; tinted blocks without it); their textures are freed when the section leaves the screen.
  `HERONITE_DEBUG_CONSOLE_SETTINGS=editor[:<group>]|search:<text>|colour:<row id>|create|pending:<theme id>|import:<file>`
  opens a view. `HERONITE_DEBUG_CONSOLE_SCREEN=tab:<n>|game|menu`
  makes the pictures draw that screen. A new option = a field in
  `ConsoleTheme` (with a default and `#[serde(default)]`, so old and new theme files keep loading), its rows in
  `theme_rows`, and its use. Nothing console-only may run, read files or allocate while console mode is off;
  no "HERONITE" wordmark in console screens (the theme's `interface.label` may add a text of its own).
- Debug: `HERONITE_DEBUG_CONSOLE=1` (or `=game:<key>` for a game page, `=menu`, `=customize`, `=filters`) starts in console mode
  without saving it; `HERONITE_DEBUG_CONSOLE_PRESET=<theme id>` / `HERONITE_DEBUG_CONSOLE_LAYOUT=<layout>` start with
  that theme / every tab in that layout (not saved); `HERONITE_DEBUG_IDLE_SECS=<n>` shows the idle screen sooner;
  `HERONITE_NO_GAMEPAD=1` ignores controllers (test copies must not follow the one in use; `shot.sh` and `idle-cost.sh` set it); `HERONITE_DEBUG_PAD=down,a,wait:500,b` plays presses (`up down left right a b x y lb rb lt rt start select sup sdown`); `HERONITE_DEBUG_MOUSE=click:0.5:0.4,wheel:-1,right` plays mouse events (points as fractions of the window; on a
  hidden workspace keep capturing it with `grim -T`, or the clicks hit the last frame it presented).

**Do not** use uppercase labels, outlined buttons, underlined tabs, square
checkboxes or flat grey modals in new UI. Colours come from `theme` (`text_primary`,
`text_muted`, `text_dim`, `accent_tint`, `ink_a`, ...) and must read on both themes (see "Theme").

**Verifying**: captures with the app on a hidden workspace only show what was
drawn before the compositor stopped sending frames, so late images (art, covers)
are missing. Take a first capture to wake the window, wait a second, then take
the real one (a modified copy of `scripts/shot.sh` doing `grim -T` twice).
`HERONITE_DEBUG_DROPDOWN=nav|view|view:<text>|groups-select|more-filters|manage`, `HERONITE_DEBUG_MENU=<key>`,
`HERONITE_DEBUG_DIALOG=...`, `HERONITE_DEBUG_PAGE=...`. Copy
`~/.local/share/heronite/images-cache` into the fixture or no art appears at all.

**In-game overlay** (`app/src/overlay/`, Settings → Overlay; off by default, options in `overlay.json` of the app
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
user's `MangoHud.conf` or their Steam launch options was ruled out). An Achievements tab (last, so the other tabs keep
their place when it appears; `panel/achievements.rs`) shows when the game has a set in the achievements service,
whatever the source: a progress card (count, bar, gold at 100%, source, points, rare unlocks), what this session
unlocked (a gold ring and the time), "Up next" and "Recently unlocked", 5 each, every achievement with its icon (a small
`ImageStore` the tab makes when first drawn; a trophy or a lock when there is none), its rarity in its tier's colour,
its description (in full, up to 3 lines, when focused) and a bar when the source counts its progress, then how many are
secret (never their names); showing it reads the sources again (at most once a minute), and Home
gets an "Achievements" row that leads to it. Quitting the game asks
first (focus on Cancel). Guide + RB, Guide + ↑ / ↓ and Guide + X (the replay, Settings → Replay; Home and Screenshots
show "Save replay" while it records, and Screenshots a switch that turns it off until the game ends) show a notice in
the top corner opposite the panel. Achievement notices (`OverlayService::show_achievement`, Settings → Achievements →
"Over the game" tries one) queue instead of replacing: 5 s each with the achievement's icon (loaded first, 1.5 s at
most) or a gold trophy, "Achievement unlocked · 9.7%" in the rarity's colour when rare, up to 4 waiting, then one
"N more achievements" (a single extra one shows as itself), in the order of the unlocks even when their icons load out
of order; a volume or screenshot notice cuts in and the achievement shows again after it. The
screenshot's button is a choice (RB unless changed; not ↑ / ↓ nor the triggers, which many controllers report as axes),
shown in Settings → Overlay and in Settings → Screenshots ("With the controller"). A keyboard shortcut (Super + G unless
changed, `heronite_media::capture::GlobalHotkey`: a temporary consuming Hyprland bind that sends SIGUSR1, else the
GlobalShortcuts portal) opens and closes it too; it is registered only while a game launched by the app runs and removed
on quit. Opened that way the surface takes the keyboard (`KeyboardInteractivity::Exclusive`, so the game gets no keys
until it closes), console mode's keys drive it (`console::input::pad_from_key`) and the hints show keys
(`kit::key_glyph`: Enter, Esc, Q / E). "Open it without a game too" (`withoutGame`, off by default) keeps the input thread and the
shortcut running while the app runs; the panel then has no Screenshots tab, the header says "No game running", the mixer
has only System, and Home ends with "Open Heronite".
The panel takes controller presses from the service (`OverlayPanel::on_pad`) and the mouse as console mode does
(`console::pointer`: tabs, rows, hints; a click on a volume bar sets that level, on its icon mutes it; a capture
opens on a click); it never reads `OverlayService`
(which is usually updating when it calls in) and reaches it through deferred `with_service`. GNOME and X11 cannot show
it (no layer-shell): Settings says so. Debug: `HERONITE_DEBUG_OVERLAY=window[:<app_name>]` draws the panel in a normal
window (`app.heronite.overlay`) as if the game ran (Hyprland ignores its size requests there; audio changes are skipped
under any `HERONITE_DEBUG_OVERLAY`), `=layer` the real surface; `HERONITE_DEBUG_OVERLAY_PAD=rb,down,a`
plays presses, then `HERONITE_DEBUG_OVERLAY_MOUSE` mouse events (as `HERONITE_DEBUG_MOUSE`); `HERONITE_DEBUG_OVERLAY_NOTICE=volume|shot|replay` adds a notice (`app.heronite.overlay-notice`), `=achievement|achievement-queue` one or six made-up achievement notices (icon from `HERONITE_DEBUG_OVERLAY_NOTICE_ICON`);
`HERONITE_DEBUG_OVERLAY_FPS=<limit>` shows the FPS limit row (nothing applied); `HERONITE_DEBUG_OVERLAY=window:none` opens it with no game;
`HERONITE_DEBUG_OVERLAY_KEYBOARD=1` opens
it as the keyboard shortcut does, `=shortcut` registers the real shortcut instead; `HERONITE_DEBUG_OVERLAY_DISPLAY=<px>` stands in for the display height (800 for a Steam Deck, 2160 for 4K), and
the panel's size follows it. On Windows, `HERONITE_DEBUG_OVERLAY_MONITOR=<x>,<y>` puts the surfaces on the monitor holding
that desktop point (physical px) instead of the foreground window's, so another monitor is tested without moving focus.

On Windows the same panel uses a topmost tool window on the foreground
monitor, over windowed and borderless games. Nothing opens while a game
holds exclusive fullscreen (`SHQueryUserNotificationState`), which a
topmost window would minimise. Ctrl+Shift+G is the default shortcut
(`RegisterHotKey`: other apps do not get it while it is registered, which
Settings says); Back+Start substitutes for Guide on XInput controllers.
There is no controller grab: settings must say that the game can still
receive presses. A keyboard-opened panel asks for focus (Windows may
refuse; it stays open) and returns it on close only while it still owns
foreground. Notices never
take focus and pass pointer input through. The Audio tab uses Core Audio;
device names are informational (choose defaults in Windows settings).
Replay controls, live Linux FPS controls and Bluetooth disconnect are hidden.

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
- A game with Steam Achievement Sets (`AchievementSet::sections`: two or more groups with achievements; the
  ones in no group never split a game alone) gets a row of its sets in the dialog (All first, each "name · DLC · unlocked/total"; the filters count within the set shown; in All each
  achievement carries its set's name) and a "Sets" section in the overlay's Achievements tab (bar and count per
  set). Steam's unnamed base group reads "Base game", achievements in no known group "Other achievements".
  Counts are Heronite's unlocked/total, never Steam's "set complete"; with DLC sets the dialog says the counts
  include them whether or not the DLC is owned (no source says which). `HERONITE_DEBUG_DIALOG=achievements`
  opens the dialog of the selected game, `HERONITE_DEBUG_ACH_SET=<group id>|other` on that set.
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
  `HERONITE_DEBUG_MENU=<key>` (context menu), `HERONITE_DEBUG_DROPDOWN=view|view:<text>|nav|downloads|metadata`,
  `HERONITE_DEBUG_DIALOG=settings|statuses|gamesettings:<tab>|onboarding[:<step>]|achievements` (extend the match in
  `CollectionView::render` for new dialogs; `settings:<section>` opens that Settings section,
  `onboarding:<step>` "Getting started" where that step lives: `steam`, `stores` or `emulators` open its games view, `appearance` its look view, any other step the three cards).
  `scripts/shot.sh` sets `HERONITE_NO_ONBOARDING=1`: "Getting started" does not open by itself on a new state folder.
  Settings → Metadata (`pages/settings/metadata/`): `HERONITE_DEBUG_METADATA_OPEN=<source id>[,<source id>…]` opens
  those sources' rows (`advanced`: "Order for one field") and scrolls to the last; with `HERONITE_DEBUG_PAGE=settings:metadata`.
  `HERONITE_DEBUG_SOURCE_DOWNLOAD=<source id>:<none|downloading[:<percent>]|preparing[:<percent>]|ready|failed>` shows
  that source's database in a made-up state, without the network.
  `HERONITE_DEBUG_METADATA_OPEN` also takes an extension's id (its row opens once the extensions are loaded).
  Extensions (`dialogs/extensions/`): `HERONITE_DEBUG_DIALOG=sources[:<id>]` opens "Get more sources" (that entry
  picked) and `HERONITE_DEBUG_DIALOG=extension-file:<path>` the installer of a package (a zip, a folder, or a file in
  it; nothing is installed until Install is pressed: use a data copy). `HERONITE_DEBUG_EXTENSIONS=<state>[+<state>…]`
  makes the lists up, without the network: `list` (the two examples and made-up sources, one adult, one for a newer
  Heronite), `update` (the installed extensions count as from the app's list, which offers Wikipedia 1.1.0), `old`
  (a copy 3 days old, not readable now), `offline`, `404`, `invalid`, `empty`, `other` (a list the user added), `lists`
  (the "Lists…" manager open), `pick:<id>`, `busy` / `error` (the picked entry installing, or its install failed;
  installing from a made-up list always fails as a download would). `HERONITE_EXTENSIONS_INDEX=<https address|path
  of an index.json>` replaces the app's own list in any build (an entry of a list on this computer is read from
  `<its folder>/<id>/`). Without it a debug build reads the repository's `extensions/metadata/index.json` (found from the `app`
  crate's folder; "From this copy of the repository") and a release build
  `heronite_metadata_ext::lists::HERONITE_LIST` (which answers 404 until the list is published on the main branch);
  with `HERONITE_DEBUG_EXTENSIONS` the made-up lists stand for that one.
- Main Collection screen: compare with the reference PNG (crop the same region with PIL, view both). Dialogs/menus/settings: one sanity check is enough — they need to work and be consistent, not pixel-identical (user decision).
- Anything that WRITES data must be exercised only against a copy:
  `HERONITE_DATA_DIR=/tmp/heronite-fixture-<you> scripts/shot.sh …`
  (copy `/tmp/heronite-fixture`, which mirrors the data folder's JSON).
- Never launch games, emulators, Steam or Ludusavi backups for real.
