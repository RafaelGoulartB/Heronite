//! `.lnk` shortcuts (Heroic `shortcuts/shortcuts/shortcuts.ts`, Windows
//! branch): `<Desktop>\<title>.lnk` and `<Start menu>\Programs\<title>.lnk`,
//! starting this program with `--launch <runner> <appName>` from its own
//! folder, the title as the tooltip and the game's `.ico` (or the program's
//! own icon) as the picture.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_platform::shortcut::{self, Link};

use crate::desktop::sanitize_filename;
use crate::menu::{Roots, Written};
use crate::GameId;

/// `(Desktop file, Start menu file)` for a title.
pub fn shortcut_files(roots: &Roots, title: &str) -> (Option<PathBuf>, PathBuf) {
    let name = format!("{}.lnk", sanitize_filename(title));
    (roots.desktop.as_ref().map(|d| d.join(&name)), roots.applications.join(name))
}

/// The shortcut of `game`: `exe` started with `--launch <runner> <appName>`.
pub fn link(game: &GameId, exe: &Path, icon: Option<&Path>) -> Link {
    Link {
        target: exe.to_path_buf(),
        arguments: shortcut::join_args(["--launch", &game.runner, &game.app_name]),
        working_dir: exe.parent().map(Path::to_path_buf),
        // Index 0 of the program: its own icon.
        icon: Some((icon.unwrap_or(exe).to_path_buf(), 0)),
        description: game.title.clone(),
    }
}

/// Adds the shortcuts (see [`crate::menu::add`]).
pub fn add(roots: &Roots, game: &GameId, exe: &Path, icon: Option<&Path>, desktop: bool, menu: bool) -> Result<Written> {
    let (desktop_file, menu_file) = shortcut_files(roots, &game.title);
    let bytes = shortcut::encode(&link(game, exe, icon)).context("making the shortcut")?;
    let mut written = Written::default();
    if desktop {
        if let Some(file) = desktop_file {
            write_file(&file, &bytes).with_context(|| format!("writing {}", file.display()))?;
            log::info!("shortcut saved on {}", file.display());
            written.desktop = Some(file);
        }
    }
    if menu {
        write_file(&menu_file, &bytes).with_context(|| format!("writing {}", menu_file.display()))?;
        log::info!("shortcut saved on {}", menu_file.display());
        written.menu = Some(menu_file);
    }
    Ok(written)
}

/// Whole or not at all: through a temp file next to it that the startup
/// sweep knows (`heronite_data::temps`).
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(path, bytes, None)?;
    Ok(())
}

/// Removes both files when present; returns what was removed.
pub fn remove(roots: &Roots, title: &str) -> Result<Vec<PathBuf>> {
    let (desktop_file, menu_file) = shortcut_files(roots, title);
    let mut removed = Vec::new();
    for file in desktop_file.into_iter().chain([menu_file]) {
        if shortcut::remove(&file).with_context(|| format!("removing {}", file.display()))? {
            log::info!("shortcut removed: {}", file.display());
            removed.push(file);
        }
    }
    Ok(removed)
}

/// `shortcutsExists`: either file is there.
pub fn exists(roots: &Roots, title: &str) -> bool {
    let (desktop_file, menu_file) = shortcut_files(roots, title);
    desktop_file.is_some_and(|f| f.exists()) || menu_file.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> GameId {
        GameId { runner: "sideload".into(), app_name: "Quail \"Deluxe\" 2".into(), title: "Half-Life: Alyx".into() }
    }

    #[test]
    fn add_exists_remove_in_tempdirs() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("Start Menu").join("Programs") };
        let exe = dir.path().join(r"Program Files\Heronite\heronite-rs.exe");
        let icon = dir.path().join("icons").join("Quail.ico");
        let g = game();
        assert!(!exists(&roots, &g.title));
        let w = add(&roots, &g, &exe, Some(&icon), true, true).unwrap();
        assert_eq!(w.desktop.as_deref(), Some(dir.path().join(r"Desktop\Half-Life Alyx.lnk").as_path()));
        assert_eq!(w.menu.as_deref(), Some(dir.path().join(r"Start Menu\Programs\Half-Life Alyx.lnk").as_path()));
        assert!(exists(&roots, &g.title));
        for file in [w.desktop.unwrap(), w.menu.unwrap()] {
            let read = shortcut::read(&file).unwrap();
            assert_eq!(read.target, exe);
            assert_eq!(shortcut::split_args(&read.arguments), ["--launch", "sideload", "Quail \"Deluxe\" 2"]);
            assert_eq!(read.working_dir.as_deref(), exe.parent());
            assert_eq!(read.icon, Some((icon.clone(), 0)));
            assert_eq!(read.description, "Half-Life: Alyx");
        }
        // No temp file left next to them.
        assert_eq!(std::fs::read_dir(dir.path().join("Desktop")).unwrap().count(), 1);
        let removed = remove(&roots, &g.title).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!exists(&roots, &g.title));
        assert!(remove(&roots, &g.title).unwrap().is_empty());
    }

    #[test]
    fn without_an_icon_the_program_s_own_is_shown() {
        let exe = Path::new(r"C:\Heronite\heronite-rs.exe");
        let l = link(&game(), exe, None);
        assert_eq!(l.icon, Some((exe.to_path_buf(), 0)));
        assert_eq!(l.working_dir.as_deref(), Some(Path::new(r"C:\Heronite")));
        assert_eq!(l.arguments, r#"--launch sideload "Quail \"Deluxe\" 2""#);
    }

    #[test]
    fn only_requested_targets_are_written_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("Programs") };
        let g = game();
        let exe = std::env::current_exe().unwrap();
        let w = add(&roots, &g, &exe, None, false, true).unwrap();
        assert!(w.desktop.is_none());
        assert!(!dir.path().join("Desktop").exists());
        // Adding again replaces the shortcut (the program moved).
        let moved = dir.path().join("elsewhere").join("heronite-rs.exe");
        add(&roots, &g, &moved, None, false, true).unwrap();
        assert_eq!(shortcut::read(&w.menu.unwrap()).unwrap().target, moved);
    }
}
