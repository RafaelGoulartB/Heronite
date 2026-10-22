//! Bethesda's `Plugins.txt`: the plugins of the enabled mods are listed in
//! the Wine prefix while the game runs, and the user's own file is put back
//! afterwards.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;

use crate::fsutil::{entries, find_ci, remove_any};
use crate::games::PluginsSpec;
use crate::model::ModList;
use crate::paths::GameDirs;

const HEADER: &str = "# This file is used by the game to keep track of your downloaded content.";
const BACKUP_SUFFIX: &str = ".heronite-backup";
/// Left when there was no `Plugins.txt` before ours, so a crash can be undone.
const NEW_SUFFIX: &str = ".heronite-new";

/// Puts the user's `Plugins.txt` back when dropped.
#[derive(Debug)]
pub struct PluginsGuard {
    file: PathBuf,
    backup: PathBuf,
    marker: PathBuf,
    had_original: bool,
    count: usize,
}

impl PluginsGuard {
    /// Plugins of the mods that were listed.
    pub fn count(&self) -> usize {
        self.count
    }
}

impl Drop for PluginsGuard {
    fn drop(&mut self) {
        let result = if self.had_original {
            fs::rename(&self.backup, &self.file)
        } else {
            remove_any(&self.file).and_then(|_| remove_any(&self.marker))
        };
        match result {
            Ok(()) => log::info!("mods: Plugins.txt restored"),
            Err(e) => log::warn!("mods: could not restore {}: {e}", self.file.display()),
        }
    }
}

/// Writes the plugin list of the enabled mods in `list`. `None` when the
/// prefix has no user folder yet (a first launch makes it; the next launch
/// writes the list). The folder is found as `drive_c/users/<user>/AppData/
/// Local/<spec folder>` of `prefix` (or its `pfx`).
pub fn write(prefix: &Path, spec: &PluginsSpec, dirs: &GameDirs, list: &ModList, game_dir: &Path) -> Result<Option<PluginsGuard>, String> {
    let Some(folder) = app_data_folder(prefix, spec)? else { return Ok(None) };
    let file = folder.join("Plugins.txt");
    let backup = folder.join(format!("Plugins.txt{BACKUP_SUFFIX}"));
    let marker = folder.join(format!("Plugins.txt{NEW_SUFFIX}"));
    recover(&file, &backup, &marker)?;

    let original = fs::read(&file).ok();
    let ordered = ordered_plugins(spec, list);
    let content = render(spec, list, &ordered, original.as_deref().unwrap_or_default());
    let err = |p: &Path, e: std::io::Error| tr!("Could not write {path}: {error}", path = p.display(), error = e);
    let guard = PluginsGuard {
        file: file.clone(),
        backup: backup.clone(),
        marker: marker.clone(),
        had_original: original.is_some(),
        count: ordered.len(),
    };
    match original {
        Some(_) => fs::rename(&file, &backup).map_err(|e| err(&backup, e))?,
        None => fs::write(&marker, b"Heronite wrote Plugins.txt; this marks it as not the user's.\n").map_err(|e| err(&marker, e))?,
    }
    write_atomic(&file, &content, Some(0o644)).map_err(|e| err(&file, e))?;
    if !spec.star_format {
        order_by_time(dirs, list, game_dir, &ordered);
    }
    log::info!("mods: Plugins.txt written with {} plugins", ordered.len());
    Ok(Some(guard))
}

/// A crash while the game ran leaves our file (and the user's as a backup):
/// the user's comes back before anything else is read.
fn recover(file: &Path, backup: &Path, marker: &Path) -> Result<(), String> {
    if backup.exists() {
        log::warn!("mods: restoring the Plugins.txt a crash left behind");
        fs::rename(backup, file).map_err(|e| tr!("Could not write {path}: {error}", path = file.display(), error = e))?;
    } else if marker.exists() {
        let _ = remove_any(file);
    }
    let _ = remove_any(marker);
    Ok(())
}

