# Your brief: UNIFY FORM HELPERS (agent "refactor")

app/src/ui/fields.rs (≈240 lines) and app/src/dialogs/form.rs (≈376 lines) are two sets of input/field helpers that
grew in parallel (text inputs, labeled rows, selects/menus, textareas, tag fields, etc.). ~35 files use ui::fields and
~7 use dialogs::form. Unify them into ONE module (keep it in app/src/ui/, e.g. app/src/ui/fields.rs or a ui/form/
folder) with one consistent API and look, migrate every call site, delete the duplicate. Pure refactor: no visual
regressions — capture before/after screenshots (scripts/shot.sh, workspace 7) of a representative set of dialogs/pages
that use each helper (game settings tabs, add game, metadata dialogs, settings page sections, emulator dialogs,
epic install) and compare; behaviour (focus, typing, events, debounced saves) must stay identical. Also look for other
obvious duplicated small UI helpers next to them (toggles/checkbox rows, section titles) and fold them into
app/src/ui/controls.rs if clearly duplicated — but keep the scope reasonable. Update docs/UI-CONVENTIONS.md.
Run `cargo build` + `cargo test -p heronite` and fix warnings you introduce. You are the only agent running.
