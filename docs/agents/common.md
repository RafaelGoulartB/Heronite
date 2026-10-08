# Common rules (all agents) — HeroniteRust

Repo: /home/rafa/Documents/Projects/Personal/game-launcher/HeroniteRust (Rust, GPUI via gpui-kit 0.7.1).
Original app (TypeScript/Electron, read-only reference): /home/rafa/Documents/Projects/Personal/game-launcher/HeroicGamesLauncher
(Heroic Games Launcher fork "Heronite"). Our app is a standalone Rust rewrite that reads/writes THE SAME data files in
~/.config/heroic so both apps can be used during the transition. Read first: PROGRESSO.md, docs/UI-CONVENTIONS.md,
app/src/pages/mod.rs, app/src/collection/nav.rs, app/src/dialogs/mod.rs, app/src/ui/{mod,controls,dialog,dropdown}.rs.

Product decisions:
- The Collection screen is THE interface. New features become pages in the ☰ menu (app/src/pages/*), dialogs
  (app/src/dialogs/*), context-menu items and focus-panel buttons. Do NOT copy Heroic's sidebar/layout; reuse our
  visual style (dark nord tokens in app/src/theme.rs, controls in app/src/ui). Dialogs/pages need to work and look
  consistent with the existing ones, not pixel-identical to Heroic.
- English UI strings (Heroic's en locale: HeroicGamesLauncher/public/locales/en/translation.json).
- File formats must stay compatible with Heroic/Heronite: reuse heronite_data::json_store (electron-store style:
  tab indent, atomic write, read-modify-write, unknown fields preserved). Check the exact format Heroic writes for each
  file you touch (e.g. GamesConfig/<app>.json is written with JSON.stringify(config, null, 2) — 2 spaces).
- Linux first. Keep code portable where cheap (cfg(target_os) for Linux-only bits).

Engineering rules:
- Several agents work IN PARALLEL in the same working tree. Only edit the files you own (listed in your brief) plus the
  small hook points explicitly allowed. Never reformat or rewrite other files. Never run `cargo fmt` on the whole
  workspace (format only your files: `rustfmt --edition 2021 <file>` if you want).
- Keep the tree compiling: work in small compilable steps. Develop backend crates with `cargo test -p <crate>`.
  If `cargo build -p heronite` fails because of errors in files you do NOT own, another agent is mid-edit: continue
  with other work (or your crate's tests) and retry a bit later; do not fix their files.
- No I/O or blocking work on the UI thread: use cx.background_spawn / threads, then update the entity.
- Commit your own work at milestones with explicit pathspecs only: `git add <your paths> && git commit -m "..."`
  (never `git add -A`, never amend, never touch other agents' changes). If you hit `index.lock`, retry a few seconds later.
- Tests: add unit tests for parsing/format code. Existing suite: `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test -p <crate>`.

SAFETY (strict — the user's real data and Heronite are live on this machine):
- Never write to the real ~/.config/heroic from tests or manual runs. Use a fixture copy and point the app at it with
  HERONITE_HEROIC_DIR=/tmp/heronite-fixture-<yourname> (copy JSON files from /tmp/heronite-fixture or ~/.config/heroic).
- Never launch games, emulators, Steam, real winecfg/winetricks against real prefixes, Ludusavi, or real backups.
- Never delete/modify /data/Games/** or ~/Games/** or the real ~/.config/heroic/tools.
- Never print credentials/tokens (legendary user.json, IGDB keys).
- GUI windows: ONLY via scripts/shot.sh (opens on Hyprland workspace 2 silently, captures with grim -T, closes only
  its own window). Never open windows any other way, never switch workspaces, never move/click the mouse.
  Usage: `SHOT_ENV="HERONITE_DISABLE_IDLE_JOBS=1 HERONITE_HEROIC_DIR=/tmp/heronite-fixture-x HERONITE_DEBUG_PAGE=wine" scripts/shot.sh /tmp/x.png 3`
  then view the PNG (crop/resize with python PIL if large). Note: workspace 2 is usually hidden so late-arriving
  images may be missing from captures — that is a capture limitation, not a bug. You cannot click; add debug env hooks
  (like HERONITE_DEBUG_PAGE / HERONITE_DEBUG_DIALOG) to open the states you need to verify.
- Unset ELECTRON_RUN_AS_NODE if you ever run Electron (you shouldn't need to).

When done, reply with: what you implemented (files), what is verified and how, what is NOT verified/limitations,
and any hook points you added in shared files. Also append a short section to PROGRESSO.md describing your feature
(in Portuguese, like the rest of that file) — only add your own section, don't rewrite others.
