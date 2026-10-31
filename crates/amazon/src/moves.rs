//! Moving an installed game to another folder, then pointing nile's
//! `installed.json` at it.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::cmd::Cancel;
use crate::{installed, library};

/// Moves the game's folder into `new_base` (it keeps its folder name) and
/// returns the new install path. A rename when both are on one disk, else a
/// copy that reports `on_percent` (`0..=100`) and stops on `cancel`; the old
/// folder is removed only after a complete copy.
pub fn move_install(paths: &DataPaths, app_name: &str, new_base: &str, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<String> {
    move_with(paths, app_name, new_base, cancel, on_percent, false)
}

fn move_with(
    paths: &DataPaths,
    app_name: &str,
    new_base: &str,
    cancel: &Cancel,
    on_percent: &mut dyn FnMut(f64),
    force_copy: bool,
) -> Result<String> {
    let Some(game) = installed::get(paths, app_name) else { bail!(tr!("The game is not installed")) };
    let old = PathBuf::from(&game.path);
    if new_base.trim().is_empty() {
        bail!(tr!("Pick a folder to move the game to"));
    }
    if !old.is_dir() {
        bail!(tr!("The install folder {path} does not exist", path = game.path));
    }
    // Never act on a root or top-level folder by mistake (`/games`, `C:\Games`).
    if old.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2 {
        bail!(tr!("Refusing to move {path}", path = game.path));
    }
    let name = old.file_name().ok_or_else(|| anyhow!(tr!("Refusing to move {path}", path = game.path)))?;
    let target = Path::new(new_base).join(name);
    if target == old {
        return Ok(game.path);
    }
    if target.starts_with(&old) {
        bail!(tr!("A game cannot be moved into its own folder"));
    }
    if fs::read_dir(&target).is_ok_and(|mut d| d.next().is_some()) || (target.exists() && !target.is_dir()) {
        bail!(tr!("{path} already exists and is not empty", path = target.display()));
    }
    fs::create_dir_all(new_base).with_context(|| format!("creating {new_base}"))?;
    // An empty folder there: Windows renames nothing onto it.
    if target.is_dir() {
        let _ = fs::remove_dir(&target);
    }

    on_percent(0.);
    if force_copy || fs::rename(&old, &target).is_err() {
        copy_then_remove(&old, &target, cancel, on_percent)?;
    }
    on_percent(100.);

    let new_path = target.to_string_lossy().into_owned();
    if let Err(err) = installed::set_path(paths, app_name, &new_path) {
        // Put the folder back: nile still points at the old place.
        let _ = fs::rename(&target, &old);
        return Err(err);
    }
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    Ok(new_path)
}

/// Copies `old` to `target` and removes `old`. A failed or cancelled copy
/// removes what it wrote and leaves `old` as it was.
fn copy_then_remove(old: &Path, target: &Path, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<()> {
    let total = tree_size(old).max(1);
    let mut copied = 0u64;
    let mut last = -1.;
    let mut progress = |bytes: u64| {
        copied += bytes;
        let percent = (copied as f64 / total as f64 * 100.).min(99.);
        if percent - last >= 0.1 {
            last = percent;
            on_percent(percent);
        }
    };
    if let Err(err) = copy_tree(old, target, cancel, &mut progress) {
        if let Err(clean) = fs::remove_dir_all(target) {
            log::warn!("removing the partial copy {}: {clean}", target.display());
        }
        return Err(err);
    }
    if let Err(err) = fs::remove_dir_all(old) {
        // The copy is complete and nile will point at it.
        log::warn!("removing the old folder {}: {err}", old.display());
    }
    Ok(())
}

/// Bytes of the regular files under `dir`.
fn tree_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tree_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn copy_tree(src: &Path, dst: &Path, cancel: &Cancel, progress: &mut dyn FnMut(u64)) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to, cancel, progress)?;
        } else if kind.is_symlink() {
            copy_link(&from, &to, cancel, progress)?;
        } else {
            copy_file(&from, &to, cancel, progress)?;
        }
    }
    fs::set_permissions(dst, fs::metadata(src)?.permissions())?;
    Ok(())
}

