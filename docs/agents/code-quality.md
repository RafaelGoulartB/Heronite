# Agent brief: code-quality pass (split big files, trim comments)

Read `docs/CODE-STYLE.md` first and follow it: it is the spec for this task. Also read the SAFETY section of
`docs/agents/common.md` (you should not need to run the GUI at all).

## Goal
Make your area easier to read and maintain **without changing behaviour**:
1. Split the big files listed for your area along natural seams (see "Modules and file size"). Also look at any other
   file in your area above ~600 lines of non-test code. Long `render` functions become `render_*` helpers.
2. Go through **every** `.rs` file in your area and apply the comment rules: remove comments that repeat the code,
   change history, mockup/brief references, obvious section labels and rulers in short files; keep the why,
   Heroic-compatibility notes, invariants and useful `///` docs. Tighten wordy comments. Do not invent new long
   comments. Do not reword comments that are already good.
3. Small cleanups you meet on the way: duplicated helpers inside your area (merge them), needless `pub`,
   `clone()`s that are not needed, overly nested code that a `let ... else` or an early return flattens. Keep these
   local and obviously safe.
4. App areas only: replace every `std::env::var*("HERONITE_...")` read with `crate::env::{flag, value, parse}`
   (`app/src/env.rs`). `flag` = set and not ""/"0"; `value` = set and not empty; `parse::<T>`. Leave non-HERONITE
   variables (HOME, XDG_*, PATH, RUST_LOG) alone.

## Hard rules
- No behaviour change: same strings, same files/keys/formats written, same order of side effects, same debug hooks.
- Public API of every crate and every path used **outside your area** must keep working. If you move an item, add a
  `pub use` in the parent so callers compile unchanged. Do not edit files outside your area.
- Tests move with the code they test. Do not delete tests. Do not weaken assertions.
- Never run `cargo fmt` or `rustfmt`: there is no rustfmt.toml and the code uses longer lines than rustfmt's
  default, so it would reflow untouched code. Format by hand like the surrounding code.
- Never touch `docs/design/`, `PROGRESSO.md`, `Cargo.toml` files, or other areas' files.

## Build setup (important)
You work in your own git worktree. Share the main build cache so you do not rebuild GPUI:
`export CARGO_TARGET_DIR=/home/rafa/Documents/Projects/Personal/game-launcher/HeroniteRust/target`
on every cargo command. Other agents use the same cache, so cargo may print "Blocking waiting for file lock": just
wait. Prefer `cargo check -p <crate>` / `cargo test -p <crate>` while working; run the full gate once at the end.
Tests that need the fixture: `HERONITE_FIXTURE=/tmp/heronite-fixture`.

## Gate before you finish
- `cargo clippy --workspace --all-targets` → zero warnings (all of them, not only yours).
- `cargo test -p <each crate you touched>` green (the app crate is `-p heronite`).
- Commit in your worktree with explicit pathspecs (`git add <paths>`; new files too) — one commit per logical step
  is fine (e.g. "refactor(services): split downloads.rs into services/downloads/"). Never amend, never push.

## Report
Reply with: the branch name and commits; for each split, old file → new files with line counts; a rough count of
comments removed/rewritten; other cleanups; anything you saw but did not change because it was risky or outside
your area (with file:line) — that list matters, it feeds the final assessment.
