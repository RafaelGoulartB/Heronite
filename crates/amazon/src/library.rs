//! The Amazon library: nile's `library.json` (entitlements) joined with its
//! `installed.json`, written as `store_cache/nile_library.json`
//! (`{"library": [GameInfo], "__timestamp": {...}}`) for the Collection.
//!
//! An entitlement is `{ id, product: { id, title, productDetail: { iconUrl,
//! details: { shortDescription, developer, genres, releaseDate,
//! backgroundUrl1, backgroundUrl2, logoUrl, ... } } } }`. The game's id is the
//! *product* id, not the entitlement id.

use std::collections::HashSet;

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_epic::util::{file_size, remove_special_characters};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::cmd::Nile;
use crate::installed::{self, Installed};

const CACHE: &str = "nile_library";
const KEY: &str = "library";

fn store(paths: &DataPaths) -> CacheStore<'_> {
    CacheStore::new(paths, CACHE, None)
}

/// `nile library sync`, then the library file is rebuilt. Returns the number
/// of games.
pub fn refresh(paths: &DataPaths, nile: &Nile) -> Result<usize> {
    let out = nile.run(&["library", "sync"])?;
    let text = format!("{}\n{}", out.stderr, out.stdout);
    if text.contains("User not logged in") {
        bail!(tr!("You are not signed in to Amazon Games"));
    }
    if !out.success() {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("nile exited with {code}", code = format!("{:?}", out.code)));
        }
        bail!(tr!("Failed to sync the Amazon Games library: {error}", error = why));
    }
    write_library(paths)
}

/// Rebuilds `store_cache/nile_library.json` from nile's files, without the
/// network. Returns the number of games.
pub fn write_library(paths: &DataPaths) -> Result<usize> {
    let games = build(paths);
    let count = games.len();
    store(paths).set(KEY, Value::Array(games))?;
    Ok(count)
}

/// The entitlements of nile's `library.json`.
fn entitlements(paths: &DataPaths) -> Vec<Value> {
    match json_store::read_value(&paths.nile_config_dir().join("nile").join("library.json")) {
        Ok(Value::Array(list)) => list,
        Ok(_) => Vec::new(),
        Err(err) => {
            log::warn!("reading nile's library: {err:#}");
            Vec::new()
        }
    }
}

fn build(paths: &DataPaths) -> Vec<Value> {
    let installed = installed::load(paths);
    let mut seen = HashSet::new();
    entitlements(paths)
        .iter()
        .filter_map(|entry| {
            let id = entry.pointer("/product/id").and_then(Value::as_str)?;
            // The same product can be owned through several entitlements.
            if !seen.insert(id.to_string()) {
                return None;
            }
            to_game_info(entry, installed.iter().find(|i| i.id == id))
        })
        .collect()
}

fn text<'a>(entry: &'a Value, pointer: &str) -> &'a str {
    entry.pointer(pointer).and_then(Value::as_str).unwrap_or_default()
}

/// The first of the texts that is not empty.
fn first<'a>(options: &[&'a str]) -> &'a str {
    options.iter().copied().find(|s| !s.is_empty()).unwrap_or_default()
}

/// Maps one entitlement (and the game's `installed.json` entry) to a
/// `GameInfo`. `None` when it has no product id.
pub(crate) fn to_game_info(entry: &Value, installed: Option<&Installed>) -> Option<Value> {
    let id = entry.pointer("/product/id").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let title = Some(text(entry, "/product/title")).filter(|t| !t.is_empty()).unwrap_or("???");
    let icon = text(entry, "/product/productDetail/iconUrl");
    let details = |key: &str| text(entry, &format!("/product/productDetail/details/{key}")).to_string();
    let description = details("shortDescription");
    let genres: Vec<Value> = entry
        .pointer("/product/productDetail/details/genres")
        .and_then(Value::as_array)
        .map(|g| g.iter().filter(|v| v.is_string()).cloned().collect())
        .unwrap_or_default();
    let folder = remove_special_characters(title);
    let (is_installed, install) = match installed {
        Some(i) => (
            true,
            json!({
                "install_path": i.path,
                "install_size": file_size(i.size.unwrap_or(0) as f64),
                "version": i.version,
                "platform": "Windows",
                "is_dlc": false,
            }),
        ),
        None => (false, json!({})),
    };
    Some(json!({
        "runner": "nile",
        "app_name": id,
        "title": title,
        "art_cover": first(&[&details("backgroundUrl2"), icon]),
        "art_square": icon,
        "art_logo": details("logoUrl"),
        "art_background": first(&[&details("backgroundUrl1"), &details("backgroundUrl2")]),
        "developer": details("developer"),
        "description": description,
        "is_installed": is_installed,
        "install": install,
        "folder_name": if folder.is_empty() { id.to_string() } else { folder },
        "is_linux_native": false,
        "is_mac_native": false,
        "cloud_save_enabled": false,
        "canRunOffline": true,
        "extra": {
            "about": { "description": description },
            "genres": genres,
            "releaseDate": details("releaseDate"),
        },
    }))
}

