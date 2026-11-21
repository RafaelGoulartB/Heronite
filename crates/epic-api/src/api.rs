//! Epic's launcher, catalog, library and download services (legendary's
//! `api/egs.py`), and fetching a build's manifest and chunks.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1_smol::Sha1;

use crate::auth::Session;
use crate::client::Client;
use crate::crypt::Secrets;
use crate::http::{host, ApiError, Body};
use crate::manifest::ChunkInfo;

/// A build the account may install (legendary's `GameAsset`, as
/// `assets.json` stores it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Asset {
    pub app_name: String,
    pub asset_id: String,
    pub build_version: String,
    pub catalog_item_id: String,
    pub label_name: String,
    pub namespace: String,
    pub metadata: Value,
    pub sidecar_rev: i64,
}

impl Asset {
    /// From the launcher service's answer (camel case).
    pub fn from_service(v: &Value) -> Self {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        Asset {
            app_name: text("appName"),
            asset_id: text("assetId"),
            build_version: text("buildVersion"),
            catalog_item_id: text("catalogItemId"),
            label_name: text("labelName"),
            namespace: text("namespace"),
            metadata: v.get("metadata").cloned().filter(|m| m.is_object()).unwrap_or_else(|| json!({})),
            sidecar_rev: v.get("sidecarRvn").and_then(Value::as_i64).unwrap_or(0),
        }
    }
}

/// Where a build's manifest is, and what its download needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestInfo {
    /// Manifest URLs, each with its query (CDN tokens), in Epic's order.
    pub urls: Vec<String>,
    /// The folders the chunk paths go below, in Epic's order.
    pub base_urls: Vec<String>,
    /// SHA-1 (hex) of the manifest file.
    pub hash: String,
    pub build_version: String,
    /// Chunks are fetched through signed URLs ([`Client::signed_chunk_urls`]).
    pub use_signed_url: bool,
    /// A preload: the build is out but not playable yet.
    pub is_preloaded: bool,
    /// Keys of an encrypted manifest and its chunks.
    pub secrets: Secrets,
    /// The sidecar (`{"config": <object>, "rev": n}`), when the build has one.
    pub sidecar: Option<Value>,
}

/// `secrets` of the manifest answer: an object of GUID → key, or a list of
/// such pairs.
fn parse_secrets(v: Option<&Value>) -> Secrets {
    let mut out = Secrets::new();
    match v {
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if let Some(key) = v.as_str() {
                    out.insert(k.to_ascii_uppercase(), key.to_string());
                }
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                let field = |names: &[&str]| names.iter().find_map(|n| item.get(*n).and_then(Value::as_str)).map(str::to_string);
                if let (Some(id), Some(key)) = (field(&["secretGuid", "guid", "id"]), field(&["key", "secret", "value"])) {
                    out.insert(id.replace('-', "").to_ascii_uppercase(), key);
                }
            }
        }
        _ => {}
    }
    out
}

impl ManifestInfo {
    /// Reads `elements[0]` of the assets v2 answer (legendary's `get_cdn_urls`).
    pub fn parse(doc: &Value) -> Result<Self> {
        let elements = doc.get("elements").and_then(Value::as_array).ok_or_else(|| anyhow!("the manifest answer has no elements"))?;
        if elements.len() > 1 {
            log::warn!("epic: the manifest answer has {} elements, using the first", elements.len());
        }
        let el = elements.first().ok_or_else(|| anyhow!("the manifest answer is empty"))?;
        let mut info = ManifestInfo {
            hash: el.get("hash").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase(),
            build_version: el.get("buildVersion").and_then(Value::as_str).unwrap_or_default().to_string(),
            use_signed_url: el.get("useSignedUrl").and_then(Value::as_bool).unwrap_or(false),
            is_preloaded: el.get("isPreloaded").and_then(Value::as_bool).unwrap_or(false),
            secrets: parse_secrets(el.get("secrets")),
            ..Default::default()
        };
        for m in el.get("manifests").and_then(Value::as_array).into_iter().flatten() {
            let Some(uri) = m.get("uri").and_then(Value::as_str) else { continue };
            let base = uri.rsplit_once('/').map_or(uri, |(b, _)| b).to_string();
            if !info.base_urls.contains(&base) {
                info.base_urls.push(base);
            }
            let params: Vec<String> = m
                .get("queryParams")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| Some(format!("{}={}", p.get("name")?.as_str()?, p.get("value")?.as_str()?)))
                .collect();
            info.urls.push(if params.is_empty() { uri.to_string() } else { format!("{uri}?{}", params.join("&")) });
        }
        if let Some(sidecar) = el.get("sidecar") {
            // The configuration is a JSON object written as a string.
            let config = sidecar.get("config").and_then(Value::as_str).and_then(|s| serde_json::from_str::<Value>(s).ok());
            if let Some(config) = config {
                info.sidecar = Some(json!({"config": config, "rev": sidecar.get("rvn").and_then(Value::as_i64).unwrap_or(0)}));
            }
        }
        Ok(info)
    }
}

