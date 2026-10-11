# Code style (Rust)

How the code is organised and commented. `docs/UI-CONVENTIONS.md` covers the
look of the UI; `docs/ARCHITECTURE.md` the map of crates and modules.

## Modules and file size
- Aim for files of up to ~500 lines of non-test code. Above ~600, split along
  a natural seam. A cohesive file (one parser, one table) may stay larger;
  never split only to hit a number.
- Split by responsibility: state and data, rendering, actions and event
  handling, background work, debug hooks.
- A big GPUI view `Foo` becomes a directory: `foo/mod.rs` holds the struct,
  `new` and `impl Render`. Siblings add more `impl Foo` blocks, for example
  `foo/actions.rs` or `foo/render_list.rs`. The siblings are child modules,
  so they already see the private fields of `foo/mod.rs`; the fields need no
  extra visibility.
- A `render` longer than ~120 lines becomes `render_*` helpers, one per part
  of the screen.
- Use the narrowest visibility that works: `pub(super)`, then `pub(crate)`.
  Inside the `app` crate, `pub` means "used by another top-level module".
- When code moves, keep the old paths that other modules use working with a
  `pub use` in the parent. Do not touch the callers.
- Tests stay next to the code they test. When a function moves, its tests move
  with it.

## Comments
Comments explain what the code cannot say by itself. Keep:
- **Why:** reasons, constraints, trade-offs, and what breaks if this changes.
- **Origin in Heroic/Heronite:** file formats, key names, and behaviour we
  copy on purpose. Name the Heroic function when it helps
  (`Heroic's getDefaultWinePrefix`). The data is this app's own: never write
  comments (or code) that assume Heroic shares or reads it.
- **Invariants and safety:** `// SAFETY:`, lock and borrow rules, threads, and
  "no I/O in render".
- **Units, ranges and special values** of fields and parameters when the type
  does not show them (`0..=100`, `None` = unknown).
- **`///` docs** on items used outside their module, when the name and the
  signature do not say everything.

Remove:
- Comments that repeat the code: `// Save the file` above `save()`, or
  `/// The id.` on `id`.
- Section labels that the next line already makes clear (`// History.`,
  `// Buttons`).
- Change history: "was X", "moved from Y", "the old dialog is gone",
  "legacy name". Git keeps the history.
- References to design mockups, briefs or other files that are not in the
  repository (`o1-1.jpg`, "OPTION 1"). `docs/UI-CONVENTIONS.md` describes the
  design.
- Commented-out code.

Long `// ------` section rulers are for long files that cannot be split. In
any other file, the module is the section.

Write in English, in the present tense, and keep it short. One line is
usually enough.

## Environment switches
Read `HERONITE_*` variables in the app only through `crate::env::{flag, value,
parse}`. Describe each one once, where it is read. A switch is "on" when it is
set to anything other than `""` or `"0"`.

## Behaviour-preserving refactors
A refactor commit changes no behaviour. User-visible strings, files written,
JSON keys and formats, and the order of side effects all stay the same. Keep
it separate from feature commits.

## Lints
Run `cargo clippy --workspace --all-targets`. It must report zero warnings.
The workspace allows `type_complexity` and `too_many_arguments` (GPUI
callbacks and builder-style render helpers); do not add `#[allow]` for other
lints without a comment saying why.