/// The written library, or one rebuilt from nile's files when none was
/// written yet.
fn cached(paths: &DataPaths) -> Vec<Value> {
    match store(paths).get(KEY) {
        Some(Value::Array(list)) => list,
        _ => build(paths),
    }
}

/// The `GameInfo` of one game.
pub fn game_info(paths: &DataPaths, app_name: &str) -> Option<Value> {
    cached(paths).into_iter().find(|g| g.get("app_name").and_then(Value::as_str) == Some(app_name))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub owned: usize,
    pub installed: usize,
}

pub fn stats(paths: &DataPaths) -> Stats {
    let games = cached(paths);
    Stats { owned: games.len(), installed: games.iter().filter(|g| is_installed(g)).count() }
}

/// Games that can be installed.
pub fn not_installed(paths: &DataPaths) -> Vec<Value> {
    cached(paths).into_iter().filter(|g| !is_installed(g)).collect()
}

fn is_installed(game: &Value) -> bool {
    game.get("is_installed").and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::Env;

    const LIBRARY: &str = r#"[
      {"id": "ent-1", "product": {"id": "amzn1.adg.product.AAA", "title": "Cool: Game™ 2", "sku": "X",
        "productDetail": {"iconUrl": "https://img/icon-a.png", "details": {
          "shortDescription": "A cool game", "developer": "Dev A", "publisher": "Pub", "genres": ["Action", "RPG"],
          "releaseDate": "2020-01-02T00:00:00Z", "backgroundUrl1": "https://img/bg1.png", "backgroundUrl2": "https://img/bg2.png",
          "logoUrl": "https://img/logo.png", "screenshots": []}}}},
      {"id": "ent-1b", "product": {"id": "amzn1.adg.product.AAA", "title": "Cool: Game™ 2"}},
      {"id": "ent-2", "product": {"id": "amzn1.adg.product.BBB", "title": "Plain",
        "productDetail": {"iconUrl": "https://img/icon-b.png", "details": {"backgroundUrl2": "", "backgroundUrl1": ""}}}},
      {"id": "ent-3", "product": {"id": "amzn1.adg.product.CCC"}},
      {"id": "ent-4", "product": {"title": "No id"}}
    ]"#;

    fn setup(env: &Env) {
        env.write_nile_file("library.json", LIBRARY);
        env.write_nile_file(
            "installed.json",
            r#"[{"id": "amzn1.adg.product.AAA", "version": "v9", "path": "/games/Cool Game 2", "size": 2147483648}]"#,
        );
    }

    #[test]
    fn entitlements_map_to_game_infos() {
        let env = Env::new();
        setup(&env);
        let games = build(&env.paths);
        // Duplicate product and entry without a product id are dropped.
        assert_eq!(games.len(), 3);

        let a = &games[0];
        assert_eq!(a["runner"], "nile");
        assert_eq!(a["app_name"], "amzn1.adg.product.AAA");
        assert_eq!(a["title"], "Cool: Game™ 2");
        assert_eq!(a["folder_name"], "Cool Game 2");
        assert_eq!(a["art_cover"], "https://img/bg2.png");
        assert_eq!(a["art_square"], "https://img/icon-a.png");
        assert_eq!(a["art_background"], "https://img/bg1.png");
        assert_eq!(a["art_logo"], "https://img/logo.png");
        assert_eq!(a["developer"], "Dev A");
        assert_eq!(a["description"], "A cool game");
        assert_eq!(a["extra"]["about"]["description"], "A cool game");
        assert_eq!(a["extra"]["genres"], json!(["Action", "RPG"]));
        assert_eq!(a["extra"]["releaseDate"], "2020-01-02T00:00:00Z");
        assert_eq!(a["canRunOffline"], true);
        assert_eq!(a["cloud_save_enabled"], false);
        assert_eq!(a["is_linux_native"], false);
        assert_eq!(a["is_installed"], true);
        assert_eq!(a["install"]["install_path"], "/games/Cool Game 2");
        assert_eq!(a["install"]["install_size"], "2 GiB");
        assert_eq!(a["install"]["version"], "v9");
        assert_eq!(a["install"]["platform"], "Windows");
        assert_eq!(a["install"]["is_dlc"], false);

        // Empty background urls fall back to the icon / the other background.
        let b = &games[1];
        assert_eq!(b["is_installed"], false);
        assert_eq!(b["install"], json!({}));
        assert_eq!(b["art_cover"], "https://img/icon-b.png");
        assert_eq!(b["art_background"], "");

        // No title: placeholder title, folder from the id.
        let c = &games[2];
        assert_eq!(c["title"], "???");
        assert_eq!(c["folder_name"], "amzn1.adg.product.CCC");
    }

    #[test]
    fn written_library_has_a_timestamp_and_the_collection_sees_it() {
        let env = Env::new();
        setup(&env);
        assert_eq!(write_library(&env.paths).unwrap(), 3);
        let raw = json_store::read_value(&env.paths.nile_library()).unwrap();
        assert!(raw["__timestamp"]["library"].is_string());
        assert_eq!(raw["library"].as_array().unwrap().len(), 3);

        let library = heronite_data::Library::load(&env.paths).unwrap();
        let game = library.games.iter().find(|g| g.game.app_name == "amzn1.adg.product.AAA").expect("nile game in the Collection");
        assert_eq!(game.game.runner, "nile");
        assert!(game.game.is_installed);
        assert_eq!(game.game.install.install_path.as_deref(), Some("/games/Cool Game 2"));
        assert_eq!(library.games.iter().filter(|g| g.game.runner == "nile").count(), 3);
    }

    #[test]
    fn stats_and_not_installed_follow_the_written_library() {
        let env = Env::new();
        setup(&env);
        assert_eq!(stats(&env.paths), Stats { owned: 3, installed: 1 });
        write_library(&env.paths).unwrap();
        assert_eq!(stats(&env.paths), Stats { owned: 3, installed: 1 });
        let titles: Vec<_> = not_installed(&env.paths).iter().map(|g| g["title"].as_str().unwrap().to_string()).collect();
        assert_eq!(titles, ["Plain", "???"]);
        assert_eq!(game_info(&env.paths, "amzn1.adg.product.BBB").unwrap()["title"], "Plain");
        assert!(game_info(&env.paths, "missing").is_none());
    }

    #[test]
    fn refresh_syncs_then_writes_the_library() {
        let env = Env::new();
        let body = format!("mkdir -p \"$NILE_CONFIG_PATH/nile\"; cat > \"$NILE_CONFIG_PATH/nile/library.json\" <<'EOF'\n{LIBRARY}\nEOF");
        let nile = env.nile(&body);
        assert_eq!(refresh(&env.paths, &nile).unwrap(), 3);
        assert_eq!(env.calls(), ["library sync"]);
        assert!(!heronite_data::Library::load(&env.paths).unwrap().games.is_empty());
    }

    #[test]
    fn refresh_reports_a_missing_login() {
        let env = Env::new();
        let nile = env.nile("echo 'ERROR [CLI]:\t User not logged in' >&2; exit 1");
        let err = refresh(&env.paths, &nile).unwrap_err().to_string();
        assert!(err.contains("not signed in"), "{err}");
        assert!(!env.paths.nile_library().exists());
    }

    #[test]
    fn refresh_reports_other_failures() {
        let env = Env::new();
        let nile = env.nile("echo 'ERROR [LIBRARY]:\t There was an error syncing library' >&2; exit 1");
        let err = refresh(&env.paths, &nile).unwrap_err().to_string();
        assert!(err.contains("error syncing library"), "{err}");
    }
}
