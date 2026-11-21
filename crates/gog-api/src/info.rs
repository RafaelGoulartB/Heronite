//! Install information: the JSON `gogdl info <id> --os <os>` printed (sizes
//! per language, languages, DLCs, build, folder name), which the app caches
//! and reads with its own parser.

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::api::Client;
use crate::builds::Builds;
use crate::linux;
use crate::redist::Repository;
use crate::v2::Sizes;
use crate::{language, v1, v2};

fn sizes_json(s: Option<&Sizes>) -> Value {
    let mut map = Map::new();
    for (lang, (download, disk)) in s.into_iter().flatten() {
        map.insert(lang.clone(), json!({"download_size": download, "disk_size": disk}));
    }
    if !map.contains_key("*") {
        map.insert("*".into(), json!({"download_size": 0, "disk_size": 0}));
    }
    Value::Object(map)
}

/// The document for a content-system build (`windows` / `osx`), from its
/// parts.
pub fn content_system_info(
    game_id: &str,
    builds: &Builds,
    meta_doc: &Value,
    etag: Option<&str>,
    owned: &[String],
    repo: Option<&Repository>,
) -> Result<Value> {
    let build = builds.pick(None, None).ok_or_else(|| anyhow!("no build"))?;
    let (mut sizes, dlcs, languages, folder, deps) = if build.generation == 1 {
        let m = v1::Meta::parse(meta_doc)?;
        let dlcs = m.owned_dlcs(owned, None);
        (m.sizes(&dlcs), dlcs.clone(), m.languages(&dlcs), m.install_directory.clone(), m.dependencies.clone())
    } else {
        let m = v2::Meta::parse(meta_doc)?;
        let dlcs = m.owned_dlcs(owned, None);
        (m.sizes(&dlcs), dlcs.clone(), m.languages(&dlcs), m.install_directory.clone(), m.dependencies.clone())
    };
    if let Some(repo) = repo {
        let (download, disk) = repo.game_folder_size(&deps);
        let star = sizes.entry(game_id.to_string()).or_default().entry("*".into()).or_default();
        star.0 += download;
        star.1 += disk;
    }
    let dlcs: Vec<Value> = dlcs.iter().map(|d| json!({"title": d.title, "id": d.id, "size": sizes_json(sizes.get(&d.id))})).collect();
    Ok(json!({
        "size": sizes_json(sizes.get(game_id)),
        "dlcs": dlcs,
        "buildId": build.recorded_id(),
        "languages": languages,
        "folder_name": folder,
        "dependencies": deps,
        "versionEtag": etag,
        "versionName": build.version_name,
        "available_branches": builds.branches(),
        "builds": builds.raw,
    }))
}

/// The document for a Linux installer.
pub fn linux_info(
    installer: &linux::Installer,
    product: &Value,
    installers: &[&Value],
    version: Option<&Value>,
    folder_name: &str,
    owned: &[String],
) -> Value {
    let (download, disk) = installer.sizes();
    let dlcs: Vec<Value> = product
        .get("expanded_dlcs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|d| d.get("id").and_then(crate::api::id_text).is_some_and(|id| owned.contains(&id)))
        .filter(|d| d.pointer("/downloads/installers").and_then(Value::as_array).is_some_and(|l| !l.is_empty()))
        .map(|d| {
            let langs: Vec<Value> = linux::linux_installers(d).iter().filter_map(|i| i.get("language").cloned()).collect();
            json!({"title": d.get("title"), "id": d.get("id"), "languages": [langs]})
        })
        .collect();
    let languages: Vec<String> = installers
        .iter()
        .filter_map(|i| i.get("language").and_then(Value::as_str))
        .filter_map(language::parse)
        .map(|l| l.code.to_string())
        .collect();
    json!({
        "download_size": download,
        "disk_size": disk,
        "dlcs": dlcs,
        "languages": languages,
        "folder_name": folder_name,
        "dependencies": [],
        "versionName": version.cloned().unwrap_or(Value::Null),
        "builds": null,
    })
}

/// The folder a Linux install goes in: the Windows build's, as gogdl named it
/// (`UnknownGame` when there is none).
pub fn folder_from_windows_build(client: &Client, game_id: &str) -> String {
    let found = (|| -> Result<String> {
        let builds = client.builds(game_id, "windows", None)?;
        let build = builds.items.first().ok_or_else(|| anyhow!("no build"))?;
        let (meta, _) = client.meta(build)?;
        let dir = if build.generation == 2 { meta.get("installDirectory") } else { meta.pointer("/product/installDirectory") };
        dir.and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("no folder"))
    })();
    found.unwrap_or_else(|_| "UnknownGame".to_string())
}

