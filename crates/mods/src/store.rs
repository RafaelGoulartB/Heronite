//! `mods.json`: reading and writing the list, and the edits that touch a
//! mod's folder (remove, forget a whole game).

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use heronite_data::temps::write_atomic;
use heronite_i18n::tr;

use crate::fsutil::remove_any;
use crate::model::{ModList, FORMAT};
use crate::paths::{GameDirs, ModsRoot};

/// The list of a game. A missing file gives a new list; a file that does not
/// parse is kept as `mods.json.corrupt-<unix time>` and a new list is used.
pub fn load(dirs: &GameDirs) -> ModList {
    let path = dirs.list_file();
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("mods: could not read {}: {e}", path.display());
            }
            return ModList::new("", "");
        }
    };
    match serde_json::from_slice::<ModList>(&bytes) {
        Ok(mut list) => {
            if list.format == 0 {
                list.format = FORMAT;
            }
            list
        }
        Err(e) => {
            let copy = path.with_file_name(format!("mods.json.corrupt-{}", unix_now()));
            log::warn!("mods: {} is not valid ({e}); a copy is kept as {}", path.display(), copy.display());
            let _ = fs::copy(&path, &copy);
            ModList::new("", "")
        }
    }
}

/// [`load`] for a game that may have no list yet: the new list carries the
/// game's key and title.
pub fn load_or_new(root: &ModsRoot, game_key: &str, title: &str) -> ModList {
    let mut list = load(&root.game(game_key));
    list.game_key = game_key.to_string();
    if !title.is_empty() {
        list.title = title.to_string();
    }
    list
}

pub fn save(dirs: &GameDirs, list: &ModList) -> io::Result<()> {
    fs::create_dir_all(dirs.root())?;
    let mut json = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
    json.push(b'\n');
    write_atomic(&dirs.list_file(), &json, Some(0o644))
}

/// Keys (folder names) of the games that have a `mods.json`, sorted.
pub fn game_keys(root: &ModsRoot) -> Vec<String> {
    let mut keys: Vec<String> = fs::read_dir(root.path())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("mods.json").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    keys.sort();
    keys
}

/// An id for a mod called `base` that no entry of the list has: lower case
/// letters, digits and `-`, then `-2`, `-3`... when taken.
pub fn unique_id(list: &ModList, base: &str) -> String {
    let mut slug = String::new();
    for c in base.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "mod" } else { slug };
    let slug: String = slug.chars().take(60).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if list.get(&slug).is_none() {
        return slug;
    }
    (2..).map(|n| format!("{slug}-{n}")).find(|id| list.get(id).is_none()).unwrap_or(slug)
}

/// Puts the mod at `to_index` of the load order (clamped to the end).
pub fn move_mod(list: &mut ModList, id: &str, to_index: usize) {
    let Some(from) = list.mods.iter().position(|m| m.id == id) else { return };
    let entry = list.mods.remove(from);
    list.mods.insert(to_index.min(list.mods.len()), entry);
}

pub fn rename_mod(list: &mut ModList, id: &str, name: &str) {
    let name = name.trim();
    if let (Some(m), false) = (list.get_mut(id), name.is_empty()) {
        m.name = name.to_string();
    }
}

