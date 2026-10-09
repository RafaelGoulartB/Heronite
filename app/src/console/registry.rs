//! Finds the controller hook of a dialog kept as `AnyView` (the hosts open
//! dialogs generically). Every dialog with a console layout is listed here.

use gpui_kit::AnyView;

use super::PadHook;

pub fn hook_for(view: &AnyView) -> Option<PadHook> {
    macro_rules! try_as {
        ($($t:ty),* $(,)?) => {
            $(
                if let Ok(e) = view.clone().downcast::<$t>() {
                    return Some(PadHook::new(&e));
                }
            )*
        };
    }
    try_as!(
        crate::dialogs::epic_confirm::EpicConfirmDialog,
        crate::dialogs::epic_install::EpicInstallDialog,
        crate::dialogs::remove_game::RemoveGameDialog,
        crate::dialogs::quit_confirm::QuitConfirmDialog,
        crate::dialogs::ludusavi_backup::LudusaviBackupDialog,
        crate::dialogs::epic_import::EpicImportPicker,
        crate::dialogs::epic_modify::EpicModifyDialog,
        crate::dialogs::game_settings::GameSettingsDialog,
        crate::dialogs::add_game::AddGameDialog,
    );
    None
}
