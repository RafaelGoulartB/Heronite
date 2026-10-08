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

pub mod add_game;
pub mod emulator_confirm;
pub mod emulator_detect;
pub mod emulator_edit;
pub mod epic_confirm;
pub mod epic_import;
pub mod epic_install;
pub mod epic_modify;
pub mod launch_state;
pub mod log_viewer;
pub mod ludusavi_backup;
pub mod pick_rom;
pub mod art_images;
pub mod game_settings;
pub mod heroic_settings;
pub mod manage_statuses;
pub mod metadata;
pub mod metadata_wizard;
pub mod screenshot_art;
pub mod web_image;
pub mod playtime;
pub mod quit_confirm;
pub mod remove_game;
pub mod rom_folder;
pub mod scan_roms;

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
