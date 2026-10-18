//! What Steam keeps per game in each library's `steamapps`: the Proton
//! prefix (`compatdata/<id>`) and the shader cache (`shadercache/<id>`).
//! Uninstalling a game leaves both behind. Non-Steam shortcuts (ids from
//! 2^31 up) are left out: their entries in Steam are not read here.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use heronite_data::steam_owned::SteamOwnedFile;
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;
use heronite_launch::steam::{read_manifests, SteamLocator};

use crate::{Group, Item, Kind, State};

pub(crate) struct Found {
    /// Prefixes and shader caches of games no longer installed.
    pub leftovers: Vec<Item>,
    /// Shader caches of the installed games (one cache item).
    pub shader_caches: Vec<PathBuf>,
}

/// A Steam game's id, not a shortcut's or a folder of something else.
fn app_id(name: &str) -> Option<u32> {
    let id: u32 = name.parse().ok().filter(|_| name.bytes().all(|b| b.is_ascii_digit()))?;
    (id > 0 && id < 1 << 31).then_some(id)
}

pub(crate) fn items(lib: &Library, data_paths: &DataPaths, steam: &SteamLocator) -> Found {
    let mut found = Found { leftovers: Vec::new(), shader_caches: Vec::new() };
    let roots = steam.library_roots();
    if roots.is_empty() {
        return found;
    }
    // Every manifest, even of a game being downloaded or updated.
    let installed: HashSet<String> = read_manifests(steam).into_iter().map(|m| m.app_id).collect();
    let names = titles(lib, data_paths);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        let apps = root.join("steamapps");
        for (sub, prefix) in [("compatdata", true), ("shadercache", false)] {
            let Ok(rd) = fs::read_dir(apps.join(sub)) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                if app_id(&name).is_none() || !path.is_dir() {
                    continue;
                }
                // `~/.steam/steam` and `~/.local/share/Steam` are usually one folder.
                if !seen.insert(fs::canonicalize(&path).unwrap_or_else(|_| path.clone())) {
                    continue;
                }
                if installed.contains(&name) {
                    if !prefix {
                        found.shader_caches.push(path);
                    }
                    continue;
                }
                let title = names.get(&name).cloned().unwrap_or_else(|| tr!("Steam app {id}", id = name));
                let kind = if prefix { Kind::SteamPrefix { app_id: name } } else { Kind::SteamShaders { app_id: name } };
                found.leftovers.push(Item::new(Group::Steam, kind, State::Unused, title, path));
            }
        }
    }
    found
}

/// Steam id -> title, from the Steam library read from the client and the
/// Collection's games.
fn titles(lib: &Library, data_paths: &DataPaths) -> HashMap<String, String> {
    let mut out: HashMap<String, String> =
        lib.games.iter().filter_map(|g| Some((g.steam_launch_id.clone().or_else(|| g.steam_app_id.clone())?, g.title.clone()))).collect();
    for g in SteamOwnedFile::read(data_paths).games {
        out.entry(g.app_id).or_insert(g.name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::app_id;

    #[test]
    fn only_steam_ids() {
        assert_eq!(app_id("1245620"), Some(1245620));
        assert_eq!(app_id("0"), None);
        assert_eq!(app_id("2578321389"), None, "a non-Steam shortcut");
        assert_eq!(app_id("pfx"), None);
        assert_eq!(app_id("+12"), None);
    }
}
