//! The console options that act while console mode is off: how the app
//! opens, what a controller connecting does and the button held to enter
//! console mode (`console::summon` acts on the last two). They are kept in
//! `console.json` and mirrored in `ui-state.json` (read at start anyway), so
//! the start reads no other file for them.

use heronite_data::console_theme::{AutoConsole, ConsoleButton, OpenIn};
use heronite_data::paths::AppPaths;
use heronite_data::ui_state;

pub fn set_open_in(app: &AppPaths, v: OpenIn) {
    super::themes::update(app, |p| p.open_in = v);
    if let Err(e) = ui_state::set_console_open_in(app, v) {
        log::warn!("mirroring openIn: {e:#}");
    }
}

pub fn set_auto_console(app: &AppPaths, v: AutoConsole) {
    super::themes::update(app, |p| p.auto_console = v);
    if let Err(e) = ui_state::set_console_auto(app, v) {
        log::warn!("mirroring autoConsole: {e:#}");
    }
}

pub fn set_console_button(app: &AppPaths, v: ConsoleButton) {
    super::themes::update(app, |p| p.console_button = v);
    if let Err(e) = ui_state::set_console_button(app, v) {
        log::warn!("mirroring consoleButton: {e:#}");
    }
}

pub fn set_fullscreen(app: &AppPaths, v: bool) {
    super::themes::update(app, |p| p.fullscreen = v);
    if let Err(e) = ui_state::set_console_fullscreen(app, v) {
        log::warn!("mirroring fullscreen: {e:#}");
    }
}

/// Whether a window opening now opens fullscreen for console mode: the
/// option is on and the app opens in console mode (`console`: it was asked
/// to). Reads the values mirrored in ui-state.json.
pub fn opens_fullscreen(app: &AppPaths, console: bool) -> bool {
    let ui = ui_state::UiState::load(app);
    ui.console_fullscreen && (console || opens_in_console(ui.console_open_in, ui.console_mode))
}

/// Whether the app starts in console mode: `Last` repeats how it was left.
pub fn opens_in_console(open_in: OpenIn, left_in_console: bool) -> bool {
    match open_in {
        OpenIn::Last => left_in_console,
        OpenIn::Console => true,
        OpenIn::Desktop => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_where_the_option_says() {
        assert!(opens_in_console(OpenIn::Last, true));
        assert!(!opens_in_console(OpenIn::Last, false));
        assert!(opens_in_console(OpenIn::Console, false));
        assert!(!opens_in_console(OpenIn::Desktop, true));
    }

    #[test]
    fn a_window_opens_fullscreen_only_for_console_mode() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert!(!opens_fullscreen(&app, false), "opens in desktop mode");
        assert!(opens_fullscreen(&app, true), "on by default");
        ui_state::set_console_open_in(&app, OpenIn::Console).unwrap();
        assert!(opens_fullscreen(&app, false));
        ui_state::set_console_fullscreen(&app, false).unwrap();
        assert!(!opens_fullscreen(&app, false) && !opens_fullscreen(&app, true));
    }
}