/// A ticket for signed chunk URLs.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    pub signed_ticket: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Client {
    /// Every build of `platform` the account may install
    /// (`/launcher/api/public/assets/<platform>`).
    pub fn assets(&self, session: &Session, platform: &str, label: &str) -> Result<Vec<Asset>> {
        let url = format!("{}/launcher/api/public/assets/{platform}", self.endpoints.launcher);
        let doc = self.http.get_json(&self.get(&url).query(&[("label", label)]).bearer(session.access_token()))?;
        let list = doc.as_array().ok_or_else(|| anyhow!("the assets answer is not a list"))?;
        Ok(list.iter().map(Asset::from_service).collect())
    }

    /// The catalog entry of an item (legendary's `get_game_info`), with its
    /// DLCs and main game. `None` when the catalog has none.
    pub fn catalog_item(&self, session: &Session, namespace: &str, catalog_item_id: &str) -> Result<Option<Value>> {
        let url = format!("{}/catalog/api/shared/namespace/{namespace}/bulk/items", self.endpoints.catalog);
        let query = [
            ("id", catalog_item_id),
            ("includeDLCDetails", "True"),
            ("includeMainGameDetails", "True"),
            ("country", self.country.as_str()),
            ("locale", self.locale.as_str()),
        ];
        let doc = self.http.get_json(&self.get(&url).query(&query).bearer(session.access_token()))?;
        Ok(doc.get(catalog_item_id).cloned().filter(|v| !v.is_null()))
    }

    /// Every item of the account's library, page after page.
    pub fn library_items(&self, session: &Session) -> Result<Vec<Value>> {
        let url = format!("{}/library/api/public/items", self.endpoints.library);
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut query = vec![("includeMetadata", "true")];
            if let Some(c) = cursor.as_deref() {
                query.push(("cursor", c));
            }
            let doc = self.http.get_json(&self.get(&url).query(&query).bearer(session.access_token()))?;
            out.extend(doc.get("records").and_then(Value::as_array).cloned().unwrap_or_default());
            let next = doc.pointer("/responseMetadata/nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
            if next.is_none() || next == cursor {
                return Ok(out);
            }
            cursor = next;
        }
    }

    /// Every entitlement of the account (what it owns, DLCs without a build
    /// included), 1000 at a time.
    pub fn entitlements(&self, session: &Session) -> Result<Vec<Value>> {
        let url = format!("{}/entitlement/api/account/{}/entitlements", self.endpoints.entitlements, session.account_id());
        let mut out = Vec::new();
        loop {
            let start = out.len().to_string();
            let doc = self.http.get_json(&self.get(&url).query(&[("start", &start), ("count", "1000")]).bearer(session.access_token()))?;
            let page = doc.as_array().cloned().unwrap_or_default();
            let n = page.len();
            out.extend(page);
            if n < 1000 {
                return Ok(out);
            }
        }
    }

    /// Where the manifest of a build is (`/launcher/api/public/assets/v2/...`).
    /// `token` is an account's, or [`Client::anonymous_token`] for public builds.
    pub fn manifest_info(
        &self,
        token: &str,
        namespace: &str,
        catalog_item_id: &str,
        app_name: &str,
        platform: &str,
        label: &str,
    ) -> Result<ManifestInfo> {
        let url = format!(
            "{}/launcher/api/public/assets/v2/platform/{platform}/namespace/{namespace}/catalogItem/{catalog_item_id}/app/{app_name}/label/{label}",
            self.endpoints.launcher
        );
        ManifestInfo::parse(&self.http.get_json(&self.get(&url).bearer(token))?)
    }

    /// Downloads the manifest from the first URL that answers, and checks it
    /// against the hash Epic gave.
    pub fn download_manifest(&self, info: &ManifestInfo) -> Result<Vec<u8>> {
        if info.urls.is_empty() {
            bail!("Epic gave no manifest URL");
        }
        let mut last = String::new();
        for url in &info.urls {
            match self.http.send(&self.get(url)) {
                Ok(resp) if resp.ok() => {
                    let hash = Sha1::from(&resp.body).digest().to_string();
                    if !info.hash.is_empty() && hash != info.hash {
                        bail!("the manifest from {} is damaged (hash mismatch)", host(url));
                    }
                    return Ok(resp.body);
                }
                Ok(resp) => last = format!("{}: HTTP {}", host(url), resp.status),
                Err(e) => last = format!("{e:#}"),
            }
            log::warn!("epic: manifest download failed ({last}), trying the next CDN");
        }
        bail!("Could not download the manifest from any CDN: {last}")
    }

    /// The optimized delta manifest between two builds, when the CDN has one
    /// (most games have none).
    pub fn delta_manifest(&self, base_url: &str, old_build_id: &str, new_build_id: &str) -> Option<Vec<u8>> {
        if old_build_id == new_build_id {
            return None;
        }
        let url = format!("{base_url}/Deltas/{new_build_id}/{old_build_id}.delta");
        self.http.send(&self.get(&url)).ok().filter(|r| r.status == 200 && !r.body.is_empty()).map(|r| r.body)
    }

    /// A ticket for the signed chunk URLs of `asset`'s build.
    pub fn download_ticket(&self, session: &Session, asset: &Asset) -> Result<Ticket> {
        let url = format!("{}/launcher/api/public/assets/v2/ticket", self.endpoints.launcher_tickets);
        // `labelName` is `<label>-<platform>`, as `Live-Windows`.
        let (label, platform) = asset.label_name.split_once('-').unwrap_or((asset.label_name.as_str(), "Windows"));
        let body = json!({
            "catalogItemId": asset.catalog_item_id, "buildVersion": asset.build_version, "appName": asset.app_name,
            "namespace": asset.namespace, "label": label, "platform": platform,
        });
        let doc = self.http.json(&self.post(&url, Body::Json(&body)).bearer(session.access_token()))?;
        let signed_ticket =
            doc.get("signedTicket").and_then(Value::as_str).ok_or_else(|| anyhow!("Epic gave no download ticket"))?.to_string();
        let expires_at = doc
            .get("expiresAt")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        Ok(Ticket { signed_ticket, expires_at })
    }

    /// Signed URLs of `paths` (chunk paths), at most ~50 at a time.
    pub fn signed_chunk_urls(&self, session: &Session, ticket: &Ticket, paths: &[String]) -> Result<HashMap<String, String>> {
        let url = format!(
            "{}/artifact-delivery/api/public/v1/delivery/account/{}/downloadurls",
            self.endpoints.artifact_delivery,
            session.account_id()
        );
        let body = json!({"signedTicket": ticket.signed_ticket, "chunkIds": paths, "deltaId": "", "clientMetrics": {}});
        let doc = self.http.json(&self.post(&url, Body::Json(&body)).bearer(session.access_token()))?;
        // Path → URL, at the top or below a key.
        let map =
            doc.as_object().map(|m| if m.values().all(Value::is_string) { m.clone() } else { Default::default() }).unwrap_or_default();
        let map = if map.is_empty() {
            doc.as_object().and_then(|m| m.values().find_map(Value::as_object)).cloned().unwrap_or_default()
        } else {
            map
        };
        Ok(map.into_iter().filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string()))).collect())
    }

    /// Downloads and decodes the chunk `info` (at `path` below a base URL),
    /// trying each base URL in turn.
    pub fn fetch_chunk(&self, base_urls: &[String], path: &str, info: &ChunkInfo, secrets: &Secrets) -> Result<Vec<u8>> {
        let mut last = None;
        for base in base_urls {
            let url = format!("{}/{path}", base.trim_end_matches('/'));
            match self.http.bytes(&url).and_then(|raw| crate::chunk::decode_for(&raw, info, secrets)) {
                Ok(data) => return Ok(data),
                Err(e) => {
                    log::debug!("epic: chunk from {}: {e:#}", host(base));
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no CDN to download from")))
            .with_context(|| format!("downloading the chunk {}", info.guid.text()))
    }

    /// Downloads and decodes a chunk from a signed URL.
    pub fn fetch_signed_chunk(&self, url: &str, info: &ChunkInfo, secrets: &Secrets) -> Result<Vec<u8>> {
        let raw = self.http.bytes(url)?;
        crate::chunk::decode_for(&raw, info, secrets)
    }

    /// Epic's own answer to a request that must succeed (for the callers'
    /// one-off requests).
    pub fn check(resp: crate::http::Response) -> Result<crate::http::Response> {
        if resp.ok() {
            Ok(resp)
        } else {
            Err(ApiError::from_response(&resp).into())
        }
    }
}

/// `url` over plain HTTP (the launcher's own default, which a local CDN cache
/// can serve; `downloadNoHttps`).
pub fn without_https(url: &str) -> String {
    url.strip_prefix("https://").map_or_else(|| url.to_string(), |rest| format!("http://{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_answers_give_urls_bases_and_secrets() {
        let doc = json!({"elements": [{
            "appName": "Calluna", "labelName": "Live-Windows", "buildVersion": "1.33", "hash": "ABCDEF",
            "useSignedUrl": false,
            "manifests": [
                {"uri": "https://cdn1/Builds/x/Calluna/default/m.manifest", "queryParams": [{"name": "f_token", "value": "t"}, {"name": "x", "value": "1"}]},
                {"uri": "https://cdn2/Builds/x/Calluna/default/m.manifest"},
                {"uri": "https://cdn1/Builds/x/Calluna/default/m.manifest"}
            ],
            "secrets": {"0000000a0000000b0000000c0000000d": "00ff"},
            "sidecar": {"config": "{\"deploymentId\": \"dep\"}", "rvn": 3}
        }]});
        let info = ManifestInfo::parse(&doc).unwrap();
        assert_eq!(info.urls[0], "https://cdn1/Builds/x/Calluna/default/m.manifest?f_token=t&x=1");
        assert_eq!(info.urls.len(), 3);
        assert_eq!(info.base_urls, ["https://cdn1/Builds/x/Calluna/default", "https://cdn2/Builds/x/Calluna/default"]);
        assert_eq!(info.hash, "abcdef");
        assert_eq!(info.secrets["0000000A0000000B0000000C0000000D"], "00ff");
        assert_eq!(info.sidecar, Some(json!({"config": {"deploymentId": "dep"}, "rev": 3})));
        assert!(ManifestInfo::parse(&json!({"elements": []})).is_err());
        assert_eq!(parse_secrets(Some(&json!([{"secretGuid": "aa-bb", "key": "k"}])))["AABB"], "k");
    }

    #[test]
    fn assets_from_the_service() {
        let a = Asset::from_service(
            &json!({"appName": "A", "labelName": "Live-Windows", "buildVersion": "1", "catalogItemId": "c", "namespace": "n", "assetId": "A", "sidecarRvn": 2}),
        );
        assert_eq!((a.app_name.as_str(), a.sidecar_rev, a.metadata.clone()), ("A", 2, json!({})));
        let stored = serde_json::to_value(&a).unwrap();
        assert_eq!(stored["catalog_item_id"], "c");
        assert_eq!(serde_json::from_value::<Asset>(stored).unwrap(), a);
        assert_eq!(without_https("https://a/b"), "http://a/b");
    }
}
