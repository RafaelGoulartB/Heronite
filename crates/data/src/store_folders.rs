//! A store's own install folder (`heroniteStoreFolders` in the global
//! defaults, `{"epic": "/data/Games/Epic", "ea": "/data/Games/EA"}`), chosen
//! on the store's page. None is set at first: the stores Heronite installs
//! for (Epic Games, GOG, Amazon Games) use the default install folder
//! (`defaultInstallPath`), and the store clients (Battle.net, the EA app,
//! Ubisoft Connect, Rockstar Games) their own folder, inside their Wine
//! prefix. Steam and the Xbox app install their games themselves and have
//! none.
//!
//! The keys are [`EPIC`], [`GOG`], [`AMAZON`] and the clients' ids
//! ([`crate::clients::Client::id`]). The folders a store had before
//! ([`BEFORE_KEY`]) are kept: games installed there stay there, and a store
//! client still has to see them.

use anyhow::Result;
use serde_json::{Map, Value};

use crate::game_config::{self, Settings};
use crate::paths::DataPaths;

pub const KEY: &str = "heroniteStoreFolders";
/// The folders each store had before its current one (`{"ea": [..]}`).
pub const BEFORE_KEY: &str = "heroniteStoreFoldersBefore";

pub const EPIC: &str = "epic";
pub const GOG: &str = "gog";
pub const AMAZON: &str = "amazon";

/// The stores whose games Heronite downloads itself.
pub const INSTALLED_BY_HERONITE: [&str; 3] = [EPIC, GOG, AMAZON];

/// The folder chosen for `store`, `~` expanded; `None` when it has none.
pub fn own(global: &Settings, store: &str, home: &str) -> Option<String> {
    let folder = global.get(KEY)?.get(store)?.as_str()?.trim();
    (!folder.is_empty()).then(|| game_config::expand_tilde(folder, home))
}

/// The folders `store` had before its current one, `~` expanded.
pub fn before(global: &Settings, store: &str, home: &str) -> Vec<String> {
    let Some(Value::Array(list)) = global.get(BEFORE_KEY).and_then(|b| b.get(store)) else { return Vec::new() };
    list.iter().filter_map(Value::as_str).map(str::trim).filter(|f| !f.is_empty()).map(|f| game_config::expand_tilde(f, home)).collect()
}

/// Every folder chosen for a store, now or before (no duplicates): folders
/// a cleanup must never delete.
pub fn all(global: &Settings, home: &str) -> Vec<String> {
    let stores = [KEY, BEFORE_KEY].into_iter().filter_map(|k| global.get(k)?.as_object()).flat_map(|m| m.keys());
    let mut out: Vec<String> = Vec::new();
    for store in stores {
        for folder in own(global, store, home).into_iter().chain(before(global, store, home)) {
            if !out.contains(&folder) {
                out.push(folder);
            }
        }
    }
    out
}

/// The default install folder (`defaultInstallPath`, else
/// [`game_config::default_install_dir`]).
pub fn default_dir(global: &Settings, home: &str) -> String {
    global
        .get("defaultInstallPath")
        .and_then(Value::as_str)
        .map(|s| s.trim().trim_matches('\''))
        .filter(|s| !s.is_empty())
        .map(|s| game_config::expand_tilde(s, home))
        .unwrap_or_else(|| game_config::default_install_dir(home))
}

/// Where a new game of `store` is installed: its own folder, else the
/// default one.
pub fn install_dir(global: &Settings, store: &str, home: &str) -> String {
    own(global, store, home).unwrap_or_else(|| default_dir(global, home))
}

