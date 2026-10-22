//! Folders in the games folders that nothing uses any more: what an
//! uninstall left behind (saves, configs, a half-removed install), or a game
//! removed from the Collection. A folder is kept out when anything known
//! lives in it: an installed game's files, a prefix, a ROM folder, an
//! emulator, a download in the queue, a Steam library, a game of Lutris,
//! Bottles or Faugus, or any path the settings name. Folders changed in the last hours
//! are left out too (an install may be writing to them).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use heronite_data::install_size::SizeCache;
use heronite_data::{game_config, json_store, CollectionGame, Library};
use serde_json::Value;

use crate::{games, Ctx, Group, Item, Kind, State};

/// A folder changed more recently than this is not offered.
const RECENT: Duration = Duration::from_secs(2 * 3600);

pub(crate) fn items(ctx: &Ctx, lib: &Library, listed: &[Item], walked: &SizeCache) -> Vec<Item> {
    let home = ctx.home_str();
    let global = game_config::load_global(&ctx.data_paths, &home);
    let mut roots: Vec<PathBuf> = [
        global.get("defaultInstallPath").and_then(Value::as_str).map(|s| game_config::expand_tilde(s, &home)),
        Some(game_config::default_install_dir(&home)),
        Some(format!("{home}/Games/Heroic")),
    ]
    .into_iter()
    .flatten()
    .map(|s| PathBuf::from(s.trim_end_matches('/')))
    .filter(|p| p.is_absolute() && p.is_dir())
    .collect();
    roots.sort();
    roots.dedup();

    let installed_paths: Vec<PathBuf> = lib.games.iter().filter(|g| g.installed).flat_map(|g| game_paths(g)).collect();
    let mut known = installed_paths.clone();
    known.extend(listed.iter().flat_map(|i| i.paths.iter().cloned().chain([i.folder.clone()])));
    known.push(ctx.data_paths.root.clone());
    // What Lutris, Bottles and Faugus keep is theirs, installed there or not.
    known.extend(heronite_data::launchers::managed_paths(&ctx.data_paths).into_iter().map(|m| m.path));
    known.extend(json_paths(&Value::Object(global)));
    for file in [ctx.data_paths.rom_scanners(), ctx.data_paths.emulators(), ctx.data_paths.download_manager()] {
        if let Ok(v) = json_store::read_value(&file) {
            known.extend(json_paths(&v));
        }
    }
    // The folder an uninstalled game was in names what is left of it.
    let former: HashMap<PathBuf, String> =
        lib.games.iter().filter(|g| !g.installed).flat_map(|g| game_paths(g).into_iter().map(|p| (p, g.title.clone()))).collect();

    let mut out = Vec::new();
    for root in roots {
        // A games folder that is itself (inside) a game.
        if installed_paths.iter().any(|p| root.starts_with(p)) {
            continue;
        }
        for dir in candidates(&root) {
            if known.iter().any(|p| p.starts_with(&dir)) {
                continue;
            }
            let title = former
                .iter()
                .find(|(p, _)| p.starts_with(&dir))
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let size = walked.get(&dir);
            let mut item = Item::new(Group::Leftovers, Kind::Leftover, State::Unused, title, dir);
            item.size = size;
            item.needs_walk = size.is_none();
            out.push(item);
        }
    }
    out
}

/// Every path a game's entry names: install folder, executable, ROMs.
fn game_paths(g: &CollectionGame) -> Vec<PathBuf> {
    let meta = g.meta.as_deref();
    [g.details.install_path.as_deref(), g.game.install.install_path.as_deref(), g.game.install.executable.as_deref()]
        .into_iter()
        .flatten()
        .chain(meta.and_then(|m| m.selected_rom_path.as_deref()))
        .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| p.is_absolute())
        .chain(games::install_dir(g))
        .collect()
}

/// The visible, non-empty folders of `root` not changed for a while.
fn candidates(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root) else { return Vec::new() };
    let now = SystemTime::now();
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        // Not followed: a link to a folder elsewhere is not this folder's.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| fs::read_dir(p).is_ok_and(|mut d| d.next().is_some()))
        .filter(|p| {
            let changed = fs::metadata(p).and_then(|m| m.modified()).unwrap_or(now);
            now.duration_since(changed).unwrap_or_default() >= RECENT
        })
        .collect();
    out.sort();
    out
}

/// Absolute paths among the string values of `v`, at any depth.
fn json_paths(v: &Value) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::String(s) if s.starts_with('/') => out.push(PathBuf::from(s.trim_end_matches('/'))),
            Value::Array(a) => stack.extend(a),
            Value::Object(o) => stack.extend(o.values()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_paths_at_any_depth() {
        let v = json!({"a": "/games/x/", "b": [{"c": "/roms"}, "relative", 3], "d": "https://x"});
        let mut p = json_paths(&v);
        p.sort();
        assert_eq!(p, [PathBuf::from("/games/x"), PathBuf::from("/roms")]);
    }

    #[test]
    fn candidates_are_old_non_empty_folders() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Old Game/saves")).unwrap();
        fs::create_dir_all(r.join("Empty")).unwrap();
        fs::create_dir_all(r.join(".hidden/x")).unwrap();
        fs::write(r.join("file.txt"), "x").unwrap();
        // Everything was just made: nothing is old enough yet.
        assert!(candidates(r).is_empty());
        let old = SystemTime::now() - Duration::from_secs(3 * 3600);
        for d in ["Old Game", "Empty", ".hidden"] {
            fs::File::open(r.join(d)).unwrap().set_modified(old).unwrap();
        }
        assert_eq!(candidates(r), [r.join("Old Game")]);
    }
}
