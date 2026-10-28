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

pub mod achievements;
pub mod add_game;
pub mod art_images;
pub mod batch_edit;
pub mod client_settings;
pub mod emulator_confirm;
pub mod emulator_detect;
pub mod emulator_edit;
pub mod epic_confirm;
pub mod epic_import;
pub mod epic_install;
pub mod epic_modify;
pub mod game_settings;
pub mod launch_problem;
pub mod launch_state;
pub mod log_viewer;
pub mod ludusavi_backup;
pub mod manage_categories;
pub mod manage_fields;
pub mod manage_statuses;
pub mod metadata;
pub mod metadata_wizard;
pub mod onboarding;
pub mod pick_rom;
pub mod quit_confirm;
pub mod remove_game;
pub mod rom_folder;
pub mod save_backups;
pub mod scan_roms;
pub mod screenshot_art;
pub mod script_install;
pub mod settings_panel;
pub mod shortcuts;
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

/// Esc, offered to the open dialog before the host closes it. True when the
/// dialog used the key: it closed something inside itself (an image picker,
/// a question) or asked before throwing away what was typed.
pub fn handle_escape(view: &AnyView, window: &mut Window, cx: &mut App) -> bool {
    if let Ok(d) = view.clone().downcast::<add_game::AddGameDialog>() {
        return d.update(cx, |d, cx| d.on_escape(cx));
    }
    if let Ok(d) = view.clone().downcast::<manage_statuses::ManageStatusesDialog>() {
        return d.update(cx, |d, cx| d.on_escape(cx));
    }
    if let Ok(d) = view.clone().downcast::<manage_categories::ManageCategoriesDialog>() {
        return d.update(cx, |d, cx| d.on_escape(window, cx));
    }
    if let Ok(d) = view.clone().downcast::<batch_edit::BatchEditDialog>() {
        return d.update(cx, |d, cx| d.on_escape(cx));
    }
    if let Ok(d) = view.clone().downcast::<game_settings::GameSettingsDialog>() {
        return d.update(cx, |d, cx| d.on_escape(cx));
    }
    if let Ok(d) = view.clone().downcast::<client_settings::ClientSettingsDialog>() {
        return d.update(cx, |d, cx| d.on_escape(cx));
    }
    false
}

/// What every dialog may need.
#[derive(Clone)]
pub struct DialogCtx {
    pub data_paths: DataPaths,
    pub app: AppPaths,
    pub lib: Arc<Library>,
}

/// The Wine field of the install dialogs while this PC has no Wine or
/// Proton: what the automatic GE-Proton is doing.
fn no_wine_text(cx: &App) -> String {
    use crate::services::proton_setup::Setup;
    match crate::services::proton_setup::state(cx) {
        Setup::Looking => heronite_i18n::tr!("Looking for the newest GE-Proton\u{2026}").into(),
        Setup::Downloading { version, percentage } => {
            heronite_i18n::tr!("Downloading {version}\u{2026} {percent}%", version = &version, percent = percentage.round())
        }
        Setup::Unzipping { version } => heronite_i18n::tr!("Unpacking {version}\u{2026}", version = &version),
        // Until the dialog has read its list again.
        Setup::Ready { version } => version,
        _ => heronite_i18n::tr!("No Wine or Proton found").into(),
    }
}

/// Under the Wine field of the install dialogs when this PC has no Wine or
/// Proton: the newest GE-Proton downloads on its own and the game can
/// install meanwhile; when that fails, where to get one.
fn no_proton_note(cx: &App) -> Div {
    use crate::services::proton_setup::Setup;
    let note = |text: String, error: bool| {
        div()
            .mt(px(-4.))
            .text_size(px(12.5))
            .line_height(px(17.))
            .text_color(if error { crate::theme::danger_text() } else { crate::theme::text_muted() })
            .child(text)
    };
    let state = crate::services::proton_setup::state(cx);
    let percent = match &state {
        Setup::Downloading { percentage, .. } => Some(*percentage as f32 / 100.),
        Setup::Unzipping { .. } => Some(1.),
        _ => None,
    };
    match state {
        Setup::Looking | Setup::Downloading { .. } | Setup::Unzipping { .. } => div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(percent.map(|p| {
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(crate::theme::white_a(0.1))
                    .child(div().h_full().w(relative(p.clamp(0., 1.))).rounded_full().bg(crate::theme::accent()))
            }))
            .child(note(
                heronite_i18n::tr!(
                    "No Wine or Proton on this PC: the newest GE-Proton is downloading and will run your Windows games. You can install now."
                )
                .into(),
                false,
            )),
        Setup::Ready { version } => note(heronite_i18n::tr!("{version} is installed: your Windows games will run with it.", version = &version), false),
        Setup::Failed(why) => note(
            heronite_i18n::tr!(
                "GE-Proton could not be downloaded ({why}). You can install now, but Windows games need Proton to run. Download it from Wine Manager in the menu (\u{2630}).",
                why = &why
            )
            ,
            true,
        ),
        _ => note(
            heronite_i18n::tr!(
                "You can install now, but Windows games need Proton to run. Download it from Wine Manager in the menu (\u{2630})."
            )
            .into(),
            true,
        ),
    }
}

/// The dialog found no Wine or Proton: starts the automatic GE-Proton and
/// calls `reload` (the dialog's Wine list) once it is installed.
fn auto_proton<T: 'static>(cx: &mut Context<T>, reload: fn(&mut T, &mut Context<T>)) {
    use crate::services::proton_setup::{self, Setup};
    if !cfg!(target_os = "linux") {
        return;
    }
    let setup = proton_setup::get(cx);
    cx.observe(&setup, move |this, setup, cx| {
        if matches!(setup.read(cx).state(), Setup::Ready { .. }) {
            reload(this, cx);
        }
        cx.notify();
    })
    .detach();
    proton_setup::ensure(cx);
}

/// The text of an input, trimmed.
fn text_of(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}
