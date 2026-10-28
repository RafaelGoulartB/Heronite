//! This app's own executable: what shortcuts, autostart, a restart and an
//! update start or replace.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The AppImage this process runs from. `$APPIMAGE` alone is not enough: a
/// process started from another AppImage (a terminal, an editor) inherits
/// that one's. The AppImage runtime mounts the image at `$APPDIR`, so this
/// executable must be inside it.
pub fn own_appimage() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    appimage_of(std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"), &exe)
}

/// What starts this app again: its AppImage, else its executable.
pub fn own_executable() -> Option<PathBuf> {
    own_appimage().or_else(|| std::env::current_exe().ok())
}

fn appimage_of(appimage: Option<OsString>, appdir: Option<OsString>, exe: &Path) -> Option<PathBuf> {
    let appimage = appimage.filter(|p| !p.is_empty())?;
    let appdir = appdir.filter(|p| !p.is_empty())?;
    exe.starts_with(&appdir).then(|| PathBuf::from(appimage))
}

#[cfg(test)]
mod tests {
    use super::appimage_of;
    use std::path::{Path, PathBuf};

    #[test]
    fn appimage_only_when_this_executable_is_inside_its_mount() {
        let os = |s: &str| Some(s.into());
        let inside = Path::new("/tmp/.mount_HeroAb/usr/bin/heronite-rs");
        assert_eq!(
            appimage_of(os("/apps/Heronite.AppImage"), os("/tmp/.mount_HeroAb"), inside),
            Some(PathBuf::from("/apps/Heronite.AppImage"))
        );
        // Inherited from another AppImage (started from its terminal).
        let other = Path::new("/home/me/heronite/target/debug/heronite-rs");
        assert_eq!(appimage_of(os("/apps/Editor.AppImage"), os("/tmp/.mount_EditXy"), other), None);
        assert_eq!(appimage_of(os("/apps/Heronite.AppImage"), None, inside), None);
        assert_eq!(appimage_of(os(""), os("/tmp/.mount_HeroAb"), inside), None);
    }
}
