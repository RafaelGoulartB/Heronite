//! Dialogs of the Collection screen. Each dialog is its own view (an
//! `Entity<V>` with `V: Render + EventEmitter<DialogEvent>`) living in its own
//! file; `CollectionView::open_dialog` hosts one at a time.
//!
//! Conventions (see `docs/UI-CONVENTIONS.md`):
//! - Build the chrome with `crate::ui::dialog::dialog` and controls from
//!   `crate::ui::controls`; match `docs/reference/dialogs/*.png`.
//! - Do I/O off the UI thread (`cx.background_spawn`) and write shared files
//!   only through `heronite_data` / backend crates.
//! - Emit `DialogEvent::Changed` after writing data the Collection shows
//!   (it reloads the library), `DialogEvent::Close` to close.

pub mod manage_statuses;
pub mod settings;

use std::sync::Arc;

use heronite_data::{AppPaths, HeroicPaths, Library};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogEvent {
    Close,
    /// Data on disk changed; the Collection reloads (dialog stays open).
    Changed,
    /// Like `Changed`, then close.
    ChangedAndClose,
}

/// What every dialog may need.
#[derive(Clone)]
pub struct DialogCtx {
    pub heroic: HeroicPaths,
    pub app: AppPaths,
    pub lib: Arc<Library>,
}
