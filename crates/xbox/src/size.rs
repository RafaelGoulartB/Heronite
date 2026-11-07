//! How much room each Xbox game takes, counted as the Xbox app counts it,
//! kept on its Collection entry (`install.install_size`) so the Details panel,
//! the sort by size and the Storage page have it without walking the folder.
//!
//! Windows tells nothing about a package's size: the folder is walked. A game
//! made with the Game Development Kit lives in `<drive>\XboxGames\<game>`,
//! whose `Content` folder is the package; next to it are the shaders and the
//! package's own files (`*.xvs`, `*.xvi`, …), which the Xbox app counts too
//! ([`measure_dir`]). Other packages are their folder (under
//! `WindowsApps`, often a link to another drive).
//!
//! A walk is redone only when the package changed (an update gives it a new
//! full name, [`PACKAGE_KEY`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_data::install_size::{folder_bytes, walkable};
use heronite_data::xbox::PACKAGE_KEY;
use heronite_data::{json_store, DataPaths};
use serde_json::{json, Value};

use crate::collection::app_name;
use crate::XboxGame;

/// Folders a GDK game's folder holds besides its package (`Content`).
const GDK_DIRS: [&str; 2] = ["Content", "Shaders"];

/// The folder to measure for a package whose files are in `folder`: the
/// game's whole folder when `folder` is (or links to) the `Content` of a GDK
/// game (the folder above holds no other folder than [`GDK_DIRS`]), else
/// `folder`. Windows names a GDK package's folder under `WindowsApps`, a
/// link to `<drive>\XboxGames\<game>\Content`: the link is followed.
pub fn measure_dir(folder: &Path) -> PathBuf {
    let real = real_path(folder);
    let is_content = real.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("Content"));
    let Some(parent) = real.parent().filter(|_| is_content) else { return folder.to_path_buf() };
    let Ok(entries) = std::fs::read_dir(parent) else { return folder.to_path_buf() };
    let only_the_game = entries.flatten().all(|e| {
        let dir = e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink());
        !dir || e.file_name().to_str().is_some_and(|n| GDK_DIRS.iter().any(|g| g.eq_ignore_ascii_case(n)))
    });
    if only_the_game {
        parent.to_path_buf()
    } else {
        folder.to_path_buf()
    }
}

/// Where `folder` really is, links followed (`\\?\` taken off on Windows);
/// `folder` itself when that cannot be told.
fn real_path(folder: &Path) -> PathBuf {
    let Ok(real) = std::fs::canonicalize(folder) else { return folder.to_path_buf() };
    #[cfg(windows)]
    {
        let text = real.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\").filter(|r| !r.starts_with(r"UNC\")) {
            return PathBuf::from(rest);
        }
    }
    real
}

/// Measures the games of `games` (installed now) whose Collection entry has
/// no size, or one measured for another version of the package, and writes
/// it there as `format` writes sizes. Blocking: walks folders, possibly
/// large ones. Stops between games, or inside a walk, as soon as `stop`
/// says so. Returns how many entries got a new size.
pub fn measure(paths: &DataPaths, games: &[XboxGame], format: &dyn Fn(u64) -> String, stop: &dyn Fn() -> bool) -> Result<usize> {
    let home = heronite_platform::dirs::home();
    let doc = json_store::read_value(&paths.sideload_library())?;
    let mut done = HashSet::new();
    let mut measured = 0;
    for game in games {
        if stop() {
            break;
        }
        let app = app_name(&game.family);
        if !done.insert(app.clone()) || !needs_measure(&doc, &app, &game.package) {
            continue;
        }
        let dir = measure_dir(&game.folder);
        if !walkable(&dir, home.as_deref()) {
            continue;
        }
        // Nothing readable is no size: the entry keeps what it had.
        let Some(bytes) = folder_bytes(&dir, stop).filter(|b| *b > 0) else { continue };
        let size = format(bytes);
        let wrote = json_store::update_value_if_changed(&paths.sideload_library(), |doc| {
            let Some(entry) = entry_mut(doc, &app) else { return Ok(false) };
            let install = entry.entry("install").or_insert_with(|| json!({}));
            if !install.is_object() {
                *install = json!({});
            }
            install["install_size"] = json!(size);
            entry.insert(PACKAGE_KEY.into(), json!(game.package));
            Ok(true)
        })?;
        if wrote {
            log::debug!("{} takes {size} ({})", game.title, dir.display());
            measured += 1;
        }
    }
    Ok(measured)
}

/// Whether the entry `app` of the sideload library `doc` is there and has no
/// size measured for `package`.
fn needs_measure(doc: &Value, app: &str, package: &str) -> bool {
    let Some(entry) = entries(doc).find(|e| e.get("app_name").and_then(Value::as_str) == Some(app)) else { return false };
    let size = entry.get("install").and_then(|i| i.get("install_size")).and_then(Value::as_str).unwrap_or("");
    let sized = !size.trim().is_empty() && size.trim() != "0";
    !sized || entry.get(PACKAGE_KEY).and_then(Value::as_str) != Some(package)
}

