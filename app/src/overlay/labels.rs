//! The texts that name the overlay's controller chord, as this system has it:
//! the Guide button on Linux, Back + Start on Windows (XInput has no Guide,
//! `heronite_overlay::input`). Every screen that names the chord takes its
//! text from here, so the two systems cannot drift apart.
//!
//! Each system has whole sentences of its own (never built from fragments).
//! The labels the Settings search lists are English keys ([`BUTTON_HELD`],
//! [`VOLUME`], [`MOMENT`], through `n_!`) drawn with `tr`; the rest come
//! translated. Guide + X is Linux-only (`Feature::Replay`), so its texts stay
//! where they are drawn.

use heronite_i18n::{n_, tr};

/// The buttons that stand for Guide, as key caps (names: not translated).
const GUIDE_KEYS: &[&str] = if cfg!(windows) { &["Back", "Start"] } else { &["Guide"] };

/// Guide and then `button`, as key caps.
pub fn chord_keys(button: &'static str) -> Vec<&'static str> {
    GUIDE_KEYS.iter().copied().chain([button]).collect()
}

/// "Guide + `button`" ("Back + Start + `button`" on Windows): names, not
/// translated.
pub fn chord_text(button: &'static str) -> String {
    chord_keys(button).join(" + ")
}

/// The English key of the screenshot button's row.
pub const BUTTON_HELD: &str = if cfg!(windows) { n_!("Button held with Back + Start") } else { n_!("Button held with Guide") };

/// The English key of the volume shortcut's row.
pub const VOLUME: &str =
    if cfg!(windows) { n_!("Back + Start + ↑ / ↓: change the volume") } else { n_!("Guide + ↑ / ↓: change the volume") };

/// The English key of the moment shortcut's row.
pub const MOMENT: &str = if cfg!(windows) { n_!("Back + Start + Y: mark a moment") } else { n_!("Guide + Y: mark a moment") };

/// [`BUTTON_HELD`], translated.
pub fn button_held() -> &'static str {
    tr(BUTTON_HELD)
}

/// [`VOLUME`], translated.
pub fn volume() -> &'static str {
    tr(VOLUME)
}

/// [`MOMENT`], translated.
pub fn moment() -> &'static str {
    tr(MOMENT)
}

/// The screenshot shortcut's row, `button` written as after "Guide +".
pub fn screenshot(button: &str) -> String {
    if cfg!(windows) {
        tr!("Back + Start + {button}: take a screenshot", button)
    } else {
        tr!("Guide + {button}: take a screenshot", button)
    }
}

/// Under the screenshot button's choice.
pub fn screenshot_button_desc() -> &'static str {
    if cfg!(windows) {
        tr!("Hold Back + Start and press this button. The game also receives the presses. ↑ / ↓ belong to the volume.")
    } else {
        tr!("The game does not get it while Guide is held. ↑ / ↓ belong to the volume.")
    }
}

/// Under the moment shortcut while Guide + Y takes the screenshot.
pub fn moment_taken_by_screenshot() -> &'static str {
    if cfg!(windows) {
        tr!("Off while Back + Start + Y takes the screenshot: pick another button for it above.")
    } else {
        tr!("Off while Guide + Y takes the screenshot: pick another button for it above.")
    }
}

/// Under "Turn on the overlay" (Settings → Overlay).
pub fn turn_on_desc() -> &'static str {
    if cfg!(windows) {
        tr!("During a game opened by Heronite, press Back + Start together to open it; press them again or B to close it. The game also receives controller input.")
    } else {
        tr!("During a game opened by Heronite, press the controller's Guide button (the centre one) to open it; press it again or B to close it.")
    }
}

/// [`turn_on_desc`] in one line (console mode).
pub fn turn_on_short() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start opens it during a game opened by Heronite")
    } else {
        tr!("The Guide button opens it during a game opened by Heronite")
    }
}

/// Under "Open it without a game too" (Settings → Overlay).
pub fn without_game_desc() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start and the keyboard shortcut open it at any time Heronite is running. Heronite keeps reading XInput controllers; other apps still receive their input. The keyboard shortcut is reserved for the overlay: it stops working in every other app while Heronite runs.")
    } else {
        tr!("Guide and the keyboard shortcut open it at any time Heronite is running, with the volume, the controllers and a way back to Heronite. Heronite then reads the controllers all the time, and Guide is the overlay's even outside games.")
    }
}

/// [`without_game_desc`] in one line (console mode).
pub fn without_game_short() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start and the keyboard shortcut open it at any time")
    } else {
        tr!("Guide and the keyboard shortcut open it at any time")
    }
}

/// The lead of Settings → Overlay.
pub fn section_blurb() -> &'static str {
    if cfg!(windows) {
        tr!("A panel Back + Start or a keyboard shortcut opens over a game launched by Heronite: screenshots, volume, quitting the game.")
    } else {
        tr!("A panel the controller's Guide button or a keyboard shortcut opens over a game launched by Heronite: screenshots, volume, quitting the game.")
    }
}

/// Settings → Overlay in the command palette.
pub fn palette_help() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start panel over games (beta)")
    } else {
        tr!("Guide button panel over games (beta)")
    }
}

/// The pill of the overlay's picture in "Getting started".
pub fn opens_it() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start opens it")
    } else {
        tr!("Guide opens it")
    }
}

/// The overlay's line in "Getting started".
pub fn onboarding_line() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start opens the overlay; hold both with Y to mark a moment. The game also receives controller input.")
    } else {
        tr!("The overlay: Guide opens it, Guide + Y marks a moment")
    }
}

/// The empty session of the panel's Notes tab.
pub fn moment_hint() -> &'static str {
    if cfg!(windows) {
        tr!("Back + Start + Y marks a moment without opening the panel.")
    } else {
        tr!("Guide + Y marks a moment without opening the panel.")
    }
}

#[cfg(test)]
mod tests {
    use super::{chord_keys, chord_text};

    #[test]
    fn chords_name_the_buttons_of_this_system() {
        let expected: &[&str] = if cfg!(windows) { &["Back", "Start", "RB"] } else { &["Guide", "RB"] };
        assert_eq!(chord_keys("RB"), expected);
        assert_eq!(chord_text("Y"), if cfg!(windows) { "Back + Start + Y" } else { "Guide + Y" });
    }
}