/// The same link at `to`. Windows makes links only in Developer Mode or for
/// an administrator: there a copy of what it points to does instead.
fn copy_link(from: &Path, to: &Path, cancel: &Cancel, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let target = fs::read_link(from)?;
    #[cfg(windows)]
    {
        let dir = from.is_dir();
        let linked = if dir { std::os::windows::fs::symlink_dir(&target, to) } else { std::os::windows::fs::symlink_file(&target, to) };
        match linked {
            Ok(()) => Ok(()),
            Err(err) => {
                log::debug!("linking {}: {err}; copying it instead", to.display());
                if dir {
                    copy_tree(from, to, cancel, progress)
                } else {
                    copy_file(from, to, cancel, progress)
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (cancel, progress);
        heronite_platform::fs::symlink(&target, to).with_context(|| format!("linking {}", to.display()))
    }
}

/// Copies in chunks so a big file can be cancelled and shows progress.
fn copy_file(from: &Path, to: &Path, cancel: &Cancel, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let mut input = fs::File::open(from).with_context(|| format!("opening {}", from.display()))?;
    let mut output = fs::File::create(to).with_context(|| format!("creating {}", to.display()))?;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        if cancel.is_requested() {
            bail!(tr!("The move was cancelled"));
        }
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n]).with_context(|| format!("writing {}", to.display()))?;
        progress(n as u64);
    }
    output.flush()?;
    drop(output);
    fs::set_permissions(to, input.metadata()?.permissions())?;
    Ok(())
}