fn entries(doc: &Value) -> impl Iterator<Item = &Value> {
    doc.get("games").and_then(Value::as_array).into_iter().flatten()
}

fn entry_mut<'a>(doc: &'a mut Value, app: &str) -> Option<&'a mut serde_json::Map<String, Value>> {
    doc.get_mut("games")?
        .as_array_mut()?
        .iter_mut()
        .filter_map(Value::as_object_mut)
        .find(|e| e.get("app_name").and_then(Value::as_str) == Some(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::sync;
    use heronite_data::Library;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    fn game(family: &str, folder: &Path, package: &str) -> XboxGame {
        XboxGame {
            family: family.into(),
            app_id: "Game".into(),
            title: family.into(),
            folder: folder.to_path_buf(),
            store_id: None,
            title_id: None,
            package: package.into(),
        }
    }

    fn bytes(n: u64) -> String {
        format!("{n} B")
    }

    #[test]
    fn a_gdk_game_counts_its_whole_folder() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("XboxGames").join("Avowed");
        write(&game.join("Content").join("Avowed.exe"), 10);
        write(&game.join("Shaders").join("cache.bin"), 5);
        write(&game.join("A069FE95.xvs"), 1);
        assert_eq!(measure_dir(&game.join("Content")), game);

        // Another folder next to Content: not one game's folder.
        std::fs::create_dir_all(game.join("Other Game")).unwrap();
        assert_eq!(measure_dir(&game.join("Content")), game.join("Content"));

        // A package folder of WindowsApps is the package.
        let uwp = dir.path().join("WindowsApps").join("Studio.Game_1.0.0.0_x64__abc");
        std::fs::create_dir_all(&uwp).unwrap();
        assert_eq!(measure_dir(&uwp), uwp);
        assert_eq!(measure_dir(&dir.path().join("missing").join("Content")), dir.path().join("missing").join("Content"));
    }

    #[test]
    fn a_link_to_a_gdk_games_content_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("XboxGames").join("Avowed");
        write(&game.join("Content").join("Avowed.exe"), 10);
        write(&game.join("Shaders").join("cache.bin"), 5);
        let link = dir.path().join("WindowsApps").join("Studio.Avowed_1.0.0.0_x64__abc");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(game.join("Content"), &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(game.join("Content"), &link);
        // Windows lets only some accounts make links: nothing to check then.
        if made.is_err() {
            return;
        }
        let real = std::fs::canonicalize(&game).unwrap();
        assert_eq!(std::fs::canonicalize(measure_dir(&link)).unwrap(), real);
    }

    #[test]
    fn measures_once_per_package_version() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let root = dir.path().join("XboxGames").join("Forza");
        write(&root.join("Content").join("game.exe"), 100);
        write(&root.join("Shaders").join("s.bin"), 20);
        write(&root.join("pkg.xvi"), 3);
        let empty = dir.path().join("XboxGames").join("Empty").join("Content");
        std::fs::create_dir_all(&empty).unwrap();
        let v1 = [game("Studio.Forza_abc", &root.join("Content"), "Studio.Forza_1.0.0.0_x64__abc"), game("Studio.Empty_abc", &empty, "E")];
        sync(&paths, &v1).unwrap();

        assert_eq!(measure(&paths, &v1, &bytes, &|| false).unwrap(), 1, "an empty folder gets no size");
        let lib = Library::load(&paths).unwrap();
        let forza = lib.get("sideload_xbox_studio_forza_abc").unwrap();
        assert_eq!(forza.details.install_size.as_deref(), Some("123 B"));
        assert!(lib.get("sideload_xbox_studio_empty_abc").unwrap().details.install_size.is_none());

        // Same package: nothing walked, nothing written; a sync keeps the size.
        let before = json_store::mtime(&paths.sideload_library());
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(measure(&paths, &v1, &bytes, &|| false).unwrap(), 0);
        sync(&paths, &v1).unwrap();
        assert_eq!(json_store::mtime(&paths.sideload_library()), before);

        // An update: measured again.
        write(&root.join("Content").join("patch.pak"), 1000);
        let v2 = [game("Studio.Forza_abc", &root.join("Content"), "Studio.Forza_1.1.0.0_x64__abc")];
        assert_eq!(measure(&paths, &v2, &bytes, &|| false).unwrap(), 1);
        let lib = Library::load(&paths).unwrap();
        assert_eq!(lib.get("sideload_xbox_studio_forza_abc").unwrap().details.install_size.as_deref(), Some("1123 B"));
    }

    #[test]
    fn stops_when_asked_and_leaves_out_games_not_in_the_collection() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let folder = dir.path().join("WindowsApps").join("Studio.Game_1.0.0.0_x64__abc");
        write(&folder.join("game.exe"), 10);
        let games = [game("Studio.Game_abc", &folder, "Studio.Game_1.0.0.0_x64__abc")];
        assert_eq!(measure(&paths, &games, &bytes, &|| false).unwrap(), 0, "no entry yet");
        sync(&paths, &games).unwrap();
        assert_eq!(measure(&paths, &games, &bytes, &|| true).unwrap(), 0);
        assert_eq!(measure(&paths, &games, &bytes, &|| false).unwrap(), 1);
    }
}
