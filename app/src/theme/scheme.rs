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

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use gpui_kit::WindowAppearance;
use heronite_data::appearance::{Palette, Theme};

static SYSTEM_LIGHT: AtomicBool = AtomicBool::new(false);
static DARK_ONLY: AtomicBool = AtomicBool::new(false);
/// The scheme [`drawn_as`] forces: 0 none, then 1 + [`Scheme`].
static FORCED: AtomicU8 = AtomicU8::new(0);

/// The colours a window or a picture is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
    /// The user's own colours (Settings → Appearance → Theme → Custom).
    Custom,
}

/// The desktop's style, as the window reports it. True when it changed.
pub fn set_system_appearance(a: WindowAppearance) -> bool {
    let light = matches!(a, WindowAppearance::Light | WindowAppearance::VibrantLight);
    SYSTEM_LIGHT.swap(light, Ordering::Relaxed) != light
}

/// The window being drawn stays dark (console mode, the overlay, Statistics).
pub fn set_dark_only(on: bool) {
    DARK_ONLY.store(on, Ordering::Relaxed);
}

/// Builds `f`'s elements as `scheme` draws them: a picture of a screen in
/// another theme, or of one that stays dark. The colours are read while
/// building, so the elements keep them; nothing in `f` may read one later
/// (a canvas's paint).
pub fn drawn_as<R>(scheme: Scheme, f: impl FnOnce() -> R) -> R {
    let was = FORCED.swap(1 + scheme as u8, Ordering::Relaxed);
    let out = f();
    FORCED.store(was, Ordering::Relaxed);
    out
}

/// The desktop's style is light.
pub fn system_is_light() -> bool {
    SYSTEM_LIGHT.load(Ordering::Relaxed)
}

/// `t` resolved: light or not (System: the desktop's style).
pub fn theme_light(t: Theme) -> bool {
    match t {
        Theme::Dark => false,
        Theme::Light => true,
        Theme::System => system_is_light(),
        Theme::Custom => super::appearance().custom.light,
    }
}

/// The theme the user picked, resolved. The window being drawn may still be
/// dark only; tokens read [`is_light`].
pub fn theme_is_light() -> bool {
    theme_light(super::appearance().theme)
}

/// The scheme being drawn.
pub fn scheme() -> Scheme {
    match FORCED.load(Ordering::Relaxed) {
        1 => Scheme::Dark,
        2 => Scheme::Light,
        3 => Scheme::Custom,
        _ if DARK_ONLY.load(Ordering::Relaxed) => Scheme::Dark,
        _ if super::appearance().theme == Theme::Custom => Scheme::Custom,
        _ if theme_is_light() => Scheme::Light,
        _ => Scheme::Dark,
    }
}

/// The custom colours while they are what is being drawn; the tokens take
/// them over their own values.
pub fn custom_palette() -> Option<Palette> {
    (scheme() == Scheme::Custom).then(|| super::appearance().custom)
}

/// Is the interface being drawn light?
pub fn is_light() -> bool {
    match scheme() {
        Scheme::Dark => false,
        Scheme::Light => true,
        Scheme::Custom => super::appearance().custom.light,
    }
}
