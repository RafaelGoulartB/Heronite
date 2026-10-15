//! Dialogs of the Collection screen. Each dialog is its own view (an
//! `Entity<V>` with `V: Render + EventEmitter<DialogEvent>`) living in its own
//! file; `CollectionView::open_dialog` hosts one at a time.
//!
//! Conventions (see `docs/UI-CONVENTIONS.md`):
//! - Build the chrome with `crate::ui::dialog::dialog` and controls from
//!   `crate::ui::controls`.
//! - Do I/O off the UI thread (`cx.background_spawn`) and write shared files
//!   only through `heronite_data` / backend crates.
//! - Emit `DialogEvent::Changed` after writing data the Collection shows
//!   (it reloads the library), `DialogEvent::Close` to close.

pub mod add_game;
pub mod art_images;
pub mod emulator_confirm;
pub mod emulator_detect;
pub mod emulator_edit;
pub mod epic_confirm;
pub mod epic_import;
pub mod epic_install;
pub mod epic_modify;
pub mod game_settings;
pub mod launch_state;
pub mod log_viewer;
pub mod ludusavi_backup;
pub mod manage_categories;
pub mod manage_statuses;
pub mod metadata;
pub mod metadata_wizard;
pub mod onboarding;
pub mod pick_rom;
pub mod playtime;
pub mod quit_confirm;
pub mod remove_game;
pub mod rom_folder;
pub mod save_backups;
pub mod scan_roms;
pub mod screenshot_art;
pub mod settings_panel;
pub mod store_import;
pub mod store_install;
pub mod web_image;

use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths, Library};

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
    pub data_paths: DataPaths,
    pub app: AppPaths,
    pub lib: Arc<Library>,
}

/// Under the Wine field of the install dialogs when this PC has no Wine or
/// Proton: the game installs, but cannot start until one is downloaded.
fn no_proton_note() -> Div {
    div().mt(px(-4.)).text_size(px(12.5)).line_height(px(17.)).text_color(crate::theme::danger_text()).child(heronite_i18n::tr!(
        "You can install now, but Windows games need Proton to run. Download it from Wine Manager in the menu (\u{2630})."
    ))
}

/// The text of an input, trimmed.
fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}
