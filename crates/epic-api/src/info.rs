//! The install information `legendary info <app> --json --platform <p>`
//! prints: the game (versions, cloud saves, DLCs owned), the install (when
//! there is one) and the build's manifest (sizes, tags, prerequisites).
//! [`info_json`] builds it from what is known; [`fetch`] asks Epic first.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::api::ManifestInfo;
use crate::auth::Session;
use crate::client::Client;
use crate::installed::{self, InstalledGame};
use crate::library::{self, GameMeta};
use crate::manifest::{plan::tag_sizes, Manifest};

/// A build's manifest and how it came.
#[derive(Debug, Clone, Copy)]
pub struct ManifestPart<'a> {
    pub manifest: &'a Manifest,
    /// Size of the manifest file, bytes.
    pub size: usize,
    pub is_preloaded: bool,
    /// `None` when the manifest came from the installed copy.
    pub use_signed_url: Option<bool>,
}

/// Everything [`info_json`] reads.
#[derive(Debug, Clone, Copy)]
pub struct InfoInput<'a> {
    pub game: &'a GameMeta,
    pub platform: &'a str,
    /// `None` offline: owned DLCs are then not listed.
    pub entitlements: Option<&'a [Value]>,
    /// App names of the account's builds for `platform`.
    pub owned_apps: &'a HashSet<String>,
    pub installed: &'a BTreeMap<String, InstalledGame>,
    pub manifest: Option<ManifestPart<'a>>,
}

fn game_part(i: &InfoInput) -> Map<String, Value> {
    let g = i.game;
    let mut out = Map::new();
    out.insert("app_name".into(), json!(g.app_name));
    out.insert("title".into(), json!(g.app_title));
    out.insert("version".into(), g.app_version(i.platform).map_or(Value::Null, |v| json!(v)));
    let versions: Map<String, Value> = g.asset_infos.iter().map(|(p, a)| (p.clone(), json!(a.build_version))).collect();
    out.insert("platform_versions".into(), Value::Object(versions));
    // When the account got the game: its entitlement, else the catalog's date.
    let grant = i
        .entitlements
        .into_iter()
        .flatten()
        .find(|e| e.get("namespace").and_then(Value::as_str) == Some(g.namespace()))
        .and_then(|e| e.get("grantDate"))
        .or_else(|| g.metadata.get("creationDate"))
        .cloned()
        .unwrap_or(Value::Null);
    out.insert("grant_date".into(), grant);
    out.insert("cloud_saves_supported".into(), json!(g.supports_cloud_saves() || g.supports_mac_cloud_saves()));
    out.insert("cloud_save_folder".into(), g.custom_attribute("CloudSaveFolder").map_or(Value::Null, |v| json!(v)));
    out.insert("cloud_save_folder_mac".into(), g.custom_attribute("CloudSaveFolder_MAC").map_or(Value::Null, |v| json!(v)));
    out.insert("is_dlc".into(), json!(g.is_dlc()));
    let external = g.third_party_store().or_else(|| g.custom_attribute("partnerLinkType").filter(|s| !s.is_empty()));
    out.insert("external_activation".into(), external.map_or(Value::Null, |v| json!(v)));

    let mut options = Vec::new();
    for n in 1.. {
        let (Some(name), Some(args)) =
            (g.custom_attribute(&format!("extraLaunchOption_{n:03}_Name")), g.custom_attribute(&format!("extraLaunchOption_{n:03}_Args")))
        else {
            break;
        };
        options.push((name.to_string(), args.to_string()));
    }
    options.sort();
    out.insert("launch_options".into(), Value::Array(options.into_iter().map(|(n, a)| json!({"name": n, "parameters": a})).collect()));
    out.insert("command_line".into(), g.additional_command_line().map_or(Value::Null, |v| json!(v)));

    let mut owned = Vec::new();
    if let (Some(ents), false) = (i.entitlements, g.is_dlc()) {
        let names: HashSet<&str> = ents.iter().filter_map(|e| e.get("entitlementName").and_then(Value::as_str)).collect();
        for dlc in g.metadata.get("dlcItemList").and_then(Value::as_array).into_iter().flatten() {
            let installable = dlc.get("releaseInfo").filter(|r| r.as_array().is_some_and(|a| !a.is_empty())).cloned();
            let title = dlc.get("title").cloned().unwrap_or(Value::Null);
            let id = dlc.get("id").cloned().unwrap_or(Value::Null);
            let entitled = dlc.get("entitlementName").and_then(Value::as_str).is_some_and(|n| names.contains(n));
            if entitled {
                owned.push(json!({"app_name": null, "title": title, "installable": installable, "id": id}));
            } else if let Some(release) = &installable {
                let app = release.pointer("/0/appId").and_then(Value::as_str).unwrap_or_default();
                if i.owned_apps.contains(app) {
                    owned.push(json!({"app_name": app, "title": title, "installable": release, "id": id}));
                }
            }
        }
    }
    out.insert("owned_dlc".into(), Value::Array(owned));
    out
}

