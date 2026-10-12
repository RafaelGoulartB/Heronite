//! The interface language: set at startup from the saved choice (the system
//! language by default, English when it is not one we have) and switched live
//! from Settings. Text is translated while rendering, so a redraw is enough;
//! the few strings kept in state (input placeholders...) follow on reopen.

use gpui_kit::App;
use heronite_data::{language, AppPaths};
use heronite_i18n::Preference;

/// Before the first window: the language every view starts with.
pub fn init(app_paths: &AppPaths) {
    heronite_i18n::set(language::load(app_paths).resolve());
}

/// Saves `pref` and redraws everything in the new language.
pub fn choose(app_paths: &AppPaths, pref: Preference, cx: &mut App) {
    if let Err(e) = language::save(app_paths, pref) {
        log::warn!("could not save the interface language: {e:#}");
    }
    heronite_i18n::set(pref.resolve());
    crate::system::refresh_tray(cx);
    cx.refresh_windows();
}