/// The folder for the game's `Plugins.txt`, made if no build of the game has
/// made it yet.
fn app_data_folder(prefix: &Path, spec: &PluginsSpec) -> Result<Option<PathBuf>, String> {
    let users = [prefix.join("drive_c/users"), prefix.join("pfx/drive_c/users")].into_iter().find(|p| p.is_dir());
    let Some(users) = users else { return Ok(None) };
    let homes: Vec<(String, PathBuf)> = entries(&users)
        .into_iter()
        .filter(|(n, is_dir)| *is_dir && !n.eq_ignore_ascii_case("public"))
        .map(|(n, _)| (n.clone(), users.join(n).join("AppData/Local")))
        .collect();
    if homes.is_empty() {
        return Ok(None);
    }
    for name in spec.appdata_folders {
        if let Some((_, local)) = homes.iter().find(|(_, local)| local.join(name).is_dir()) {
            return Ok(Some(local.join(name)));
        }
    }
    let (_, local) = homes.iter().find(|(n, _)| n == "steamuser").unwrap_or(&homes[0]);
    let folder = local.join(spec.appdata_folders.first().copied().unwrap_or("Game"));
    fs::create_dir_all(&folder).map_err(|e| tr!("Could not write {path}: {error}", path = folder.display(), error = e))?;
    Ok(Some(folder))
}

/// A plugin of an enabled mod, with the mod it comes from.
struct Listed<'a> {
    name: &'a str,
    mod_id: &'a str,
}

/// The plugins of the enabled mods in load order, masters first; the ones
/// the game loads by itself and repeats left out.
fn ordered_plugins<'a>(spec: &PluginsSpec, list: &'a ModList) -> Vec<Listed<'a>> {
    let implicit: HashSet<String> = spec.implicit.iter().map(|n| n.to_lowercase()).collect();
    let mut seen = HashSet::new();
    let mut out: Vec<Listed> = list
        .enabled()
        .flat_map(|m| m.plugins.iter().map(move |p| Listed { name: p.as_str(), mod_id: m.id.as_str() }))
        .filter(|p| !implicit.contains(&p.name.to_lowercase()) && seen.insert(p.name.to_lowercase()))
        .collect();
    out.sort_by_key(|p| !is_master(p.name));
    out
}

fn is_master(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".esm") || lower.ends_with(".esl")
}

fn render(spec: &PluginsSpec, list: &ModList, ordered: &[Listed], original: &[u8]) -> Vec<u8> {
    let ours: HashSet<String> = ordered.iter().map(|p| p.name.to_lowercase()).collect();
    let implicit: HashSet<String> = spec.implicit.iter().map(|n| n.to_lowercase()).collect();
    let disabled: HashSet<String> = list.disabled_plugins.iter().map(|n| n.to_lowercase()).collect();
    let mut out: Vec<u8> = Vec::new();
    let mut line = |bytes: &[u8]| {
        out.extend_from_slice(bytes);
        out.extend_from_slice(b"\r\n");
    };
    line(HEADER.as_bytes());
    // The user's own plugins (Creation Club, tools) keep their place.
    for raw in original.split(|&b| b == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let name = String::from_utf8_lossy(raw.strip_prefix(b"*").unwrap_or(raw)).trim().to_lowercase();
        if name.is_empty() || name.starts_with('#') || ours.contains(&name) || implicit.contains(&name) {
            continue;
        }
        line(raw);
    }
    for p in ordered {
        let off = disabled.contains(&p.name.to_lowercase());
        match (spec.star_format, off) {
            (true, false) => line(format!("*{}", p.name).as_bytes()),
            (true, true) => line(p.name.as_bytes()),
            (false, false) => line(p.name.as_bytes()),
            (false, true) => {}
        }
    }
    out
}