/// Sets `store`'s folder (`None` or a blank one: back to the default),
/// keeping the other stores'. The folder it replaces goes to [`before`].
pub fn set(paths: &DataPaths, home: &str, store: &str, folder: Option<&str>) -> Result<()> {
    let global = game_config::load_global(paths, home);
    let object = |key: &str| match global.get(key) {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let (mut set, mut earlier) = (object(KEY), object(BEFORE_KEY));
    let folder = folder.map(str::trim).filter(|f| !f.is_empty());
    let old = set.get(store).and_then(Value::as_str).map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    match folder {
        Some(f) => set.insert(store.to_string(), Value::String(f.to_string())),
        None => set.remove(store),
    };
    let mut list: Vec<String> =
        earlier.get(store).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    list.extend(old.filter(|o| Some(o.as_str()) != folder && !list.contains(o)));
    list.retain(|f| Some(f.as_str()) != folder);
    match list.is_empty() {
        true => earlier.remove(store),
        false => earlier.insert(store.to_string(), list.into_iter().map(Value::String).collect()),
    };
    let changes = [(KEY.to_string(), Value::Object(set)), (BEFORE_KEY.to_string(), Value::Object(earlier))];
    game_config::set_global_settings(paths, home, &changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(v: Value) -> Settings {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_store_without_a_folder_uses_the_default_one() {
        let g = settings(json!({"defaultInstallPath": "/data/Games", KEY: {"epic": "/data/Epic", "gog": "  "}}));
        assert_eq!(install_dir(&g, EPIC, "/home/u"), "/data/Epic");
        assert_eq!(install_dir(&g, GOG, "/home/u"), "/data/Games");
        assert_eq!(install_dir(&g, AMAZON, "/home/u"), "/data/Games");
        assert_eq!(own(&g, GOG, "/home/u"), None);
        // Nothing set at all: the install folder of a new setup.
        assert_eq!(install_dir(&Settings::new(), EPIC, "/home/u"), game_config::default_install_dir("/home/u"));
        let quoted = settings(json!({"defaultInstallPath": "'/data/My Games'"}));
        assert_eq!(default_dir(&quoted, "/home/u"), "/data/My Games");
    }

    #[test]
    fn a_folder_may_start_at_home() {
        let g = settings(json!({KEY: {"ea": "~/Games/EA", "ubisoft": "~/Games/EA"}, "defaultInstallPath": "~/Games"}));
        assert_eq!(own(&g, "ea", "/home/u").as_deref(), Some("/home/u/Games/EA"));
        assert_eq!(default_dir(&g, "/home/u"), "/home/u/Games");
        assert_eq!(all(&g, "/home/u"), ["/home/u/Games/EA"]);
        let g = settings(json!({KEY: {"ea": "/new"}, BEFORE_KEY: {"ea": ["/old", "/new"], "gog": ["~/G"]}}));
        assert_eq!(before(&g, "ea", "/home/u"), ["/old", "/new"]);
        assert_eq!(all(&g, "/home/u"), ["/new", "/old", "/home/u/G"]);
        assert!(all(&settings(json!({KEY: "x"})), "/home/u").is_empty());
    }

    #[test]
    fn setting_one_store_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        set(&paths, "/home/u", EPIC, Some("/data/Epic")).unwrap();
        set(&paths, "/home/u", "rockstar", Some(" /data/Rockstar ")).unwrap();
        let g = game_config::load_global(&paths, "/home/u");
        assert_eq!(g[KEY], json!({"epic": "/data/Epic", "rockstar": "/data/Rockstar"}));
        // The rest of the settings are there too.
        assert!(g.contains_key("defaultInstallPath"));

        set(&paths, "/home/u", EPIC, None).unwrap();
        set(&paths, "/home/u", "rockstar", Some("")).unwrap();
        let g = game_config::load_global(&paths, "/home/u");
        assert_eq!(g[KEY], json!({}));
        // The folders they had are remembered, once each.
        assert_eq!(g[BEFORE_KEY], json!({"epic": ["/data/Epic"], "rockstar": ["/data/Rockstar"]}));
        set(&paths, "/home/u", "rockstar", Some("/data/R2")).unwrap();
        set(&paths, "/home/u", "rockstar", Some("/data/Rockstar")).unwrap();
        set(&paths, "/home/u", "rockstar", Some("/data/R2")).unwrap();
        let g = game_config::load_global(&paths, "/home/u");
        assert_eq!(g[BEFORE_KEY]["rockstar"], json!(["/data/Rockstar"]));
        assert_eq!(own(&g, "rockstar", "/home/u").as_deref(), Some("/data/R2"));
    }
}
