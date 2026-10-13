//! Which installed games have a newer build. Windows and macOS builds are
//! compared by the `ETag` of their manifest, Linux installers by version.
//! The result is saved in `gog_store/updates.json` (`{"updates": [app names]}`).

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::DataPaths;
use serde_json::{json, Value};

use crate::api::{self, bearer, parallel_map};
use crate::cmd::Gogdl;
use crate::installed::{self, Installed};
use crate::user;

const WORKERS: usize = 4;

fn builds_url(app_name: &str, platform: &str) -> String {
    format!("https://content-system.gog.com/products/{app_name}/os/{platform}/builds?generation=2&_version=2")
}

fn downloads_url(app_name: &str) -> String {
    format!("https://api.gog.com/products/{app_name}?expand=downloads")
}

/// Manifest addresses of the build to follow: the one without a branch (the
/// public build), else the first.
fn manifest_urls(builds: &Value) -> Vec<String> {
    let items = builds.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let Some(build) = items.iter().find(|b| b.get("branch").is_none_or(Value::is_null)).or_else(|| items.first()) else {
        return Vec::new();
    };
    let urls: Vec<String> = build
        .get("urls")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|u| u.get("url").and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default();
    if urls.is_empty() {
        build.get("link").and_then(Value::as_str).map(str::to_string).into_iter().collect()
    } else {
        urls
    }
}

/// Whether a conditional request for the manifest shows a new build: `304`
/// means the stored `ETag` is current, `200` with another `ETag` means a new one.
fn newer_manifest(status: u16, etag: Option<&str>, stored: &str) -> bool {
    status == 200 && etag.is_some_and(|e| e != stored)
}

/// The version of the first Linux installer in a `products?expand=downloads` answer.
fn linux_version(product: &Value) -> Option<String> {
    let installers = product.pointer("/downloads/installers")?.as_array()?;
    let installer = installers.iter().find(|i| i.get("os").and_then(Value::as_str) == Some("linux"))?;
    installer.get("version").and_then(Value::as_str).map(str::to_string)
}

fn linux_update(product: &Value, installed_version: &str) -> bool {
    linux_version(product).is_some_and(|v| !v.is_empty() && v != installed_version)
}

fn check_one(token: &str, game: &Installed) -> Result<bool> {
    let id = game.app_name.as_str();
    if game.platform == "linux" {
        let product = api::get_json(&downloads_url(id), token)?;
        return Ok(linux_update(&product, &game.version));
    }
    if game.version_etag.is_empty() {
        log::debug!("GOG game {id}: no saved build tag, skipping the update check");
        return Ok(false);
    }
    let builds = api::get_json(&builds_url(id, &game.platform), token)?;
    let auth = bearer(token);
    let mut last_err = None;
    for url in manifest_urls(&builds) {
        match api::head_like(&url, &[("Authorization", auth.as_str()), ("If-None-Match", game.version_etag.as_str())]) {
            Ok((status, etag)) if status == 304 || status == 200 => return Ok(newer_manifest(status, etag.as_deref(), &game.version_etag)),
            Ok((status, _)) => log::debug!("{url}: HTTP {status}"),
            Err(err) => last_err = Some(err),
        }
    }
    match last_err {
        Some(err) => Err(err),
        None => Ok(false),
    }
}

/// Asks GOG about every installed game that is not pinned to a version, saves
/// and returns the app names with an update. Network.
pub fn check(paths: &DataPaths, gogdl: &Gogdl) -> Result<Vec<String>> {
    let creds = user::credentials(paths, gogdl)?;
    let games: Vec<Installed> = installed::load(paths).into_iter().filter(|g| !g.pinned_version).collect();
    let results = parallel_map(&games, WORKERS, |g| (g.app_name.clone(), check_one(&creds.access_token, g)));
    let mut updates = Vec::new();
    for (app, result) in results {
        match result {
            Ok(true) => updates.push(app),
            Ok(false) => {}
            Err(err) => log::warn!("checking the update of GOG game {app}: {err:#}"),
        }
    }
    save(paths, &updates)?;
    Ok(updates)
}

