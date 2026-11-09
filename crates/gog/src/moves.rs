//! Moving an installed game to another folder.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::disk::{copy_tree, dir_size, remove_tree, safe_to_remove};
use crate::{installed, library};

/// Windows' error for a rename to another drive.
const ERROR_NOT_SAME_DEVICE: i32 = 17;

/// Moves the game's folder into `new_base` (the folder keeps its name) and
/// updates `install_path`. A rename is tried first; across file systems the
/// folder is copied (`on_percent` is `0..=100`) and then deleted. Returns the
/// new install path.
pub fn move_install(paths: &DataPaths, app_name: &str, new_base: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<String> {
    let Some(mut entry) = installed::get(paths, app_name) else { bail!(tr!("{app} is not installed", app = app_name)) };
    if entry.from_galaxy() {
        bail!(tr!("GOG Galaxy installed this game: move it in GOG Galaxy"));
    }
    let source = Path::new(&entry.install_path);
    let home = crate::disk::home();
    if entry.install_path.is_empty() || !safe_to_remove(source, home.as_deref()) {
        bail!(tr!("Refusing to move the game at {path}", path = entry.install_path));
    }
    if !source.is_dir() {
        bail!(tr!("{path} does not exist", path = entry.install_path));
    }
    let name = source.file_name().ok_or_else(|| anyhow!(tr!("Refusing to move the game at {path}", path = entry.install_path)))?;
    let target = Path::new(new_base).join(name);
    if target == source {
        bail!(tr!("The game is already in that folder"));
    }
    if target.starts_with(source) {
        bail!(tr!("The new folder is inside the game's folder"));
    }
    if target.exists() {
        bail!(tr!("{path} already exists", path = target.display()));
    }
    std::fs::create_dir_all(new_base).map_err(|e| anyhow!(tr!("Could not create {path}: {error}", path = new_base, error = e)))?;

    if let Err(err) = std::fs::rename(source, &target) {
        // On Windows only another drive is copied: a folder that cannot be
        // renamed because a file is in use would not be deleted either.
        if cfg!(windows) && err.raw_os_error() != Some(ERROR_NOT_SAME_DEVICE) {
            bail!(tr!("Could not move the game: {error}", error = err));
        }
        let total = dir_size(source).max(1);
        let mut copied = 0u64;
        let copy = copy_tree(source, &target, cancel, &mut |n| {
            copied += n;
            on_percent((copied as f64 / total as f64 * 100.).min(100.));
        });
        match copy {
            Ok(true) => {
                remove_tree(source).map_err(|e| anyhow!(tr!("Could not delete {path}: {error}", path = entry.install_path, error = e)))?
            }
            Ok(false) => {
                let _ = remove_tree(&target);
                bail!(tr!("The move was canceled"));
            }
            Err(err) => {
                let _ = remove_tree(&target);
                bail!(tr!("Could not copy the game: {error}", error = err));
            }
        }
    }
    on_percent(100.);
    entry.install_path = target.display().to_string();
    installed::upsert(paths, &entry)?;
    library::sync_install_state(paths, app_name, None);
    Ok(entry.install_path)
}

/// The game's folder was moved or renamed outside the app: checks that
/// `folder` holds the game (its `goggame-<id>.info`, or a Linux build's
/// `gameinfo` / `start.sh`) and points the install at it. Returns the new
/// install path.
pub fn relocate(paths: &DataPaths, app_name: &str, folder: &str) -> Result<String> {
    let Some(mut entry) = installed::get(paths, app_name) else { bail!(tr!("{app} is not installed", app = app_name)) };
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let folder = folder.trim().trim_end_matches(separators);
    let dir = Path::new(folder);
    if !dir.is_dir() {
        bail!(tr!("{path} does not exist", path = folder));
    }
    let info = dir.join(format!("goggame-{app_name}.info")).is_file();
    let linux = entry.platform == "linux" && (dir.join("gameinfo").is_file() || dir.join("start.sh").is_file());
    if !info && !linux {
        bail!(tr!("That folder does not look like this game's install (goggame-{app}.info is missing)", app = app_name));
    }
    entry.install_path = folder.to_string();
    installed::upsert(paths, &entry)?;
    library::sync_install_state(paths, app_name, dir.file_name().and_then(|n| n.to_str()));
    Ok(entry.install_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::Installed;

    fn install(paths: &DataPaths, dir: &Path) -> std::path::PathBuf {
        let game = dir.join("one/The Game");
        std::fs::create_dir_all(game.join("sub")).unwrap();
        std::fs::write(game.join("a.bin"), [1u8; 100]).unwrap();
        std::fs::write(game.join("sub/b.bin"), [2u8; 50]).unwrap();
        let mut entry = Installed::new("42");
        entry.install_path = game.display().to_string();
        entry.version = "1.0".into();
        installed::upsert(paths, &entry).unwrap();
        game
    }

    #[test]
    fn the_folder_moves_and_the_record_follows() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let game = install(&paths, dir.path());
        let base = dir.path().join("two/games");
        let mut last = 0.;
        let moved = move_install(&paths, "42", &base.display().to_string(), &Cancel::new(), &mut |p| last = p).unwrap();
        assert_eq!(moved, base.join("The Game").display().to_string());
        assert!(!game.exists());
        assert_eq!(std::fs::read(base.join("The Game/sub/b.bin")).unwrap(), [2u8; 50]);
        let entry = installed::get(&paths, "42").unwrap();
        assert_eq!((entry.install_path, entry.version.as_str()), (moved, "1.0"));
        assert_eq!(last, 100.);
    }

    #[test]
    fn bad_targets_are_refused_and_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let game = install(&paths, dir.path());
        let cancel = Cancel::new();
        let parent = game.parent().unwrap().display().to_string();
        assert!(move_install(&paths, "42", &parent, &cancel, &mut |_| {}).is_err());
        assert!(move_install(&paths, "42", &game.display().to_string(), &cancel, &mut |_| {}).is_err());
        let taken = dir.path().join("taken");
        std::fs::create_dir_all(taken.join("The Game")).unwrap();
        assert!(move_install(&paths, "42", &taken.display().to_string(), &cancel, &mut |_| {}).is_err());
        assert!(move_install(&paths, "nope", "/tmp/x", &cancel, &mut |_| {}).is_err());
        assert!(game.join("a.bin").exists());
        assert_eq!(installed::get(&paths, "42").unwrap().install_path, game.display().to_string());
    }

    #[test]
    fn relocate_points_the_record_at_a_folder_moved_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let game = install(&paths, dir.path());
        let moved = dir.path().join("elsewhere/Renamed Game");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::rename(&game, &moved).unwrap();
        // Not this game's folder until its info file is there.
        assert!(relocate(&paths, "42", &moved.display().to_string()).is_err());
        std::fs::write(moved.join("goggame-42.info"), "{}").unwrap();
        assert!(relocate(&paths, "42", "/nonexistent-dir").is_err());
        assert!(relocate(&paths, "7", &moved.display().to_string()).is_err(), "not installed");
        let now = relocate(&paths, "42", &format!("{}{}", moved.display(), std::path::MAIN_SEPARATOR)).unwrap();
        assert_eq!(now, moved.display().to_string());
        assert_eq!(installed::get(&paths, "42").unwrap().install_path, now);
        assert!(moved.join("a.bin").exists(), "nothing was moved or deleted");
    }
}
