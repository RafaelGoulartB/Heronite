//! A game's shortcuts on this computer: one on the Desktop and one in the
//! menu of programs, each named after the game's title. What the app calls
//! on every system:
//!
//! * Linux: `.desktop` files on the Desktop and in the application menu
//!   (`~/.local/share/applications`), see [`crate::desktop`].
//! * Windows: `.lnk` files on the Desktop and in the Start menu
//!   (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`, where Heroic puts
//!   them too), see `crate::lnk`.
//!
//! Both start this program with `--launch <runner> <appName>`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::GameId;

#[cfg(not(windows))]
use crate::desktop as imp;
#[cfg(windows)]
use crate::lnk as imp;

/// Where shortcuts go. Built from the environment by [`Roots::detect`].
#[derive(Debug, Clone)]
pub struct Roots {
    /// The Desktop folder (`None` when the system has none).
    pub desktop: Option<PathBuf>,
    /// The menu of programs: `~/.local/share/applications` on Linux, the
    /// Start menu's `Programs` folder on Windows.
    pub applications: PathBuf,
}

impl Roots {
    /// `HERONITE_DESKTOP_DIR` / `HERONITE_APPLICATIONS_DIR` override the real
    /// folders (tests and manual runs against fixtures).
    pub fn detect() -> Self {
        let desktop = std::env::var_os("HERONITE_DESKTOP_DIR")
            .map(PathBuf::from)
            .or_else(dirs::desktop_dir)
            .or_else(|| dirs::home_dir().map(|h| h.join("Desktop")));
        // In a Flatpak `dirs::data_dir()` is the sandbox's: the menu reads
        // the host's.
        let data_dir = || {
            if heronite_platform::flatpak::active() {
                heronite_platform::flatpak::host_xdg("XDG_DATA_HOME", ".local/share")
            } else {
                dirs::data_dir()
            }
        };
        let applications = std::env::var_os("HERONITE_APPLICATIONS_DIR")
            .map(PathBuf::from)
            .or_else(|| data_dir().map(|d| d.join(MENU_UNDER_DATA_DIR)))
            .unwrap_or_else(|| PathBuf::from(MENU_UNDER_DATA_DIR));
        Self { desktop, applications }
    }
}

/// The menu of programs under `dirs::data_dir()` (`%APPDATA%` on Windows).
#[cfg(not(windows))]
const MENU_UNDER_DATA_DIR: &str = "applications";
#[cfg(windows)]
const MENU_UNDER_DATA_DIR: &str = r"Microsoft\Windows\Start Menu\Programs";

/// What [`add`] wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Written {
    pub desktop: Option<PathBuf>,
    pub menu: Option<PathBuf>,
}

/// `(Desktop file, menu file)` of a title.
pub fn shortcut_files(roots: &Roots, title: &str) -> (Option<PathBuf>, PathBuf) {
    imp::shortcut_files(roots, title)
}

/// Adds the shortcuts: `desktop` and/or `menu` choose the targets
/// (`addDesktopShortcuts` / `addStartMenuShortcuts`, or both when asked from
/// the menu). Existing ones are replaced (the program may have moved).
/// `icon`: a picture on Linux, an `.ico` on Windows (the program's own icon
/// without one).
pub fn add(roots: &Roots, game: &GameId, exe: &Path, icon: Option<&Path>, desktop: bool, menu: bool) -> Result<Written> {
    imp::add(roots, game, exe, icon, desktop, menu)
}

/// Removes both shortcuts when present; returns what was removed.
pub fn remove(roots: &Roots, title: &str) -> Result<Vec<PathBuf>> {
    imp::remove(roots, title)
}

/// `shortcutsExists`: either shortcut is there.
pub fn exists(roots: &Roots, title: &str) -> bool {
    imp::exists(roots, title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_calls_work_on_every_system() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("menu") };
        let game = GameId { runner: "sideload".into(), app_name: "Quail".into(), title: "A Game: Part 2".into() };
        let exe = std::env::current_exe().unwrap();
        assert!(!exists(&roots, &game.title));
        let written = add(&roots, &game, &exe, None, true, false).unwrap();
        let (desktop, menu) = shortcut_files(&roots, &game.title);
        assert_eq!(written, Written { desktop: desktop.clone(), menu: None });
        let ext = if cfg!(windows) { "lnk" } else { "desktop" };
        assert_eq!(desktop.unwrap(), dir.path().join("Desktop").join(format!("A Game Part 2.{ext}")));
        assert!(exists(&roots, &game.title));
        assert!(!menu.exists());
        assert_eq!(remove(&roots, &game.title).unwrap().len(), 1);
        assert!(!exists(&roots, &game.title));
    }
}
