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
    try_as!(crate::dialogs::epic_confirm::EpicConfirmDialog);
    None
}
