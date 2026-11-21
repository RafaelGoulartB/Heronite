//! What legendary's own API publishes for every client
//! (`api.legendary.gl/v1/version.json`, cached in `version.json`): the
//! launcher version to present as, and per-game workarounds such as another
//! program to start. Fetched at most once a day.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::client::Client;
use crate::files;
use crate::http::Request;

const URL: &str = "https://api.legendary.gl/v1/version.json";
const MAX_AGE_SECS: f64 = 24.0 * 3600.0;

/// Programs legendary starts instead of the manifest's, by app name and
/// system (`linux`, `win32`, `darwin`), until the API says otherwise.
const EXE_OVERRIDES: &[(&str, &str, &str)] = &[
    ("kinglet", "linux", "Base/Binaries/Win64EOS/CivilizationVI.exe"),
    ("kinglet", "darwin", "Base/Binaries/Win64EOS/CivilizationVI.exe"),
    ("kinglet", "win32", "LaunchPad/LaunchPad.exe"),
];

pub fn cache_file(config_dir: &Path) -> PathBuf {
    config_dir.join("version.json")
}

/// The cached answer (`data` of `version.json`).
pub fn cached(config_dir: &Path) -> Option<Value> {
    files::read_json(&cache_file(config_dir))?.get("data").cloned().filter(|d| d.is_object())
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Fetches the API's answer when the cached one is older than a day, and
/// stores it like legendary (`{"data": .., "last_update": <unix time>}`).
/// Failures keep the old answer.
pub fn refresh(client: &Client, config_dir: &Path) -> Option<Value> {
    let path = cache_file(config_dir);
    let stored = files::read_json(&path);
    let fresh = stored.as_ref().and_then(|d| d.get("last_update").and_then(Value::as_f64)).is_some_and(|t| now() - t < MAX_AGE_SECS);
    if fresh {
        return cached(config_dir);
    }
    let req = Request { user_agent: "Legendary/0.20.34 (Linux)", ..client.get(URL) };
    match client.http.get_json(&req) {
        Ok(data) if data.is_object() => {
            if let Err(e) = files::write_json(&path, &json!({"data": data, "last_update": now()}), false) {
                log::warn!("epic: saving legendary's configuration: {e:#}");
            }
            Some(data)
        }
        Ok(_) => cached(config_dir),
        Err(e) => {
            log::debug!("epic: legendary's configuration: {e:#}");
            cached(config_dir)
        }
    }
}

/// Presents `client` as the launcher version the API names, with its OAuth
/// client when it names one (legendary's `update_egs_params`).
pub fn apply(client: &mut Client, data: &Value) {
    let Some(egl) = data.get("egl_config") else { return };
    if let Some(version) = egl.get("version").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        client.user_agent = format!("UELauncher/{version} Windows/10.0.19041.1.256.64bit");
    }
    if let (Some(id), Some(secret)) = (egl.get("client_id").and_then(Value::as_str), egl.get("client_secret").and_then(Value::as_str)) {
        client.client_id = id.to_string();
        client.client_secret = secret.to_string();
    }
}

/// A client set up like legendary's for `config_dir`: the cached API answer
/// applied (no network).
pub fn client_for(config_dir: &Path) -> Client {
    let mut client = Client::default();
    if let Some(data) = cached(config_dir) {
        apply(&mut client, &data);
    }
    client
}

/// legendary's name of this system in its overrides.
fn system() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// The program to start instead of the manifest's for `app_name` on this
/// system, from the API's `executable_override` (else the built-in list).
pub fn exe_override(data: Option<&Value>, app_name: &str) -> Option<String> {
    let key = app_name.to_lowercase();
    if let Some(table) = data.and_then(|d| d.pointer("/game_overrides/executable_override")).and_then(Value::as_object) {
        return table.get(&key).and_then(|v| v.get(system())).and_then(Value::as_str).map(str::to_string);
    }
    EXE_OVERRIDES.iter().find(|(app, sys, _)| *app == key && *sys == system()).map(|(_, _, exe)| exe.to_string())
}

/// Shorthand of [`exe_override`] with the cached answer.
pub fn cached_exe_override(config_dir: &Path, app_name: &str) -> Option<String> {
    exe_override(cached(config_dir).as_ref(), app_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_answer_sets_the_user_agent_and_overrides() {
        let data = json!({
            "egl_config": {"version": "15.18.2-29993784+++Portal+Release-Live", "client_id": "id", "client_secret": "s"},
            "game_overrides": {"executable_override": {"xcom": {"linux": "Bin/X.exe", "win32": "Bin/X.exe", "darwin": "Bin/X.exe"}}}
        });
        let mut client = Client::default();
        apply(&mut client, &data);
        assert_eq!(client.user_agent, "UELauncher/15.18.2-29993784+++Portal+Release-Live Windows/10.0.19041.1.256.64bit");
        assert_eq!(client.client_id, "id");
        assert_eq!(exe_override(Some(&data), "XCOM").as_deref(), Some("Bin/X.exe"));
        assert_eq!(exe_override(Some(&data), "Kinglet"), None, "the API's list replaces the built-in one");
        assert!(exe_override(None, "Kinglet").is_some());
    }

    #[test]
    fn a_fresh_cache_is_used_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        files::write_json(&cache_file(dir.path()), &json!({"data": {"egl_config": {"version": "9"}}, "last_update": now()}), false)
            .unwrap();
        let client = Client::new(crate::client::Endpoints::all_at("http://127.0.0.1:9"));
        assert_eq!(refresh(&client, dir.path()).unwrap()["egl_config"]["version"], "9");
        assert!(client_for(dir.path()).user_agent.starts_with("UELauncher/9 "));
    }
}
