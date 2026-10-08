//! The one set of form helpers (Heroic `TextInputField`, `PathSelectionBox`,
//! `SelectField`, textarea and the Collection tag field) plus the dialog
//! typography and the grey panel box. Everything shares the same look: the
//! `--input-background` surface of [`input::input_surface`], the label of
//! [`text::field_label`] and the `Rubik` input font.
//!
//! * [`text`]: paragraphs, labels, [`text::labeled`] and [`text::panel`].
//! * [`input`]: single-line / multi-line inputs, path fields, file chooser.
//! * [`select`]: [`select::select_field`] (label, select, open menu).
//! * [`tags`]: [`tags::TagField`] (chips + autocomplete) and tag helpers.
//!
//! Values come from `TextInputField/index.css`, `SelectField/index.css` and
//! the captures in `docs/reference/dialogs/`. Callers import from
//! `crate::ui::fields` only; the submodules are an implementation detail.

mod input;
mod select;
mod tags;
mod text;

pub use input::{choose_path, input_box, new_input, new_textarea, on_commit, path_field, search_box, text_field, textarea_box};
pub use select::select_field;
pub use tags::{TagEvent, TagField};
pub use text::{field_label, labeled, meta_text, panel, paragraph};
