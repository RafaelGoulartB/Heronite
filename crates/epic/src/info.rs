//! Install information (`legendary info --json`), selective downloads (SDL)
//! and the per-game overrides published by the legendary API.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache_store::CacheStore;
use crate::cmd::Legendary;

/// A DLC the account owns for a game (`game.owned_dlc`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dlc {
    pub app_name: String,
    pub title: String,
}

/// Parsed view over the output of `legendary info <app> --json`.
#[derive(Debug, Clone)]
pub struct InstallInfo(pub Value);

impl InstallInfo {
    fn manifest_num(&self, key: &str) -> f64 {
        self.0.pointer(&format!("/manifest/{key}")).and_then(Value::as_f64).unwrap_or(0.)
    }

    /// Bytes to download for a fresh install.
    pub fn download_size(&self) -> f64 {
        self.manifest_num("download_size")
    }

    /// Bytes on disk once installed.
    pub fn disk_size(&self) -> f64 {
        self.manifest_num("disk_size")
    }

    pub fn owned_dlc(&self) -> Vec<Dlc> {
        self.0
            .pointer("/game/owned_dlc")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|d| {
                        Some(Dlc {
                            app_name: d.get("app_name")?.as_str()?.to_string(),
                            title: d.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Bytes to download / store for one install tag (SDL component).
    pub fn tag_sizes(&self, tag: &str) -> (f64, f64) {
        let find = |key: &str| {
            self.0
                .pointer(&format!("/manifest/{key}"))
                .and_then(Value::as_array)
                .and_then(|l| l.iter().find(|t| t.get("tag").and_then(Value::as_str) == Some(tag)))
                .and_then(|t| t.get("size").and_then(Value::as_f64))
                .unwrap_or(0.)
        };
        (find("tag_download_size"), find("tag_disk_size"))
    }

    /// `manifest.prerequisites`: the program (relative to the install folder)
    /// and its arguments the game wants run before it starts.
    pub fn prerequisites(&self) -> Option<(String, String)> {
        let pre = self.0.pointer("/manifest/prerequisites")?;
        let path = pre.get("path").and_then(Value::as_str).filter(|p| !p.is_empty())?;
        Some((path.to_string(), pre.get("args").and_then(Value::as_str).unwrap_or_default().to_string()))
    }

    /// `game.cloud_saves_supported`.
    pub fn cloud_saves_supported(&self) -> bool {
        self.0.pointer("/game/cloud_saves_supported").and_then(Value::as_bool).unwrap_or(false)
    }
}

/// `getInstallInfo`: the cached info (6 hours) or `legendary info`, retried a
/// few times when the manifest is missing.
pub fn install_info(data_paths: &DataPaths, legendary: &Legendary, app_name: &str, platform: &str) -> Result<InstallInfo> {
    let cache = CacheStore::install_info(data_paths);
    if let Some(cached) = cache.get(app_name).filter(|c| c.get("manifest").is_some_and(|m| !m.is_null())) {
        log::debug!("Using cached install info for {app_name}");
        return Ok(InstallInfo(cached));
    }
    let mut last = String::new();
    for attempt in 1..=3 {
        let out = legendary.run(&["info", app_name, "--json", "--platform", platform], &[])?;
        match parse_info_output(&out.stdout) {
            Some(info) if info.get("manifest").is_some_and(|m| !m.is_null()) => {
                cache.set(app_name, info.clone())?;
                return Ok(InstallInfo(info));
            }
            _ => {
                last = out.error_lines();
                log::warn!("Install info for {app_name} does not include manifest data (attempt {attempt})");
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(700));
                }
            }
        }
    }
    if last.is_empty() {
        bail!(tr!("Failed to get install info for {app_name}", app_name))
    } else {
        bail!(tr!("Failed to get install info for {app_name}: {error}", app_name, error = last))
    }
}

/// The JSON object in `legendary info --json`'s stdout (tolerates log lines
/// before it).
pub fn parse_info_output(stdout: &str) -> Option<Value> {
    let start = stdout.find('{')?;
    serde_json::from_str(stdout[start..].trim()).ok()
}

/// Component the user can pick (selective download).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sdl {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub required: bool,
}

/// Port of `getGameSdl`'s response handling: the `__required` entry goes first.
pub fn parse_sdl(doc: &Value) -> Vec<Sdl> {
    let Some(map) = doc.as_object() else { return Vec::new() };
    let mut out = Vec::new();
    for (key, value) in map {
        let entry = Sdl {
            name: value.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            description: value.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            tags: value
                .get("tags")
                .and_then(Value::as_array)
                .map(|t| t.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            required: key == "__required",
        };
        if entry.required {
            out.insert(0, entry);
        } else {
            out.push(entry);
        }
    }
    out
}

const API: &str = "https://heroic.legendary.gl/v1";

fn get_text(url: &str) -> Result<(String, bool)> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into();
    let mut resp = agent.get(url).call().map_err(|e| anyhow!("{url}: {e}"))?;
    let is_json = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/json"));
    if !resp.status().is_success() {
        bail!("{url}: HTTP {}", resp.status());
    }
    Ok((resp.body_mut().read_to_string().map_err(|e| anyhow!("{url}: {e}"))?, is_json))
}

/// `getGameOverride`: `game_overrides` of the legendary API, cached 7 days.
pub fn game_override(data_paths: &DataPaths) -> Result<Value> {
    let cache = CacheStore::games_override(data_paths);
    if let Some(v) = cache.get("gamesOverride") {
        return Ok(v);
    }
    let (body, _) = get_text(&format!("{API}/version.json"))?;
    let doc: Value = serde_json::from_str(&body)?;
    let over = doc.get("game_overrides").cloned().unwrap_or(Value::Null);
    if !over.is_null() {
        cache.set("gamesOverride", over.clone())?;
    }
    Ok(over)
}

/// SDL components of a game, when the API's `sdl_config` lists it. Failures
/// give an empty list (the install then just uses the defaults).
pub fn game_sdl(data_paths: &DataPaths, app_name: &str) -> Vec<Sdl> {
    let has_sdl = game_override(data_paths)
        .ok()
        .and_then(|o| o.get("sdl_config").and_then(|c| c.get(app_name)).cloned())
        .is_some_and(|v| !v.is_null());
    if !has_sdl {
        return Vec::new();
    }
    match get_text(&format!("{API}/sdl/{app_name}.json")) {
        Ok((body, true)) => serde_json::from_str(&body).map(|d| parse_sdl(&d)).unwrap_or_default(),
        Ok((_, false)) => Vec::new(),
        Err(err) => {
            log::warn!("Error fetching Selective Download data for {app_name}: {err:#}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::cmd::testing::{calls, fake};
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "game": {"app_name": "Calluna", "cloud_saves_supported": true,
                "owned_dlc": [{"app_name": "dlc1", "title": "Expansion"}, {"title": "no app"}]},
            "manifest": {"download_size": 27783660544u64, "disk_size": 45010292103u64,
                "tag_download_size": [{"tag": "voices_de", "count": 3, "size": 100}],
                "tag_disk_size": [{"tag": "voices_de", "count": 3, "size": 150}]}
        })
    }

    #[test]
    fn info_accessors() {
        let i = InstallInfo(sample());
        assert_eq!(i.download_size(), 27783660544.);
        assert_eq!(i.disk_size(), 45010292103.);
        assert_eq!(i.owned_dlc(), [Dlc { app_name: "dlc1".into(), title: "Expansion".into() }]);
        assert_eq!(i.tag_sizes("voices_de"), (100., 150.));
        assert_eq!(i.tag_sizes("nope"), (0., 0.));
        assert!(i.cloud_saves_supported());
        assert_eq!(i.prerequisites(), None);
        let with = InstallInfo(json!({"manifest": {"prerequisites": {
            "ids": ["x"], "name": "", "path": "Engine\\Redist\\setup.exe", "args": "/silent"}}}));
        assert_eq!(with.prerequisites(), Some(("Engine\\Redist\\setup.exe".into(), "/silent".into())));
        let empty = InstallInfo(json!({"manifest": {"prerequisites": {"path": "", "args": ""}}}));
        assert_eq!(empty.prerequisites(), None);
        assert_eq!(InstallInfo(json!({"manifest": {"prerequisites": null}})).prerequisites(), None);
    }

    #[test]
    fn output_with_leading_log_lines_parses() {
        let v = parse_info_output("[cli] INFO: x\n{\"manifest\": {}}").unwrap();
        assert!(v.get("manifest").is_some());
        assert!(parse_info_output("no json").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn info_is_fetched_once_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let body = format!("echo '{}'", sample());
        let lg = fake(dir.path(), &body);
        let a = install_info(&data_paths, &lg, "Calluna", "Windows").unwrap();
        assert_eq!(a.download_size(), 27783660544.);
        let b = install_info(&data_paths, &lg, "Calluna", "Windows").unwrap();
        assert_eq!(b.disk_size(), 45010292103.);
        assert_eq!(calls(dir.path()), ["info Calluna --json --platform Windows"]);
    }

    #[cfg(unix)]
    #[test]
    fn missing_manifest_is_an_error_after_retries() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let lg = fake(dir.path(), "echo '{\"game\": {}}'; echo 'ERROR: not owned' >&2");
        let err = install_info(&data_paths, &lg, "x", "Windows").unwrap_err().to_string();
        assert!(err.contains("not owned"), "{err}");
        assert_eq!(calls(dir.path()).len(), 3);
    }

    #[test]
    fn sdl_required_comes_first() {
        let doc = json!({
            "pack": {"name": "Voices", "description": "d", "tags": ["voices"]},
            "__required": {"name": "Base", "description": "", "tags": ["", "base"]}
        });
        let sdl = parse_sdl(&doc);
        assert_eq!(sdl[0].name, "Base");
        assert!(sdl[0].required && !sdl[1].required);
        assert_eq!(sdl[1].tags, ["voices"]);
    }
}
