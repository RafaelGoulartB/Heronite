//! Keeps the AreWeAntiCheatYet list (`areweanticheatyet.json`) current. The
//! releases document that already gives the known-fixes hash also carries
//! the list's MD5 (`anticheatFiles.shaLinux`); a different hash means a new
//! list to fetch. The focus panel's "On Linux" reads it
//! (`heronite_data::anticheat`).

use std::fs;

use heronite_data::DataPaths;
use serde_json::Value;

use crate::known_fixes::{file_md5, RELEASES_INFO_URL};
use crate::net::{Cancel, Net};

pub const ANTICHEAT_URL: &str = "https://raw.githubusercontent.com/Starz0r/AreWeAntiCheatYet/HEAD/games.json";

/// Downloads the latest list unless the local file already has the published
/// hash. `Ok(true)` when it was replaced.
pub fn update_if_needed(paths: &DataPaths, net: &dyn Net, cancel: Cancel) -> Result<bool, String> {
    if !net.is_online() {
        return Ok(false);
    }
    let release = net.get_json(RELEASES_INFO_URL)?;
    let latest = release.pointer("/anticheatFiles/shaLinux").and_then(Value::as_str).unwrap_or_default();
    let target = paths.anticheat();
    if !latest.is_empty() && file_md5(&target) == latest {
        log::debug!("AreWeAntiCheatYet data did not change. Skipping.");
        return Ok(false);
    }
    let part = target.with_extension("json.part");
    net.download(ANTICHEAT_URL, &part, cancel).map_err(|e| {
        let _ = fs::remove_file(&part);
        format!("Failed download of AreWeAntiCheatYet data: {e}")
    })?;
    // A broken download must not replace a good file.
    let valid = fs::read_to_string(&part)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    if !valid {
        let _ = fs::remove_file(&part);
        return Err("Failed download of AreWeAntiCheatYet data: the file is not a list of games".into());
    }
    fs::rename(&part, &target).map_err(|e| format!("installing areweanticheatyet.json: {e}"))?;
    log::info!("AreWeAntiCheatYet data downloaded");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::known_fixes::md5_hex;
    use crate::net::FakeNet;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        fs::create_dir_all(&paths.root).unwrap();
        (dir, paths)
    }

    const BODY: &str = r#"[{"name":"Elden Ring","status":"Supported","anticheats":["Easy Anti-Cheat"],"storeIds":{"steam":"1245620"}}]"#;

    #[test]
    fn the_list_is_downloaded_when_the_hash_differs() {
        let (_dir, paths) = fixture();
        fs::write(paths.anticheat(), "[]").unwrap();
        let net = FakeNet::online()
            .with_json(RELEASES_INFO_URL, json!({"anticheatFiles": {"shaLinux": md5_hex(BODY.as_bytes()), "shaMac": "x"}}))
            .with_file(ANTICHEAT_URL, BODY.as_bytes().to_vec());
        assert!(update_if_needed(&paths, &net, &|| false).unwrap());
        assert_eq!(fs::read_to_string(paths.anticheat()).unwrap(), BODY);
        // Same hash now: only the releases document is read.
        let again = FakeNet::online().with_json(RELEASES_INFO_URL, json!({"anticheatFiles": {"shaLinux": md5_hex(BODY.as_bytes())}}));
        assert!(!update_if_needed(&paths, &again, &|| false).unwrap());
        assert_eq!(again.requests().len(), 1);
    }

    #[test]
    fn offline_or_broken_downloads_keep_the_current_file() {
        let (_dir, paths) = fixture();
        fs::write(paths.anticheat(), BODY).unwrap();
        assert!(!update_if_needed(&paths, &FakeNet::offline(), &|| false).unwrap());
        for broken in [&b"<html>nope"[..], b"[]", b"{\"a\":1}"] {
            let net = FakeNet::online()
                .with_json(RELEASES_INFO_URL, json!({"anticheatFiles": {"shaLinux": "other"}}))
                .with_file(ANTICHEAT_URL, broken.to_vec());
            assert!(update_if_needed(&paths, &net, &|| false).is_err());
            assert_eq!(fs::read_to_string(paths.anticheat()).unwrap(), BODY);
            assert!(!paths.anticheat().with_extension("json.part").exists());
        }
    }
}
