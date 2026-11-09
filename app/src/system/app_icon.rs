//! The app's icon, the controller-H on a graphite tile, and the window that
//! needs it from the app.
//!
//! - The tray keeps its own plain H (`assets/tray/`), sharper at its sizes.
//! - The main window on X11 gets it when it opens (`WindowOptions::icon`).
//! - Everywhere else the system shows the packaged icon: the executable's on
//!   Windows (`packaging/windows/heronite.ico`), and on Linux the one the
//!   `.desktop` entry names, `app.heronite.Heronite`, which the launcher, the
//!   dock and Wayland's windows show.
//!
//! The pictures come from `packaging/icons/` through
//! `scripts/make-app-icons.sh`.

use std::sync::Arc;

use image::RgbaImage;

/// The icon, 256 px.
const PNG: &[u8] = include_bytes!("../../assets/app-icon/graphite.png");

fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes).ok().map(|img| img.to_rgba8())
}

/// The window's icon on X11.
pub fn x11_window_icon() -> Option<Arc<RgbaImage>> {
    decode(PNG).map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes_at_256() {
        assert_eq!(x11_window_icon().map(|i| i.dimensions()), Some((256, 256)));
    }
}