/// Games without the star format load plugins by file time: the mods' files
/// get times after the game's own, in load order.
fn order_by_time(dirs: &GameDirs, list: &ModList, game_dir: &Path, ordered: &[Listed]) {
    let newest_own = plugin_dir(game_dir)
        .and_then(|d| {
            entries(&d)
                .into_iter()
                .filter(|(n, is_dir)| !is_dir && is_plugin(n))
                .filter_map(|(n, _)| d.join(n).metadata().ok()?.modified().ok())
                .max()
        })
        .unwrap_or_else(|| SystemTime::now() - Duration::from_secs(86_400));
    let disabled: HashSet<String> = list.disabled_plugins.iter().map(|n| n.to_lowercase()).collect();
    let mut n = 0u64;
    for p in ordered.iter().filter(|p| !disabled.contains(&p.name.to_lowercase())) {
        n += 1;
        let Some(dir) = plugin_dir(&dirs.mod_dir(p.mod_id)) else { continue };
        let time = newest_own + Duration::from_secs(n);
        if let Err(e) = fs::File::open(dir.join(p.name)).and_then(|f| f.set_modified(time)) {
            log::warn!("mods: could not order {}: {e}", p.name);
        }
    }
}

fn plugin_dir(base: &Path) -> Option<PathBuf> {
    find_ci(base, "Data").filter(|(_, is_dir)| *is_dir).map(|(n, _)| base.join(n))
}