/// What `gogdl info <id> --os <os>` printed, from GOG (network): `os` is
/// `windows`, `osx` or `linux`.
pub fn fetch(client: &Client, game_id: &str, os: &str) -> Result<Value> {
    let owned = client.owned()?;
    if os == "linux" {
        let product = client.product_downloads(game_id)?;
        let installers = linux::linux_installers(&product);
        let installer = linux::pick_installer(&installers, "en-US")
            .ok_or_else(|| anyhow!(heronite_i18n::tr!("This game cannot be installed for {platform}", platform = os)))?;
        let opened = client.open_installer(installer, game_id)?;
        let folder = folder_from_windows_build(client, game_id);
        return Ok(linux_info(&opened, &product, &installers, installer.get("version"), &folder, &owned));
    }
    let builds = client.builds(game_id, os, None)?;
    let build = builds.pick(None, None).ok_or_else(|| anyhow!("no build"))?;
    let (meta, etag) = client.meta(build)?;
    let repo = match client.repository() {
        Ok(r) => Some(r),
        Err(err) => {
            log::warn!("the GOG redist repository: {err:#}");
            None
        }
    };
    content_system_info(game_id, &builds, &meta, etag.as_deref(), &owned, repo.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builds() -> Builds {
        Builds::parse(
            &json!({"items": [{"build_id": "55", "version_name": "1.5", "branch": null, "generation": 2, "link": "l"}, {"build_id": "66", "version_name": "b", "branch": "beta", "generation": 2, "link": "l2"}]}),
        )
    }

    #[test]
    fn content_system_info_has_gogdls_keys() {
        let meta = json!({
            "version": 2, "baseProductId": "1", "installDirectory": "Game", "dependencies": ["INGAME", "MSVC"],
            "products": [{"productId": "1", "name": "Game"}, {"productId": "2", "name": "DLC"}],
            "depots": [
                {"productId": "1", "languages": ["*"], "size": 100, "compressedSize": 60, "manifest": "a"},
                {"productId": "1", "languages": ["en-US"], "size": 10, "compressedSize": 6, "manifest": "b"},
                {"productId": "2", "languages": ["*"], "size": 5, "compressedSize": 4, "manifest": "c"}
            ]
        });
        let repo = Repository::parse(&json!({"depots": [
            {"dependencyId": "INGAME", "executable": {"path": "bin/x.exe"}, "manifest": "r", "compressedSize": 1, "size": 2},
            {"dependencyId": "MSVC", "executable": {"path": "__redist/MSVC/x.exe"}, "manifest": "s", "compressedSize": 50, "size": 50}
        ]}));
        let doc = content_system_info("1", &builds(), &meta, Some("\"tag\""), &["2".into()], Some(&repo)).unwrap();
        assert_eq!(doc["size"]["*"], json!({"download_size": 61, "disk_size": 102}));
        assert_eq!(doc["size"]["en-US"], json!({"download_size": 6, "disk_size": 10}));
        assert_eq!(doc["dlcs"], json!([{"title": "DLC", "id": "2", "size": {"*": {"download_size": 4, "disk_size": 5}}}]));
        assert_eq!(
            (doc["buildId"].as_str(), doc["versionName"].as_str(), doc["folder_name"].as_str()),
            (Some("55"), Some("1.5"), Some("Game"))
        );
        assert_eq!(doc["versionEtag"], "\"tag\"");
        assert_eq!(doc["languages"], json!(["en-US"]));
        assert_eq!(doc["available_branches"], json!([null, "beta"]));
        assert_eq!(doc["dependencies"], json!(["INGAME", "MSVC"]));
    }
}