fn install_part(i: &InfoInput) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(ig) = i.installed.get(&i.game.app_name) else { return out };
    out.insert("platform".into(), json!(ig.platform));
    out.insert("version".into(), json!(ig.version));
    out.insert("disk_size".into(), json!(ig.install_size));
    out.insert("install_path".into(), json!(ig.install_path));
    out.insert("save_path".into(), ig.save_path.as_ref().map_or(Value::Null, |p| json!(p)));
    out.insert("synced_egl_guid".into(), json!(ig.egl_guid));
    out.insert("install_tags".into(), json!(ig.install_tags));
    out.insert("requires_ovt".into(), json!(ig.requires_ot));
    let mut dlcs = Vec::new();
    for dlc in i.game.metadata.get("dlcItemList").and_then(Value::as_array).into_iter().flatten() {
        let Some(app) = dlc.pointer("/releaseInfo/0/appId").and_then(Value::as_str) else { continue };
        if let Some(d) = i.installed.get(app) {
            dlcs.push(json!({"app_name": d.app_name, "title": d.title, "install_size": d.install_size}));
        }
    }
    out.insert("installed_dlc".into(), Value::Array(dlcs));
    out
}

fn manifest_part(part: &ManifestPart) -> Map<String, Value> {
    let m = part.manifest;
    let meta = &m.meta;
    let mut out = Map::new();
    out.insert("size".into(), json!(part.size));
    out.insert("type".into(), json!(if m.is_json { "json" } else { "binary" }));
    out.insert("version".into(), json!(m.version));
    out.insert("feature_level".into(), json!(meta.feature_level));
    out.insert("compressed".into(), json!(m.is_compressed()));
    out.insert("encrypted".into(), json!(m.is_encrypted()));
    out.insert("app_name".into(), json!(meta.app_name));
    out.insert("launch_exe".into(), json!(meta.launch_exe));
    out.insert("launch_command".into(), json!(meta.launch_command));
    out.insert("build_version".into(), json!(meta.build_version));
    out.insert("build_id".into(), json!(meta.build_id()));
    out.insert(
        "prerequisites".into(),
        if meta.prereq_ids.is_empty() {
            Value::Null
        } else {
            json!({"ids": meta.prereq_ids, "name": meta.prereq_name, "path": meta.prereq_path, "args": meta.prereq_args})
        },
    );
    out.insert(
        "uninstaller".into(),
        if meta.uninstall_action_path.is_empty() {
            Value::Null
        } else {
            json!({"path": meta.uninstall_action_path, "args": meta.uninstall_action_args})
        },
    );
    out.insert("install_tags".into(), json!(m.install_tags()));
    out.insert("num_files".into(), json!(m.files.len()));
    out.insert("num_chunks".into(), json!(m.chunks.len()));
    out.insert("disk_size".into(), json!(m.disk_size()));
    out.insert("download_size".into(), json!(m.download_size()));
    let (disk, download) = tag_sizes(m);
    let rows = |rows: Vec<(String, u64, usize)>| -> Value {
        Value::Array(rows.into_iter().map(|(tag, size, count)| json!({"tag": tag, "size": size, "count": count})).collect())
    };
    out.insert("tag_disk_size".into(), rows(disk));
    out.insert("tag_download_size".into(), rows(download));
    out.insert("is_preloaded".into(), json!(part.is_preloaded));
    if let Some(signed) = part.use_signed_url {
        out.insert("use_signed_urls".into(), json!(signed));
    }
    out
}