pub(crate) fn save(paths: &DataPaths, updates: &[String]) -> Result<()> {
    json_store::update_value(&paths.gog_store("updates"), |doc| {
        doc.as_object_mut().expect("object").insert("updates".into(), json!(updates));
        Ok(())
    })
}

/// The last result of [`check`], without the network.
pub fn cached_updates(paths: &DataPaths) -> Vec<String> {
    let doc = json_store::read_value(&paths.gog_store("updates")).unwrap_or(Value::Null);
    doc.get("updates")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Drops a game from the saved list (after an update or an uninstall).
pub fn forget(paths: &DataPaths, app_name: &str) {
    let mut updates = cached_updates(paths);
    let before = updates.len();
    updates.retain(|a| a != app_name);
    if updates.len() != before {
        if let Err(err) = save(paths, &updates) {
            log::warn!("saving the GOG update list: {err:#}");
        }
    }
}

/// Forgets every saved update (on logout).
pub(crate) fn clear(paths: &DataPaths) {
    if paths.gog_store("updates").exists() {
        if let Err(err) = save(paths, &[]) {
            log::warn!("clearing the GOG update list: {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::*;

    #[test]
    fn the_public_build_is_followed() {
        let builds = json!({"items": [
            {"branch": "beta", "urls": [{"url": "https://cdn/beta"}]},
            {"branch": null, "urls": [{"url": "https://cdn/a"}, {"url": "https://cdn/b"}]}
        ]});
        assert_eq!(manifest_urls(&builds), ["https://cdn/a", "https://cdn/b"]);
        let only_branches = json!({"items": [{"branch": "beta", "link": "https://cdn/link"}]});
        assert_eq!(manifest_urls(&only_branches), ["https://cdn/link"]);
        assert!(manifest_urls(&json!({"items": []})).is_empty());
        assert_eq!(builds_url("9", "windows"), "https://content-system.gog.com/products/9/os/windows/builds?generation=2&_version=2");
    }

    #[test]
    fn etags_decide_windows_updates() {
        assert!(!newer_manifest(304, None, "\"a\""));
        assert!(!newer_manifest(200, Some("\"a\""), "\"a\""));
        assert!(newer_manifest(200, Some("\"b\""), "\"a\""));
        assert!(!newer_manifest(200, None, "\"a\""));
        assert!(!newer_manifest(404, Some("\"b\""), "\"a\""));
    }

    #[test]
    fn linux_versions_are_compared() {
        let product = json!({"downloads": {"installers": [
            {"os": "windows", "version": "9.9"}, {"os": "linux", "version": "1.1"}, {"os": "linux", "version": "0.5"}
        ]}});
        assert_eq!(linux_version(&product).as_deref(), Some("1.1"));
        assert!(linux_update(&product, "1.0"));
        assert!(!linux_update(&product, "1.1"));
        assert!(!linux_update(&json!({"downloads": {"installers": []}}), "1.0"));
    }

    #[test]
    fn the_saved_list_is_read_without_the_network() {
        let (_dir, paths) = setup();
        assert!(cached_updates(&paths).is_empty());
        save(&paths, &["1".into(), "2".into()]).unwrap();
        assert_eq!(cached_updates(&paths), ["1", "2"]);
        forget(&paths, "1");
        assert_eq!(cached_updates(&paths), ["2"]);
        forget(&paths, "missing");
        clear(&paths);
        assert!(cached_updates(&paths).is_empty());
    }

    #[test]
    fn pinned_games_and_missing_tags_are_not_checked() {
        let (dir, paths) = setup();
        let mut no_tag = Installed::new("5");
        no_tag.platform = "windows".into();
        assert!(!check_one("tok", &no_tag).unwrap());
        installed::upsert(&paths, &no_tag).unwrap();
        let mut pinned = Installed::new("6");
        pinned.pinned_version = true;
        installed::upsert(&paths, &pinned).unwrap();
        let g = fake(dir.path(), &paths, "echo '{\"access_token\":\"t\",\"user_id\":\"1\"}'");
        // Neither game makes a request, so no network is needed.
        assert!(check(&paths, &g).unwrap().is_empty());
        assert!(cached_updates(&paths).is_empty());
        assert!(paths.gog_store("updates").exists());
    }
}
