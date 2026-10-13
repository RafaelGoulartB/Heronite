//! Moving an installed game to another folder.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::disk::{copy_tree, dir_size, remove_tree, safe_to_remove};
use crate::{installed, library};

/// Moves the game's folder into `new_base` (the folder keeps its name) and
/// updates `install_path`. A rename is tried first; across file systems the
/// folder is copied (`on_percent` is `0..=100`) and then deleted. Returns the
/// new install path.
pub fn move_install(paths: &DataPaths, app_name: &str, new_base: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<String> {
    let Some(mut entry) = installed::get(paths, app_name) else { bail!(tr!("{app} is not installed", app = app_name)) };
    let source = Path::new(&entry.install_path);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
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

    if std::fs::rename(source, &target).is_err() {
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
}
