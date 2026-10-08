# UI conventions (app/)

The Rust app must look like Heronite's Collection. Every screen/dialog has a
reference capture in `docs/reference/` (main screen) and
`docs/reference/dialogs/` (dialogs, menus, gallery). The TypeScript/CSS
sources are in `../HeroicGamesLauncher/src/frontend/screens/LocalLibrary/**`.

## Stack
- GPUI through `gpui-kit = 0.7.1` (`use gpui_kit::*; use gpui_kit::prelude::*;`).
  Sources: `~/.cargo/registry/src/*/gpui-pre-0.3.8`, `gpui-component-0.7.1`,
  `gpui-base-0.7.1`. Read them when unsure of an API.
- Reuse from gpui-component only behaviour-heavy widgets: `input::{Input, InputState}`
  (text fields), `slider::{Slider, SliderState}`. Style everything else by hand.

## Layout of the code
- `app/src/theme.rs` — tokens (nord-dark colours, fonts, status colours,
  `W_BOLD` = CSS 600/700, `W_500` = CSS 500). Never hard-code a new colour
  twice; add it here.
- `app/src/ui/` — building blocks: `icon`, `tool_button`, `round_button`,
  `chip`, `scrollbar`, `dropdown::{dropdown, menu_panel, menu_item}`,
  `dialog::dialog` (Heroic Dialog chrome), `controls::{checkbox, button,
  tabs, select_box, section_title, help_text, mui_primary}`.
- `app/src/collection/` — the main screen (`mod.rs` = `CollectionView`).
- `app/src/dialogs/<name>.rs` — one dialog per file. A dialog is an
  `Entity<V>` where `V: Render + EventEmitter<DialogEvent>`; build it with
  `DialogCtx` (paths + `Arc<Library>`). Open it from `CollectionView` with
  `self.open_dialog(cx.new(|cx| V::new(ctx, window, cx)), cx)`.
  Emit `DialogEvent::Changed` after writing data, `Close` to close.
- Settings tabs: `app/src/dialogs/settings/<tab>.rs` with `State::new` and
  `render(&mut SettingsDialog, …)`. Save with `heronite_data::settings::*`
  and call `d.saved(result, cx)`.

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
  `CollectionView::render` for new dialogs, e.g. `settings:screenshots` to open a tab).
- Compare with the reference PNG (crop the same region with PIL, view both).
- Anything that WRITES data must be exercised only against a copy:
  `HERONITE_HEROIC_DIR=/tmp/heronite-fixture-<you> scripts/shot.sh …`
  (copy `/tmp/heronite-fixture`, which mirrors `~/.config/heroic` JSON).
- Never launch games, emulators, Steam or Ludusavi backups for real.
