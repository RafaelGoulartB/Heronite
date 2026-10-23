//! Light or dark (Settings → Appearance → Theme). System follows the
//! desktop's style, which GPUI reads from the XDG settings portal
//! (`org.freedesktop.appearance color-scheme`). Console mode, the in-game
//! overlay and Statistics stay dark whatever the setting: their colours are
//! made for a dark room and dark art.
//!
//! Like the appearance, the scheme is a process-wide value every token reads
//! without a `cx`. The root view of each window says at the start of its
//! render whether that window is dark only ([`set_dark_only`]): GPUI draws
//! one window at a time on the UI thread, and the root renders first, so
//! every token read while a window draws sees its own value.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::WindowAppearance;
use heronite_data::appearance::Theme;

static SYSTEM_LIGHT: AtomicBool = AtomicBool::new(false);
static DARK_ONLY: AtomicBool = AtomicBool::new(false);

/// The desktop's style, as the window reports it. True when it changed.
pub fn set_system_appearance(a: WindowAppearance) -> bool {
    let light = matches!(a, WindowAppearance::Light | WindowAppearance::VibrantLight);
    SYSTEM_LIGHT.swap(light, Ordering::Relaxed) != light
}

/// The window being drawn stays dark (console mode, the overlay, Statistics).
pub fn set_dark_only(on: bool) {
    DARK_ONLY.store(on, Ordering::Relaxed);
}

/// The theme the user picked, System resolved: light or not. The window
/// being drawn may still be dark only; tokens read [`is_light`].
pub fn theme_is_light() -> bool {
    match super::appearance().theme {
        Theme::Dark => false,
        Theme::Light => true,
        Theme::System => SYSTEM_LIGHT.load(Ordering::Relaxed),
    }
}

/// Is the interface being drawn light?
pub fn is_light() -> bool {
    !DARK_ONLY.load(Ordering::Relaxed) && theme_is_light()
}