/// The game's folder was moved or renamed outside the app: checks that
/// `folder` holds the game (its `fuel.json`) and points nile's record at it.
/// Returns the new install path.
pub fn relocate(paths: &DataPaths, app_name: &str, folder: &str) -> Result<String> {
    if installed::get(paths, app_name).is_none() {
        bail!(tr!("The game is not installed"));
    }
    let folder = folder.trim().trim_end_matches(['/', '\\']);
    let dir = Path::new(folder);
    if !dir.is_dir() {
        bail!(tr!("The install folder {path} does not exist", path = folder));
    }
    if !dir.join("fuel.json").is_file() {
        bail!(tr!("That folder does not look like this game's install (fuel.json is missing)"));
    }
    installed::set_path(paths, app_name, folder)?;
    if let Err(err) = library::write_library(paths) {
        log::warn!("rebuilding the Amazon library: {err:#}");
    }
    Ok(folder.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::Env;
    use heronite_data::json_store;
    use serde_json::json;

    const ID: &str = "amzn1.adg.product.AAA";

    /// A game folder with a nested file, a symlink and an executable.
    fn setup(env: &Env) -> PathBuf {
        let game = env.root().join("old").join("Cool Game");
        fs::create_dir_all(game.join("data/sub")).unwrap();
        fs::write(game.join("data/sub/big.bin"), vec![7u8; 3 * (1 << 20) + 5]).unwrap();
        fs::write(game.join("run.exe"), "exe").unwrap();
        // Windows makes symlinks only in Developer Mode or as an administrator.
        #[cfg(unix)]
        std::os::unix::fs::symlink("run.exe", game.join("link")).unwrap();
        env.write_nile_file(
            "installed.json",
            &json!([
                {"id": "other", "version": "1", "path": "/o"},
                {"id": ID, "version": "v1", "path": game.to_string_lossy(), "size": 99, "custom": true}
            ])
            .to_string(),
        );
        env.write_nile_file("library.json", &json!([{"id": "e", "product": {"id": ID, "title": "Cool Game"}}]).to_string());
        game
    }

    fn check_moved(env: &Env, new_path: &str, old: &Path) {
        let new = Path::new(new_path);
        assert_eq!(new, env.root().join("new").join("Cool Game"));
        assert!(!old.exists());
        assert_eq!(fs::metadata(new.join("data/sub/big.bin")).unwrap().len(), 3 * (1 << 20) + 5);
        assert_eq!(fs::read_to_string(new.join("run.exe")).unwrap(), "exe");
        #[cfg(unix)]
        assert_eq!(fs::read_link(new.join("link")).unwrap(), Path::new("run.exe"));
        let doc = json_store::read_value(&installed::file(&env.paths)).unwrap();
        assert_eq!(doc[0]["path"], "/o");
        assert_eq!(doc[1]["path"], new_path);
        assert_eq!(doc[1]["size"], 99);
        assert_eq!(doc[1]["custom"], true);
        assert_eq!(doc[1]["version"], "v1");
        // The Collection's list follows.
        let game = library::game_info(&env.paths, ID).unwrap();
        assert_eq!(game["install"]["install_path"], new_path);
    }

    #[test]
    fn a_rename_moves_the_folder_and_rewrites_the_path() {
        let env = Env::new();
        let old = setup(&env);
        let new_base = env.root().join("new");
        let mut seen = Vec::new();
        let path = move_install(&env.paths, ID, &new_base.to_string_lossy(), &Cancel::new(), &mut |p| seen.push(p)).unwrap();
        check_moved(&env, &path, &old);
        assert_eq!(seen.first(), Some(&0.));
        assert_eq!(seen.last(), Some(&100.));
    }

    #[test]
    fn a_copy_reports_progress_and_removes_the_old_folder() {
        let env = Env::new();
        let old = setup(&env);
        let new_base = env.root().join("new");
        let mut seen = Vec::new();
        let path = move_with(&env.paths, ID, &new_base.to_string_lossy(), &Cancel::new(), &mut |p| seen.push(p), true).unwrap();
        check_moved(&env, &path, &old);
        assert!(seen.len() > 3, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&100.));
    }

    #[test]
    fn a_cancelled_copy_leaves_the_game_where_it_was() {
        let env = Env::new();
        let old = setup(&env);
        let cancel = Cancel::new();
        cancel.request();
        let new_base = env.root().join("new");
        let err = move_with(&env.paths, ID, &new_base.to_string_lossy(), &cancel, &mut |_| {}, true).unwrap_err().to_string();
        assert!(err.contains("cancelled"), "{err}");
        assert!(old.join("run.exe").exists());
        assert!(!new_base.join("Cool Game").exists());
        assert_eq!(installed::get(&env.paths, ID).unwrap().path, old.to_string_lossy());
    }

    #[test]
    fn bad_targets_are_refused() {
        let env = Env::new();
        let old = setup(&env);
        let base = env.root().join("new");
        let go = |base: &str| move_install(&env.paths, ID, base, &Cancel::new(), &mut |_| {});
        // Where it already is, into itself, onto a folder that has content, nothing picked, unknown game.
        assert!(go(&old.parent().unwrap().to_string_lossy()).is_ok_and(|p| p == old.to_string_lossy()));
        assert!(go(&old.to_string_lossy()).is_err());
        assert!(go(&old.join("data").to_string_lossy()).is_err());
        fs::create_dir_all(base.join("Cool Game")).unwrap();
        fs::write(base.join("Cool Game/x"), "x").unwrap();
        assert!(go(&base.to_string_lossy()).unwrap_err().to_string().contains("already exists"));
        assert!(go("  ").is_err());
        assert!(move_install(&env.paths, "nope", &base.to_string_lossy(), &Cancel::new(), &mut |_| {}).is_err());
        assert!(old.join("run.exe").exists());
        assert_eq!(installed::get(&env.paths, ID).unwrap().path, old.to_string_lossy());
    }

    #[test]
    fn a_top_level_folder_is_never_moved() {
        let env = Env::new();
        setup(&env);
        // A folder that exists right under the root (refused before anything is done).
        let top = if cfg!(windows) { r"C:\Windows" } else { "/tmp" };
        env.write_nile_file("installed.json", &json!([{"id": ID, "version": "v1", "path": top}]).to_string());
        let base = env.root().join("new");
        let err = move_install(&env.paths, ID, &base.to_string_lossy(), &Cancel::new(), &mut |_| {}).unwrap_err().to_string();
        assert!(err.contains("Refusing to move"), "{err}");
        assert!(!base.exists());
    }

    #[test]
    fn an_empty_folder_at_the_target_does_not_stop_a_rename() {
        let env = Env::new();
        let old = setup(&env);
        let new_base = env.root().join("new");
        fs::create_dir_all(new_base.join("Cool Game")).unwrap();
        let path = move_install(&env.paths, ID, &new_base.to_string_lossy(), &Cancel::new(), &mut |_| {}).unwrap();
        check_moved(&env, &path, &old);
    }

    #[test]
    fn relocate_points_nile_at_a_folder_moved_by_hand() {
        let env = Env::new();
        let game = setup(&env);
        let moved = env.root().join("disk2").join("Cool");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&game, &moved).unwrap();
        // Not the game until its fuel.json is there.
        assert!(relocate(&env.paths, ID, &moved.to_string_lossy()).is_err());
        fs::write(moved.join("fuel.json"), "{}").unwrap();
        assert!(relocate(&env.paths, "missing", &moved.to_string_lossy()).is_err());
        assert!(relocate(&env.paths, ID, "/nonexistent-dir").is_err());
        let now = relocate(&env.paths, ID, &format!("{}/", moved.display())).unwrap();
        #[cfg(windows)]
        assert_eq!(relocate(&env.paths, ID, &format!("{}\\", moved.display())).unwrap(), now);
        assert_eq!(now, moved.to_string_lossy());
        let doc = json_store::read_value(&installed::file(&env.paths)).unwrap();
        assert_eq!(doc[1]["path"], now.as_str());
        assert_eq!(doc[1]["custom"], true, "other keys are kept");
    }
}