fn is_plugin(name: &str) -> bool {
    let l = name.to_lowercase();
    l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::find;
    use crate::model::ModEntry;
    use crate::paths::ModsRoot;

    fn spec(steam_id: &str) -> PluginsSpec {
        find(Some(steam_id), "").unwrap().plugins.unwrap()
    }

    fn mod_with(id: &str, plugins: &[&str], enabled: bool) -> ModEntry {
        ModEntry { id: id.into(), enabled, plugins: plugins.iter().map(|p| p.to_string()).collect(), ..Default::default() }
    }

    struct Env {
        _t: tempfile::TempDir,
        prefix: PathBuf,
        game: PathBuf,
        dirs: GameDirs,
    }

    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let prefix = t.path().join("prefix");
        fs::create_dir_all(prefix.join("drive_c/users/Public")).unwrap();
        let game = t.path().join("game");
        fs::create_dir_all(game.join("Data")).unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        Env { _t: t, prefix, game, dirs }
    }

    fn local(e: &Env, user: &str, folder: &str) -> PathBuf {
        let p = e.prefix.join("drive_c/users").join(user).join("AppData/Local").join(folder);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn text(path: &Path) -> String {
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn writes_the_enabled_plugins_in_order_and_restores_the_users_file() {
        let e = env();
        let folder = local(&e, "steamuser", "Skyrim Special Edition");
        let original = "*Update.esm\r\n*ForeignCC.esl\r\nOld.esp\r\n*A.esp\r\n";
        fs::write(folder.join("Plugins.txt"), original).unwrap();
        let mut list = ModList::new("g", "Skyrim");
        list.mods = vec![
            mod_with("a", &["A.esp", "M.esm"], true),
            mod_with("off", &["Nope.esp"], false),
            mod_with("b", &["B.esp", "B2.esl", "Skyrim.esm"], true),
        ];
        list.disabled_plugins = vec!["b2.ESL".into()];
        let guard = write(&e.prefix, &spec("489830"), &e.dirs, &list, &e.game).unwrap().unwrap();
        let expected = format!("{HEADER}\r\n*ForeignCC.esl\r\nOld.esp\r\n*M.esm\r\nB2.esl\r\n*A.esp\r\n*B.esp\r\n");
        assert_eq!(text(&folder.join("Plugins.txt")), expected);
        drop(guard);
        assert_eq!(text(&folder.join("Plugins.txt")), original);
        assert!(!folder.join("Plugins.txt.heronite-backup").exists());
    }

    #[test]
    fn without_an_original_the_file_is_removed_afterwards() {
        let e = env();
        fs::create_dir_all(e.prefix.join("drive_c/users/steamuser")).unwrap();
        let mut list = ModList::new("g", "Skyrim");
        list.mods = vec![mod_with("a", &["A.esp"], true)];
        let guard = write(&e.prefix, &spec("489830"), &e.dirs, &list, &e.game).unwrap().unwrap();
        let folder = e.prefix.join("drive_c/users/steamuser/AppData/Local/Skyrim Special Edition");
        assert!(folder.join("Plugins.txt").exists(), "the folder is made under steamuser");
        drop(guard);
        assert!(!folder.join("Plugins.txt").exists() && !folder.join("Plugins.txt.heronite-new").exists());
    }

    #[test]
    fn picks_the_existing_folder_of_any_store_and_skips_public() {
        let e = env();
        local(&e, "Public", "Skyrim Special Edition");
        let gog = local(&e, "bob", "Skyrim Special Edition GOG");
        let mut list = ModList::new("g", "Skyrim");
        list.mods = vec![mod_with("a", &["A.esp"], true)];
        let _g = write(&e.prefix, &spec("489830"), &e.dirs, &list, &e.game).unwrap().unwrap();
        assert!(gog.join("Plugins.txt").exists());
        assert!(!e.prefix.join("drive_c/users/Public/AppData/Local/Skyrim Special Edition/Plugins.txt").exists());
    }

    #[test]
    fn a_crash_is_undone_by_the_next_run() {
        let e = env();
        let folder = local(&e, "steamuser", "Skyrim Special Edition");
        fs::write(folder.join("Plugins.txt"), "*Mine.esp\r\n").unwrap();
        let mut list = ModList::new("g", "Skyrim");
        list.mods = vec![mod_with("a", &["A.esp"], true)];
        std::mem::forget(write(&e.prefix, &spec("489830"), &e.dirs, &list, &e.game).unwrap());
        let g = write(&e.prefix, &spec("489830"), &e.dirs, &list, &e.game).unwrap().unwrap();
        assert!(text(&folder.join("Plugins.txt")).contains("*Mine.esp\r\n*A.esp"));
        assert_eq!(text(&folder.join("Plugins.txt")).matches("A.esp").count(), 1);
        drop(g);
        assert_eq!(text(&folder.join("Plugins.txt")), "*Mine.esp\r\n");
    }

    #[test]
    fn no_users_means_nothing_to_write() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        let list = ModList::new("g", "Skyrim");
        assert!(write(&t.path().join("none"), &spec("489830"), &dirs, &list, t.path()).unwrap().is_none());
    }

    #[test]
    fn old_games_list_only_active_plugins_and_get_file_times() {
        let e = env();
        let folder = local(&e, "steamuser", "Oblivion");
        fs::write(e.game.join("Data/Oblivion.esm"), "g").unwrap();
        let mut list = ModList::new("g", "Oblivion");
        list.mods = vec![mod_with("a", &["Z.esp", "Oblivion.esm"], true), mod_with("b", &["A.esp", "Off.esp"], true)];
        list.disabled_plugins = vec!["off.esp".into()];
        for (id, file) in [("a", "Z.esp"), ("b", "A.esp"), ("b", "Off.esp")] {
            let p = e.dirs.mod_dir(id).join("data").join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        let guard = write(&e.prefix, &spec("22330"), &e.dirs, &list, &e.game).unwrap().unwrap();
        assert_eq!(text(&folder.join("Plugins.txt")), format!("{HEADER}\r\nZ.esp\r\nA.esp\r\n"));
        let mtime = |id: &str, f: &str| e.dirs.mod_dir(id).join("data").join(f).metadata().unwrap().modified().unwrap();
        let own = e.game.join("Data/Oblivion.esm").metadata().unwrap().modified().unwrap();
        assert!(own < mtime("a", "Z.esp") && mtime("a", "Z.esp") < mtime("b", "A.esp"));
        drop(guard);
    }
}