/// legendary's `info --json`: `{"game": .., "install": .., "manifest": ..}`,
/// an empty part as `null`.
pub fn info_json(i: &InfoInput) -> Value {
    let part = |m: Map<String, Value>| if m.is_empty() { Value::Null } else { Value::Object(m) };
    json!({
        "game": part(game_part(i)),
        "install": part(install_part(i)),
        "manifest": part(i.manifest.as_ref().map(manifest_part).unwrap_or_default()),
    })
}

/// A build's manifest, downloaded and decrypted, with what came with it.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub info: ManifestInfo,
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
}

/// Downloads the current manifest of `game` for `platform`.
pub fn fetch_manifest(client: &Client, token: &str, game: &GameMeta, platform: &str) -> Result<Fetched> {
    let info = client.manifest_info(token, game.namespace(), game.catalog_item_id(), &game.app_name, platform, "Live")?;
    let bytes = client.download_manifest(&info)?;
    let mut manifest = Manifest::parse(&bytes)?;
    manifest.decrypt(&info.secrets)?;
    Ok(Fetched { info, bytes, manifest })
}

/// [`info_json`] with fresh data from Epic: the account's entitlements, the
/// game's catalog entry and its current manifest. `offline` uses only the
/// stored metadata and the installed manifest. The metadata must be there
/// (a library refresh writes it).
pub fn fetch(
    client: &Client,
    config_dir: &Path,
    session: Option<&Session>,
    app_name: &str,
    platform: &str,
    offline: bool,
) -> Result<Value> {
    let mut game =
        GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not in the Epic library; refresh it first"))?;
    let installed = installed::read_all(config_dir);
    let stored = library::read_assets(config_dir);
    let mut owned: HashSet<String> = stored.get(platform).into_iter().flatten().map(|a| a.app_name.clone()).collect();
    let session = if offline { None } else { session };

    let mut entitlements = None;
    let mut fetched = None;
    let has_build = game.asset_infos.contains_key(platform) && game.third_party_store().is_none();
    if let Some(s) = session {
        entitlements = Some(client.entitlements(s)?);
        if let Some(fresh) = client.catalog_item(s, game.namespace(), game.catalog_item_id())? {
            game.metadata = fresh;
        }
        if let Ok(list) = client.assets(s, platform, "Live") {
            owned = list.into_iter().map(|a| a.app_name).collect();
        }
        if has_build {
            fetched = Some(fetch_manifest(client, s.access_token(), &game, platform)?);
        }
    }
    let local = match (&fetched, installed.get(app_name)) {
        (None, Some(ig)) => installed::load_manifest(config_dir, app_name, &ig.version, &ig.platform)
            .and_then(|b| Manifest::parse(&b).ok().map(|m| (b.len(), m))),
        _ => None,
    };
    let manifest = match (&fetched, &local) {
        (Some(f), _) => Some(ManifestPart {
            manifest: &f.manifest,
            size: f.bytes.len(),
            is_preloaded: f.info.is_preloaded,
            use_signed_url: Some(f.info.use_signed_url),
        }),
        (None, Some((size, m))) => Some(ManifestPart { manifest: m, size: *size, is_preloaded: false, use_signed_url: None }),
        _ => None,
    };
    let ents = entitlements.as_deref();
    Ok(info_json(&InfoInput { game: &game, platform, entitlements: ents, owned_apps: &owned, installed: &installed, manifest }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;

    fn game() -> GameMeta {
        GameMeta::from_value(json!({
            "app_name": "Calluna", "app_title": "Control",
            "asset_infos": {"Windows": {"app_name": "Calluna", "build_version": "1.33"}},
            "metadata": {"id": "c", "namespace": "calluna", "customAttributes": {
                "CloudSaveFolder": {"value": "{AppData}/Remedy"},
                "extraLaunchOption_001_Name": {"value": "DX11"}, "extraLaunchOption_001_Args": {"value": "-dx11"},
                "extraLaunchOption_002_Name": {"value": "DX12"}, "extraLaunchOption_002_Args": {"value": "-dx12"}
            }, "dlcItemList": [
                {"id": "d1", "title": "AWE", "entitlementName": "awe", "releaseInfo": [{"appId": "AweDlc"}]},
                {"id": "d2", "title": "Foundation", "entitlementName": "found", "releaseInfo": [{"appId": "FoundDlc"}]},
                {"id": "d3", "title": "Skin", "entitlementName": "skin"}
            ]}
        }))
        .unwrap()
    }

    #[test]
    fn info_has_legendarys_keys() {
        let g = game();
        let m = sample();
        let ents = vec![
            json!({"entitlementName": "awe"}),
            json!({"entitlementName": "skin"}),
            json!({"namespace": "calluna", "grantDate": "2021-06-12"}),
        ];
        let owned: HashSet<String> = ["FoundDlc".to_string()].into();
        let mut installed = BTreeMap::new();
        installed.insert(
            "Calluna".into(),
            InstalledGame { app_name: "Calluna".into(), version: "1.30".into(), install_size: 5, ..Default::default() },
        );
        installed.insert(
            "AweDlc".into(),
            InstalledGame { app_name: "AweDlc".into(), title: "AWE".into(), install_size: 2, ..Default::default() },
        );
        let doc = info_json(&InfoInput {
            game: &g,
            platform: "Windows",
            entitlements: Some(&ents),
            owned_apps: &owned,
            installed: &installed,
            manifest: Some(ManifestPart { manifest: &m, size: 123, is_preloaded: false, use_signed_url: Some(false) }),
        });
        assert_eq!(doc["game"]["version"], "1.33");
        assert_eq!(doc["game"]["grant_date"], "2021-06-12");
        assert_eq!(doc["game"]["cloud_saves_supported"], true);
        assert_eq!(
            doc["game"]["launch_options"],
            json!([{"name": "DX11", "parameters": "-dx11"}, {"name": "DX12", "parameters": "-dx12"}])
        );
        let dlc = doc["game"]["owned_dlc"].as_array().unwrap();
        assert_eq!(dlc.len(), 3, "{dlc:?}");
        assert_eq!(dlc[1], json!({"app_name": "FoundDlc", "title": "Foundation", "installable": [{"appId": "FoundDlc"}], "id": "d2"}));
        assert_eq!(dlc[2]["installable"], Value::Null);
        assert_eq!(doc["install"]["version"], "1.30");
        assert_eq!(doc["install"]["installed_dlc"], json!([{"app_name": "AweDlc", "title": "AWE", "install_size": 2}]));
        let man = &doc["manifest"];
        assert_eq!(man["download_size"], 1500);
        assert_eq!(man["disk_size"], 250);
        assert_eq!(man["prerequisites"]["ids"], json!(["vcredist"]));
        assert_eq!(man["install_tags"], json!(["", "de"]));
        assert_eq!(man["tag_disk_size"][1], json!({"tag": "de", "size": 30, "count": 1}));
        assert_eq!(man["use_signed_urls"], false);
    }

    #[test]
    fn missing_parts_are_null() {
        let g = game();
        let doc = info_json(&InfoInput {
            game: &g,
            platform: "Mac",
            entitlements: None,
            owned_apps: &HashSet::new(),
            installed: &BTreeMap::new(),
            manifest: None,
        });
        assert_eq!(doc["install"], Value::Null);
        assert_eq!(doc["manifest"], Value::Null);
        assert_eq!(doc["game"]["version"], Value::Null);
        assert_eq!(doc["game"]["owned_dlc"], json!([]));
    }
}
