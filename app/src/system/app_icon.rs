//! The app's icon, one of [`AppIcon`] (Settings → Appearance): its pictures,
//! and where the system shows it.
//!
//! - The tray keeps its own plain H (`assets/tray/`), sharper at its sizes.
//! - The main window: on X11 it gets the icon when it opens
//!   (`WindowOptions::icon`), on Windows at once (`WM_SETICON`; the
//!   executable's own icon is the default one).
//! - On Linux the launcher, the dock and Wayland's windows show the icon the
//!   `.desktop` entry names, `app.heronite.Heronite`: a change writes the picked one
//!   to `$XDG_DATA_HOME/icons/hicolor/256x256/apps/`, which comes before the
//!   system's folders.
//!
//! The pictures come from `packaging/icons/` through
//! `scripts/make-app-icons.sh`.

use std::sync::Arc;

use heronite_data::appearance::AppIcon;
use image::RgbaImage;

/// Up to this size (px) the small picture is used: the buttons would be specks.
#[cfg(any(test, windows))]
const SMALL_UP_TO: u32 = 32;

/// The icon, 256 px.
pub fn png(icon: AppIcon) -> &'static [u8] {
    match icon {
        AppIcon::Graphite => include_bytes!("../../assets/app-icon/graphite.png"),
        AppIcon::Light => include_bytes!("../../assets/app-icon/light.png"),
        AppIcon::Extruded => include_bytes!("../../assets/app-icon/extruded.png"),
    }
}

/// The small icon (the H without its buttons), 64 px.
#[cfg(any(test, windows))]
pub fn small_png(icon: AppIcon) -> &'static [u8] {
    match icon {
        AppIcon::Graphite => include_bytes!("../../assets/app-icon/graphite-small.png"),
        AppIcon::Light => include_bytes!("../../assets/app-icon/light-small.png"),
        AppIcon::Extruded => include_bytes!("../../assets/app-icon/extruded-small.png"),
    }
}

fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes).ok().map(|img| img.to_rgba8())
}

#[cfg(any(test, windows))]
fn scaled(img: &RgbaImage, size: u32) -> RgbaImage {
    if img.width() == size {
        img.clone()
    } else {
        image::imageops::resize(img, size, size, image::imageops::FilterType::Lanczos3)
    }
}

/// The icon at each of `sizes` (px, square): the small one up to 32 px, the
/// full one above; empty if it cannot be decoded.
#[cfg(any(test, windows))]
pub fn window_images(icon: AppIcon, sizes: &[u32]) -> Vec<RgbaImage> {
    let (Some(small), Some(full)) = (decode(small_png(icon)), decode(png(icon))) else { return Vec::new() };
    sizes.iter().map(|&size| scaled(if size <= SMALL_UP_TO { &small } else { &full }, size)).collect()
}

/// The window's icon on X11.
pub fn x11_window_icon(icon: AppIcon) -> Option<Arc<RgbaImage>> {
    decode(png(icon)).map(Arc::new)
}

/// Writes `icon` as `app.heronite.Heronite.png` in `icons/hicolor/256x256/apps`
/// under `data_home` (`$XDG_DATA_HOME`). Blocking: call it off the UI thread.
#[cfg(target_os = "linux")]
pub fn write_desktop_icon(data_home: &std::path::Path, icon: AppIcon) -> std::io::Result<()> {
    let theme = data_home.join("icons/hicolor");
    let dir = theme.join("256x256/apps");
    std::fs::create_dir_all(&dir)?;
    heronite_data::temps::write_atomic(&dir.join("app.heronite.Heronite.png"), png(icon), Some(0o644))?;
    // A theme folder newer than its icon-theme.cache makes GTK and Qt look again.
    std::fs::File::open(&theme)?.set_modified(std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_decodes_at_its_size() {
        for &icon in AppIcon::ALL {
            assert_eq!(decode(png(icon)).map(|i| i.dimensions()), Some((256, 256)), "{icon:?}");
            assert_eq!(decode(small_png(icon)).map(|i| i.dimensions()), Some((64, 64)), "{icon:?}");
        }
    }

    #[test]
    fn small_sizes_take_the_small_picture() {
        let imgs = window_images(AppIcon::Graphite, &[16, 32, 48, 256]);
        assert_eq!(imgs.iter().map(|i| i.width()).collect::<Vec<_>>(), [16, 32, 48, 256]);
        assert_eq!(imgs[1], scaled(&decode(small_png(AppIcon::Graphite)).unwrap(), 32));
        assert_eq!(imgs[3], decode(png(AppIcon::Graphite)).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_desktop_icon_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        write_desktop_icon(dir.path(), AppIcon::Light).unwrap();
        let file = dir.path().join("icons/hicolor/256x256/apps/app.heronite.Heronite.png");
        assert_eq!(std::fs::read(&file).unwrap(), png(AppIcon::Light));
        write_desktop_icon(dir.path(), AppIcon::Extruded).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), png(AppIcon::Extruded));
    }
}