/// Removes the mod: its folder, its archive in `downloads/` when no other mod
/// uses it, and its entry; saves the list. Only paths under the game's folder
/// of the mods root are touched.
pub fn remove_mod(dirs: &GameDirs, list: &mut ModList, id: &str) -> io::Result<()> {
    let Some(at) = list.mods.iter().position(|m| m.id == id) else { return Ok(()) };
    remove_any(&dirs.mod_dir(id))?;
    let entry = list.mods.remove(at);
    if let Some(archive) = entry.archive.as_deref().filter(|a| is_plain_name(a)) {
        if !list.mods.iter().any(|m| m.archive.as_deref() == Some(archive)) {
            let _ = remove_any(&dirs.downloads().join(archive));
        }
    }
    save(dirs, list)
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Deletes everything the mods root keeps for the game. Refused while the
/// game's folder has our overlay mounted over it (`game_dir`).
pub fn forget_game(root: &ModsRoot, game_key: &str, game_dir: Option<&Path>) -> Result<(), String> {
    if game_dir.is_some_and(crate::deploy::is_mounted) {
        return Err(tr!("The game is running with mods; close it first").to_string());
    }
    let dirs = root.game(game_key);
    let dir = dirs.root();
    // The folder is `<root>/<safe name>`: never the root, never a link out.
    if dir.parent() != Some(root.path()) {
        return Err(tr!("Not a valid game key: {key}", key = game_key));
    }
    remove_any(dir).map_err(|e| tr!("Could not delete {path}: {error}", path = dir.display(), error = e))
}

/// Seconds since 1970.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The current time as RFC 3339 in UTC (`2026-10-21T14:43:09Z`).
pub fn now_rfc3339() -> String {
    rfc3339(unix_now())
}

fn rfc3339(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModEntry;

    fn entry(id: &str) -> ModEntry {
        ModEntry { id: id.into(), name: id.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn round_trips_and_lists_games() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path());
        assert!(game_keys(&root).is_empty());
        let mut list = load_or_new(&root, "legendary_Quail", "Skyrim");
        assert_eq!((list.game_key.as_str(), list.format, list.active), ("legendary_Quail", FORMAT, true));
        list.mods.push(entry("a"));
        save(&root.game("legendary_Quail"), &list).unwrap();
        assert_eq!(load(&root.game("legendary_Quail")), list);
        assert_eq!(game_keys(&root), vec!["legendary_Quail".to_string()]);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        fs::create_dir_all(dirs.root()).unwrap();
        fs::write(dirs.list_file(), b"{ nope").unwrap();
        let list = load(&dirs);
        assert!(list.mods.is_empty() && list.format == FORMAT);
        let kept: Vec<_> =
            fs::read_dir(dirs.root()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("corrupt-")).collect();
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn ids_are_slugs_and_unique() {
        let mut list = ModList::new("g", "G");
        assert_eq!(unique_id(&list, "SkyUI 5.2 SE!"), "skyui-5-2-se");
        assert_eq!(unique_id(&list, "###"), "mod");
        list.mods.push(entry("skyui"));
        list.mods.push(entry("skyui-2"));
        assert_eq!(unique_id(&list, "SkyUI"), "skyui-3");
    }

    #[test]
    fn reorders_and_renames() {
        let mut list = ModList::new("g", "G");
        for id in ["a", "b", "c"] {
            list.mods.push(entry(id));
        }
        move_mod(&mut list, "a", 2);
        assert_eq!(list.mods.iter().map(|m| m.id.as_str()).collect::<String>(), "bca");
        move_mod(&mut list, "a", 0);
        move_mod(&mut list, "c", 99);
        assert_eq!(list.mods.iter().map(|m| m.id.as_str()).collect::<String>(), "abc");
        rename_mod(&mut list, "b", "  Better  ");
        rename_mod(&mut list, "c", "  ");
        assert_eq!((list.mods[1].name.as_str(), list.mods[2].name.as_str()), ("Better", "c"));
    }

    #[test]
    fn removing_keeps_a_shared_archive_and_never_leaves_the_game_folder() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let outside = t.path().join("outside.zip");
        fs::write(&outside, b"x").unwrap();
        fs::create_dir_all(dirs.downloads()).unwrap();
        fs::write(dirs.downloads().join("pack.zip"), b"x").unwrap();
        let mut list = ModList::new("g", "G");
        for (id, archive) in [("a", "pack.zip"), ("b", "pack.zip"), ("c", "../../outside.zip")] {
            fs::create_dir_all(dirs.mod_dir(id)).unwrap();
            fs::write(dirs.mod_dir(id).join("f.txt"), b"x").unwrap();
            list.mods.push(ModEntry { archive: Some(archive.into()), ..entry(id) });
        }
        remove_mod(&dirs, &mut list, "a").unwrap();
        assert!(!dirs.mod_dir("a").exists() && dirs.downloads().join("pack.zip").exists());
        remove_mod(&dirs, &mut list, "b").unwrap();
        assert!(!dirs.downloads().join("pack.zip").exists());
        remove_mod(&dirs, &mut list, "c").unwrap();
        assert!(outside.exists());
        assert!(list.mods.is_empty());
        assert!(load(&dirs).mods.is_empty());
    }

    #[test]
    fn forgetting_a_game_deletes_its_folder() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path());
        let dirs = root.game("g");
        fs::create_dir_all(dirs.mod_dir("a")).unwrap();
        fs::write(dirs.mod_dir("a").join("f"), b"x").unwrap();
        std::os::unix::fs::symlink(t.path(), dirs.root().join("link-out")).unwrap();
        save(&dirs, &ModList::new("g", "G")).unwrap();
        forget_game(&root, "g", None).unwrap();
        assert!(!dirs.root().exists() && t.path().exists());
    }

    #[test]
    fn dates_are_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_782_052_989), "2026-06-21T14:43:09Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert!(now_rfc3339().starts_with("20"));
    }
}
