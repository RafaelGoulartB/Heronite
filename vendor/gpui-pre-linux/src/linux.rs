mod dispatcher;
mod display_connection;
mod headless;
mod keyboard;
mod platform;
mod system_notifications;
#[cfg(any(feature = "wayland", feature = "x11"))]
mod text_system;
#[cfg(feature = "wayland")]
mod wayland;
#[cfg(feature = "x11")]
mod x11;

#[cfg(any(feature = "wayland", feature = "x11"))]
mod xdg_desktop_portal;

pub use dispatcher::*;
pub(crate) use display_connection::{Backend, DisplayConnection, select_backend};
pub(crate) use headless::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
#[cfg(any(feature = "wayland", feature = "x11"))]
pub(crate) use text_system::*;
#[cfg(feature = "wayland")]
pub(crate) use wayland::*;
#[cfg(feature = "x11")]
pub(crate) use x11::*;

use std::rc::Rc;

use gpui::WindowingModes;

/// Returns the default platform implementation for the current OS.
///
/// A windowed platform connects to the display server the process environment names, or starts
/// headless when it names none, and can switch between those modes later. A headless platform
/// stays headless.
pub fn current_platform(headless: bool) -> Rc<dyn gpui::Platform> {
    if headless || std::env::var_os("ZED_HEADLESS").is_some() {
        linux_platform(WindowingModes::HEADLESS)
    } else {
        linux_platform(WindowingModes::all())
    }
}

/// Returns a platform that may switch among `allowed_modes`.
///
/// It starts windowed in the process's own environment, or headless if that names no allowed
/// display server. Set another initial mode with [`gpui::Application::with_windowing`].
pub fn linux_platform(allowed_modes: WindowingModes) -> Rc<dyn gpui::Platform> {
    let platform = Rc::new(LinuxPlatform::new(allowed_modes));
    PLATFORM.with(|p| *p.borrow_mut() = Rc::downgrade(&platform));
    platform
}

thread_local! {
    /// Heronite: the platform made last, for [`show_cursor`] and
    /// [`hide_cursor_until_mouse_moves`] (GPUI has no API for them).
    static PLATFORM: std::cell::RefCell<std::rc::Weak<LinuxPlatform>> = std::cell::RefCell::new(std::rc::Weak::new());
}

/// Heronite: hides the mouse cursor over the app's windows until the mouse
/// moves, as typing does (a game controller is in use).
pub fn hide_cursor_until_mouse_moves() {
    if let Some(platform) = PLATFORM.with(|p| p.borrow().upgrade()) {
        platform.hide_cursor_now();
    }
}

/// Heronite: shows the cursor a hide took away, without waiting for the
/// mouse to move (a key was pressed).
pub fn show_cursor() {
    if let Some(platform) = PLATFORM.with(|p| p.borrow().upgrade()) {
        platform.show_cursor_now();
    }
}
