# Your brief: COLLECTION LEFTOVERS (agent "collection")

Port the Heronite Collection features still missing in our Rust Collection. Heronite sources:
HeroicGamesLauncher/src/frontend/screens/LocalLibrary/** (Collection/*, SessionHistory.tsx) and src/local-library/**
(backend: sessions.ts, remove.ts, ludusavi.ts, playtime handling, steam-details.ts for achievements if that's where they
come from). Our Collection: app/src/collection/** (mod.rs = CollectionView, card.rs, focus.rs, context_menu.rs,
header.rs, list.rs), dialogs in app/src/dialogs/**, data in crates/data, crates/emulation (ludusavi), crates/metadata.

Implement (each must read/write the same files Heronite does, compatible formats):
1. "Edit playtime" dialog (CollectionPlaytimeDialog.tsx + the backend that stores it) — reachable from where Heronite
   puts it (check CollectionCard.tsx / context menu) — in our app: context menu item + focus panel if Heronite has it.
2. "Remove from Collection" (with Heronite's confirmation and exact semantics from src/local-library/remove.ts).
3. Manual "Backup save (Ludusavi)" action per game (Heronite's card/menu action) using crates/emulation ludusavi code.
   NEVER run a real Ludusavi backup in tests (fake binary only).
4. Session history of a game (SessionHistory.tsx: list of local sessions) in the focus panel, styled like our panel.
5. Achievements in the focus panel exactly as CollectionFocus.tsx does (find their data source; if it needs network,
   do it off the UI thread and cache like Heronite).
6. The "open details page" button Heronite's focus panel has (check what it opens; for us, open the store/details URL
   or whatever makes sense without a Heroic game page).
7. Search box inside the facet filter flyout (FacetFilterMenu.tsx), dragging the list scrollbar (app/src/ui/mod.rs
   `scrollbar` + list), and the "Steam not found" warning when launching a Steam game without the Steam client
   (steamActions.ts).
Owned: app/src/collection/{focus.rs,card.rs,list.rs,header.rs,screenshots.rs} sections you need, new files
app/src/collection/{playtime.rs,sessions.rs,achievements.rs,...}, new dialogs app/src/dialogs/{playtime,remove_game}.rs,
crates/data (new modules), app/src/ui/mod.rs (scrollbar). Shared hook points: app/src/collection/context_menu.rs (add
your items as one small block), app/src/collection/mod.rs (fields/handlers — keep edits small; another agent ("system")
also adds a few lines there), app/src/dialogs/mod.rs (register modules).
Verify with unit tests (fixture copies only) and shot.sh captures of each new UI (add HERONITE_DEBUG_* hooks).
