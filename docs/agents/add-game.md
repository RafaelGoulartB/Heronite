# Your brief: ADD GAME (sideload) (agent "addgame")

Port Heroic's "Add game" (sideload) feature: add a native Linux game, a Windows .exe (run with Wine/Proton) or a
browser game to the library, with title, cover/art (SteamGridDB/auto search like Heroic, or our metadata/web image
search), executable, platform, install folder, Wine settings; and edit/remove such games later.
Heroic sources: src/frontend/screens/Library/components/InstallModal/SideloadDialog/** (and InstallModal for platform
choice), src/backend/storeManagers/sideload/{library.ts,games.ts,setup.ts?}, src/backend/storeManagers/sideload/*
writes `sideload_apps/library.json`; plus Heronite's overlay hook (src/backend/storeManagers/sideload/library.ts
addNewApp syncing the executable into local_library meta — see HeroicGamesLauncher/FORK.md).
Our code: crates/data (library loading of sideload_apps, json_store, local_library meta models), crates/launch
(native/wine launch of sideload games already exists), crates/metadata (art/image search), app/src/dialogs/**
(form.rs, art_images.rs, web_image.rs for picking images), app/src/dialogs/heroic_settings/** (Wine settings
components — reuse for the Wine part of the dialog if practical).
Owned: new app/src/dialogs/add_game.rs (+ submodules), new crates/data/src/sideload.rs (or similar) for writing
sideload_apps/library.json exactly like Heroic (format, ids — Heroic generates app_name with a random id; check),
plus the GamesConfig defaults Heroic writes for a new sideload game.
Hook points: an "Add game" entry in the ☰ menu (app/src/collection/nav.rs — one item that opens your dialog via
CollectionView::open_dialog) and maybe a "+" tool button in the header (app/src/collection/header.rs, small); context
menu item "Edit game" for sideload games (app/src/collection/context_menu.rs, one small block — other agents also add
blocks there); app/src/dialogs/mod.rs registration; app/src/collection/mod.rs debug-dialog arm
(HERONITE_DEBUG_DIALOG=add-game / edit-game:<appName>).
Verify: unit tests writing into a fixture (byte-compatible with what Heroic writes, unknown fields preserved);
screenshots of the dialog (empty, filled, Windows exe with Wine section). Don't actually launch the added games.
